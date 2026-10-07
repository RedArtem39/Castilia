//! Promise.
use super::*;
use alloc::vec;

pub fn setup(it: &mut Interp) {
    let proto = it.protos().promise.clone();
    let ctor = it.make_ctor("Promise", 1, promise_ctor, &proto);
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&proto, Key::Y(tag.0.id), Value::str("Promise"), CONFIGURABLE);
    it.def_method(&proto, "then", 2, |it, c| {
        let p = this_promise(it, c)?;
        let f = Some(c.arg(0)).filter(|v| v.as_obj().is_some_and(|o| o.is_callable()));
        let r = Some(c.arg(1)).filter(|v| v.as_obj().is_some_and(|o| o.is_callable()));
        Ok(Value::Obj(it.promise_then(&p, f, r)))
    });
    it.def_method(&proto, "catch", 1, |it, c| {
        let then = it.get_v(&c.this, &it.names.then.clone())?;
        it.call(&then, c.this.clone(), &[Value::Undefined, c.arg(0)])
    });
    it.def_method(&proto, "finally", 1, |it, c| {
        let f = c.arg(0);
        let then = it.get_v(&c.this, &it.names.then.clone())?;
        if !f.as_obj().is_some_and(|o| o.is_callable()) {
            return it.call(&then, c.this.clone(), &[f.clone(), f]);
        }
        let ok = it.new_native_data("", 1, |it, c| finally_cb(it, c, false), f.clone(), false);
        let err = it.new_native_data("", 1, |it, c| finally_cb(it, c, true), f, false);
        it.call(&then, c.this.clone(), &[Value::Obj(ok), Value::Obj(err)])
    });
    it.def_method(&ctor, "resolve", 1, |it, c| Ok(Value::Obj(it.promise_resolved(c.arg(0))?)));
    it.def_method(&ctor, "reject", 1, |it, c| {
        let p = it.new_promise();
        it.reject_promise(&p, c.arg(0));
        Ok(Value::Obj(p))
    });
    it.def_method(&ctor, "withResolvers", 0, |it, _| {
        let p = it.new_promise();
        let (res, rej) = it.resolving_functions(&p);
        let o = it.new_object();
        it.define_data(&o, Key::str("promise"), Value::Obj(p), ALL);
        it.define_data(&o, Key::str("resolve"), res, ALL);
        it.define_data(&o, Key::str("reject"), rej, ALL);
        Ok(Value::Obj(o))
    });
    it.def_method(&ctor, "all", 1, |it, c| combinator(it, c, 0));
    it.def_method(&ctor, "allSettled", 1, |it, c| combinator(it, c, 1));
    it.def_method(&ctor, "any", 1, |it, c| combinator(it, c, 2));
    it.def_method(&ctor, "race", 1, |it, c| {
        let p = it.new_promise();
        let (res, rej) = it.resolving_functions(&p);
        let items = match it.iter_to_vec(&c.arg(0)) {
            Ok(v) => v,
            Err(Ctl::Throw(e)) => {
                it.reject_promise(&p, e);
                return Ok(Value::Obj(p));
            }
            Err(e) => return Err(e),
        };
        for item in items {
            let ip = it.promise_resolved(item)?;
            it.promise_then(&ip, Some(res.clone()), Some(rej.clone()));
        }
        Ok(Value::Obj(p))
    });
    let g = it.global.clone();
    it.def_method(&g, "queueMicrotask", 1, |it, c| {
        let f = c.arg(0);
        it.microtasks.push_back(Job::Call(f, Value::Undefined, Vec::new()));
        Ok(Value::Undefined)
    });
}

fn this_promise(it: &mut Interp, c: &Call) -> Res<Obj> {
    match &c.this {
        Value::Obj(o) if matches!(o.borrow().class, Class::Promise(_)) => Ok(o.clone()),
        _ => it.type_error("Method Promise.prototype.then called on incompatible receiver"),
    }
}

fn promise_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_none() {
        return it.type_error("Promise constructor cannot be invoked without 'new'");
    }
    let exec = c.arg(0);
    if !exec.as_obj().is_some_and(|o| o.is_callable()) {
        let d = it.describe(&exec);
        return it.type_error(&alloc::format!("Promise resolver {d} is not a function"));
    }
    let default = it.protos().promise.clone();
    let proto = it.proto_from_call(c, &default)?;
    let p = it.new_promise();
    p.borrow_mut().proto = Some(proto);
    let (res, rej) = it.resolving_functions(&p);
    if let Err(Ctl::Throw(e)) = it.call(&exec, Value::Undefined, &[res, rej.clone()]) {
        it.call(&rej, Value::Undefined, &[e])?;
    }
    Ok(Value::Obj(p))
}

fn finally_cb(it: &mut Interp, c: &Call, rejected: bool) -> Res {
    let f = it.native_data(c);
    let r = it.call(&f, Value::Undefined, &[])?;
    let p = it.promise_resolved(r)?;
    let thunk = it.new_native_data("", 0, |it, c| Ok(it.native_data(c)), c.arg(0), false);
    let thrower = it.new_native_data("", 0, |it, c| Err(Ctl::Throw(it.native_data(c))), c.arg(0), false);
    let _ = rejected;
    let h = if rejected { thrower } else { thunk };
    Ok(Value::Obj(it.promise_then(&p, Some(Value::Obj(h)), None)))
}

fn arr_get(o: &Obj, i: usize) -> Value {
    match &o.borrow().class {
        Class::Array(a) => a.get(i).cloned().map(unhole).unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    }
}

fn arr_set(o: &Obj, i: usize, v: Value) {
    if let Class::Array(a) = &mut o.borrow_mut().class {
        if i >= a.len() {
            a.resize(i + 1, Value::Undefined);
        }
        a[i] = v;
    }
}

/// State object layout (array): [promise, results, remaining]. Callback data: [state, index, called, mode, kind]
fn combinator(it: &mut Interp, c: &Call, mode: u8) -> Res {
    let p = it.new_promise();
    let (res, rej) = it.resolving_functions(&p);
    let items = match it.iter_to_vec(&c.arg(0)) {
        Ok(v) => v,
        Err(Ctl::Throw(e)) => {
            it.reject_promise(&p, e);
            return Ok(Value::Obj(p));
        }
        Err(e) => return Err(e),
    };
    let results = it.new_array(vec![Value::Undefined; items.len()]);
    let Value::Obj(results_o) = &results else { unreachable!() };
    let state = it.new_array(vec![Value::Obj(p.clone()), results.clone(), Value::Num(items.len() as f64 + 1.0)]);
    let Value::Obj(state_o) = &state else { unreachable!() };
    for (i, item) in items.into_iter().enumerate() {
        arr_set(results_o, i, Value::Undefined);
        let ip = it.promise_resolved(item)?;
        let mk = |it: &mut Interp, kind: u8| {
            let d = it.new_array(vec![state.clone(), Value::Num(i as f64), Value::Bool(false), Value::Num(mode as f64), Value::Num(kind as f64)]);
            Value::Obj(it.new_native_data("", 1, elem_cb, d, false))
        };
        let (on_f, on_r) = match mode {
            0 => (mk(it, 0), rej.clone()),
            1 => (mk(it, 0), mk(it, 1)),
            _ => (res.clone(), mk(it, 1)),
        };
        it.promise_then(&ip, Some(on_f), Some(on_r));
    }
    // drop the extra "1"
    if let Value::Num(n) = arr_get(state_o, 2) {
        arr_set(state_o, 2, Value::Num(n - 1.0));
        if n - 1.0 == 0.0 {
            finish(it, state_o, mode)?;
        }
    }
    Ok(Value::Obj(p))
}

fn finish(it: &mut Interp, state: &Obj, mode: u8) -> Res<()> {
    let Value::Obj(p) = arr_get(state, 0) else { return Ok(()) };
    let results = arr_get(state, 1);
    if mode == 2 {
        let e = it.make_error("AggregateError", "All promises were rejected");
        it.set_v(&e, Key::str("errors"), results)?;
        it.reject_promise(&p, e);
    } else {
        it.resolve_promise(&p, results)?;
    }
    Ok(())
}

fn elem_cb(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(d) = it.native_data(c) else { return Ok(Value::Undefined) };
    let Value::Obj(state) = arr_get(&d, 0) else { return Ok(Value::Undefined) };
    let idx = match arr_get(&d, 1) {
        Value::Num(n) => n as usize,
        _ => 0,
    };
    if arr_get(&d, 2).truthy() {
        return Ok(Value::Undefined);
    }
    arr_set(&d, 2, Value::Bool(true));
    let mode = match arr_get(&d, 3) {
        Value::Num(n) => n as u8,
        _ => 0,
    };
    let kind = match arr_get(&d, 4) {
        Value::Num(n) => n as u8,
        _ => 0,
    };
    let Value::Obj(results) = arr_get(&state, 1) else { return Ok(Value::Undefined) };
    let v = c.arg(0);
    let stored = if mode == 1 {
        let o = it.new_object();
        if kind == 0 {
            it.define_data(&o, Key::str("status"), Value::str("fulfilled"), ALL);
            it.define_data(&o, Key::str("value"), v, ALL);
        } else {
            it.define_data(&o, Key::str("status"), Value::str("rejected"), ALL);
            it.define_data(&o, Key::str("reason"), v, ALL);
        }
        Value::Obj(o)
    } else {
        v
    };
    arr_set(&results, idx, stored);
    if let Value::Num(n) = arr_get(&state, 2) {
        arr_set(&state, 2, Value::Num(n - 1.0));
        if n - 1.0 == 0.0 {
            finish(it, &state, mode)?;
        }
    }
    Ok(Value::Undefined)
}
