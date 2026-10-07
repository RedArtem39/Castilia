//! Map, Set, WeakMap, WeakSet, WeakRef.
use super::*;
use alloc::vec;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Map,
    Set,
    WeakMap,
    WeakSet,
}

pub fn setup(it: &mut Interp) {
    let ip = it.protos().iterator.clone();
    let ci = it.alloc(Object::new(Some(ip), Class::Plain));
    it.def_method(&ci, "next", 0, coll_iter_next);
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&ci, Key::Y(tag.0.id), Value::str("Map Iterator"), CONFIGURABLE);
    it.protos.as_mut().unwrap().extra.insert("collection_iterator", ci);

    let iter_sym = it.wk().iterator.clone();
    // Map
    let mp = it.protos().map.clone();
    let ctor = it.make_ctor("Map", 0, |it, c| construct(it, c, Kind::Map), &mp);
    it.def_method(&ctor, "groupBy", 2, |it, c| {
        let items = it.iter_to_vec(&c.arg(0))?;
        let f = c.arg(1);
        let proto = it.protos().map.clone();
        let m = it.alloc(Object::new(Some(proto), Class::Map(Box::default())));
        for (i, v) in items.into_iter().enumerate() {
            let k = it.call(&f, Value::Undefined, &[v.clone(), Value::Num(i as f64)])?;
            let existing = with_data(&m, |d| d.get(&k).cloned());
            match existing {
                Some(Value::Obj(arr)) => {
                    if let Class::Array(a) = &mut arr.borrow_mut().class {
                        a.push(v);
                    }
                }
                _ => {
                    let arr = it.new_array(vec![v]);
                    with_data(&m, |d| d.set(k, arr));
                }
            }
        }
        Ok(Value::Obj(m))
    });
    it.def_method(&mp, "get", 1, |it, c| {
        let o = this_coll(it, c, Kind::Map)?;
        Ok(with_data(&o, |d| d.get(&c.arg(0)).cloned()).unwrap_or(Value::Undefined))
    });
    it.def_method(&mp, "set", 2, |it, c| {
        let o = this_coll(it, c, Kind::Map)?;
        with_data(&o, |d| d.set(c.arg(0), c.arg(1)));
        Ok(c.this.clone())
    });
    common(it, &mp, Kind::Map);
    it.def_method(&mp, "entries", 0, |it, c| make_iter(it, c, Kind::Map, 2));
    it.def_method(&mp, "keys", 0, |it, c| make_iter(it, c, Kind::Map, 0));
    it.def_method(&mp, "values", 0, |it, c| make_iter(it, c, Kind::Map, 1));
    let entries = it.get_str(&Value::Obj(mp.clone()), "entries").unwrap_or(Value::Undefined);
    it.define_data(&mp, Key::Y(iter_sym.0.id), entries, HIDDEN);
    it.define_data(&mp, Key::Y(tag.0.id), Value::str("Map"), CONFIGURABLE);

    // Set
    let sp = it.protos().set.clone();
    it.make_ctor("Set", 0, |it, c| construct(it, c, Kind::Set), &sp);
    it.def_method(&sp, "add", 1, |it, c| {
        let o = this_coll(it, c, Kind::Set)?;
        with_data(&o, |d| d.set(c.arg(0), c.arg(0)));
        Ok(c.this.clone())
    });
    common(it, &sp, Kind::Set);
    it.def_method(&sp, "entries", 0, |it, c| make_iter(it, c, Kind::Set, 2));
    it.def_method(&sp, "values", 0, |it, c| make_iter(it, c, Kind::Set, 1));
    let values = it.get_str(&Value::Obj(sp.clone()), "values").unwrap_or(Value::Undefined);
    it.define_data(&sp, Key::str("keys"), values.clone(), HIDDEN);
    it.define_data(&sp, Key::Y(iter_sym.0.id), values, HIDDEN);
    it.define_data(&sp, Key::Y(tag.0.id), Value::str("Set"), CONFIGURABLE);
    set_algebra(it, &sp);

    // WeakMap / WeakSet
    let wm = it.protos().weakmap.clone();
    it.make_ctor("WeakMap", 0, |it, c| construct(it, c, Kind::WeakMap), &wm);
    it.def_method(&wm, "get", 1, |it, c| {
        let o = this_coll(it, c, Kind::WeakMap)?;
        Ok(with_data(&o, |d| d.get(&c.arg(0)).cloned()).unwrap_or(Value::Undefined))
    });
    it.def_method(&wm, "set", 2, |it, c| {
        let o = this_coll(it, c, Kind::WeakMap)?;
        if !matches!(c.arg(0), Value::Obj(_) | Value::Sym(_)) {
            return it.type_error("Invalid value used as weak map key");
        }
        with_data(&o, |d| d.set(c.arg(0), c.arg(1)));
        Ok(c.this.clone())
    });
    it.def_method(&wm, "has", 1, |it, c| {
        let o = this_coll(it, c, Kind::WeakMap)?;
        Ok(Value::Bool(with_data(&o, |d| d.has(&c.arg(0)))))
    });
    it.def_method(&wm, "delete", 1, |it, c| {
        let o = this_coll(it, c, Kind::WeakMap)?;
        Ok(Value::Bool(with_data(&o, |d| d.delete(&c.arg(0)))))
    });
    it.define_data(&wm, Key::Y(tag.0.id), Value::str("WeakMap"), CONFIGURABLE);
    let ws = it.protos().weakset.clone();
    it.make_ctor("WeakSet", 0, |it, c| construct(it, c, Kind::WeakSet), &ws);
    it.def_method(&ws, "add", 1, |it, c| {
        let o = this_coll(it, c, Kind::WeakSet)?;
        if !matches!(c.arg(0), Value::Obj(_) | Value::Sym(_)) {
            return it.type_error("Invalid value used in weak set");
        }
        with_data(&o, |d| d.set(c.arg(0), c.arg(0)));
        Ok(c.this.clone())
    });
    it.def_method(&ws, "has", 1, |it, c| {
        let o = this_coll(it, c, Kind::WeakSet)?;
        Ok(Value::Bool(with_data(&o, |d| d.has(&c.arg(0)))))
    });
    it.def_method(&ws, "delete", 1, |it, c| {
        let o = this_coll(it, c, Kind::WeakSet)?;
        Ok(Value::Bool(with_data(&o, |d| d.delete(&c.arg(0)))))
    });
    it.define_data(&ws, Key::Y(tag.0.id), Value::str("WeakSet"), CONFIGURABLE);

    // WeakRef / FinalizationRegistry (strong references: no GC in the engine)
    let wr = it.new_object();
    it.make_ctor("WeakRef", 1, |it, c| {
        let proto = it.protos().extra.get("weakref").cloned().unwrap();
        let p = it.proto_from_call(c, &proto)?;
        let o = it.alloc(Object::new(Some(p), Class::Plain));
        it.define_data(&o, Key::str("__target"), c.arg(0), 0);
        Ok(Value::Obj(o))
    }, &wr);
    it.def_method(&wr, "deref", 0, |it, c| it.get_str(&c.this, "__target"));
    it.protos.as_mut().unwrap().extra.insert("weakref", wr);
    let fr = it.new_object();
    it.make_ctor("FinalizationRegistry", 1, |it, c| {
        let proto = it.protos().extra.get("finreg").cloned().unwrap();
        let p = it.proto_from_call(c, &proto)?;
        Ok(Value::Obj(it.alloc(Object::new(Some(p), Class::Plain))))
    }, &fr);
    it.def_method(&fr, "register", 2, |_, _| Ok(Value::Undefined));
    it.def_method(&fr, "unregister", 1, |_, _| Ok(Value::Bool(false)));
    it.protos.as_mut().unwrap().extra.insert("finreg", fr);
}

fn common(it: &mut Interp, p: &Obj, kind: Kind) {
    let _ = kind;
    it.def_method(p, "has", 1, |it, c| {
        let o = this_any(it, c)?;
        Ok(Value::Bool(with_data(&o, |d| d.has(&c.arg(0)))))
    });
    it.def_method(p, "delete", 1, |it, c| {
        let o = this_any(it, c)?;
        Ok(Value::Bool(with_data(&o, |d| d.delete(&c.arg(0)))))
    });
    it.def_method(p, "clear", 0, |it, c| {
        let o = this_any(it, c)?;
        with_data(&o, |d| d.clear());
        Ok(Value::Undefined)
    });
    it.def_getter(p, "size", |it, c| {
        let o = this_any(it, c)?;
        Ok(Value::Num(with_data(&o, |d| d.live) as f64))
    });
    it.def_method(p, "forEach", 1, |it, c| {
        let o = this_any(it, c)?;
        let f = c.arg(0);
        let mut i = 0;
        loop {
            let e = with_data(&o, |d| {
                while i < d.entries.len() {
                    if let Some(e) = &d.entries[i] {
                        i += 1;
                        return Some(e.clone());
                    }
                    i += 1;
                }
                None
            });
            let Some((k, v)) = e else { break };
            it.call(&f, c.arg(1), &[v, k, c.this.clone()])?;
        }
        Ok(Value::Undefined)
    });
}

fn set_algebra(it: &mut Interp, sp: &Obj) {
    fn other_values(it: &mut Interp, v: &Value) -> Res<Vec<Value>> {
        if let Value::Obj(o) = v {
            if let Class::Set(d) | Class::Map(d) = &o.borrow().class {
                return Ok(d.entries.iter().flatten().map(|e| e.0.clone()).collect());
            }
        }
        let keys = it.get_str(v, "keys")?;
        let iter = it.call(&keys, v.clone(), &[])?;
        let mut out = Vec::new();
        while let Some(x) = it.iter_step(&iter)? {
            out.push(x);
        }
        Ok(out)
    }
    fn mine(it: &mut Interp, c: &Call) -> Res<Vec<Value>> {
        let o = this_coll(it, c, Kind::Set)?;
        Ok(with_data(&o, |d| d.entries.iter().flatten().map(|e| e.0.clone()).collect()))
    }
    fn new_set(it: &mut Interp, items: Vec<Value>) -> Value {
        let proto = it.protos().set.clone();
        let mut d = MapData::default();
        for v in items {
            d.set(v.clone(), v);
        }
        Value::Obj(it.alloc(Object::new(Some(proto), Class::Set(Box::new(d)))))
    }
    it.def_method(sp, "union", 1, |it, c| {
        let mut a = mine(it, c)?;
        a.extend(other_values(it, &c.arg(0))?);
        Ok(new_set(it, a))
    });
    it.def_method(sp, "intersection", 1, |it, c| {
        let a = mine(it, c)?;
        let b = other_values(it, &c.arg(0))?;
        Ok(new_set(it, a.into_iter().filter(|x| b.iter().any(|y| same_value_zero(x, y))).collect()))
    });
    it.def_method(sp, "difference", 1, |it, c| {
        let a = mine(it, c)?;
        let b = other_values(it, &c.arg(0))?;
        Ok(new_set(it, a.into_iter().filter(|x| !b.iter().any(|y| same_value_zero(x, y))).collect()))
    });
    it.def_method(sp, "symmetricDifference", 1, |it, c| {
        let a = mine(it, c)?;
        let b = other_values(it, &c.arg(0))?;
        let mut out: Vec<Value> = a.iter().filter(|x| !b.iter().any(|y| same_value_zero(x, y))).cloned().collect();
        out.extend(b.iter().filter(|x| !a.iter().any(|y| same_value_zero(x, y))).cloned());
        Ok(new_set(it, out))
    });
    it.def_method(sp, "isSubsetOf", 1, |it, c| {
        let a = mine(it, c)?;
        let b = other_values(it, &c.arg(0))?;
        Ok(Value::Bool(a.iter().all(|x| b.iter().any(|y| same_value_zero(x, y)))))
    });
    it.def_method(sp, "isSupersetOf", 1, |it, c| {
        let a = mine(it, c)?;
        let b = other_values(it, &c.arg(0))?;
        Ok(Value::Bool(b.iter().all(|x| a.iter().any(|y| same_value_zero(x, y)))))
    });
    it.def_method(sp, "isDisjointFrom", 1, |it, c| {
        let a = mine(it, c)?;
        let b = other_values(it, &c.arg(0))?;
        Ok(Value::Bool(!a.iter().any(|x| b.iter().any(|y| same_value_zero(x, y)))))
    });
}

fn with_data<T>(o: &Obj, f: impl FnOnce(&mut MapData) -> T) -> T {
    let mut b = o.borrow_mut();
    match &mut b.class {
        Class::Map(d) | Class::Set(d) | Class::WeakMap(d) | Class::WeakSet(d) => f(d),
        _ => f(&mut MapData::default()),
    }
}

fn this_any(it: &mut Interp, c: &Call) -> Res<Obj> {
    match &c.this {
        Value::Obj(o) if matches!(o.borrow().class, Class::Map(_) | Class::Set(_)) => Ok(o.clone()),
        _ => it.type_error("Method called on incompatible receiver"),
    }
}

fn this_coll(it: &mut Interp, c: &Call, kind: Kind) -> Res<Obj> {
    if let Value::Obj(o) = &c.this {
        let ok = match (&o.borrow().class, kind) {
            (Class::Map(_), Kind::Map) | (Class::Set(_), Kind::Set) | (Class::WeakMap(_), Kind::WeakMap) | (Class::WeakSet(_), Kind::WeakSet) => true,
            _ => false,
        };
        if ok {
            return Ok(o.clone());
        }
    }
    it.type_error("Method called on incompatible receiver")
}

fn construct(it: &mut Interp, c: &Call, kind: Kind) -> Res {
    if c.new_target.is_none() {
        return it.type_error("Constructor requires 'new'");
    }
    let (default, class) = match kind {
        Kind::Map => (it.protos().map.clone(), Class::Map(Box::default())),
        Kind::Set => (it.protos().set.clone(), Class::Set(Box::default())),
        Kind::WeakMap => (it.protos().weakmap.clone(), Class::WeakMap(Box::default())),
        Kind::WeakSet => (it.protos().weakset.clone(), Class::WeakSet(Box::default())),
    };
    let proto = it.proto_from_call(c, &default)?;
    let o = it.alloc(Object::new(Some(proto), class));
    let src = c.arg(0);
    if !src.is_nullish() {
        let items = it.iter_to_vec(&src)?;
        let this = Value::Obj(o.clone());
        for item in items {
            match kind {
                Kind::Map | Kind::WeakMap => {
                    if !matches!(item, Value::Obj(_)) {
                        let d = it.describe(&item);
                        return it.type_error(&alloc::format!("Iterator value {d} is not an entry object"));
                    }
                    let k = it.get_str(&item, "0")?;
                    let v = it.get_str(&item, "1")?;
                    let adder = it.get_str(&this, "set")?;
                    it.call(&adder, this.clone(), &[k, v])?;
                }
                _ => {
                    let adder = it.get_str(&this, "add")?;
                    it.call(&adder, this.clone(), &[item])?;
                }
            }
        }
    }
    Ok(Value::Obj(o))
}

fn make_iter(it: &mut Interp, c: &Call, kind: Kind, mode: u8) -> Res {
    let o = this_coll(it, c, kind)?;
    let proto = it.protos().extra.get("collection_iterator").cloned().unwrap();
    let io = it.alloc(Object::new(Some(proto), Class::Plain));
    it.define_data(&io, Key::str("__c"), Value::Obj(o), 0);
    it.define_data(&io, Key::str("__i"), Value::Num(0.0), WRITABLE);
    it.define_data(&io, Key::str("__m"), Value::Num(mode as f64), 0);
    Ok(Value::Obj(io))
}

fn coll_iter_next(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(io) = &c.this else { return it.type_error("next called on incompatible receiver") };
    let coll = it.get_str(&c.this, "__c")?;
    let Value::Obj(co) = coll else { return it.type_error("next called on incompatible receiver") };
    let mut i = match it.get_str(&c.this, "__i")? {
        Value::Num(n) => n as usize,
        _ => usize::MAX,
    };
    let mode = match it.get_str(&c.this, "__m")? {
        Value::Num(n) => n as u8,
        _ => 1,
    };
    let e = with_data(&co, |d| {
        while i < d.entries.len() {
            if let Some(e) = &d.entries[i] {
                i += 1;
                return Some(e.clone());
            }
            i += 1;
        }
        None
    });
    it.define_data(io, Key::str("__i"), Value::Num(i as f64), WRITABLE);
    match e {
        None => {
            it.define_data(io, Key::str("__i"), Value::Num(usize::MAX as f64), WRITABLE);
            Ok(it.make_iter_result(Value::Undefined, true))
        }
        Some((k, v)) => {
            let r = match mode {
                0 => k,
                1 => v,
                _ => it.new_array(vec![k, v]),
            };
            Ok(it.make_iter_result(r, false))
        }
    }
}
