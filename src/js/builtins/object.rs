//! Object, Function, Symbol, Boolean, Error, Reflect and Proxy.
use super::*;
use alloc::{format, vec};

pub fn setup(it: &mut Interp) {
    setup_function_kinds(it);
    setup_object(it);
    setup_function(it);
    setup_symbol(it);
    setup_boolean(it);
    setup_errors(it);
    setup_reflect(it);
    setup_proxy(it);
    let g = it.global.clone();
    it.define_data(&g, Key::str("globalThis"), Value::Obj(g.clone()), HIDDEN);
    it.define_data(&g, Key::str("undefined"), Value::Undefined, 0);
    it.define_data(&g, Key::str("NaN"), Value::Num(f64::NAN), 0);
    it.define_data(&g, Key::str("Infinity"), Value::Num(f64::INFINITY), 0);
    for n in ["window", "self", "top", "parent", "frames"] {
        it.define_data(&g, Key::str(n), Value::Obj(g.clone()), HIDDEN);
    }
}

// ---------------------------------------------------------------------------
// Descriptors
// ---------------------------------------------------------------------------

pub fn from_prop(it: &mut Interp, p: &Prop) -> Value {
    let o = it.new_object();
    match &p.slot {
        Slot::Data(v) => {
            it.define_data(&o, Key::str("value"), v.clone(), ALL);
            it.define_data(&o, Key::str("writable"), Value::Bool(p.flags & WRITABLE != 0), ALL);
        }
        Slot::Accessor(g, s) => {
            it.define_data(&o, Key::str("get"), g.clone().map_or(Value::Undefined, Value::Obj), ALL);
            it.define_data(&o, Key::str("set"), s.clone().map_or(Value::Undefined, Value::Obj), ALL);
        }
    }
    it.define_data(&o, Key::str("enumerable"), Value::Bool(p.flags & ENUMERABLE != 0), ALL);
    it.define_data(&o, Key::str("configurable"), Value::Bool(p.flags & CONFIGURABLE != 0), ALL);
    Value::Obj(o)
}

pub fn define_from_descriptor(it: &mut Interp, o: &Obj, key: Key, desc: &Value) -> Res<()> {
    let Value::Obj(d) = desc else {
        return it.type_error("Property description must be an object");
    };
    // functions create `prototype`/`name`/`length` lazily: make them real before redefining
    if matches!(o.borrow().class, Class::Function(_)) {
        let _ = it.get_obj(o, &key, &Value::Obj(o.clone()));
    }
    let get = |it: &mut Interp, n: &str| -> Res<Option<Value>> {
        let k = Key::str(n);
        if it.has_property(d, &k)? { Ok(Some(it.get_obj(d, &k, desc)?)) } else { Ok(None) }
    };
    let existing = it.own_prop(o, &key);
    let enumerable = get(it, "enumerable")?.map(|v| v.truthy());
    let configurable = get(it, "configurable")?.map(|v| v.truthy());
    let value = get(it, "value")?;
    let writable = get(it, "writable")?.map(|v| v.truthy());
    let getter = get(it, "get")?;
    let setter = get(it, "set")?;
    if let Some(e) = &existing {
        if e.flags & CONFIGURABLE == 0 {
            // non-configurable: only allow harmless redefinitions (same value / writable true->false)
            let same = match (&e.slot, &value) {
                (Slot::Data(old), Some(new)) => same_value(old, new) || e.flags & WRITABLE != 0,
                _ => value.is_none(),
            };
            if !same || getter.is_some() || setter.is_some() || configurable == Some(true) {
                return it.type_error(&format!("Cannot redefine property: {}", key_name(&key)));
            }
        }
    } else if !o.borrow().extensible {
        return it.type_error(&format!("Cannot define property {}, object is not extensible", key_name(&key)));
    }
    let mut flags = existing.as_ref().map_or(0, |e| e.flags);
    let apply = |flags: &mut u8, bit: u8, v: Option<bool>| {
        if let Some(v) = v {
            if v {
                *flags |= bit
            } else {
                *flags &= !bit
            }
        }
    };
    apply(&mut flags, ENUMERABLE, enumerable);
    apply(&mut flags, CONFIGURABLE, configurable);
    if getter.is_some() || setter.is_some() {
        let (mut g, mut s) = match existing.as_ref().map(|e| &e.slot) {
            Some(Slot::Accessor(g, s)) => (g.clone(), s.clone()),
            _ => (None, None),
        };
        if let Some(Value::Obj(f)) = &getter {
            g = Some(f.clone());
        } else if matches!(getter, Some(Value::Undefined)) {
            g = None;
        }
        if let Some(Value::Obj(f)) = &setter {
            s = Some(f.clone());
        } else if matches!(setter, Some(Value::Undefined)) {
            s = None;
        }
        flags &= !WRITABLE;
        o.borrow_mut().props.insert(key, Prop { slot: Slot::Accessor(g, s), flags });
    } else {
        apply(&mut flags, WRITABLE, writable);
        let v = match (value, &existing) {
            (Some(v), _) => v,
            (None, Some(Prop { slot: Slot::Data(v), .. })) => v.clone(),
            _ => Value::Undefined,
        };
        if matches!(o.borrow().class, Class::Array(_)) && (key.as_index().is_some() || matches!(&key, Key::S(s) if &**s == "length")) {
            it.define_data(o, key, v, flags);
        } else {
            o.borrow_mut().props.insert(key, Prop { slot: Slot::Data(v), flags });
        }
    }
    Ok(())
}

pub fn key_name(k: &Key) -> String {
    match k {
        Key::S(s) => s.to_string(),
        Key::Y(_) => "Symbol()".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Object
// ---------------------------------------------------------------------------

fn setup_object(it: &mut Interp) {
    let proto = it.protos().object.clone();
    let ctor = it.make_ctor("Object", 1, object_ctor, &proto);
    it.def_method(&ctor, "keys", 1, object_keys);
    it.def_method(&ctor, "values", 1, object_values);
    it.def_method(&ctor, "entries", 1, object_entries);
    it.def_method(&ctor, "assign", 2, object_assign);
    it.def_method(&ctor, "freeze", 1, object_freeze);
    it.def_method(&ctor, "isFrozen", 1, object_is_frozen);
    it.def_method(&ctor, "seal", 1, object_seal);
    it.def_method(&ctor, "isSealed", 1, object_is_frozen);
    it.def_method(&ctor, "preventExtensions", 1, object_prevent_ext);
    it.def_method(&ctor, "isExtensible", 1, object_is_extensible);
    it.def_method(&ctor, "create", 2, object_create);
    it.def_method(&ctor, "getPrototypeOf", 1, object_get_proto);
    it.def_method(&ctor, "setPrototypeOf", 2, object_set_proto);
    it.def_method(&ctor, "defineProperty", 3, object_define_property);
    it.def_method(&ctor, "defineProperties", 2, object_define_properties);
    it.def_method(&ctor, "getOwnPropertyNames", 1, object_own_names);
    it.def_method(&ctor, "getOwnPropertySymbols", 1, object_own_symbols);
    it.def_method(&ctor, "getOwnPropertyDescriptor", 2, object_get_own_desc);
    it.def_method(&ctor, "getOwnPropertyDescriptors", 1, object_get_own_descs);
    it.def_method(&ctor, "fromEntries", 1, object_from_entries);
    it.def_method(&ctor, "is", 2, object_is);
    it.def_method(&ctor, "hasOwn", 2, object_has_own);
    it.def_method(&ctor, "groupBy", 2, object_group_by);
    it.def_method(&proto, "hasOwnProperty", 1, proto_has_own);
    it.def_method(&proto, "isPrototypeOf", 1, proto_is_proto_of);
    it.def_method(&proto, "propertyIsEnumerable", 1, proto_prop_enumerable);
    it.def_method(&proto, "toString", 0, proto_to_string);
    it.def_method(&proto, "toLocaleString", 0, proto_to_locale_string);
    it.def_method(&proto, "valueOf", 0, proto_value_of);
    it.def_accessor(&proto, "__proto__", proto_get_proto, proto_set_proto);
    it.def_method(&proto, "__defineGetter__", 2, proto_define_getter);
    it.def_method(&proto, "__defineSetter__", 2, proto_define_setter);
}

fn object_ctor(it: &mut Interp, c: &Call) -> Res {
    if let Some(nt) = &c.new_target {
        if !nt.ptr_eq(c.callee) {
            let default = it.protos().object.clone();
            let proto = it.proto_from_call(c, &default)?;
            return Ok(Value::Obj(it.alloc(Object::new(Some(proto), Class::Plain))));
        }
    }
    match c.arg(0) {
        Value::Undefined | Value::Null => Ok(Value::Obj(it.new_object())),
        v => Ok(Value::Obj(it.to_object(&v)?)),
    }
}

fn enumerable_own(it: &mut Interp, v: &Value) -> Res<Vec<(Key, Value)>> {
    let o = it.to_object(v)?;
    let mut out = Vec::new();
    for k in it.own_keys(&o)? {
        if let Key::Y(_) = k {
            continue;
        }
        let enumerable = it.own_prop_mut(&o, &k)?.is_some_and(|p| p.flags & ENUMERABLE != 0);
        if enumerable {
            let val = it.get_obj(&o, &k, &Value::Obj(o.clone()))?;
            out.push((k, val));
        }
    }
    Ok(out)
}

fn object_keys(it: &mut Interp, c: &Call) -> Res {
    let items = enumerable_own(it, &c.arg(0))?;
    Ok(it.new_array(items.into_iter().map(|(k, _)| it.key_to_value(&k)).collect()))
}

fn object_values(it: &mut Interp, c: &Call) -> Res {
    let items = enumerable_own(it, &c.arg(0))?;
    Ok(it.new_array(items.into_iter().map(|(_, v)| v).collect()))
}

fn object_entries(it: &mut Interp, c: &Call) -> Res {
    let items = enumerable_own(it, &c.arg(0))?;
    let mut out = Vec::new();
    for (k, v) in items {
        let kv = it.key_to_value(&k);
        out.push(it.new_array(vec![kv, v]));
    }
    Ok(it.new_array(out))
}

fn object_assign(it: &mut Interp, c: &Call) -> Res {
    let target = it.to_object(&c.arg(0))?;
    for src in c.args.iter().skip(1) {
        if src.is_nullish() {
            continue;
        }
        let so = it.to_object(src)?;
        for k in it.own_keys(&so)? {
            let enumerable = it.own_prop_mut(&so, &k)?.is_none_or(|p| p.flags & ENUMERABLE != 0);
            if enumerable {
                let v = it.get_obj(&so, &k, &Value::Obj(so.clone()))?;
                it.set_v(&Value::Obj(target.clone()), k, v)?;
            }
        }
    }
    Ok(Value::Obj(target))
}

fn object_freeze(it: &mut Interp, c: &Call) -> Res {
    if let Value::Obj(o) = c.arg(0) {
        let mut ob = o.borrow_mut();
        ob.extensible = false;
        let keys = ob.props.keys();
        for k in keys {
            if let Some(p) = ob.props.get_mut(&k) {
                p.flags &= !(CONFIGURABLE | if matches!(p.slot, Slot::Data(_)) { WRITABLE } else { 0 });
            }
        }
        let _ = it;
    }
    Ok(c.arg(0))
}

fn object_seal(_: &mut Interp, c: &Call) -> Res {
    if let Value::Obj(o) = c.arg(0) {
        let mut ob = o.borrow_mut();
        ob.extensible = false;
        for k in ob.props.keys() {
            if let Some(p) = ob.props.get_mut(&k) {
                p.flags &= !CONFIGURABLE;
            }
        }
    }
    Ok(c.arg(0))
}

fn object_is_frozen(_: &mut Interp, c: &Call) -> Res {
    Ok(Value::Bool(match c.arg(0) {
        Value::Obj(o) => {
            let ob = o.borrow();
            !ob.extensible && ob.props.keys().iter().all(|k| ob.props.get(k).is_none_or(|p| p.flags & CONFIGURABLE == 0))
        }
        _ => true,
    }))
}

fn object_prevent_ext(_: &mut Interp, c: &Call) -> Res {
    if let Value::Obj(o) = c.arg(0) {
        o.borrow_mut().extensible = false;
    }
    Ok(c.arg(0))
}

fn object_is_extensible(_: &mut Interp, c: &Call) -> Res {
    Ok(Value::Bool(matches!(c.arg(0), Value::Obj(o) if o.borrow().extensible)))
}

fn object_create(it: &mut Interp, c: &Call) -> Res {
    let proto = match c.arg(0) {
        Value::Obj(o) => Some(o),
        Value::Null => None,
        _ => return it.type_error("Object prototype may only be an Object or null"),
    };
    let o = it.alloc(Object::new(proto, Class::Plain));
    if let Value::Obj(props) = c.arg(1) {
        define_props(it, &o, &Value::Obj(props))?;
    }
    Ok(Value::Obj(o))
}

fn define_props(it: &mut Interp, o: &Obj, props: &Value) -> Res<()> {
    for (k, desc) in enumerable_own(it, props)? {
        define_from_descriptor(it, o, k, &desc)?;
    }
    Ok(())
}

fn object_get_proto(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.arg(0))?;
    let p = it.proto_through_proxy(&o)?;
    Ok(p.map_or(Value::Null, Value::Obj))
}

/// Would making `p` the prototype of `o` create a cycle (or change a non-extensible object)?
fn proto_change_ok(o: &Obj, p: &Option<Obj>) -> bool {
    let same = match (&o.borrow().proto, p) {
        (Some(a), Some(b)) => a.ptr_eq(b),
        (None, None) => true,
        _ => false,
    };
    if same {
        return true;
    }
    if !o.borrow().extensible {
        return false;
    }
    let mut cur = p.clone();
    let mut steps = 0;
    while let Some(x) = cur {
        if x.ptr_eq(o) {
            return false;
        }
        // a Proxy ends the walk (its [[GetPrototypeOf]] is a trap), as in the spec
        if matches!(x.borrow().class, Class::Proxy { .. }) || steps > 100_000 {
            break;
        }
        steps += 1;
        cur = x.borrow().proto.clone();
    }
    true
}

fn set_proto_checked(it: &mut Interp, o: &Obj, v: Value, throw: bool) -> Res<bool> {
    let p = match v {
        Value::Obj(p) => Some(p),
        Value::Null => None,
        _ => return if throw { it.type_error("Object prototype may only be an Object or null") } else { Ok(false) },
    };
    if !proto_change_ok(o, &p) {
        return if throw { it.type_error("Cyclic __proto__ value") } else { Ok(false) };
    }
    o.borrow_mut().proto = p;
    Ok(true)
}

fn object_set_proto(it: &mut Interp, c: &Call) -> Res {
    if let Value::Obj(o) = c.arg(0) {
        set_proto_checked(it, &o, c.arg(1), true)?;
    } else if !matches!(c.arg(1), Value::Obj(_) | Value::Null) {
        return it.type_error("Object prototype may only be an Object or null");
    }
    Ok(c.arg(0))
}

fn object_define_property(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(o) = c.arg(0) else { return it.type_error("Object.defineProperty called on non-object") };
    let key = it.to_key(&c.arg(1))?;
    if let Class::Proxy { target, handler } = &o.borrow().class {
        let (t, h) = (target.clone(), handler.clone());
        let trap = it.get_obj(&h, &Key::str("defineProperty"), &Value::Obj(h.clone()))?;
        if !trap.is_undefined() {
            let kv = it.key_to_value(&key);
            it.call(&trap, Value::Obj(h), &[Value::Obj(t), kv, c.arg(2)])?;
            return Ok(c.arg(0));
        }
        define_from_descriptor(it, &t, key, &c.arg(2))?;
        return Ok(c.arg(0));
    }
    define_from_descriptor(it, &o, key, &c.arg(2))?;
    Ok(c.arg(0))
}

fn object_define_properties(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(o) = c.arg(0) else { return it.type_error("Object.defineProperties called on non-object") };
    define_props(it, &o, &c.arg(1))?;
    Ok(c.arg(0))
}

fn object_own_names(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.arg(0))?;
    let keys = it.own_keys(&o)?;
    Ok(it.new_array(keys.into_iter().filter_map(|k| if let Key::S(s) = k { Some(Value::Str(s)) } else { None }).collect()))
}

fn object_own_symbols(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.arg(0))?;
    let keys = it.own_keys(&o)?;
    let vals: Vec<Value> = keys.into_iter().filter(|k| matches!(k, Key::Y(_))).map(|k| it.key_to_value(&k)).collect();
    Ok(it.new_array(vals))
}

fn object_get_own_desc(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.arg(0))?;
    let key = it.to_key(&c.arg(1))?;
    if matches!(o.borrow().class, Class::Function(_)) {
        let _ = it.get_obj(&o, &key, &Value::Undefined);
    }
    match it.own_prop_mut(&o, &key)? {
        Some(p) => Ok(from_prop(it, &p)),
        None => Ok(Value::Undefined),
    }
}

fn object_get_own_descs(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.arg(0))?;
    let out = it.new_object();
    for k in it.own_keys(&o)? {
        if let Some(p) = it.own_prop_mut(&o, &k)? {
            let d = from_prop(it, &p);
            it.define_data(&out, k, d, ALL);
        }
    }
    Ok(Value::Obj(out))
}

fn object_from_entries(it: &mut Interp, c: &Call) -> Res {
    let out = it.new_object();
    for e in it.iter_to_vec(&c.arg(0))? {
        let k = it.get_member(&e, &Value::Num(0.0))?;
        let v = it.get_member(&e, &Value::Num(1.0))?;
        let key = it.to_key(&k)?;
        it.define_data(&out, key, v, ALL);
    }
    Ok(Value::Obj(out))
}

fn object_is(_: &mut Interp, c: &Call) -> Res {
    Ok(Value::Bool(same_value(&c.arg(0), &c.arg(1))))
}

fn object_has_own(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.arg(0))?;
    let k = it.to_key(&c.arg(1))?;
    if matches!(o.borrow().class, Class::Function(_)) {
        let _ = it.get_obj(&o, &k, &Value::Undefined);
    }
    Ok(Value::Bool(it.own_prop_mut(&o, &k)?.is_some()))
}

fn object_group_by(it: &mut Interp, c: &Call) -> Res {
    let out = it.alloc(Object::new(None, Class::Plain));
    let f = c.arg(1);
    for (i, v) in it.iter_to_vec(&c.arg(0))?.into_iter().enumerate() {
        let k = it.call(&f, Value::Undefined, &[v.clone(), Value::Num(i as f64)])?;
        let key = it.to_key(&k)?;
        let existing = it.own_prop(&out, &key);
        match existing {
            Some(Prop { slot: Slot::Data(Value::Obj(a)), .. }) => {
                if let Class::Array(arr) = &mut a.borrow_mut().class {
                    arr.push(v);
                }
            }
            _ => {
                let a = it.new_array(vec![v]);
                it.define_data(&out, key, a, ALL);
            }
        }
    }
    Ok(Value::Obj(out))
}

fn proto_has_own(it: &mut Interp, c: &Call) -> Res {
    let k = it.to_key(&c.arg(0))?;
    let o = it.to_object(&c.this)?;
    if matches!(o.borrow().class, Class::Function(_)) {
        let _ = it.get_obj(&o, &k, &Value::Undefined);
    }
    Ok(Value::Bool(it.own_prop_mut(&o, &k)?.is_some()))
}

fn proto_is_proto_of(_: &mut Interp, c: &Call) -> Res {
    let (Value::Obj(this), Value::Obj(v)) = (&c.this, c.arg(0)) else { return Ok(Value::Bool(false)) };
    let mut cur = v.borrow().proto.clone();
    while let Some(p) = cur {
        if p.ptr_eq(this) {
            return Ok(Value::Bool(true));
        }
        cur = p.borrow().proto.clone();
    }
    Ok(Value::Bool(false))
}

fn proto_prop_enumerable(it: &mut Interp, c: &Call) -> Res {
    let k = it.to_key(&c.arg(0))?;
    let o = it.to_object(&c.this)?;
    Ok(Value::Bool(it.own_prop_mut(&o, &k)?.is_some_and(|p| p.flags & ENUMERABLE != 0)))
}

pub fn class_tag(it: &mut Interp, v: &Value) -> Res<String> {
    Ok(match v {
        Value::Undefined => "Undefined".to_string(),
        Value::Null => "Null".to_string(),
        Value::Obj(o) => {
            let tag = it.get_obj(o, &Key::Y(it.wk().to_string_tag.0.id), v)?;
            if let Value::Str(s) = tag {
                return Ok(s.to_string());
            }
            match &o.borrow().class {
                Class::Array(_) => "Array",
                Class::Function(_) => "Function",
                Class::Error => "Error",
                Class::Bool(_) => "Boolean",
                Class::Num(_) => "Number",
                Class::Str(_) => "String",
                Class::Date(_) => "Date",
                Class::RegExp(_) => "RegExp",
                Class::Arguments => "Arguments",
                Class::BigInt(_) => "BigInt",
                _ => "Object",
            }
            .to_string()
        }
        Value::Bool(_) => "Boolean".to_string(),
        Value::Num(_) => "Number".to_string(),
        Value::Str(_) => "String".to_string(),
        Value::Sym(_) => "Symbol".to_string(),
        Value::BigInt(_) => "BigInt".to_string(),
    })
}

fn proto_to_string(it: &mut Interp, c: &Call) -> Res {
    let tag = class_tag(it, &c.this)?;
    Ok(Value::string(format!("[object {tag}]")))
}

fn proto_to_locale_string(it: &mut Interp, c: &Call) -> Res {
    let f = it.get_v(&c.this, &it.names.to_string.clone())?;
    it.call(&f, c.this.clone(), &[])
}

fn proto_value_of(it: &mut Interp, c: &Call) -> Res {
    Ok(Value::Obj(it.to_object(&c.this)?))
}

fn proto_get_proto(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.this)?;
    let p = o.borrow().proto.clone();
    Ok(p.map_or(Value::Null, Value::Obj))
}

fn proto_set_proto(it: &mut Interp, c: &Call) -> Res {
    if let Value::Obj(o) = &c.this {
        if matches!(c.arg(0), Value::Obj(_) | Value::Null) {
            set_proto_checked(it, o, c.arg(0), true)?;
        }
    }
    Ok(Value::Undefined)
}

fn proto_define_getter(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.this)?;
    let k = it.to_key(&c.arg(0))?;
    if let Value::Obj(f) = c.arg(1) {
        it.define_accessor_part(&o, k, Some(f), true, ENUMERABLE | CONFIGURABLE);
    }
    Ok(Value::Undefined)
}

fn proto_define_setter(it: &mut Interp, c: &Call) -> Res {
    let o = it.to_object(&c.this)?;
    let k = it.to_key(&c.arg(0))?;
    if let Value::Obj(f) = c.arg(1) {
        it.define_accessor_part(&o, k, Some(f), false, ENUMERABLE | CONFIGURABLE);
    }
    Ok(Value::Undefined)
}

// ---------------------------------------------------------------------------
// Function
// ---------------------------------------------------------------------------

fn setup_function(it: &mut Interp) {
    let proto = it.protos().function.clone();
    it.define_data(&proto, Key::str("name"), Value::str(""), CONFIGURABLE);
    it.define_data(&proto, Key::str("length"), Value::Num(0.0), CONFIGURABLE);
    let ctor = it.make_ctor("Function", 1, function_ctor, &proto);
    let _ = ctor;
    it.def_method(&proto, "call", 1, function_call);
    it.def_method(&proto, "apply", 2, function_apply);
    it.def_method(&proto, "bind", 1, function_bind);
    it.def_method(&proto, "toString", 0, function_to_string);
    let hi = it.wk().has_instance.clone();
    let f = it.new_native_data("[Symbol.hasInstance]", 1, function_has_instance, Value::Bool(true), false);
    it.define_data(&proto, Key::Y(hi.0.id), Value::Obj(f), 0);
}

fn function_ctor(it: &mut Interp, c: &Call) -> Res {
    function_ctor_kind(it, c, "function")
}

fn function_ctor_kind(it: &mut Interp, c: &Call, kind: &str) -> Res {
    let n = c.args.len();
    let mut params = Vec::new();
    for a in c.args.iter().take(n.saturating_sub(1)) {
        params.push(it.to_str(a)?.to_string());
    }
    let body = if n > 0 { it.to_str(&c.args[n - 1])?.to_string() } else { String::new() };
    let def = match super::super::parser::parse_function_source_kind(&params.join(","), &body, kind, &mut it.interner) {
        Ok(d) => d,
        Err(e) => return Err(it.error_ctl("SyntaxError", e.trim_start_matches("SyntaxError: "))),
    };
    let env = it.global_env.clone();
    Ok(Value::Obj(it.make_closure(def, &env, None, Some(Rc::from("anonymous")))))
}

fn function_call(it: &mut Interp, c: &Call) -> Res {
    let this = c.arg(0);
    let rest = if c.args.len() > 1 { &c.args[1..] } else { &[] };
    it.call(&c.this, this, rest)
}

fn function_apply(it: &mut Interp, c: &Call) -> Res {
    let args = match c.arg(1) {
        Value::Undefined | Value::Null => Vec::new(),
        v => it.array_like_to_vec(&v)?,
    };
    it.call(&c.this, c.arg(0), &args)
}

fn function_bind(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(target) = &c.this else { return it.type_error("Bind must be called on a function") };
    if !target.is_callable() {
        return it.type_error("Bind must be called on a function");
    }
    let args: Vec<Value> = c.args.iter().skip(1).cloned().collect();
    let name = match it.get_obj(target, &it.names.name.clone(), &c.this)? {
        Value::Str(s) => s,
        _ => Rc::from(""),
    };
    let len = match it.get_obj(target, &it.names.length.clone(), &c.this)? {
        Value::Num(n) => (n - args.len() as f64).max(0.0),
        _ => 0.0,
    };
    let proto = target.borrow().proto.clone();
    let b = it.alloc(Object::new(proto, Class::Function(Func::Bound { target: target.clone(), this: c.arg(0), args })));
    it.define_data(&b, it.names.name.clone(), Value::string(format!("bound {name}")), CONFIGURABLE);
    it.define_data(&b, it.names.length.clone(), Value::Num(len), CONFIGURABLE);
    Ok(Value::Obj(b))
}

fn function_to_string(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(o) = &c.this else { return it.type_error("Function.prototype.toString requires that 'this' be a Function") };
    // platform functions written in the prelude print like built-ins, as in browsers
    let from_prelude = |src: &str| src.len() == crate::js::window::PRELUDE.len() && src.as_bytes()[..src.len().min(256)] == crate::js::window::PRELUDE.as_bytes()[..src.len().min(256)];
    let src = match &o.borrow().class {
        Class::Function(Func::Closure(cl)) if from_prelude(&cl.def.source) => None,
        Class::Function(Func::Closure(cl)) => Some(cl.def.source[cl.def.span.0.min(cl.def.source.len())..cl.def.span.1.min(cl.def.source.len())].to_string()),
        Class::Function(_) => None,
        _ => return Err(Ctl::Throw(Value::Undefined)),
    };
    let native = matches!(&o.borrow().class, Class::Function(Func::Closure(cl)) if from_prelude(&cl.def.source));
    if native {
        let name = it.get_obj(o, &it.names.name.clone(), &c.this)?;
        let n = it.to_string_lossy(&name);
        return Ok(Value::string(format!("function {n}() {{ [native code] }}")));
    }
    if let Some(Value::Str(s)) = o.borrow().props.get(&Key::str("__class_src")).map(|p| match &p.slot {
        Slot::Data(v) => v.clone(),
        _ => Value::Undefined,
    }) {
        return Ok(Value::Str(s));
    }
    match src {
        Some(s) if !s.is_empty() => Ok(Value::string(s)),
        _ => {
            let name = it.get_obj(o, &it.names.name.clone(), &c.this)?;
            let n = it.to_string_lossy(&name);
            Ok(Value::string(format!("function {n}() {{ [native code] }}")))
        }
    }
}

fn function_has_instance(it: &mut Interp, c: &Call) -> Res {
    match &c.this {
        Value::Obj(f) if f.is_callable() => Ok(Value::Bool(it.ordinary_has_instance(f, &c.arg(0))?)),
        _ => Ok(Value::Bool(false)),
    }
}

// ---------------------------------------------------------------------------
// Symbol / Boolean
// ---------------------------------------------------------------------------

fn setup_symbol(it: &mut Interp) {
    let proto = it.protos().symbol.clone();
    let ctor = it.make_ctor("Symbol", 0, symbol_ctor, &proto);
    let wk: Vec<(&str, Sym)> = {
        let w = it.wk();
        vec![
            ("iterator", w.iterator.clone()),
            ("asyncIterator", w.async_iterator.clone()),
            ("toPrimitive", w.to_primitive.clone()),
            ("hasInstance", w.has_instance.clone()),
            ("toStringTag", w.to_string_tag.clone()),
            ("species", w.species.clone()),
        ]
    };
    for (n, s) in wk {
        it.define_data(&ctor, Key::str(n), Value::Sym(s), 0);
    }
    for (n, d) in [("isConcatSpreadable", "Symbol.isConcatSpreadable"), ("match", "Symbol.match"), ("matchAll", "Symbol.matchAll"), ("replace", "Symbol.replace"), ("search", "Symbol.search"), ("split", "Symbol.split"), ("unscopables", "Symbol.unscopables")] {
        let s = it.new_symbol(Some(Rc::from(d)));
        it.define_data(&ctor, Key::str(n), Value::Sym(s), 0);
    }
    it.def_method(&ctor, "for", 1, symbol_for);
    it.def_method(&ctor, "keyFor", 1, symbol_key_for);
    it.def_method(&proto, "toString", 0, symbol_to_string);
    it.def_method(&proto, "valueOf", 0, symbol_value_of);
    it.def_getter(&proto, "description", symbol_description);
    let tp = it.wk().to_primitive.clone();
    it.def_sym_method(&proto, &tp, "[Symbol.toPrimitive]", 1, symbol_value_of);
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&proto, Key::Y(tag.0.id), Value::str("Symbol"), CONFIGURABLE);
}

fn symbol_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_some() {
        return it.type_error("Symbol is not a constructor");
    }
    let desc = match c.arg(0) {
        Value::Undefined => None,
        v => Some(it.to_str(&v)?),
    };
    Ok(Value::Sym(it.new_symbol(desc)))
}

fn symbol_for(it: &mut Interp, c: &Call) -> Res {
    let k = it.arg_str(c, 0)?;
    if let Some(s) = it.symbol_registry.get(&*k) {
        return Ok(Value::Sym(s.clone()));
    }
    let s = it.new_symbol(Some(k.clone()));
    it.symbol_registry.insert(k.to_string(), s.clone());
    Ok(Value::Sym(s))
}

fn symbol_key_for(it: &mut Interp, c: &Call) -> Res {
    if let Value::Sym(s) = c.arg(0) {
        for (k, v) in &it.symbol_registry {
            if v.0.id == s.0.id {
                return Ok(Value::str(k));
            }
        }
    }
    Ok(Value::Undefined)
}

fn this_symbol(it: &mut Interp, c: &Call) -> Res<Sym> {
    match &c.this {
        Value::Sym(s) => Ok(s.clone()),
        Value::Obj(o) => match &o.borrow().class {
            Class::Symbol(s) => Ok(s.clone()),
            _ => Err(it.error_ctl("TypeError", "not a symbol")),
        },
        _ => Err(it.error_ctl("TypeError", "not a symbol")),
    }
}

fn symbol_to_string(it: &mut Interp, c: &Call) -> Res {
    let s = this_symbol(it, c)?;
    Ok(Value::string(format!("Symbol({})", s.0.desc.as_deref().unwrap_or(""))))
}

fn symbol_value_of(it: &mut Interp, c: &Call) -> Res {
    Ok(Value::Sym(this_symbol(it, c)?))
}

fn symbol_description(it: &mut Interp, c: &Call) -> Res {
    let s = this_symbol(it, c)?;
    Ok(s.0.desc.clone().map_or(Value::Undefined, Value::Str))
}

fn setup_boolean(it: &mut Interp) {
    let proto = it.protos().boolean.clone();
    it.make_ctor("Boolean", 1, boolean_ctor, &proto);
    it.def_method(&proto, "toString", 0, boolean_to_string);
    it.def_method(&proto, "valueOf", 0, boolean_value_of);
}

fn boolean_ctor(it: &mut Interp, c: &Call) -> Res {
    let b = c.arg(0).truthy();
    if c.new_target.is_some() {
        let default = it.protos().boolean.clone();
        let proto = it.proto_from_call(c, &default)?;
        return Ok(Value::Obj(it.alloc(Object::new(Some(proto), Class::Bool(b)))));
    }
    Ok(Value::Bool(b))
}

fn this_bool(it: &mut Interp, c: &Call) -> Res<bool> {
    match &c.this {
        Value::Bool(b) => Ok(*b),
        Value::Obj(o) => match &o.borrow().class {
            Class::Bool(b) => Ok(*b),
            _ => Err(it.error_ctl("TypeError", "not a boolean")),
        },
        _ => Err(it.error_ctl("TypeError", "not a boolean")),
    }
}

fn boolean_to_string(it: &mut Interp, c: &Call) -> Res {
    Ok(Value::str(if this_bool(it, c)? { "true" } else { "false" }))
}

fn boolean_value_of(it: &mut Interp, c: &Call) -> Res {
    Ok(Value::Bool(this_bool(it, c)?))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn setup_errors(it: &mut Interp) {
    let kinds: Vec<(&'static str, Obj)> = it.protos().error_kinds.iter().map(|(k, v)| (*k, v.clone())).collect();
    let mut base_ctor: Option<Obj> = None;
    for (name, proto) in kinds {
        let ctor = it.new_native_data(name, 1, error_ctor, Value::str(name), true);
        it.define_data(&ctor, it.names.prototype.clone(), Value::Obj(proto.clone()), 0);
        it.define_data(&proto, it.names.constructor.clone(), Value::Obj(ctor.clone()), HIDDEN);
        it.define_data(&proto, it.names.name.clone(), Value::str(name), HIDDEN);
        it.define_data(&proto, it.names.message.clone(), Value::str(""), HIDDEN);
        let g = it.global.clone();
        it.define_data(&g, Key::str(name), Value::Obj(ctor.clone()), HIDDEN);
        if name == "Error" {
            it.def_method(&proto, "toString", 0, error_to_string);
            it.def_method(&ctor, "captureStackTrace", 1, |_, _| Ok(Value::Undefined));
            it.define_data(&ctor, Key::str("stackTraceLimit"), Value::Num(10.0), ALL);
            base_ctor = Some(ctor);
        } else if let Some(b) = &base_ctor {
            ctor.borrow_mut().proto = Some(b.clone());
        }
    }
}

fn error_ctor(it: &mut Interp, c: &Call) -> Res {
    let kind = match it.native_data(c) {
        Value::Str(s) => s,
        _ => Rc::from("Error"),
    };
    let default = it.protos().error_kinds.get(&*kind).cloned().unwrap_or_else(|| it.protos().error.clone());
    let proto = it.proto_from_call(c, &default)?;
    let o = it.alloc(Object::new(Some(proto), Class::Error));
    let (msg_idx, opts_idx) = if &*kind == "AggregateError" { (1, 2) } else { (0, 1) };
    let msg = c.arg(msg_idx);
    let mut message = String::new();
    if !msg.is_undefined() {
        let m = it.to_str(&msg)?;
        message = m.to_string();
        it.define_data(&o, it.names.message.clone(), Value::Str(m), HIDDEN);
    }
    if let Value::Obj(opts) = c.arg(opts_idx) {
        let k = Key::str("cause");
        if it.has_property(&opts, &k)? {
            let cause = it.get_obj(&opts, &k, &Value::Obj(opts.clone()))?;
            it.define_data(&o, k, cause, HIDDEN);
        }
    }
    if &*kind == "AggregateError" {
        let errs = it.iter_to_vec(&c.arg(0))?;
        let arr = it.new_array(errs);
        it.define_data(&o, Key::str("errors"), arr, HIDDEN);
    }
    // the instance `name` comes from the prototype chain (subclasses may override)
    let name = it.get_obj(&o, &it.names.name.clone(), &Value::Obj(o.clone()))?;
    let name = it.to_string_lossy(&name);
    let stack = if message.is_empty() { format!("{name}\n    at <anonymous>") } else { format!("{name}: {message}\n    at <anonymous>") };
    it.define_data(&o, it.names.stack.clone(), Value::string(stack), HIDDEN);
    Ok(Value::Obj(o))
}

fn error_to_string(it: &mut Interp, c: &Call) -> Res {
    let name = it.get_v(&c.this, &it.names.name.clone())?;
    let msg = it.get_v(&c.this, &it.names.message.clone())?;
    let name = if name.is_undefined() { "Error".to_string() } else { it.to_str(&name)?.to_string() };
    let msg = if msg.is_undefined() { String::new() } else { it.to_str(&msg)?.to_string() };
    Ok(Value::string(if msg.is_empty() { name } else if name.is_empty() { msg } else { format!("{name}: {msg}") }))
}

// ---------------------------------------------------------------------------
// Reflect / Proxy
// ---------------------------------------------------------------------------

fn setup_reflect(it: &mut Interp) {
    let r = it.new_object();
    it.def_method(&r, "apply", 3, |it, c| {
        let args = it.array_like_to_vec(&c.arg(2))?;
        it.call(&c.arg(0), c.arg(1), &args)
    });
    it.def_method(&r, "construct", 2, |it, c| {
        let args = it.array_like_to_vec(&c.arg(1))?;
        let nt = match c.arg(2) {
            Value::Obj(o) => Some(o),
            _ => None,
        };
        it.construct(&c.arg(0), &args, nt)
    });
    it.def_method(&r, "defineProperty", 3, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.defineProperty called on non-object") };
        let k = it.to_key(&c.arg(1))?;
        Ok(Value::Bool(define_from_descriptor(it, &o, k, &c.arg(2)).is_ok()))
    });
    it.def_method(&r, "deleteProperty", 2, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.deleteProperty called on non-object") };
        let k = it.to_key(&c.arg(1))?;
        Ok(Value::Bool(it.delete_prop(&o, &k)?))
    });
    it.def_method(&r, "get", 2, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.get called on non-object") };
        let k = it.to_key(&c.arg(1))?;
        let recv = if c.args.len() > 2 { c.arg(2) } else { c.arg(0) };
        it.get_obj(&o, &k, &recv)
    });
    it.def_method(&r, "set", 3, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.set called on non-object") };
        let k = it.to_key(&c.arg(1))?;
        let recv = if c.args.len() > 3 { c.arg(3) } else { c.arg(0) };
        Ok(Value::Bool(it.set_obj(&o, k, c.arg(2), &recv)?))
    });
    it.def_method(&r, "has", 2, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.has called on non-object") };
        let k = it.to_key(&c.arg(1))?;
        Ok(Value::Bool(it.has_property(&o, &k)?))
    });
    it.def_method(&r, "getPrototypeOf", 1, object_get_proto);
    it.def_method(&r, "setPrototypeOf", 2, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.setPrototypeOf called on non-object") };
        Ok(Value::Bool(set_proto_checked(it, &o, c.arg(1), false)?))
    });
    it.def_method(&r, "getOwnPropertyDescriptor", 2, object_get_own_desc);
    it.def_method(&r, "isExtensible", 1, object_is_extensible);
    it.def_method(&r, "preventExtensions", 1, |it, c| {
        object_prevent_ext(it, c)?;
        Ok(Value::Bool(true))
    });
    it.def_method(&r, "ownKeys", 1, |it, c| {
        let Value::Obj(o) = c.arg(0) else { return it.type_error("Reflect.ownKeys called on non-object") };
        let keys = it.own_keys(&o)?;
        let vals: Vec<Value> = keys.iter().map(|k| it.key_to_value(k)).collect();
        Ok(it.new_array(vals))
    });
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&r, Key::Y(tag.0.id), Value::str("Reflect"), CONFIGURABLE);
    let g = it.global.clone();
    it.define_data(&g, Key::str("Reflect"), Value::Obj(r), HIDDEN);
}

fn setup_proxy(it: &mut Interp) {
    let ctor = it.new_native_data("Proxy", 2, proxy_ctor, Value::Undefined, true);
    it.def_method(&ctor, "revocable", 2, |it, c| {
        let p = proxy_ctor(it, &Call { this: Value::Undefined, args: c.args, callee: c.callee, new_target: Some(c.callee.clone()) })?;
        let o = it.new_object();
        it.define_data(&o, Key::str("proxy"), p, ALL);
        it.def_method(&o, "revoke", 0, |_, _| Ok(Value::Undefined));
        Ok(Value::Obj(o))
    });
    let g = it.global.clone();
    it.define_data(&g, Key::str("Proxy"), Value::Obj(ctor), HIDDEN);
}

fn proxy_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_none() {
        return it.type_error("Constructor Proxy requires 'new'");
    }
    let (Value::Obj(t), Value::Obj(h)) = (c.arg(0), c.arg(1)) else {
        return it.type_error("Cannot create proxy with a non-object as target or handler");
    };
    let proto = t.borrow().proto.clone();
    Ok(Value::Obj(it.alloc(Object::new(proto, Class::Proxy { target: t, handler: h }))))
}

fn setup_function_kinds(it: &mut Interp) {
    let fp = it.protos().function.clone();
    for (key, name, kind) in [("async_function", "AsyncFunction", 1u8), ("generator_function", "GeneratorFunction", 2), ("async_generator_function", "AsyncGeneratorFunction", 3)] {
        let proto = it.alloc(Object::new(Some(fp.clone()), Class::Plain));
        let f: NativeFn = match kind {
            1 => |it, c| function_ctor_kind(it, c, "async function"),
            2 => |it, c| function_ctor_kind(it, c, "function*"),
            _ => |it, c| function_ctor_kind(it, c, "async function*"),
        };
        let ctor = it.new_native_data(name, 1, f, Value::Undefined, true);
        it.define_data(&ctor, it.names.prototype.clone(), Value::Obj(proto.clone()), 0);
        it.define_data(&proto, it.names.constructor.clone(), Value::Obj(ctor), CONFIGURABLE);
        let tag = it.wk().to_string_tag.clone();
        it.define_data(&proto, Key::Y(tag.0.id), Value::str(name), CONFIGURABLE);
        it.protos.as_mut().unwrap().extra.insert(key, proto);
    }
}
