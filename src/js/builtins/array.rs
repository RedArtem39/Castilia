//! Array and array iterators.
use super::*;
use alloc::{format, vec};

pub fn setup(it: &mut Interp) {
    let proto = it.protos().array.clone();
    let ctor = it.make_ctor("Array", 1, array_ctor, &proto);
    it.def_method(&ctor, "isArray", 1, |_, c| Ok(Value::Bool(matches!(c.arg(0), Value::Obj(o) if o.is_array()))));
    it.def_method(&ctor, "of", 0, |it, c| Ok(it.new_array(c.args.to_vec())));
    it.def_method(&ctor, "from", 1, array_from);
    let species = it.wk().species.clone();
    let _ = species;

    let methods: &[(&str, u32, NativeFn)] = &[
        ("push", 1, push),
        ("pop", 0, pop),
        ("shift", 0, shift),
        ("unshift", 1, unshift),
        ("slice", 2, slice),
        ("splice", 2, splice),
        ("concat", 1, concat),
        ("join", 1, join),
        ("reverse", 0, reverse),
        ("indexOf", 1, index_of),
        ("lastIndexOf", 1, last_index_of),
        ("includes", 1, includes),
        ("find", 1, |it, c| find_impl(it, c, false, false)),
        ("findIndex", 1, |it, c| find_impl(it, c, true, false)),
        ("findLast", 1, |it, c| find_impl(it, c, false, true)),
        ("findLastIndex", 1, |it, c| find_impl(it, c, true, true)),
        ("filter", 1, filter),
        ("map", 1, map),
        ("forEach", 1, for_each),
        ("some", 1, some),
        ("every", 1, every),
        ("reduce", 1, |it, c| reduce_impl(it, c, false)),
        ("reduceRight", 1, |it, c| reduce_impl(it, c, true)),
        ("sort", 1, sort),
        ("flat", 0, flat),
        ("flatMap", 1, flat_map),
        ("fill", 1, fill),
        ("at", 1, at),
        ("copyWithin", 2, copy_within),
        ("toString", 0, to_string),
        ("toLocaleString", 0, to_string),
        ("toSorted", 1, to_sorted),
        ("toReversed", 0, to_reversed),
        ("toSpliced", 2, to_spliced),
        ("with", 2, with),
        ("keys", 0, |it, c| make_array_iter(it, c, 0)),
        ("entries", 0, |it, c| make_array_iter(it, c, 2)),
    ];
    for (n, l, f) in methods {
        it.def_method(&proto, n, *l, *f);
    }
    let values = it.new_native("values", 0, |it, c| make_array_iter(it, c, 1));
    it.define_data(&proto, Key::str("values"), Value::Obj(values.clone()), HIDDEN);
    let iter_sym = it.wk().iterator.clone();
    it.define_data(&proto, Key::Y(iter_sym.0.id), Value::Obj(values), HIDDEN);

    let ai = it.protos().array_iterator.clone();
    it.def_method(&ai, "next", 0, array_iter_next);
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&ai, Key::Y(tag.0.id), Value::str("Array Iterator"), CONFIGURABLE);
    // %IteratorPrototype%[Symbol.iterator]
    let ip = it.protos().iterator.clone();
    it.def_sym_method(&ip, &iter_sym, "[Symbol.iterator]", 0, |_, c| Ok(c.this.clone()));
    let gp = it.protos().generator.clone();
    it.def_method(&gp, "next", 1, gen_next);
    it.def_method(&gp, "return", 1, |it, c| {
        if let Value::Obj(o) = &c.this {
            if it.vm_gen_state(o).is_some() {
                return it.vm_gen_resume(o, crate::js::vm::Resume::Return, c.arg(0));
            }
            if let Class::Generator(d) = &mut o.borrow_mut().class {
                d.started = true;
                d.pending = None;
                d.pos = usize::MAX;
            }
        }
        Ok(it.make_iter_result(c.arg(0), true))
    });
    it.def_method(&gp, "throw", 1, |it, c| {
        if let Value::Obj(o) = &c.this {
            if it.vm_gen_state(o).is_some() {
                return it.vm_gen_resume(o, crate::js::vm::Resume::Throw, c.arg(0));
            }
        }
        Err(Ctl::Throw(c.arg(0)))
    });
    it.define_data(&gp, Key::Y(tag.0.id), Value::str("Generator"), CONFIGURABLE);
    // %AsyncGeneratorPrototype% and %AsyncIteratorPrototype%
    let op = it.protos().object.clone();
    let aip = it.alloc(Object::new(Some(op), Class::Plain));
    let async_iter = it.wk().async_iterator.clone();
    it.def_sym_method(&aip, &async_iter, "[Symbol.asyncIterator]", 0, |_, c| Ok(c.this.clone()));
    let agp = it.alloc(Object::new(Some(aip), Class::Plain));
    for (name, mode) in [("next", 0u8), ("return", 1), ("throw", 2)] {
        let f: NativeFn = match mode {
            0 => |it, c| async_gen_method(it, c, crate::js::vm::Resume::Next),
            1 => |it, c| async_gen_method(it, c, crate::js::vm::Resume::Return),
            _ => |it, c| async_gen_method(it, c, crate::js::vm::Resume::Throw),
        };
        it.def_method(&agp, name, 1, f);
    }
    it.define_data(&agp, Key::Y(tag.0.id), Value::str("AsyncGenerator"), CONFIGURABLE);
    it.protos.as_mut().expect("realm").extra.insert("async_generator", agp);
}

fn array_ctor(it: &mut Interp, c: &Call) -> Res {
    let proto = it.protos().array.clone();
    let p = it.proto_from_call(c, &proto)?;
    let items = if c.args.len() == 1 {
        if let Value::Num(n) = c.args[0] {
            if n < 0.0 || n != crate::math::trunc(n) || n > 4294967295.0 {
                return Err(it.error_ctl("RangeError", "Invalid array length"));
            }
            if n > 50_000_000.0 {
                return Err(it.error_ctl("RangeError", "Invalid array length"));
            }
            vec![hole(); n as usize]
        } else {
            c.args.to_vec()
        }
    } else {
        c.args.to_vec()
    };
    Ok(Value::Obj(it.alloc(Object::new(Some(p), Class::Array(items)))))
}

fn array_from(it: &mut Interp, c: &Call) -> Res {
    let src = c.arg(0);
    let map_fn = c.arg(1);
    let this_arg = c.arg(2);
    let iter_key = Key::Y(it.wk().iterator.0.id);
    let has_iter = !src.is_nullish() && !it.get_v(&src, &iter_key)?.is_nullish();
    let items = if has_iter {
        it.iter_to_vec(&src)?
    } else if src.is_nullish() {
        return it.type_error("undefined is not iterable");
    } else {
        it.array_like_to_vec(&src)?
    };
    let items = if map_fn.is_undefined() {
        items
    } else {
        let mut out = Vec::with_capacity(items.len());
        for (i, v) in items.into_iter().enumerate() {
            out.push(it.call(&map_fn, this_arg.clone(), &[v, Value::Num(i as f64)])?);
        }
        out
    };
    // `Array.from` called on a subclass constructor
    if let Value::Obj(ctor) = &c.this {
        if !ctor.ptr_eq(c.callee) && ctor.is_callable() {
            let o = it.construct(&c.this, &[], None)?;
            if let Value::Obj(oo) = &o {
                let mut b = oo.borrow_mut();
                if let Class::Array(a) = &mut b.class {
                    *a = items;
                    drop(b);
                    return Ok(o);
                }
            }
        }
    }
    Ok(it.new_array(items))
}

// ---- helpers ----

fn list(it: &mut Interp, c: &Call) -> Res<Vec<Value>> {
    list_raw(it, c).map(unhole_vec)
}

/// Elements with holes kept (for the methods that skip missing elements).
fn list_raw(it: &mut Interp, c: &Call) -> Res<Vec<Value>> {
    if let Value::Obj(o) = &c.this {
        if let Class::Array(a) = &o.borrow().class {
            return Ok(a.clone());
        }
    }
    if c.this.is_nullish() {
        return it.type_error("Array.prototype method called on null or undefined");
    }
    it.array_like_to_vec(&c.this)
}

fn arr_len(c: &Call) -> Option<usize> {
    if let Value::Obj(o) = &c.this {
        if let Class::Array(a) = &o.borrow().class {
            return Some(a.len());
        }
    }
    None
}

fn rel(n: f64, len: usize) -> usize {
    let n = to_integer(n);
    if n < 0.0 { (len as f64 + n).max(0.0) as usize } else { n.min(len as f64) as usize }
}

fn rel_arg(it: &mut Interp, c: &Call, i: usize, len: usize, default: usize) -> Res<usize> {
    match c.arg(i) {
        Value::Undefined => Ok(default),
        v => {
            let n = it.to_number(&v)?;
            Ok(rel(n, len))
        }
    }
}

fn check_fn(it: &mut Interp, v: &Value) -> Res<()> {
    if matches!(v, Value::Obj(o) if o.is_callable()) {
        Ok(())
    } else {
        let d = it.describe(v);
        it.type_error(&format!("{d} is not a function"))
    }
}

fn set_arr(this: &Value, items: Vec<Value>) {
    if let Value::Obj(o) = this {
        if let Class::Array(a) = &mut o.borrow_mut().class {
            *a = items;
        }
    }
}

fn require_array(it: &mut Interp, c: &Call) -> Res<Obj> {
    match &c.this {
        Value::Obj(o) if matches!(o.borrow().class, Class::Array(_)) => Ok(o.clone()),
        _ => it.type_error("operation requires an array"),
    }
}

fn is_array(v: &Value) -> bool {
    matches!(v, Value::Obj(o) if matches!(o.borrow().class, Class::Array(_)))
}

/// Run an Array-only mutator on an array-like receiver (`Array.prototype.splice.call(arguments)`,
/// NodeList, plain objects with `length`): operate on a copy, then write elements and `length` back.
fn generic_mutator(it: &mut Interp, c: &Call, f: fn(&mut Interp, &Call) -> Res) -> Res {
    if c.this.is_nullish() {
        return it.type_error("Array.prototype method called on null or undefined");
    }
    let items = it.array_like_to_vec(&c.this)?;
    let old_len = items.len();
    let tmp = it.new_array(items);
    let r = f(it, &Call { this: tmp.clone(), args: c.args, callee: c.callee, new_target: None })?;
    let new_items = match &tmp {
        Value::Obj(o) => match &o.borrow().class {
            Class::Array(a) => a.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    let new_len = new_items.len();
    for (i, v) in new_items.into_iter().enumerate() {
        if is_hole(&v) {
            it.delete_v(&c.this, &Key::S(Rc::from(num_to_string(i as f64))))?;
        } else {
            it.set_v(&c.this, Key::S(Rc::from(num_to_string(i as f64))), v)?;
        }
    }
    for i in new_len..old_len {
        it.delete_v(&c.this, &Key::S(Rc::from(num_to_string(i as f64))))?;
    }
    it.set_v(&c.this, Key::str("length"), Value::Num(new_len as f64))?;
    let same = matches!((&r, &tmp), (Value::Obj(a), Value::Obj(b)) if a.ptr_eq(b));
    Ok(if same { c.this.clone() } else { r })
}

// ---- mutators ----

fn push(it: &mut Interp, c: &Call) -> Res {
    if let Value::Obj(o) = &c.this {
        let mut b = o.borrow_mut();
        if !b.extensible {
            drop(b);
            return it.type_error("Cannot add property, object is not extensible");
        }
        if let Class::Array(a) = &mut b.class {
            a.extend(c.args.iter().cloned());
            return Ok(Value::Num(a.len() as f64));
        }
    }
    // generic array-like
    let len = it.get_v(&c.this, &Key::str("length"))?;
    let mut n = it.to_number(&len)?;
    for v in c.args {
        it.set_v(&c.this, Key::S(Rc::from(num_to_string(n))), v.clone())?;
        n += 1.0;
    }
    it.set_v(&c.this, Key::str("length"), Value::Num(n))?;
    Ok(Value::Num(n))
}

fn pop(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, pop);
    }
    let o = require_array(it, c)?;
    if let Class::Array(a) = &mut o.borrow_mut().class {
        return Ok(a.pop().unwrap_or(Value::Undefined));
    }
    Ok(Value::Undefined)
}

fn shift(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, shift);
    }
    let o = require_array(it, c)?;
    if let Class::Array(a) = &mut o.borrow_mut().class {
        if a.is_empty() {
            return Ok(Value::Undefined);
        }
        return Ok(a.remove(0));
    }
    Ok(Value::Undefined)
}

fn unshift(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, unshift);
    }
    let o = require_array(it, c)?;
    if let Class::Array(a) = &mut o.borrow_mut().class {
        for (i, v) in c.args.iter().enumerate() {
            a.insert(i, v.clone());
        }
        return Ok(Value::Num(a.len() as f64));
    }
    Ok(Value::Undefined)
}

fn splice(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, splice);
    }
    let o = require_array(it, c)?;
    let len = arr_len(c).unwrap_or(0);
    let start = rel_arg(it, c, 0, len, 0)?;
    let del = if c.args.is_empty() {
        0
    } else if c.args.len() == 1 {
        len - start
    } else {
        let n = it.arg_num(c, 1)?;
        (to_integer(n).max(0.0) as usize).min(len - start)
    };
    let ins: Vec<Value> = c.args.iter().skip(2).cloned().collect();
    let removed: Vec<Value> = if let Class::Array(a) = &mut o.borrow_mut().class {
        a.splice(start..start + del, ins).collect()
    } else {
        Vec::new()
    };
    Ok(it.new_array(removed))
}

fn reverse(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, reverse);
    }
    let o = require_array(it, c)?;
    if let Class::Array(a) = &mut o.borrow_mut().class {
        a.reverse();
    }
    Ok(c.this.clone())
}

fn fill(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, fill);
    }
    let o = require_array(it, c)?;
    let len = arr_len(c).unwrap_or(0);
    let s = rel_arg(it, c, 1, len, 0)?;
    let e = rel_arg(it, c, 2, len, len)?;
    if let Class::Array(a) = &mut o.borrow_mut().class {
        for i in s..e.min(a.len()) {
            a[i] = c.arg(0);
        }
    }
    Ok(c.this.clone())
}

fn copy_within(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, copy_within);
    }
    let o = require_array(it, c)?;
    let len = arr_len(c).unwrap_or(0);
    let target = rel_arg(it, c, 0, len, 0)?;
    let s = rel_arg(it, c, 1, len, 0)?;
    let e = rel_arg(it, c, 2, len, len)?;
    if let Class::Array(a) = &mut o.borrow_mut().class {
        let chunk: Vec<Value> = a[s.min(e)..e].to_vec();
        for (i, v) in chunk.into_iter().enumerate() {
            if target + i < a.len() {
                a[target + i] = v;
            }
        }
    }
    Ok(c.this.clone())
}

// ---- accessors ----

fn slice(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let len = items.len();
    let s = rel_arg(it, c, 0, len, 0)?;
    let e = rel_arg(it, c, 1, len, len)?;
    Ok(it.new_array(if s < e { items[s..e].to_vec() } else { Vec::new() }))
}

fn concat(it: &mut Interp, c: &Call) -> Res {
    let mut out = list_raw(it, c)?;
    for a in c.args {
        match a {
            Value::Obj(o) if o.is_array() => {
                let raw = match &o.borrow().class {
                    Class::Array(v) => Some(v.clone()),
                    _ => None,
                };
                match raw {
                    Some(v) => out.extend(v),
                    None => out.extend(it.array_like_to_vec(a)?),
                }
            }
            v => out.push(v.clone()),
        }
    }
    Ok(it.new_array(out))
}

fn join(it: &mut Interp, c: &Call) -> Res {
    let items = list(it, c)?;
    let sep: JsStr = match c.arg(0) {
        Value::Undefined => Rc::from(","),
        v => it.to_str(&v)?,
    };
    // cycle protection
    if let Value::Obj(o) = &c.this {
        let id = o.id();
        if it.join_stack.contains(&id) {
            return Ok(Value::str(""));
        }
        it.join_stack.push(id);
    }
    let mut out = String::new();
    let mut err = None;
    for (i, v) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(&sep);
        }
        if !v.is_nullish() {
            match it.to_str(v) {
                Ok(s) => out.push_str(&s),
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
    }
    if let Value::Obj(_) = &c.this {
        it.join_stack.pop();
    }
    if let Some(e) = err {
        return Err(e);
    }
    Ok(Value::string(out))
}

fn to_string(it: &mut Interp, c: &Call) -> Res {
    let j = it.get_v(&c.this, &Key::str("join"))?;
    if j.as_obj().is_some_and(|o| o.is_callable()) {
        it.call(&j, c.this.clone(), &[])
    } else {
        Ok(Value::str("[object Array]"))
    }
}

fn index_of(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let from = rel_arg(it, c, 1, items.len(), 0)?;
    let t = c.arg(0);
    for (i, v) in items.iter().enumerate().skip(from) {
        if strict_equals(v, &t) {
            return Ok(Value::Num(i as f64));
        }
    }
    Ok(Value::Num(-1.0))
}

fn last_index_of(it: &mut Interp, c: &Call) -> Res {
    let items = list(it, c)?;
    let t = c.arg(0);
    let from = if c.args.len() > 1 {
        let n = to_integer(it.arg_num(c, 1)?);
        if n < 0.0 { items.len() as f64 + n } else { n.min(items.len() as f64 - 1.0) }
    } else {
        items.len() as f64 - 1.0
    };
    if from < 0.0 {
        return Ok(Value::Num(-1.0));
    }
    for i in (0..=from as usize).rev() {
        if strict_equals(&items[i], &t) {
            return Ok(Value::Num(i as f64));
        }
    }
    Ok(Value::Num(-1.0))
}

fn includes(it: &mut Interp, c: &Call) -> Res {
    let items = list(it, c)?;
    let from = rel_arg(it, c, 1, items.len(), 0)?;
    let t = c.arg(0);
    Ok(Value::Bool(items.iter().skip(from).any(|v| same_value_zero(v, &t))))
}

fn at(it: &mut Interp, c: &Call) -> Res {
    let items = list(it, c)?;
    let n = to_integer(it.arg_num(c, 0)?);
    let i = if n < 0.0 { items.len() as f64 + n } else { n };
    if i < 0.0 || i >= items.len() as f64 {
        return Ok(Value::Undefined);
    }
    Ok(items[i as usize].clone())
}

// ---- iteration ----

fn find_impl(it: &mut Interp, c: &Call, want_index: bool, from_end: bool) -> Res {
    let items = list(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    let this_arg = c.arg(1);
    let n = items.len();
    for k in 0..n {
        let i = if from_end { n - 1 - k } else { k };
        let v = items[i].clone();
        if it.call(&f, this_arg.clone(), &[v.clone(), Value::Num(i as f64), c.this.clone()])?.truthy() {
            return Ok(if want_index { Value::Num(i as f64) } else { v });
        }
    }
    Ok(if want_index { Value::Num(-1.0) } else { Value::Undefined })
}

fn filter(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    let mut out = Vec::new();
    for (i, v) in items.into_iter().enumerate() {
        if is_hole(&v) {
            continue;
        }
        if it.call(&f, c.arg(1), &[v.clone(), Value::Num(i as f64), c.this.clone()])?.truthy() {
            out.push(v);
        }
    }
    Ok(it.new_array(out))
}

fn map(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    let mut out = Vec::with_capacity(items.len());
    for (i, v) in items.into_iter().enumerate() {
        if is_hole(&v) {
            out.push(v);
            continue;
        }
        out.push(it.call(&f, c.arg(1), &[v, Value::Num(i as f64), c.this.clone()])?);
    }
    Ok(it.new_array(out))
}

fn for_each(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    for (i, v) in items.into_iter().enumerate() {
        if is_hole(&v) {
            continue;
        }
        it.call(&f, c.arg(1), &[v, Value::Num(i as f64), c.this.clone()])?;
    }
    Ok(Value::Undefined)
}

fn some(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    for (i, v) in items.into_iter().enumerate() {
        if is_hole(&v) {
            continue;
        }
        if it.call(&f, c.arg(1), &[v, Value::Num(i as f64), c.this.clone()])?.truthy() {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn every(it: &mut Interp, c: &Call) -> Res {
    let items = list_raw(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    for (i, v) in items.into_iter().enumerate() {
        if is_hole(&v) {
            continue;
        }
        if !it.call(&f, c.arg(1), &[v, Value::Num(i as f64), c.this.clone()])?.truthy() {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn reduce_impl(it: &mut Interp, c: &Call, right: bool) -> Res {
    let mut items = list_raw(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    let n = items.len();
    let idx: Vec<usize> = if right { (0..n).rev().filter(|&i| !is_hole(&items[i])).collect() } else { (0..n).filter(|&i| !is_hole(&items[i])).collect() };
    let mut iter = idx.into_iter();
    let mut acc = if c.args.len() > 1 {
        c.args[1].clone()
    } else {
        match iter.next() {
            Some(i) => core::mem::replace(&mut items[i], Value::Undefined),
            None => return it.type_error("Reduce of empty array with no initial value"),
        }
    };
    for i in iter {
        let v = items[i].clone();
        acc = it.call(&f, Value::Undefined, &[acc, v, Value::Num(i as f64), c.this.clone()])?;
    }
    Ok(acc)
}

// ---- sort ----

pub fn sort_values(it: &mut Interp, items: Vec<Value>, cmp: &Value) -> Res<Vec<Value>> {
    let mut undef = 0;
    let mut vals: Vec<Value> = Vec::with_capacity(items.len());
    for v in items {
        if v.is_undefined() {
            undef += 1;
        } else {
            vals.push(v);
        }
    }
    // pre-stringify for default comparison
    let mut keyed: Vec<(Option<JsStr>, Value)> = Vec::with_capacity(vals.len());
    for v in vals {
        let k = if cmp.is_undefined() { Some(it.to_str(&v)?) } else { None };
        keyed.push((k, v));
    }
    let sorted = merge_sort(it, keyed, cmp)?;
    let mut out: Vec<Value> = sorted.into_iter().map(|x| x.1).collect();
    for _ in 0..undef {
        out.push(Value::Undefined);
    }
    Ok(out)
}

fn compare(it: &mut Interp, a: &(Option<JsStr>, Value), b: &(Option<JsStr>, Value), cmp: &Value) -> Res<bool> {
    // returns true if b should come before a (i.e. a > b)
    if let (Some(x), Some(y)) = (&a.0, &b.0) {
        if x.is_ascii() && y.is_ascii() {
            return Ok(x.as_bytes() > y.as_bytes());
        }
        return Ok(x.encode_utf16().cmp(y.encode_utf16()) == core::cmp::Ordering::Greater);
    }
    let r = it.call(cmp, Value::Undefined, &[a.1.clone(), b.1.clone()])?;
    let n = it.to_number(&r)?;
    Ok(n > 0.0)
}

fn merge_sort(it: &mut Interp, mut v: Vec<(Option<JsStr>, Value)>, cmp: &Value) -> Res<Vec<(Option<JsStr>, Value)>> {
    let n = v.len();
    if n <= 1 {
        return Ok(v);
    }
    if n <= 8 {
        // insertion sort
        for i in 1..n {
            let mut j = i;
            while j > 0 && compare(it, &v[j - 1], &v[j], cmp)? {
                v.swap(j - 1, j);
                j -= 1;
            }
        }
        return Ok(v);
    }
    let right = v.split_off(n / 2);
    let l = merge_sort(it, v, cmp)?;
    let r = merge_sort(it, right, cmp)?;
    let mut out = Vec::with_capacity(n);
    let mut li = l.into_iter().peekable();
    let mut ri = r.into_iter().peekable();
    loop {
        match (li.peek(), ri.peek()) {
            (Some(a), Some(b)) => {
                if compare(it, a, b, cmp)? {
                    out.push(ri.next().unwrap());
                } else {
                    out.push(li.next().unwrap());
                }
            }
            (Some(_), None) => out.push(li.next().unwrap()),
            (None, Some(_)) => out.push(ri.next().unwrap()),
            (None, None) => break,
        }
    }
    Ok(out)
}

fn sort(it: &mut Interp, c: &Call) -> Res {
    if !is_array(&c.this) {
        return generic_mutator(it, c, sort);
    }
    let cmp = c.arg(0);
    if !cmp.is_undefined() {
        check_fn(it, &cmp)?;
    }
    let items = list(it, c)?;
    let sorted = sort_values(it, items, &cmp)?;
    set_arr(&c.this, sorted);
    Ok(c.this.clone())
}

fn to_sorted(it: &mut Interp, c: &Call) -> Res {
    let cmp = c.arg(0);
    let items = list(it, c)?;
    let sorted = sort_values(it, items, &cmp)?;
    Ok(it.new_array(sorted))
}

fn to_reversed(it: &mut Interp, c: &Call) -> Res {
    let mut items = list(it, c)?;
    items.reverse();
    Ok(it.new_array(items))
}

fn to_spliced(it: &mut Interp, c: &Call) -> Res {
    let mut items = list(it, c)?;
    let len = items.len();
    let start = rel_arg(it, c, 0, len, 0)?;
    let del = if c.args.is_empty() {
        0
    } else if c.args.len() == 1 {
        len - start
    } else {
        (to_integer(it.arg_num(c, 1)?).max(0.0) as usize).min(len - start)
    };
    let ins: Vec<Value> = c.args.iter().skip(2).cloned().collect();
    items.splice(start..start + del, ins);
    Ok(it.new_array(items))
}

fn with(it: &mut Interp, c: &Call) -> Res {
    let mut items = list(it, c)?;
    let n = to_integer(it.arg_num(c, 0)?);
    let i = if n < 0.0 { items.len() as f64 + n } else { n };
    if i < 0.0 || i >= items.len() as f64 {
        return Err(it.error_ctl("RangeError", "Invalid index"));
    }
    items[i as usize] = c.arg(1);
    Ok(it.new_array(items))
}

fn flatten(it: &mut Interp, items: Vec<Value>, depth: f64, out: &mut Vec<Value>) -> Res<()> {
    for v in items {
        match &v {
            Value::Obj(o) if o.is_array() && depth >= 1.0 => {
                let inner = it.array_like_to_vec(&v)?;
                flatten(it, inner, depth - 1.0, out)?;
            }
            _ => out.push(v),
        }
    }
    Ok(())
}

fn flat(it: &mut Interp, c: &Call) -> Res {
    let items = list(it, c)?;
    let depth = match c.arg(0) {
        Value::Undefined => 1.0,
        v => to_integer(it.to_number(&v)?),
    };
    let mut out = Vec::new();
    flatten(it, items, depth, &mut out)?;
    Ok(it.new_array(out))
}

fn flat_map(it: &mut Interp, c: &Call) -> Res {
    let items = list(it, c)?;
    let f = c.arg(0);
    check_fn(it, &f)?;
    let mut mapped = Vec::new();
    for (i, v) in items.into_iter().enumerate() {
        mapped.push(it.call(&f, c.arg(1), &[v, Value::Num(i as f64), c.this.clone()])?);
    }
    let mut out = Vec::new();
    flatten(it, mapped, 1.0, &mut out)?;
    Ok(it.new_array(out))
}

// ---- iterators ----

fn make_array_iter(it: &mut Interp, c: &Call, kind: u8) -> Res {
    let target = Value::Obj(it.to_object(&c.this)?);
    let proto = it.protos().array_iterator.clone();
    let o = it.alloc(Object::new(Some(proto), Class::Plain));
    it.define_data(&o, Key::str("__t"), target, 0);
    it.define_data(&o, Key::str("__i"), Value::Num(0.0), WRITABLE);
    it.define_data(&o, Key::str("__k"), Value::Num(kind as f64), 0);
    Ok(Value::Obj(o))
}

fn array_iter_next(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(o) = &c.this else { return it.type_error("next called on incompatible receiver") };
    let t = it.get_v(&c.this, &Key::str("__t"))?;
    if t.is_undefined() {
        // snapshot list iterator
        return list_iterator_next(it, c);
    }
    let i = match it.get_v(&c.this, &Key::str("__i"))? {
        Value::Num(n) => n as usize,
        _ => usize::MAX,
    };
    let kind = match it.get_v(&c.this, &Key::str("__k"))? {
        Value::Num(n) => n as u8,
        _ => 1,
    };
    let len = if let Value::Obj(to) = &t {
        match &to.borrow().class {
            Class::Array(a) => Some(a.len()),
            _ => None,
        }
    } else {
        None
    };
    let len = match len {
        Some(l) => l,
        None => {
            let l = it.get_v(&t, &Key::str("length"))?;
            to_integer(it.to_number(&l)?).max(0.0) as usize
        }
    };
    if i >= len {
        it.define_data(o, Key::str("__i"), Value::Num(usize::MAX as f64), WRITABLE);
        return Ok(it.make_iter_result(Value::Undefined, true));
    }
    it.define_data(o, Key::str("__i"), Value::Num((i + 1) as f64), WRITABLE);
    let v = match kind {
        0 => Value::Num(i as f64),
        _ => {
            let v = it.get_v(&t, &Key::S(Rc::from(i.to_string())))?;
            if kind == 2 { it.new_array(vec![Value::Num(i as f64), v]) } else { v }
        }
    };
    Ok(it.make_iter_result(v, false))
}

fn async_gen_method(it: &mut Interp, c: &Call, mode: crate::js::vm::Resume) -> Res {
    match &c.this {
        Value::Obj(o) if it.vm_gen_state(o).is_some() => it.vm_async_gen_request(o, mode, c.arg(0)),
        _ => {
            let p = it.new_promise();
            let e = it.make_error("TypeError", "not an async generator");
            it.reject_promise(&p, e);
            Ok(Value::Obj(p))
        }
    }
}

fn gen_next(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(o) = &c.this else { return it.type_error("next method called on incompatible receiver") };
    if it.vm_gen_state(o).is_some() {
        return it.vm_gen_resume(o, crate::js::vm::Resume::Next, c.arg(0));
    }
    it.gen_fill(o)?;
    let (v, done) = {
        let mut b = o.borrow_mut();
        let Class::Generator(d) = &mut b.class else {
            drop(b);
            return it.type_error("next method called on incompatible receiver");
        };
        if d.pos < d.values.len() {
            d.pos += 1;
            (d.values[d.pos - 1].clone(), false)
        } else if d.pos == usize::MAX {
            (Value::Undefined, true)
        } else {
            let r = core::mem::replace(&mut d.ret, Value::Undefined);
            d.pos = usize::MAX;
            (r, true)
        }
    };
    Ok(it.make_iter_result(v, done))
}
