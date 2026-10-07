//! Object model operations: properties, conversions, operators, promises, event loop.
use super::{ast::BinOp, interp::*, value::*};
use alloc::{
    boxed::Box,
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

const MAX_ARRAY_LEN: usize = 50_000_000;

impl Interp {
    // -----------------------------------------------------------------------
    // Construction helpers
    // -----------------------------------------------------------------------

    pub fn new_object(&self) -> Obj {
        self.alloc(Object::new(Some(self.protos().object.clone()), Class::Plain))
    }

    pub fn new_array(&self, v: Vec<Value>) -> Value {
        Value::Obj(self.alloc(Object::new(Some(self.protos().array.clone()), Class::Array(v))))
    }

    pub fn new_native(&mut self, name: &str, len: u32, f: NativeFn) -> Obj {
        self.new_native_data(name, len, f, Value::Undefined, false)
    }

    pub fn new_native_data(&mut self, name: &str, len: u32, f: NativeFn, data: Value, ctor: bool) -> Obj {
        let o = self.alloc(Object::new(Some(self.protos().function.clone()), Class::Function(Func::Native { f, data, ctor })));
        self.define_data(&o, self.names.name.clone(), Value::str(name), CONFIGURABLE);
        self.define_data(&o, self.names.length.clone(), Value::Num(len as f64), CONFIGURABLE);
        o
    }

    pub fn native_data(&self, call: &Call) -> Value {
        match &call.callee.borrow().class {
            Class::Function(Func::Native { data, .. }) => data.clone(),
            _ => Value::Undefined,
        }
    }

    pub fn def_method(&mut self, o: &Obj, name: &str, len: u32, f: NativeFn) {
        let fo = self.new_native(name, len, f);
        self.define_data(o, Key::str(name), Value::Obj(fo), HIDDEN);
    }

    pub fn def_sym_method(&mut self, o: &Obj, sym: &Sym, name: &str, len: u32, f: NativeFn) {
        let fo = self.new_native(name, len, f);
        self.define_data(o, Key::Y(sym.0.id), Value::Obj(fo), HIDDEN);
    }

    pub fn def_getter(&mut self, o: &Obj, name: &str, f: NativeFn) {
        let g = self.new_native(&format!("get {name}"), 0, f);
        self.define_accessor_part(o, Key::str(name), Some(g), true, CONFIGURABLE);
    }

    pub fn def_accessor(&mut self, o: &Obj, name: &str, g: NativeFn, s: NativeFn) {
        let go = self.new_native(&format!("get {name}"), 0, g);
        let so = self.new_native(&format!("set {name}"), 1, s);
        self.define_accessor_part(o, Key::str(name), Some(go), true, CONFIGURABLE);
        self.define_accessor_part(o, Key::str(name), Some(so), false, CONFIGURABLE);
    }

    pub fn def_const(&mut self, o: &Obj, name: &str, v: Value) {
        self.define_data(o, Key::str(name), v, 0);
    }

    pub fn make_error(&mut self, kind: &str, msg: &str) -> Value {
        let proto = self.protos().error_kinds.get(kind).cloned().unwrap_or_else(|| self.protos().error.clone());
        let o = self.alloc(Object::new(Some(proto), Class::Error));
        self.define_data(&o, self.names.message.clone(), Value::str(msg), HIDDEN);
        let stack = format!("{kind}: {msg}\n    at <anonymous>");
        self.define_data(&o, self.names.stack.clone(), Value::string(stack), HIDDEN);
        Value::Obj(o)
    }

    pub fn error_ctl(&mut self, kind: &str, msg: &str) -> Ctl {
        Ctl::Throw(self.make_error(kind, msg))
    }

    pub fn type_error<T>(&mut self, msg: &str) -> Res<T> {
        Err(self.error_ctl("TypeError", msg))
    }

    /// Short description for error messages.
    pub fn describe(&self, v: &Value) -> String {
        match v {
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => num_to_string(*n),
            Value::Str(s) => format!("\"{s}\""),
            Value::Sym(_) => "Symbol()".to_string(),
            Value::BigInt(b) => alloc::format!("{}n", b.to_string_radix(10)),
            Value::Obj(o) => {
                if o.is_callable() {
                    "function".to_string()
                } else if o.is_array() {
                    "array".to_string()
                } else {
                    "object".to_string()
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Own properties
    // -----------------------------------------------------------------------

    pub fn own_prop(&self, o: &Obj, key: &Key) -> Option<Prop> {
        let ob = o.borrow();
        match &ob.class {
            Class::Array(a) => {
                if let Some(i) = key.as_index() {
                    return a.get(i).filter(|v| !is_hole(v)).map(|v| Prop { slot: Slot::Data(v.clone()), flags: ALL });
                }
                if matches!(key, Key::S(s) if &**s == "length") {
                    return Some(Prop { slot: Slot::Data(Value::Num(a.len() as f64)), flags: WRITABLE });
                }
            }
            Class::Typed(v) => {
                if let Some(i) = key.as_index() {
                    return v.get(i).map(|n| Prop { slot: Slot::Data(Value::Num(n)), flags: WRITABLE | ENUMERABLE | CONFIGURABLE });
                }
            }
            Class::Str(s) => {
                if let Some(i) = key.as_index() {
                    return u16_at(s, i).map(|u| Prop { slot: Slot::Data(Value::string(from_u16(&[u]))), flags: ENUMERABLE });
                }
                if matches!(key, Key::S(k) if &**k == "length") {
                    return Some(Prop { slot: Slot::Data(Value::Num(u16_len(s) as f64)), flags: 0 });
                }
            }
            _ => {}
        }
        ob.props.get(key).cloned()
    }

    /// Own property lookup that honours Proxy `getOwnPropertyDescriptor` traps.
    pub fn own_prop_mut(&mut self, o: &Obj, key: &Key) -> Res<Option<Prop>> {
        let proxy = match &o.borrow().class {
            Class::Proxy { target, handler } => Some((target.clone(), handler.clone())),
            _ => None,
        };
        let Some((target, handler)) = proxy else { return Ok(self.own_prop(o, key)) };
        let trap = self.get_obj(&handler, &Key::str("getOwnPropertyDescriptor"), &Value::Obj(handler.clone()))?;
        if trap.is_undefined() {
            return self.own_prop_mut(&target, key);
        }
        let kv = self.key_to_value(key);
        let d = self.call(&trap, Value::Obj(handler), &[Value::Obj(target), kv])?;
        let Value::Obj(d) = d else { return Ok(None) };
        let dv = Value::Obj(d.clone());
        let flag = |it: &mut Interp, n: &str, bit: u8| -> Res<u8> { Ok(if it.get_obj(&d, &Key::str(n), &dv)?.truthy() { bit } else { 0 }) };
        let flags = flag(self, "enumerable", ENUMERABLE)? | flag(self, "configurable", CONFIGURABLE)? | flag(self, "writable", WRITABLE)?;
        let value = self.get_obj(&d, &Key::str("value"), &dv)?;
        Ok(Some(Prop { slot: Slot::Data(value), flags }))
    }

    pub fn has_own(&self, o: &Obj, key: &Key) -> bool {
        self.own_prop(o, key).is_some()
    }

    /// Define or overwrite a data property (no checks).
    pub fn define_data(&self, o: &Obj, key: Key, v: Value, flags: u8) {
        let mut ob = o.borrow_mut();
        if let Class::Array(a) = &mut ob.class {
            if let Some(i) = key.as_index() {
                if i < a.len() {
                    a[i] = v;
                    return;
                }
                if i < MAX_ARRAY_LEN && i <= a.len() + 10_000_000 {
                    a.resize(i, hole());
                    a.push(v);
                    return;
                }
            }
            if matches!(&key, Key::S(s) if &**s == "length") {
                if let Value::Num(n) = v {
                    let n = (n as usize).min(MAX_ARRAY_LEN);
                    a.resize(n, hole());
                }
                return;
            }
        }
        ob.props.insert(key, Prop { slot: Slot::Data(v), flags });
    }

    pub fn define_accessor_part(&self, o: &Obj, key: Key, f: Option<Obj>, is_get: bool, flags: u8) {
        let mut ob = o.borrow_mut();
        let (mut g, mut s) = match ob.props.get(&key) {
            Some(Prop { slot: Slot::Accessor(g, s), .. }) => (g.clone(), s.clone()),
            _ => (None, None),
        };
        if is_get {
            g = f;
        } else {
            s = f;
        }
        ob.props.insert(key, Prop { slot: Slot::Accessor(g, s), flags: flags & !WRITABLE });
    }

    pub fn own_keys(&mut self, o: &Obj) -> Res<Vec<Key>> {
        if let Class::Proxy { target, handler } = &o.borrow().class {
            let (t, h) = (target.clone(), handler.clone());
            let trap = self.get_obj(&h, &Key::str("ownKeys"), &Value::Obj(h.clone()))?;
            if trap.is_undefined() {
                return self.own_keys(&t);
            }
            let r = self.call(&trap, Value::Obj(h), &[Value::Obj(t)])?;
            let items = self.iter_to_vec(&r)?;
            let mut out = Vec::new();
            for i in items {
                out.push(self.to_key(&i)?);
            }
            return Ok(out);
        }
        let ob = o.borrow();
        let mut out = Vec::new();
        match &ob.class {
            Class::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    if !is_hole(v) {
                        out.push(Key::S(Rc::from(i.to_string())));
                    }
                }
                out.push(Key::str("length"));
            }
            Class::Str(s) => {
                for i in 0..u16_len(s) {
                    out.push(Key::S(Rc::from(i.to_string())));
                }
                out.push(Key::str("length"));
            }
            Class::Typed(v) => {
                for i in 0..v.len {
                    out.push(Key::S(Rc::from(i.to_string())));
                }
            }
            _ => {}
        }
        out.extend(ob.props.keys());
        Ok(out)
    }

    pub fn delete_prop(&mut self, o: &Obj, key: &Key) -> Res<bool> {
        if let Class::Proxy { target, handler } = &o.borrow().class {
            let (t, h) = (target.clone(), handler.clone());
            let trap = self.get_obj(&h, &Key::str("deleteProperty"), &Value::Obj(h.clone()))?;
            if trap.is_undefined() {
                return self.delete_prop(&t, key);
            }
            let kv = self.key_to_value(key);
            return Ok(self.call(&trap, Value::Obj(h), &[Value::Obj(t), kv])?.truthy());
        }
        let mut ob = o.borrow_mut();
        if let Class::Array(a) = &mut ob.class {
            if let Some(i) = key.as_index() {
                if i < a.len() {
                    a[i] = hole();
                }
                return Ok(true);
            }
        }
        if let Some(p) = ob.props.get(key) {
            if p.flags & CONFIGURABLE == 0 {
                return Ok(false);
            }
        }
        ob.props.remove(key);
        Ok(true)
    }

    pub fn delete_v(&mut self, v: &Value, key: &Key) -> Res {
        match v {
            Value::Obj(o) => Ok(Value::Bool(self.delete_prop(o, key)?)),
            Value::Undefined | Value::Null => Err(self.error_ctl("TypeError", "Cannot convert undefined or null to object")),
            _ => Ok(Value::Bool(true)),
        }
    }

    pub fn key_to_value(&self, k: &Key) -> Value {
        match k {
            Key::S(s) => Value::Str(s.clone()),
            Key::Y(id) => self.symbol_by_id(*id).map_or(Value::Undefined, Value::Sym),
        }
    }

    pub fn symbol_by_id(&self, id: u32) -> Option<Sym> {
        let wk = self.wk();
        for s in [&wk.iterator, &wk.async_iterator, &wk.to_primitive, &wk.has_instance, &wk.to_string_tag, &wk.species] {
            if s.0.id == id {
                return Some(s.clone());
            }
        }
        self.symbol_registry.values().find(|s| s.0.id == id).cloned().or_else(|| self.dom_symbols.get(&id).cloned())
    }

    pub fn new_symbol(&mut self, desc: Option<JsStr>) -> Sym {
        self.sym_counter += 1;
        let s = Sym(Rc::new(SymData { id: self.sym_counter, desc }));
        self.dom_symbols.insert(s.0.id, s.clone());
        s
    }

    // -----------------------------------------------------------------------
    // Property access
    // -----------------------------------------------------------------------

    fn materialize(&mut self, o: &Obj, key: &Key) {
        let Key::S(k) = key else { return };
        if !matches!(&**k, "prototype" | "name" | "length") {
            return;
        }
        let info = match &o.borrow().class {
            Class::Function(Func::Closure(c)) => Some((c.def.clone(), c.is_class_ctor)),
            _ => None,
        };
        let Some((def, _)) = info else { return };
        if o.borrow().props.get(key).is_some() {
            return;
        }
        match &**k {
            "prototype" => {
                if def.is_arrow || def.is_async && !def.is_generator || matches!(def.kind, super::ast::FnKind::Method | super::ast::FnKind::Getter | super::ast::FnKind::Setter) {
                    return;
                }
                let proto_proto = match (def.is_async, def.is_generator) {
                    (true, true) => self.protos().extra.get("async_generator").cloned().unwrap_or_else(|| self.protos().generator.clone()),
                    (false, true) => self.protos().generator.clone(),
                    _ => self.protos().object.clone(),
                };
                let p = self.alloc(Object::new(Some(proto_proto), Class::Plain));
                if !def.is_generator {
                    self.define_data(&p, self.names.constructor.clone(), Value::Obj(o.clone()), HIDDEN);
                }
                self.define_data(o, key.clone(), Value::Obj(p), WRITABLE);
            }
            "name" => {
                let n = def.name.map(|n| self.interner.get(n).clone()).unwrap_or_else(|| Rc::from(""));
                self.define_data(o, key.clone(), Value::Str(n), CONFIGURABLE);
            }
            _ => {
                self.define_data(o, key.clone(), Value::Num(def.length as f64), CONFIGURABLE);
            }
        }
    }

    /// An accessor held by a shared DOM prototype reached from a receiver that is not a node.
    fn dom_accessor_on_non_node(&self, holder: &Obj, receiver: &Value) -> bool {
        let Value::Obj(r) = receiver else { return false };
        if matches!(r.borrow().class, Class::Node(_) | Class::Document | Class::Window) {
            return false;
        }
        let p = self.protos();
        holder.ptr_eq(&p.html_element) || holder.ptr_eq(&p.element) || holder.ptr_eq(&p.node)
    }

    pub fn get_obj(&mut self, o: &Obj, key: &Key, receiver: &Value) -> Res {
        let mut cur = o.clone();
        for _ in 0..2000 {
            let is_proxy = matches!(cur.borrow().class, Class::Proxy { .. });
            if is_proxy {
                return self.proxy_get(&cur, key, receiver);
            }
            if matches!(cur.borrow().class, Class::Function(_)) {
                self.materialize(&cur, key);
            }
            if let Some(p) = self.own_prop(&cur, key) {
                return match p.slot {
                    Slot::Data(v) => Ok(v),
                    Slot::Accessor(Some(_), _) if self.dom_accessor_on_non_node(&cur, receiver) => Ok(Value::Undefined),
                    Slot::Accessor(Some(g), _) => self.call_obj(&g, receiver.clone(), &[], None),
                    Slot::Accessor(None, _) => Ok(Value::Undefined),
                };
            }
            let next = cur.borrow().proto.clone();
            match next {
                Some(n) => cur = n,
                None => return Ok(Value::Undefined),
            }
        }
        Ok(Value::Undefined)
    }

    fn proxy_get(&mut self, p: &Obj, key: &Key, receiver: &Value) -> Res {
        let (target, handler) = match &p.borrow().class {
            Class::Proxy { target, handler } => (target.clone(), handler.clone()),
            _ => unreachable!(),
        };
        let trap = self.get_obj(&handler, &Key::str("get"), &Value::Obj(handler.clone()))?;
        if trap.is_undefined() {
            return self.get_obj(&target, key, receiver);
        }
        let kv = self.key_to_value(key);
        self.call(&trap, Value::Obj(handler), &[Value::Obj(target), kv, receiver.clone()])
    }

    pub fn get_v(&mut self, v: &Value, key: &Key) -> Res {
        match v {
            Value::Obj(o) => self.get_obj(o, key, v),
            Value::Str(s) => {
                if let Key::S(k) = key {
                    if &**k == "length" {
                        return Ok(Value::Num(u16_len(s) as f64));
                    }
                    if let Some(i) = key.as_index() {
                        return Ok(u16_at(s, i).map_or(Value::Undefined, |u| Value::string(from_u16(&[u]))));
                    }
                }
                let p = self.protos().string.clone();
                self.get_obj(&p, key, v)
            }
            Value::Num(_) => {
                let p = self.protos().number.clone();
                self.get_obj(&p, key, v)
            }
            Value::Bool(_) => {
                let p = self.protos().boolean.clone();
                self.get_obj(&p, key, v)
            }
            Value::Sym(_) => {
                let p = self.protos().symbol.clone();
                self.get_obj(&p, key, v)
            }
            Value::BigInt(_) => match self.protos().extra.get("BigInt").cloned() {
                Some(p) => self.get_obj(&p, key, v),
                None => Ok(Value::Undefined),
            },
            Value::Undefined | Value::Null => {
                let k = match key {
                    Key::S(s) => s.to_string(),
                    _ => "symbol".to_string(),
                };
                Err(self.error_ctl("TypeError", &format!("Cannot read properties of {} (reading '{}')", self.describe(v), k)))
            }
        }
    }

    pub fn get_str(&mut self, v: &Value, name: &str) -> Res {
        self.get_v(v, &Key::str(name))
    }

    pub fn get_member(&mut self, base: &Value, k: &Value) -> Res {
        if let (Value::Obj(o), Value::Num(n)) = (base, k) {
            match &o.borrow().class {
                Class::Array(a) => {
                    let i = *n as usize;
                    if i as f64 == *n && i < a.len() && !is_hole(&a[i]) {
                        return Ok(a[i].clone());
                    }
                }
                Class::Typed(v) => {
                    let i = *n as usize;
                    if i as f64 == *n {
                        return Ok(v.get(i).map_or(Value::Undefined, Value::Num));
                    }
                }
                _ => {}
            }
        }
        if let (Value::Str(s), Value::Num(n)) = (base, k) {
            let i = *n as usize;
            if i as f64 == *n {
                return Ok(u16_at(s, i).map_or(Value::Undefined, |u| Value::string(from_u16(&[u]))));
            }
        }
        let key = self.to_key(k)?;
        self.get_v(base, &key)
    }

    pub fn set_member(&mut self, base: &Value, k: &Value, v: Value) -> Res<()> {
        if let (Value::Obj(o), Value::Num(n), Value::Num(x)) = (base, k, &v) {
            if let Class::Typed(view) = &o.borrow().class {
                let i = *n as usize;
                if i as f64 == *n {
                    view.set(i, *x);
                    return Ok(());
                }
            }
        }
        if let (Value::Obj(o), Value::Num(n)) = (base, k) {
            let mut ob = o.borrow_mut();
            if let Class::Array(a) = &mut ob.class {
                let i = *n as usize;
                if i as f64 == *n {
                    if i < a.len() {
                        a[i] = v;
                        return Ok(());
                    }
                    if i == a.len() && ob.extensible {
                        if let Class::Array(a) = &mut ob.class {
                            a.push(v);
                        }
                        return Ok(());
                    }
                }
            }
        }
        let key = self.to_key(k)?;
        self.set_v(base, key, v)
    }

    pub fn set_v(&mut self, target: &Value, key: Key, v: Value) -> Res<()> {
        match target {
            Value::Obj(o) => {
                self.set_obj(o, key, v, target)?;
                Ok(())
            }
            Value::Undefined | Value::Null => {
                let k = match &key {
                    Key::S(s) => s.to_string(),
                    _ => "symbol".to_string(),
                };
                Err(self.error_ctl("TypeError", &format!("Cannot set properties of {} (setting '{}')", self.describe(target), k)))
            }
            _ => Ok(()),
        }
    }

    pub fn set_obj(&mut self, o: &Obj, key: Key, v: Value, receiver: &Value) -> Res<bool> {
        // typed array elements: numeric keys never create properties
        if let Some(i) = key.as_index() {
            let is_typed = matches!(o.borrow().class, Class::Typed(_));
            if is_typed {
                let n = self.to_number(&v)?;
                if let Class::Typed(view) = &o.borrow().class {
                    view.set(i, n);
                }
                return Ok(true);
            }
        }
        // fast path: own writable data property on the receiver
        {
            let recv_is_o = matches!(receiver, Value::Obj(r) if r.ptr_eq(o));
            if recv_is_o {
                let mut ob = o.borrow_mut();
                let is_plain = !matches!(ob.class, Class::Proxy { .. } | Class::Array(_) | Class::Str(_));
                if is_plain {
                    if let Some(p) = ob.props.get_mut(&key) {
                        if let Slot::Data(slot) = &mut p.slot {
                            if p.flags & WRITABLE != 0 {
                                *slot = v;
                                return Ok(true);
                            }
                            return Ok(false);
                        }
                    }
                }
            }
        }
        let mut cur = o.clone();
        for _ in 0..2000 {
            let proxy = match &cur.borrow().class {
                Class::Proxy { target, handler } => Some((target.clone(), handler.clone())),
                _ => None,
            };
            if let Some((target, handler)) = proxy {
                let trap = self.get_obj(&handler, &Key::str("set"), &Value::Obj(handler.clone()))?;
                if trap.is_undefined() {
                    return self.set_obj(&target, key, v, receiver);
                }
                let kv = self.key_to_value(&key);
                let r = self.call(&trap, Value::Obj(handler), &[Value::Obj(target), kv, v, receiver.clone()])?;
                return Ok(r.truthy());
            }
            if let Some(p) = self.own_prop(&cur, &key) {
                if matches!(p.slot, Slot::Accessor(..)) && self.dom_accessor_on_non_node(&cur, receiver) {
                    // element accessors live on shared prototypes here; for a non-element receiver
                    // (a class prototype, a plain object) behave as if the accessor were absent
                    if let Value::Obj(r) = receiver {
                        self.define_data(r, key, v, ALL);
                        return Ok(true);
                    }
                }
                match p.slot {
                    Slot::Accessor(_, Some(s)) => {
                        self.call_obj(&s, receiver.clone(), &[v], None)?;
                        return Ok(true);
                    }
                    Slot::Accessor(_, None) => return Ok(false),
                    Slot::Data(_) => {
                        if p.flags & WRITABLE == 0 {
                            return Ok(false);
                        }
                        break;
                    }
                }
            }
            let next = cur.borrow().proto.clone();
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        // define / update on the receiver
        let Value::Obj(r) = receiver else { return Ok(false) };
        if matches!(r.borrow().class, Class::Proxy { .. }) {
            let target = match &r.borrow().class {
                Class::Proxy { target, .. } => target.clone(),
                _ => unreachable!(),
            };
            return self.set_obj(&target, key, v, &Value::Obj(target.clone()));
        }
        if let Some(existing) = self.own_prop(r, &key) {
            if existing.flags & WRITABLE == 0 {
                return Ok(false);
            }
            if let Slot::Data(_) = existing.slot {
                self.define_data_keep_flags(r, key, v, existing.flags);
                return Ok(true);
            }
        }
        if !r.borrow().extensible {
            return Ok(false);
        }
        self.define_data(r, key, v, ALL);
        Ok(true)
    }

    fn define_data_keep_flags(&self, o: &Obj, key: Key, v: Value, flags: u8) {
        self.define_data(o, key, v, flags);
    }

    pub fn has_property(&mut self, o: &Obj, key: &Key) -> Res<bool> {
        let mut cur = o.clone();
        for _ in 0..2000 {
            let proxy = match &cur.borrow().class {
                Class::Proxy { target, handler } => Some((target.clone(), handler.clone())),
                _ => None,
            };
            if let Some((target, handler)) = proxy {
                let trap = self.get_obj(&handler, &Key::str("has"), &Value::Obj(handler.clone()))?;
                if trap.is_undefined() {
                    return self.has_property(&target, key);
                }
                let kv = self.key_to_value(key);
                return Ok(self.call(&trap, Value::Obj(handler), &[Value::Obj(target), kv])?.truthy());
            }
            if matches!(cur.borrow().class, Class::Function(_)) {
                self.materialize(&cur, key);
            }
            if self.own_prop(&cur, key).is_some() {
                return Ok(true);
            }
            let next = cur.borrow().proto.clone();
            match next {
                Some(n) => cur = n,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    // -----------------------------------------------------------------------
    // Conversions
    // -----------------------------------------------------------------------

    pub fn to_key(&mut self, v: &Value) -> Res<Key> {
        match v {
            Value::Str(s) => Ok(Key::S(s.clone())),
            Value::Sym(s) => Ok(Key::Y(s.0.id)),
            Value::Num(n) => Ok(Key::S(Rc::from(num_to_string(*n)))),
            Value::Obj(_) => {
                let p = self.to_primitive(v, "string")?;
                self.to_key(&p)
            }
            other => Ok(Key::S(self.to_str(other)?)),
        }
    }

    pub fn to_primitive(&mut self, v: &Value, hint: &str) -> Res {
        let Value::Obj(o) = v else { return Ok(v.clone()) };
        let tp = Key::Y(self.wk().to_primitive.0.id);
        let ex = self.get_obj(o, &tp, v)?;
        if !ex.is_nullish() {
            let r = self.call(&ex, v.clone(), &[Value::str(hint)])?;
            if !matches!(r, Value::Obj(_)) {
                return Ok(r);
            }
            return Err(self.error_ctl("TypeError", "Cannot convert object to primitive value"));
        }
        let is_date = matches!(o.borrow().class, Class::Date(_));
        let order: [&Key; 2] = if hint == "string" || (hint == "default" && is_date) { [&self.names.to_string.clone(), &self.names.value_of.clone()] } else { [&self.names.value_of.clone(), &self.names.to_string.clone()] };
        let order = [order[0].clone(), order[1].clone()];
        for k in order.iter() {
            let f = self.get_obj(o, k, v)?;
            if matches!(&f, Value::Obj(fo) if fo.is_callable()) {
                let r = self.call(&f, v.clone(), &[])?;
                if !matches!(r, Value::Obj(_)) {
                    return Ok(r);
                }
            }
        }
        Err(self.error_ctl("TypeError", "Cannot convert object to primitive value"))
    }

    pub fn to_str(&mut self, v: &Value) -> Res<JsStr> {
        Ok(match v {
            Value::Str(s) => s.clone(),
            Value::Undefined => Rc::from("undefined"),
            Value::Null => Rc::from("null"),
            Value::Bool(b) => Rc::from(if *b { "true" } else { "false" }),
            Value::Num(n) => Rc::from(num_to_string(*n)),
            Value::BigInt(b) => Rc::from(b.to_string_radix(10)),
            Value::Sym(_) => return Err(self.error_ctl("TypeError", "Cannot convert a Symbol value to a string")),
            Value::Obj(_) => {
                let p = self.to_primitive(v, "string")?;
                return self.to_str(&p);
            }
        })
    }

    pub fn to_string_lossy(&mut self, v: &Value) -> String {
        match v {
            Value::Sym(s) => format!("Symbol({})", s.0.desc.as_deref().unwrap_or("")),
            other => self.to_str(other).map(|s| s.to_string()).unwrap_or_else(|_| "[object]".to_string()),
        }
    }

    pub fn to_number(&mut self, v: &Value) -> Res<f64> {
        Ok(match v {
            Value::Num(n) => *n,
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => *b as u8 as f64,
            Value::Str(s) => str_to_num(s),
            Value::Sym(_) => return Err(self.error_ctl("TypeError", "Cannot convert a Symbol value to a number")),
            Value::BigInt(_) => return Err(self.error_ctl("TypeError", "Cannot convert a BigInt value to a number")),
            Value::Obj(_) => {
                let p = self.to_primitive(v, "number")?;
                return self.to_number(&p);
            }
        })
    }

    pub fn to_object(&mut self, v: &Value) -> Res<Obj> {
        match v {
            Value::Obj(o) => Ok(o.clone()),
            Value::Undefined | Value::Null => Err(self.error_ctl("TypeError", "Cannot convert undefined or null to object")),
            Value::Bool(b) => Ok(self.alloc(Object::new(Some(self.protos().boolean.clone()), Class::Bool(*b)))),
            Value::Num(n) => Ok(self.alloc(Object::new(Some(self.protos().number.clone()), Class::Num(*n)))),
            Value::Str(s) => Ok(self.alloc(Object::new(Some(self.protos().string.clone()), Class::Str(s.clone())))),
            Value::Sym(s) => Ok(self.alloc(Object::new(Some(self.protos().symbol.clone()), Class::Symbol(s.clone())))),
            Value::BigInt(b) => {
                let proto = self.protos().extra.get("BigInt").cloned();
                Ok(self.alloc(Object::new(proto, Class::BigInt(b.clone()))))
            }
        }
    }

    pub fn to_int(&mut self, v: &Value) -> Res<f64> {
        Ok(to_integer(self.to_number(v)?))
    }

    // -----------------------------------------------------------------------
    // Operators
    // -----------------------------------------------------------------------

    pub fn loose_equals(&mut self, a: &Value, b: &Value) -> Res<bool> {
        Ok(match (a, b) {
            (Value::BigInt(x), Value::Num(n)) | (Value::Num(n), Value::BigInt(x)) => x.cmp_f64(*n) == Some(core::cmp::Ordering::Equal),
            (Value::BigInt(x), Value::Str(s)) | (Value::Str(s), Value::BigInt(x)) => super::bigint::BigInt::parse(s).is_some_and(|y| **x == y),
            (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
            (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
            (Value::Num(_), Value::Str(s)) => strict_equals(a, &Value::Num(str_to_num(s))),
            (Value::Str(s), Value::Num(_)) => strict_equals(&Value::Num(str_to_num(s)), b),
            (Value::Bool(x), _) => return self.loose_equals(&Value::Num(*x as u8 as f64), b),
            (_, Value::Bool(y)) => return self.loose_equals(a, &Value::Num(*y as u8 as f64)),
            (Value::Obj(_), Value::Obj(_)) => strict_equals(a, b),
            (Value::Obj(_), _) => {
                let p = self.to_primitive(a, "default")?;
                return self.loose_equals(&p, b);
            }
            (_, Value::Obj(_)) => {
                let p = self.to_primitive(b, "default")?;
                return self.loose_equals(a, &p);
            }
            _ => strict_equals(a, b),
        })
    }

    /// Operators with a BigInt operand (after primitive conversion).
    fn bigint_op(&mut self, op: BinOp, l: &Value, r: &Value) -> Option<Res> {
        use super::bigint::BigInt;
        use BinOp::*;
        let (lb, rb) = (matches!(l, Value::BigInt(_)), matches!(r, Value::BigInt(_)));
        if !lb && !rb {
            return None;
        }
        let big = |b: BigInt| Value::BigInt(Rc::new(b));
        if matches!(op, Lt | Gt | Le | Ge) {
            let ord = match (l, r) {
                (Value::BigInt(a), Value::BigInt(b)) => Some(a.cmp(b)),
                (Value::BigInt(a), other) => match other {
                    Value::Str(s) => BigInt::parse(s).map(|b| a.cmp(&b)),
                    v => self.to_number(v).ok().and_then(|n| a.cmp_f64(n)),
                },
                (other, Value::BigInt(b)) => match other {
                    Value::Str(s) => BigInt::parse(s).map(|a| a.cmp(b)),
                    v => self.to_number(v).ok().and_then(|n| b.cmp_f64(n).map(|o| o.reverse())),
                },
                _ => None,
            };
            return Some(Ok(Value::Bool(match ord {
                None => false,
                Some(o) => match op {
                    Lt => o.is_lt(),
                    Gt => o.is_gt(),
                    Le => o.is_le(),
                    _ => o.is_ge(),
                },
            })));
        }
        if matches!(op, Eq | Ne | StrictEq | StrictNe | In | InstanceOf) {
            return None;
        }
        if op == Add && (matches!(l, Value::Str(_)) || matches!(r, Value::Str(_))) {
            return None;
        }
        let (Value::BigInt(a), Value::BigInt(b)) = (l, r) else {
            return Some(Err(self.error_ctl("TypeError", "Cannot mix BigInt and other types, use explicit conversions")));
        };
        let shift = |b: &BigInt| -> i64 { b.to_f64().clamp(-1e9, 1e9) as i64 };
        Some(match op {
            Add => Ok(big(a.add(b))),
            Sub => Ok(big(a.sub(b))),
            Mul => Ok(big(a.mul(b))),
            Div | Mod => match a.divrem(b) {
                Some((q, rem)) => Ok(big(if op == Div { q } else { rem })),
                None => Err(self.error_ctl("RangeError", "Division by zero")),
            },
            Exp => match a.pow(b) {
                Some(p) => Ok(big(p)),
                None => Err(self.error_ctl("RangeError", "Maximum BigInt size exceeded")),
            },
            BitAnd => Ok(big(a.bitop(b, b'&'))),
            BitOr => Ok(big(a.bitop(b, b'|'))),
            BitXor => Ok(big(a.bitop(b, b'^'))),
            Shl => Ok(big(a.shl(shift(b)))),
            Shr => Ok(big(a.shr(shift(b)))),
            _ => Err(self.error_ctl("TypeError", "BigInts have no unsigned right shift, use >> instead")),
        })
    }

    pub fn binary_op(&mut self, op: BinOp, l: &Value, r: &Value) -> Res {
        use BinOp::*;
        if matches!(l, Value::BigInt(_)) || matches!(r, Value::BigInt(_)) {
            if let Some(res) = self.bigint_op(op, l, r) {
                return res;
            }
        } else if matches!(l, Value::Obj(_)) || matches!(r, Value::Obj(_)) {
            // `Object(1n) + 1n` and friends: decide after primitive conversion
            if !matches!(op, Eq | Ne | StrictEq | StrictNe | In | InstanceOf) {
                let lp = self.to_primitive(l, if op == Add { "default" } else { "number" })?;
                let rp = self.to_primitive(r, if op == Add { "default" } else { "number" })?;
                if matches!(lp, Value::BigInt(_)) || matches!(rp, Value::BigInt(_)) {
                    if let Some(res) = self.bigint_op(op, &lp, &rp) {
                        return res;
                    }
                }
            }
        }
        // number fast path
        if let (Value::Num(a), Value::Num(b)) = (l, r) {
            let (a, b) = (*a, *b);
            match op {
                Add => return Ok(Value::Num(a + b)),
                Sub => return Ok(Value::Num(a - b)),
                Mul => return Ok(Value::Num(a * b)),
                Div => return Ok(Value::Num(a / b)),
                Lt => return Ok(Value::Bool(a < b)),
                Gt => return Ok(Value::Bool(a > b)),
                Le => return Ok(Value::Bool(a <= b)),
                Ge => return Ok(Value::Bool(a >= b)),
                StrictEq | Eq => return Ok(Value::Bool(a == b)),
                StrictNe | Ne => return Ok(Value::Bool(a != b)),
                _ => {}
            }
        }
        match op {
            Add => {
                let lp = self.to_primitive(l, "default")?;
                let rp = self.to_primitive(r, "default")?;
                if matches!(lp, Value::Str(_)) || matches!(rp, Value::Str(_)) {
                    let a = self.to_str(&lp)?;
                    let b = self.to_str(&rp)?;
                    let mut s = String::with_capacity(a.len() + b.len());
                    s.push_str(&a);
                    s.push_str(&b);
                    return Ok(Value::string(s));
                }
                Ok(Value::Num(self.to_number(&lp)? + self.to_number(&rp)?))
            }
            Sub | Mul | Div | Mod | Exp => {
                let a = self.to_number(l)?;
                let b = self.to_number(r)?;
                Ok(Value::Num(match op {
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    Mod => {
                        if b == 0.0 || a.is_infinite() {
                            f64::NAN
                        } else if b.is_infinite() {
                            a
                        } else if a >= 0.0 && b > 0.0 && a < 9.0e15 && b < 9.0e15 && a == (a as i64) as f64 && b == (b as i64) as f64 {
                            // integer fast path (libm fmod is slow)
                            ((a as i64) % (b as i64)) as f64
                        } else {
                            a % b
                        }
                    }
                    _ => {
                        if b.is_infinite() && a.abs() == 1.0 {
                            f64::NAN
                        } else {
                            crate::math::powf(a, b)
                        }
                    }
                }))
            }
            BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                let a = self.to_number(l)?;
                let b = self.to_number(r)?;
                let (ai, bi) = (to_int32(a), to_int32(b));
                Ok(Value::Num(match op {
                    BitAnd => (ai & bi) as f64,
                    BitOr => (ai | bi) as f64,
                    BitXor => (ai ^ bi) as f64,
                    Shl => ai.wrapping_shl(bi as u32 & 31) as f64,
                    Shr => (ai >> (bi as u32 & 31)) as f64,
                    _ => ((ai as u32) >> (bi as u32 & 31)) as f64,
                }))
            }
            Eq => Ok(Value::Bool(self.loose_equals(l, r)?)),
            Ne => Ok(Value::Bool(!self.loose_equals(l, r)?)),
            StrictEq => Ok(Value::Bool(strict_equals(l, r))),
            StrictNe => Ok(Value::Bool(!strict_equals(l, r))),
            Lt | Gt | Le | Ge => {
                let lp = self.to_primitive(l, "number")?;
                let rp = self.to_primitive(r, "number")?;
                if let (Value::Str(a), Value::Str(b)) = (&lp, &rp) {
                    let ord = if a.is_ascii() && b.is_ascii() { a.cmp(b) } else { to_u16(a).cmp(&to_u16(b)) };
                    return Ok(Value::Bool(match op {
                        Lt => ord.is_lt(),
                        Gt => ord.is_gt(),
                        Le => ord.is_le(),
                        _ => ord.is_ge(),
                    }));
                }
                let a = self.to_number(&lp)?;
                let b = self.to_number(&rp)?;
                Ok(Value::Bool(match op {
                    Lt => a < b,
                    Gt => a > b,
                    Le => a <= b,
                    _ => a >= b,
                }))
            }
            In => {
                let Value::Obj(o) = r else {
                    return Err(self.error_ctl("TypeError", "Cannot use 'in' operator to search for a key in a non-object"));
                };
                let key = self.to_key(l)?;
                Ok(Value::Bool(self.has_property(o, &key)?))
            }
            InstanceOf => Ok(Value::Bool(self.instance_of(l, r)?)),
        }
    }

    pub fn instance_of(&mut self, v: &Value, c: &Value) -> Res<bool> {
        let Value::Obj(co) = c else {
            return Err(self.error_ctl("TypeError", "Right-hand side of 'instanceof' is not callable"));
        };
        let hi = Key::Y(self.wk().has_instance.0.id);
        let h = self.get_obj(co, &hi, c)?;
        if matches!(&h, Value::Obj(f) if f.is_callable()) && !matches!(&h, Value::Obj(f) if matches!(&f.borrow().class, Class::Function(Func::Native { data: Value::Bool(true), .. }))) {
            return Ok(self.call(&h, c.clone(), &[v.clone()])?.truthy());
        }
        if !co.is_callable() {
            return Err(self.error_ctl("TypeError", "Right-hand side of 'instanceof' is not callable"));
        }
        self.ordinary_has_instance(co, v)
    }

    pub fn ordinary_has_instance(&mut self, c: &Obj, v: &Value) -> Res<bool> {
        let bound = match &c.borrow().class {
            Class::Function(Func::Bound { target, .. }) => Some(target.clone()),
            _ => None,
        };
        if let Some(t) = bound {
            return self.instance_of(v, &Value::Obj(t));
        }
        let Value::Obj(o) = v else { return Ok(false) };
        let proto = self.get_obj(c, &self.names.prototype.clone(), &Value::Obj(c.clone()))?;
        let Value::Obj(p) = proto else {
            let name = self.get_obj(c, &self.names.name.clone(), &Value::Obj(c.clone())).ok().map(|n| self.to_string_lossy(&n)).unwrap_or_default();
            let kind = match &c.borrow().class {
                Class::Function(Func::Closure(cl)) => alloc::format!("{:?}{}{}", cl.def.kind, if cl.def.is_arrow { " arrow" } else { "" }, if cl.def.is_async { " async" } else { "" }),
                Class::Function(Func::Native { .. }) => "native".to_string(),
                _ => "other".to_string(),
            };
            let d = self.describe(&proto);
            return Err(self.error_ctl("TypeError", &format!("Function has non-object prototype '{d}' in instanceof check ({name}, {kind})")));
        };
        let mut cur = o.borrow().proto.clone();
        while let Some(x) = cur {
            if x.ptr_eq(&p) {
                return Ok(true);
            }
            cur = x.borrow().proto.clone();
        }
        Ok(false)
    }

    // -----------------------------------------------------------------------
    // Promises
    // -----------------------------------------------------------------------

    pub fn new_promise(&mut self) -> Obj {
        self.alloc(Object::new(
            Some(self.protos().promise.clone()),
            Class::Promise(Box::new(PromiseData { state: PromiseState::Pending, reactions: Vec::new(), handled: false })),
        ))
    }

    pub fn resolve_promise(&mut self, p: &Obj, v: Value) -> Res<()> {
        if !matches!(&p.borrow().class, Class::Promise(d) if matches!(d.state, PromiseState::Pending)) {
            return Ok(());
        }
        if let Value::Obj(o) = &v {
            if o.ptr_eq(p) {
                let e = self.make_error("TypeError", "Chaining cycle detected for promise");
                self.reject_promise(p, e);
                return Ok(());
            }
            let then = match self.get_obj(o, &self.names.then.clone(), &v) {
                Ok(t) => t,
                Err(Ctl::Throw(e)) => {
                    self.reject_promise(p, e);
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            if matches!(&then, Value::Obj(t) if t.is_callable()) {
                self.microtasks.push_back(Job::ResolveThenable { promise: p.clone(), thenable: v, then });
                return Ok(());
            }
        }
        self.settle(p, PromiseState::Fulfilled(v));
        Ok(())
    }

    pub fn reject_promise(&mut self, p: &Obj, e: Value) {
        if !matches!(&p.borrow().class, Class::Promise(d) if matches!(d.state, PromiseState::Pending)) {
            return;
        }
        self.settle(p, PromiseState::Rejected(e));
    }

    fn settle(&mut self, p: &Obj, state: PromiseState) {
        let (reactions, rejected, val, handled) = {
            let mut ob = p.borrow_mut();
            let Class::Promise(d) = &mut ob.class else { return };
            let rejected = matches!(state, PromiseState::Rejected(_));
            let val = match &state {
                PromiseState::Fulfilled(v) | PromiseState::Rejected(v) => v.clone(),
                PromiseState::Pending => Value::Undefined,
            };
            d.state = state;
            (core::mem::take(&mut d.reactions), rejected, val, d.handled)
        };
        if rejected && !handled && reactions.is_empty() {
            self.unhandled.push(Value::Obj(p.clone()));
        }
        for r in reactions {
            let handler = if rejected { r.on_rejected } else { r.on_fulfilled };
            self.microtasks.push_back(Job::Reaction { handler, arg: val.clone(), result: r.result, rejected });
        }
    }

    pub fn promise_then(&mut self, p: &Obj, on_f: Option<Value>, on_r: Option<Value>) -> Obj {
        let derived = self.new_promise();
        let state = {
            let mut ob = p.borrow_mut();
            let Class::Promise(d) = &mut ob.class else { return derived };
            d.handled = true;
            match &d.state {
                PromiseState::Pending => {
                    d.reactions.push(Reaction { on_fulfilled: on_f.clone(), on_rejected: on_r.clone(), result: Some(derived.clone()) });
                    None
                }
                PromiseState::Fulfilled(v) => Some((v.clone(), false)),
                PromiseState::Rejected(v) => Some((v.clone(), true)),
            }
        };
        if let Some((v, rejected)) = state {
            let handler = if rejected { on_r } else { on_f };
            self.microtasks.push_back(Job::Reaction { handler, arg: v, result: Some(derived.clone()), rejected });
        }
        derived
    }

    pub fn promise_resolved(&mut self, v: Value) -> Res<Obj> {
        if let Value::Obj(o) = &v {
            if matches!(o.borrow().class, Class::Promise(_)) {
                return Ok(o.clone());
            }
        }
        let p = self.new_promise();
        self.resolve_promise(&p, v)?;
        Ok(p)
    }

    pub fn resolving_functions(&mut self, p: &Obj) -> (Value, Value) {
        let state = self.new_array(vec![Value::Obj(p.clone()), Value::Bool(false)]);
        let f = self.new_native_data("", 1, native_resolve, state.clone(), false);
        let r = self.new_native_data("", 1, native_reject, state, false);
        (Value::Obj(f), Value::Obj(r))
    }

    pub fn report_error(&mut self, v: &Value) {
        let text = self.error_text(v);
        self.logs.push(LogEntry { level: "error", text: format!("Uncaught {text}") });
    }

    pub fn error_text(&mut self, v: &Value) -> String {
        if let Value::Obj(o) = v {
            if matches!(o.borrow().class, Class::Error) || self.has_property(o, &self.names.message.clone()).unwrap_or(false) {
                let name = self.get_obj(o, &self.names.name.clone(), v).ok().map(|n| self.to_string_lossy(&n)).unwrap_or_default();
                let msg = self.get_obj(o, &self.names.message.clone(), v).ok().map(|n| self.to_string_lossy(&n)).unwrap_or_default();
                return if msg.is_empty() { name } else { format!("{name}: {msg}") };
            }
        }
        self.to_string_lossy(v)
    }

    // -----------------------------------------------------------------------
    // Event loop
    // -----------------------------------------------------------------------

    pub fn run_one_microtask(&mut self) -> Res<bool> {
        let Some(job) = self.microtasks.pop_front() else { return Ok(false) };
        match job {
            Job::Call(f, this, args) => {
                if let Err(Ctl::Throw(e)) = self.call(&f, this, &args) {
                    self.report_error(&e);
                }
            }
            Job::Reaction { handler, arg, result, rejected } => match handler {
                Some(h) if matches!(&h, Value::Obj(o) if o.is_callable()) => match self.call(&h, Value::Undefined, &[arg]) {
                    Ok(v) => {
                        if let Some(r) = result {
                            self.resolve_promise(&r, v)?;
                        }
                    }
                    Err(Ctl::Throw(e)) => match result {
                        Some(r) => self.reject_promise(&r, e),
                        None => self.report_error(&e),
                    },
                    Err(other) => return Err(other),
                },
                _ => {
                    if let Some(r) = result {
                        if rejected {
                            self.reject_promise(&r, arg);
                        } else {
                            self.resolve_promise(&r, arg)?;
                        }
                    }
                }
            },
            Job::ResolveThenable { promise, thenable, then } => {
                let (res, rej) = self.resolving_functions(&promise);
                if let Err(Ctl::Throw(e)) = self.call(&then, thenable, &[res, rej.clone()]) {
                    let _ = self.call(&rej, Value::Undefined, &[e]);
                }
            }
        }
        Ok(true)
    }

    pub fn run_microtasks(&mut self) {
        let mut n = 0;
        self.draining += 1;
        while n < 2_000_000 {
            match self.run_one_microtask() {
                Ok(true) => n += 1,
                Ok(false) => break,
                Err(Ctl::Throw(e)) => self.report_error(&e),
                Err(_) => break,
            }
        }
        self.draining -= 1;
        if self.draining == 0 {
            self.report_unhandled();
        }
    }

    fn report_unhandled(&mut self) {
        let list = core::mem::take(&mut self.unhandled);
        for p in list {
            let (handled, reason) = match &p.as_obj().map(|o| o.borrow()) {
                Some(b) => match &b.class {
                    Class::Promise(d) => (d.handled, if let PromiseState::Rejected(v) = &d.state { v.clone() } else { Value::Undefined }),
                    _ => (true, Value::Undefined),
                },
                None => (true, Value::Undefined),
            };
            if !handled {
                let t = self.error_text(&reason);
                self.logs.push(LogEntry { level: "error", text: format!("Uncaught (in promise) {t}") });
            }
        }
    }

    /// Fire the earliest pending timer (advancing the virtual clock). Returns false if none.
    pub fn run_next_timer(&mut self) -> Res<bool> {
        let Some(idx) = self.timers.iter().enumerate().min_by(|a, b| a.1.due.partial_cmp(&b.1.due).unwrap_or(core::cmp::Ordering::Equal).then(a.1.id.cmp(&b.1.id))).map(|(i, _)| i) else {
            return Ok(false);
        };
        let (callback, args, id, raf) = {
            let t = &mut self.timers[idx];
            let raf = t.raf;
            if t.due > self.now {
                self.now = t.due;
            }
            let cb = t.callback.clone();
            let args = t.args.clone();
            let id = t.id;
            match t.interval {
                Some(iv) => t.due = self.now + iv.max(1.0),
                None => {
                    self.timers.remove(idx);
                }
            }
            (cb, args, id, raf)
        };
        let _ = id;
        let args = if raf { vec![Value::Num(self.now)] } else { args };
        // each timer callback is its own task with its own budget
        self.begin_task();
        if let Err(Ctl::Throw(e)) = self.call(&callback, Value::Undefined, &args) {
            self.report_error(&e);
        }
        self.run_microtasks();
        self.maybe_gc();
        Ok(true)
    }

    /// Run all timers due up to `until` (virtual milliseconds), draining microtasks in between.
    pub fn run_timers_until(&mut self, until: f64, max_fires: usize) {
        self.run_microtasks();
        let mut fired = 0;
        while fired < max_fires {
            let next = self.timers.iter().map(|t| t.due).fold(f64::INFINITY, f64::min);
            if next > until {
                break;
            }
            if !matches!(self.run_next_timer(), Ok(true)) {
                break;
            }
            fired += 1;
        }
        if self.now < until {
            self.now = until;
        }
    }
}

fn native_resolve(it: &mut Interp, c: &Call) -> Res {
    settle_native(it, c, false)
}

fn native_reject(it: &mut Interp, c: &Call) -> Res {
    settle_native(it, c, true)
}

fn settle_native(it: &mut Interp, c: &Call, reject: bool) -> Res {
    let data = it.native_data(c);
    let Value::Obj(st) = &data else { return Ok(Value::Undefined) };
    let (p, done) = match &st.borrow().class {
        Class::Array(a) => (a[0].clone(), a[1].truthy()),
        _ => return Ok(Value::Undefined),
    };
    if done {
        return Ok(Value::Undefined);
    }
    if let Class::Array(a) = &mut st.borrow_mut().class {
        a[1] = Value::Bool(true);
    }
    if let Value::Obj(po) = p {
        if reject {
            it.reject_promise(&po, c.arg(0));
        } else {
            it.resolve_promise(&po, c.arg(0))?;
        }
    }
    Ok(Value::Undefined)
}
