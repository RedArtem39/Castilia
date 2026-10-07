//! The bytecode loop.
use super::{Code, Completion, Frame, FrameKind, Handler, Op, Outcome, Resume, ERROR_KINDS};
use crate::js::{
    ast::BinOp,
    interp::{Interp, Scope, VarMap},
    value::*,
};
use alloc::{boxed::Box, format, rc::Rc, string::String, vec, vec::Vec};
use core::cell::RefCell;

enum Step {
    Call(Box<Frame>),
    Ret(Value),
    Yield(Value),
    Await(Value),
}

/// Result of finishing a frame: value for the caller or an exception.
enum Finish {
    Value(Value),
    Throw(Value),
}

impl Interp {
    pub fn vm_run(&mut self, frame: Frame) -> Outcome {
        self.vm_loop(Box::new(frame))
    }

    pub fn vm_run_resumed(&mut self, frame: Box<Frame>) -> Outcome {
        self.vm_loop(frame)
    }

    fn vm_loop(&mut self, mut base: Box<Frame>) -> Outcome {
        let mut pending: Option<Ctl> = None;
        if let Some((mode, v)) = base.pending_resume.take() {
            let at_star = matches!(base.code.ops.get(base.pc), Some(Op::YieldStar(..)));
            if at_star {
                base.pending_resume = Some((mode, v));
            } else {
                match mode {
                    Resume::Throw => pending = Some(Ctl::Throw(v)),
                    Resume::Return => match Self::do_return(&mut base, v) {
                        Some(r) => {
                            // no finally blocks: complete right away
                            return self.finish_base(base, r);
                        }
                        None => {}
                    },
                    Resume::Next => base.stack.push(v),
                }
            }
        }
        let base_depth = self.vm_frames;
        let mut frames: Vec<Box<Frame>> = vec![base];
        self.vm_frames += 1;
        let out = loop {
            let res = match pending.take() {
                Some(c) => Err(c),
                None => {
                    let f = frames.last_mut().expect("frame");
                    self.exec_frame(f)
                }
            };
            match res {
                Ok(Step::Call(nf)) => {
                    if self.vm_frames >= self.max_frames {
                        pending = Some(self.error_ctl("RangeError", "Maximum call stack size exceeded"));
                        continue;
                    }
                    self.vm_frames += 1;
                    frames.push(nf);
                }
                Ok(Step::Ret(v)) => {
                    let f = frames.pop().expect("frame");
                    self.vm_frames -= 1;
                    let fin = self.finish_frame(&f, v);
                    match (frames.last_mut(), fin) {
                        (None, Finish::Value(v)) => break Outcome::Return(v),
                        (None, Finish::Throw(e)) => break Outcome::Throw(e),
                        (Some(caller), Finish::Value(v)) => caller.stack.push(v),
                        (Some(_), Finish::Throw(e)) => pending = Some(Ctl::Throw(e)),
                    }
                }
                Ok(Step::Yield(v)) => {
                    let f = frames.pop().expect("frame");
                    self.vm_frames -= 1;
                    if frames.is_empty() {
                        break Outcome::Yield(v, f);
                    }
                    // cannot happen: generator frames are never pushed by calls
                    pending = Some(self.error_ctl("SyntaxError", "yield outside generator"));
                }
                Ok(Step::Await(v)) => {
                    let f = frames.pop().expect("frame");
                    self.vm_frames -= 1;
                    if frames.is_empty() {
                        break Outcome::Await(v, f);
                    }
                    // an async function called from this loop: park it, the caller gets its promise
                    let p = f.promise.clone();
                    self.vm_park(v, f, None);
                    let caller = frames.last_mut().expect("caller");
                    caller.stack.push(p.map_or(Value::Undefined, Value::Obj));
                }
                Err(Ctl::Throw(e)) => {
                    // unwind to the nearest handler
                    let mut e = e;
                    loop {
                        let Some(f) = frames.last_mut() else { break };
                        if let Some(h) = f.handlers.pop() {
                            Self::restore(f, &h);
                            if h.finally {
                                f.completions.push(Completion::Throw(e.clone()));
                            } else {
                                f.stack.push(e.clone());
                            }
                            f.pc = h.target as usize;
                            break;
                        }
                        let f = frames.pop().expect("frame");
                        self.vm_frames -= 1;
                        if self.trace_errors {
                            let msg = self.error_text(&e);
                            let name = if f.code.name.is_empty() { "<anonymous>" } else { &f.code.name };
                            let op = f.code.ops.get(f.pc.saturating_sub(1)).copied();
                            let detail = match op {
                                Some(Op::GetProp(k) | Op::SetProp(k) | Op::GetMethod(k) | Op::InitProp(k)) => format!("{:?}", f.code.keys[k as usize]),
                                Some(Op::Call(_, d) | Op::New(_, d) | Op::CallSpread(d) | Op::NewSpread(d)) => self.callee_text(&f.code, d),
                                _ => String::new(),
                            };
                            let src = f.code.source.as_ref().map(|(s, (a, b))| {
                                let end = (*b).min(s.len()).min(a + 160);
                                let mut a = *a;
                                while a < end && !s.is_char_boundary(a) { a += 1; }
                                let mut e = end;
                                while e > a && !s.is_char_boundary(e) { e -= 1; }
                                String::from(&s[a..e])
                            }).unwrap_or_default();
                            let text = format!("unwind {msg} at {name} (op {} {:?} {detail}) src: {src}", f.pc.saturating_sub(1), op);
                            self.logs.push(crate::js::interp::LogEntry { level: "trace", text });
                        }
                        if f.kind == FrameKind::Async {
                            if let Some(p) = &f.promise {
                                self.reject_promise(p, e.clone());
                                let pv = Value::Obj(p.clone());
                                match frames.last_mut() {
                                    Some(caller) => {
                                        caller.stack.push(pv);
                                        break;
                                    }
                                    None => {
                                        frames.clear();
                                        e = pv;
                                        // async functions never throw to their caller
                                        self.vm_frames = base_depth;
                                        return Outcome::Return(e);
                                    }
                                }
                            }
                        }
                    }
                    if frames.is_empty() {
                        break Outcome::Throw(e);
                    }
                }
                Err(Ctl::GenStop) => {
                    // aborted by the AST walker (unsettled await): drop everything silently
                    frames.clear();
                    break Outcome::Throw(Value::Undefined);
                }
                Err(_) => {
                    pending = Some(self.error_ctl("SyntaxError", "illegal control flow"));
                }
            }
        };
        self.vm_frames = base_depth;
        out
    }

    fn finish_base(&mut self, f: Box<Frame>, v: Value) -> Outcome {
        match self.finish_frame(&f, v) {
            Finish::Value(v) => Outcome::Return(v),
            Finish::Throw(e) => Outcome::Throw(e),
        }
    }

    fn finish_frame(&mut self, f: &Frame, v: Value) -> Finish {
        match f.kind {
            FrameKind::Construct => {
                if let Value::Obj(_) = v {
                    return Finish::Value(v);
                }
                match self.this_value(&f.fenv) {
                    Ok(t) => Finish::Value(t),
                    Err(Ctl::Throw(e)) => Finish::Throw(e),
                    Err(_) => Finish::Value(Value::Undefined),
                }
            }
            FrameKind::Async => {
                let Some(p) = &f.promise else { return Finish::Value(Value::Undefined) };
                if let Err(Ctl::Throw(e)) = self.resolve_promise(p, v) {
                    self.reject_promise(p, e);
                }
                Finish::Value(Value::Obj(p.clone()))
            }
            _ => Finish::Value(v),
        }
    }

    fn restore(f: &mut Frame, h: &Handler) {
        f.stack.truncate(h.stack as usize);
        Self::restore_scopes(f, h.scopes as usize);
        f.completions.truncate(h.completions as usize);
    }

    fn restore_scopes(f: &mut Frame, depth: usize) {
        if f.scopes.len() > depth {
            f.env = f.scopes[depth].clone();
            f.scopes.truncate(depth);
        }
    }

    /// Return through pending `finally` blocks. `Some(v)` when the frame really returns.
    fn do_return(f: &mut Frame, v: Value) -> Option<Value> {
        while let Some(h) = f.handlers.pop() {
            if h.finally {
                Self::restore(f, &h);
                f.completions.push(Completion::Return(v));
                f.pc = h.target as usize;
                return None;
            }
        }
        Some(v)
    }

    fn do_leave(f: &mut Frame, target: u32, packed: u32) {
        let hd = (packed >> 16) as usize;
        let sd = (packed & 0xffff) as usize;
        while f.handlers.len() > hd {
            let h = f.handlers.pop().expect("handler");
            if h.finally {
                Self::restore(f, &h);
                f.completions.push(Completion::Jump(target, packed));
                f.pc = h.target as usize;
                return;
            }
        }
        Self::restore_scopes(f, sd);
        f.pc = target as usize;
    }

    fn callee_text(&self, code: &Code, d: u32) -> String {
        let raw = &code.strs[d as usize];
        if !raw.contains('\u{1}') {
            return String::from(&**raw);
        }
        let mut out = String::new();
        let mut parts = raw.split('\u{1}');
        if let Some(first) = parts.next() {
            out.push_str(first);
        }
        let mut is_name = true;
        for p in parts {
            if is_name {
                match p.parse::<u32>() {
                    Ok(id) => out.push_str(self.interner.get(id)),
                    Err(_) => out.push_str(p),
                }
            } else {
                out.push_str(p);
            }
            is_name = !is_name;
        }
        out
    }

    /// Set up an in-loop call of a VM closure, or `None` to call it the normal way.
    fn inline_call(&mut self, func: &Value, this: Value, args: Vec<Value>) -> Result<Result<Box<Frame>, (Value, Vec<Value>)>, Ctl> {
        let Value::Obj(fo) = func else { return Ok(Err((this, args))) };
        let info = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) if !c.is_class_ctor && !c.def.is_generator => Some((c.def.clone(), c.env.clone(), c.home.clone())),
            _ => None,
        };
        let Some((def, env, home)) = info else { return Ok(Err((this, args))) };
        let Some(code) = self.vm_code(&def) else { return Ok(Err((this, args))) };
        self.tick()?;
        let mut frame = self.vm_frame(code, fo, &def, &env, home, Some(this), &args, None);
        if frame.kind == FrameKind::Async {
            frame.promise = Some(self.new_promise());
        }
        Ok(Ok(Box::new(frame)))
    }

    /// `new` of a plain (non-derived) closure as an in-loop frame.
    fn inline_construct(&mut self, func: &Value, args: &[Value]) -> Result<Option<Box<Frame>>, Ctl> {
        let Value::Obj(fo) = func else { return Ok(None) };
        let info = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) if !c.derived && !c.def.is_arrow && !c.def.is_async && !c.def.is_generator && (c.is_class_ctor || matches!(c.def.kind, crate::js::ast::FnKind::Normal | crate::js::ast::FnKind::Ctor)) => {
                Some((c.def.clone(), c.env.clone(), c.home.clone(), c.fields.clone()))
            }
            _ => None,
        };
        let Some((def, env, home, fields)) = info else { return Ok(None) };
        let Some(code) = self.vm_code(&def) else { return Ok(None) };
        self.tick()?;
        let proto = self.proto_for(fo, false)?;
        let this = self.alloc(Object::new(Some(proto), Class::Plain));
        self.init_fields(&this, &fields)?;
        let frame = self.vm_frame(code, fo, &def, &env, home, Some(Value::Obj(this)), args, Some(fo.clone()));
        Ok(Some(Box::new(frame)))
    }

    fn array_values(&mut self, v: &Value) -> Vec<Value> {
        match v {
            Value::Obj(o) => match &o.borrow().class {
                Class::Array(a) => unhole_vec(a.clone()),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn arguments_object(&mut self, f: &Frame) -> Value {
        let argobj = self.new_array(f.args.clone());
        if let Value::Obj(o) = &argobj {
            let op = self.protos().object.clone();
            o.borrow_mut().proto = Some(op);
            let it = self.wk().iterator.clone();
            let values = self.get_obj(&self.protos().array.clone(), &Key::str("values"), &Value::Undefined).unwrap_or(Value::Undefined);
            self.define_data(o, Key::Y(it.0.id), values, HIDDEN);
            if let Some(c) = &f.callee {
                self.define_data(o, Key::str("callee"), Value::Obj(c.clone()), HIDDEN);
            }
            let len = f.args.len();
            self.define_data(o, self.names.length.clone(), Value::Num(len as f64), HIDDEN);
        }
        argobj
    }

    fn iter_init(&mut self, f: &mut Frame, r: usize, v: Value) -> Res<()> {
        if let Value::Obj(o) = &v {
            if matches!(o.borrow().class, Class::Array(_)) && self.array_iter_pristine_pub(o) {
                f.regs[r] = v.clone();
                f.regs[r + 1] = Value::Num(0.0);
                return Ok(());
            }
        }
        if let Value::Str(s) = &v {
            let chars: Vec<Value> = s.chars().map(|c| Value::string(String::from(c))).collect();
            f.regs[r] = self.new_array(chars);
            f.regs[r + 1] = Value::Num(0.0);
            return Ok(());
        }
        if v.is_nullish() {
            return Err(self.error_ctl("TypeError", &format!("{} is not iterable", self.describe(&v))));
        }
        let it = self.get_iterator(&v, false)?;
        let next = self.get_v(&it, &self.names.next.clone())?;
        f.regs[r] = it;
        f.regs[r + 1] = next;
        Ok(())
    }

    /// Next value, or None when exhausted (the state is marked done).
    fn iter_step_regs(&mut self, f: &mut Frame, r: usize) -> Res<Option<Value>> {
        match f.regs[r + 1].clone() {
            Value::Num(i) => {
                let i = i as usize;
                let v = match &f.regs[r] {
                    Value::Obj(o) => match &o.borrow().class {
                        Class::Array(a) => a.get(i).cloned(),
                        _ => None,
                    },
                    _ => None,
                };
                match v {
                    Some(v) => {
                        f.regs[r + 1] = Value::Num((i + 1) as f64);
                        Ok(Some(unhole(v)))
                    }
                    None => {
                        f.regs[r + 1] = Value::Undefined;
                        Ok(None)
                    }
                }
            }
            Value::Undefined => Ok(None),
            next => {
                let it = f.regs[r].clone();
                let res = self.call(&next, it, &[])?;
                if !matches!(res, Value::Obj(_)) {
                    f.regs[r + 1] = Value::Undefined;
                    return Err(self.error_ctl("TypeError", "Iterator result is not an object"));
                }
                let done = self.get_v(&res, &self.names.done.clone())?;
                if done.truthy() {
                    f.regs[r + 1] = Value::Undefined;
                    return Ok(None);
                }
                Ok(Some(self.get_v(&res, &self.names.value.clone())?))
            }
        }
    }

    fn iter_close_regs(&mut self, f: &mut Frame, r: usize) {
        if let Value::Obj(_) = &f.regs[r + 1] {
            let it = f.regs[r].clone();
            f.regs[r + 1] = Value::Undefined;
            self.iter_close(&it);
        }
    }

    #[inline(always)]
    fn pop(f: &mut Frame) -> Value {
        f.stack.pop().unwrap_or(Value::Undefined)
    }

    /// Execute instructions until the frame calls, returns or suspends.
    fn exec_frame(&mut self, f: &mut Frame) -> Result<Step, Ctl> {
        let code = f.code.clone();
        let ops = &code.ops[..];
        loop {
            let op = ops[f.pc];
            f.pc += 1;
            match op {
                Op::Nop | Op::Debugger => {}
                Op::Undef => f.stack.push(Value::Undefined),
                Op::Null => f.stack.push(Value::Null),
                Op::True => f.stack.push(Value::Bool(true)),
                Op::False => f.stack.push(Value::Bool(false)),
                Op::Int(i) => f.stack.push(Value::Num(i as f64)),
                Op::Const(i) => f.stack.push(code.consts[i as usize].clone()),
                Op::Pop => {
                    f.stack.pop();
                }
                Op::Dup => {
                    let v = f.stack.last().cloned().unwrap_or(Value::Undefined);
                    f.stack.push(v);
                }
                Op::Dup2 => {
                    let n = f.stack.len();
                    let a = f.stack[n - 2].clone();
                    let b = f.stack[n - 1].clone();
                    f.stack.push(a);
                    f.stack.push(b);
                }
                Op::Swap => {
                    let n = f.stack.len();
                    f.stack.swap(n - 1, n - 2);
                }
                Op::Rot3 => {
                    let v = Self::pop(f);
                    let n = f.stack.len();
                    f.stack.insert(n - 2, v);
                }
                Op::Rot4 => {
                    let v = Self::pop(f);
                    let n = f.stack.len();
                    f.stack.insert(n - 3, v);
                }
                Op::GetReg(r) => f.stack.push(f.regs[r as usize].clone()),
                Op::SetReg(r) => f.regs[r as usize] = Self::pop(f),
                Op::PutReg(r) => f.regs[r as usize] = f.stack.last().cloned().unwrap_or(Value::Undefined),
                Op::GetName(n) => {
                    let v = self.lookup(n, &f.env)?;
                    f.stack.push(v);
                }
                Op::TypeofName(n) => {
                    let v = self.lookup_typeof(n, &f.env)?;
                    f.stack.push(Value::str(v.type_of()));
                }
                Op::SetName(n) => {
                    let v = f.stack.last().cloned().unwrap_or(Value::Undefined);
                    self.assign_var(n, v, &f.env)?;
                }
                Op::DeclLet(n, k) => self.declare(&f.env, n, k, Value::Undefined, false),
                Op::InitLet(n, k) => {
                    let v = Self::pop(f);
                    f.env.vars.borrow_mut().insert(n, crate::js::interp::Var { v, kind: k, init: true });
                }
                Op::DeclVar(n) => {
                    if !f.env.vars.borrow().contains_key(&n) {
                        self.declare(&f.env, n, 0, Value::Undefined, true);
                    }
                }
                Op::InitVar(n) => {
                    let v = Self::pop(f);
                    self.assign_var(n, v, &f.env)?;
                }
                Op::SetVarOuter(n) => {
                    let v = Self::pop(f);
                    let set = {
                        let mut vars = f.fenv.vars.borrow_mut();
                        match vars.get_mut(&n) {
                            Some(var) if var.kind == 0 => {
                                var.v = v.clone();
                                true
                            }
                            _ => false,
                        }
                    };
                    if !set {
                        let fenv = f.fenv.clone();
                        self.assign_var(n, v, &fenv)?;
                    }
                }
                Op::InitGlobal(n) => {
                    let v = Self::pop(f);
                    let key = Key::S(self.interner.get(n).clone());
                    let g = self.global.clone();
                    self.define_data(&g, key, v, ALL);
                }
                Op::DeleteName(n) => {
                    let key = Key::S(self.interner.get(n).clone());
                    let g = self.global.clone();
                    let r = if self.has_own(&g, &key) { self.delete_prop(&g, &key)? } else { true };
                    f.stack.push(Value::Bool(r));
                }
                Op::PushScope => {
                    let s = self.new_scope(&f.env, None);
                    let old = core::mem::replace(&mut f.env, s);
                    f.scopes.push(old);
                }
                Op::PopScope => {
                    if let Some(e) = f.scopes.pop() {
                        f.env = e;
                    }
                }
                Op::CopyScope => {
                    if Rc::strong_count(&f.env) > 1 {
                        let parent = f.env.parent.clone();
                        let mut nv = VarMap::new();
                        for (k, v) in f.env.vars.borrow().entries() {
                            nv.insert(k, crate::js::interp::Var { v: v.v.clone(), kind: v.kind, init: v.init });
                        }
                        f.env = self.track_scope(Rc::new(Scope { vars: RefCell::new(nv), parent, func: None, with_obj: None }));
                    }
                }
                Op::PushWith => {
                    let v = Self::pop(f);
                    let o = self.to_object(&v)?;
                    let s = self.track_scope(Rc::new(Scope { vars: RefCell::new(VarMap::new()), parent: Some(f.env.clone()), func: None, with_obj: Some(o) }));
                    let old = core::mem::replace(&mut f.env, s);
                    f.scopes.push(old);
                }
                Op::This => {
                    let v = self.this_value(&f.env)?;
                    f.stack.push(v);
                }
                Op::NewTarget => {
                    let v = self.func_ctx_pub(&f.env).map_or(Value::Undefined, |c| c.new_target.clone());
                    f.stack.push(v);
                }
                Op::ImportMeta => {
                    let o = self.new_object();
                    let url = self.current_module_url();
                    self.define_data(&o, Key::str("url"), Value::string(url), ALL);
                    f.stack.push(Value::Obj(o));
                }
                Op::LoadArg(i) => {
                    let v = f.args.get(i as usize).cloned().unwrap_or(Value::Undefined);
                    f.stack.push(v);
                }
                Op::ArgsFrom(i) => {
                    let rest = f.args.get(i as usize..).map(|s| s.to_vec()).unwrap_or_default();
                    let a = self.new_array(rest);
                    f.stack.push(a);
                }
                Op::Arguments => {
                    let a = self.arguments_object(f);
                    f.stack.push(a);
                }
                Op::Callee => f.stack.push(f.callee.clone().map_or(Value::Undefined, Value::Obj)),
                Op::Closure(i) => {
                    let def = code.funcs[i as usize].clone();
                    let name = def.name.map(|n| self.interner.get(n).clone());
                    let o = self.make_closure(def, &f.env, None, name);
                    f.stack.push(Value::Obj(o));
                }
                Op::FuncExpr(i) => {
                    let def = code.funcs[i as usize].clone();
                    let n = def.name.expect("named function expression");
                    let scope = self.new_scope(&f.env, None);
                    let name = Some(self.interner.get(n).clone());
                    let o = self.make_closure(def, &scope, None, name);
                    self.declare(&scope, n, 0, Value::Obj(o.clone()), true);
                    f.stack.push(Value::Obj(o));
                }
                Op::ClosureNamed(i, n) => {
                    let def = code.funcs[i as usize].clone();
                    let name = Some(self.interner.get(n).clone());
                    let o = self.make_closure(def, &f.env, None, name);
                    f.stack.push(Value::Obj(o));
                }
                Op::ClosureStrName(i, s) => {
                    let def = code.funcs[i as usize].clone();
                    let o = self.make_closure(def, &f.env, None, Some(code.strs[s as usize].clone()));
                    f.stack.push(Value::Obj(o));
                }
                Op::Class(i, n) => {
                    let c = code.classes[i as usize].clone();
                    let name = if n == u32::MAX { None } else { Some(self.interner.get(n).clone()) };
                    let env = f.env.clone();
                    let v = self.eval_class(&c, &env, name)?;
                    f.stack.push(v);
                }
                Op::Regex(i) => {
                    let (p, fl) = code.regexes[i as usize].clone();
                    let v = self.new_regexp(&p, &fl)?;
                    f.stack.push(v);
                }
                Op::TemplateObj(i) => {
                    let cached = code.template_cache.borrow().get(i as usize).cloned().flatten();
                    let v = match cached {
                        Some(v) => v,
                        None => {
                            let t = code.templates[i as usize].clone();
                            let v = self.template_object_pub(&t);
                            let mut c = code.template_cache.borrow_mut();
                            if c.len() <= i as usize {
                                c.resize(i as usize + 1, None);
                            }
                            c[i as usize] = Some(v.clone());
                            v
                        }
                    };
                    f.stack.push(v);
                }
                Op::ToStr => {
                    let v = Self::pop(f);
                    let s = match v {
                        Value::Str(s) => s,
                        Value::Sym(_) => return Err(self.error_ctl("TypeError", "Cannot convert a Symbol value to a string")),
                        other => self.to_str(&other)?,
                    };
                    f.stack.push(Value::Str(s));
                }
                Op::Concat(n) => {
                    let start = f.stack.len() - n as usize;
                    let mut out = String::new();
                    for v in &f.stack[start..] {
                        if let Value::Str(s) = v {
                            out.push_str(s);
                        }
                    }
                    f.stack.truncate(start);
                    f.stack.push(Value::string(out));
                }
                Op::GetProp(k) => {
                    let o = Self::pop(f);
                    let v = self.get_v(&o, &code.keys[k as usize])?;
                    f.stack.push(v);
                }
                Op::GetMethod(k) => {
                    let o = f.stack.last().cloned().unwrap_or(Value::Undefined);
                    let v = self.get_v(&o, &code.keys[k as usize])?;
                    f.stack.push(v);
                }
                Op::GetElem => {
                    let k = Self::pop(f);
                    let o = Self::pop(f);
                    let v = self.get_elem(&o, &k)?;
                    f.stack.push(v);
                }
                Op::GetElemMethod => {
                    let k = Self::pop(f);
                    let o = f.stack.last().cloned().unwrap_or(Value::Undefined);
                    let v = self.get_elem(&o, &k)?;
                    f.stack.push(v);
                }
                Op::SetProp(k) => {
                    let v = Self::pop(f);
                    let o = Self::pop(f);
                    self.set_v(&o, code.keys[k as usize].clone(), v.clone())?;
                    f.stack.push(v);
                }
                Op::SetElem => {
                    let v = Self::pop(f);
                    let k = Self::pop(f);
                    let o = Self::pop(f);
                    if o.is_nullish() {
                        let ks = self.to_str(&k).unwrap_or_else(|_| Rc::from("?"));
                        return Err(self.error_ctl("TypeError", &format!("Cannot set properties of {} (setting '{}')", self.describe(&o), ks)));
                    }
                    self.set_member(&o, &k, v.clone())?;
                    f.stack.push(v);
                }
                Op::GetSuper(k) => {
                    let this = self.this_value(&f.env)?;
                    let home = self.func_ctx_pub(&f.env).and_then(|c| c.home.clone());
                    let v = self.super_get_pub(home, &code.keys[k as usize], &this)?;
                    f.stack.push(v);
                }
                Op::GetSuperElem => {
                    let kv = Self::pop(f);
                    let key = self.to_key(&kv)?;
                    let this = self.this_value(&f.env)?;
                    let home = self.func_ctx_pub(&f.env).and_then(|c| c.home.clone());
                    let v = self.super_get_pub(home, &key, &this)?;
                    f.stack.push(v);
                }
                Op::GetSuperMethod(k) => {
                    let this = self.this_value(&f.env)?;
                    let home = self.func_ctx_pub(&f.env).and_then(|c| c.home.clone());
                    let v = self.super_get_pub(home, &code.keys[k as usize], &this)?;
                    f.stack.push(this);
                    f.stack.push(v);
                }
                Op::SetSuper(k) => {
                    let v = Self::pop(f);
                    let this = self.this_value(&f.env)?;
                    self.set_v(&this, code.keys[k as usize].clone(), v.clone())?;
                    f.stack.push(v);
                }
                Op::SetSuperElem => {
                    let v = Self::pop(f);
                    let kv = Self::pop(f);
                    let key = self.to_key(&kv)?;
                    let this = self.this_value(&f.env)?;
                    self.set_v(&this, key, v.clone())?;
                    f.stack.push(v);
                }
                Op::DelProp(k) => {
                    let o = Self::pop(f);
                    let r = self.delete_v(&o, &code.keys[k as usize])?;
                    f.stack.push(r);
                }
                Op::DelElem => {
                    let kv = Self::pop(f);
                    let o = Self::pop(f);
                    let key = self.to_key(&kv)?;
                    let r = self.delete_v(&o, &key)?;
                    f.stack.push(r);
                }
                Op::Bin(op) => {
                    let r = Self::pop(f);
                    let l = Self::pop(f);
                    let v = match (&l, &r) {
                        (Value::Num(a), Value::Num(b)) => match op {
                            BinOp::Add => Value::Num(a + b),
                            BinOp::Sub => Value::Num(a - b),
                            BinOp::Mul => Value::Num(a * b),
                            BinOp::Div => Value::Num(a / b),
                            BinOp::Lt => Value::Bool(a < b),
                            BinOp::Le => Value::Bool(a <= b),
                            BinOp::Gt => Value::Bool(a > b),
                            BinOp::Ge => Value::Bool(a >= b),
                            BinOp::StrictEq | BinOp::Eq => Value::Bool(a == b),
                            BinOp::StrictNe | BinOp::Ne => Value::Bool(a != b),
                            _ => self.binary_op(op, &l, &r)?,
                        },
                        _ => match op {
                            BinOp::StrictEq => Value::Bool(strict_equals(&l, &r)),
                            BinOp::StrictNe => Value::Bool(!strict_equals(&l, &r)),
                            _ => self.binary_op(op, &l, &r)?,
                        },
                    };
                    f.stack.push(v);
                }
                Op::In => {
                    let r = Self::pop(f);
                    let l = Self::pop(f);
                    let v = self.binary_op(BinOp::In, &l, &r)?;
                    f.stack.push(v);
                }
                Op::Neg => {
                    let v = Self::pop(f);
                    let n = match v {
                        Value::Num(n) => n,
                        other => self.to_number(&other)?,
                    };
                    f.stack.push(Value::Num(-n));
                }
                Op::Pos => {
                    let v = Self::pop(f);
                    let n = match v {
                        Value::Num(n) => n,
                        other => self.to_number(&other)?,
                    };
                    f.stack.push(Value::Num(n));
                }
                Op::Not => {
                    let v = Self::pop(f);
                    f.stack.push(Value::Bool(!v.truthy()));
                }
                Op::BitNot => {
                    let v = Self::pop(f);
                    let n = self.to_number(&v)?;
                    f.stack.push(Value::Num(!to_int32(n) as f64));
                }
                Op::TypeOf => {
                    let v = Self::pop(f);
                    f.stack.push(Value::str(v.type_of()));
                }
                Op::Inc | Op::Dec => {
                    let d = if op == Op::Inc { 1.0 } else { -1.0 };
                    let v = Self::pop(f);
                    let n = match v {
                        Value::Num(n) => n,
                        other => self.to_number(&other)?,
                    };
                    f.stack.push(Value::Num(n + d));
                }
                Op::ToKey => {
                    let v = Self::pop(f);
                    let v = match v {
                        Value::Str(_) | Value::Sym(_) | Value::Num(_) => v,
                        other => {
                            let k = self.to_key(&other)?;
                            self.key_to_value(&k)
                        }
                    };
                    f.stack.push(v);
                }
                Op::NewObject => {
                    let o = self.new_object();
                    f.stack.push(Value::Obj(o));
                }
                Op::InitProp(k) => {
                    let v = Self::pop(f);
                    if let Some(Value::Obj(o)) = f.stack.last() {
                        self.define_data(o, code.keys[k as usize].clone(), v, ALL);
                    }
                }
                Op::InitElem => {
                    let v = Self::pop(f);
                    let kv = Self::pop(f);
                    let key = self.to_key(&kv)?;
                    if let Some(Value::Obj(o)) = f.stack.last() {
                        self.define_data(o, key, v, ALL);
                    }
                }
                Op::InitMethod(i, kind) => {
                    let kv = Self::pop(f);
                    let key = self.to_key(&kv)?;
                    let Some(Value::Obj(o)) = f.stack.last().cloned() else { continue };
                    let def = code.funcs[i as usize].clone();
                    if kind == 0 {
                        let name = crate::js::interp::key_to_name(&key);
                        let fo = self.make_closure(def, &f.env, Some(o.clone()), Some(name));
                        self.define_data(&o, key, Value::Obj(fo), ALL);
                    } else {
                        let fo = self.make_closure(def, &f.env, Some(o.clone()), None);
                        self.define_accessor_part(&o, key, Some(fo), kind == 1, ENUMERABLE | CONFIGURABLE);
                    }
                }
                Op::CopyProps => {
                    let src = Self::pop(f);
                    if let Some(Value::Obj(o)) = f.stack.last().cloned() {
                        self.copy_data_properties(&o, &src, &[])?;
                    }
                }
                Op::CopyPropsExcept => {
                    let src = Self::pop(f);
                    let keys = Self::pop(f);
                    let mut ex = Vec::new();
                    for k in self.array_values(&keys) {
                        ex.push(self.to_key(&k)?);
                    }
                    if let Some(Value::Obj(o)) = f.stack.last().cloned() {
                        self.copy_data_properties(&o, &src, &ex)?;
                    }
                }
                Op::SetProtoLit => {
                    let v = Self::pop(f);
                    if let Some(Value::Obj(o)) = f.stack.last() {
                        match v {
                            Value::Obj(p) => o.borrow_mut().proto = Some(p),
                            Value::Null => o.borrow_mut().proto = None,
                            _ => {}
                        }
                    }
                }
                Op::NewArray => {
                    let a = self.new_array(Vec::new());
                    f.stack.push(a);
                }
                Op::ArrPush | Op::ArrHole => {
                    let v = if op == Op::ArrPush { Self::pop(f) } else { hole() };
                    if let Some(Value::Obj(o)) = f.stack.last() {
                        if let Class::Array(a) = &mut o.borrow_mut().class {
                            a.push(v);
                        }
                    }
                }
                Op::ArrSpread => {
                    let v = Self::pop(f);
                    let items = self.iter_to_vec(&v)?;
                    if let Some(Value::Obj(o)) = f.stack.last() {
                        if let Class::Array(a) = &mut o.borrow_mut().class {
                            a.extend(items);
                        }
                    }
                }
                Op::Call(argc, d) => {
                    if let Some(step) = self.op_call(f, &code, argc, Some(d))? {
                        return Ok(step);
                    }
                }
                Op::CallEval(argc) => {
                    if let Some(step) = self.op_call(f, &code, argc, None)? {
                        return Ok(step);
                    }
                }
                Op::CallSpread(d) => {
                    let arr = Self::pop(f);
                    let args = self.array_values(&arr);
                    let func = Self::pop(f);
                    let this = Self::pop(f);
                    if !matches!(&func, Value::Obj(o) if o.is_callable()) {
                        let desc = self.callee_text(&code, d);
                        return Err(self.error_ctl("TypeError", &format!("{desc} is not a function")));
                    }
                    match self.inline_call(&func, this, args)? {
                        Ok(frame) => return Ok(Step::Call(frame)),
                        Err((this, args)) => {
                            let v = self.call(&func, this, &args)?;
                            f.stack.push(v);
                        }
                    }
                }
                Op::New(argc, d) => {
                    let start = f.stack.len() - argc as usize;
                    let args = f.stack.split_off(start);
                    let func = Self::pop(f);
                    if !matches!(&func, Value::Obj(o) if o.is_callable()) {
                        let desc = self.callee_text(&code, d);
                        return Err(self.error_ctl("TypeError", &format!("{desc} is not a constructor")));
                    }
                    if let Some(frame) = self.inline_construct(&func, &args)? {
                        return Ok(Step::Call(frame));
                    }
                    let v = self.construct(&func, &args, None)?;
                    f.stack.push(v);
                }
                Op::NewSpread(d) => {
                    let arr = Self::pop(f);
                    let args = self.array_values(&arr);
                    let func = Self::pop(f);
                    if !matches!(&func, Value::Obj(o) if o.is_callable()) {
                        let desc = self.callee_text(&code, d);
                        return Err(self.error_ctl("TypeError", &format!("{desc} is not a constructor")));
                    }
                    let v = self.construct(&func, &args, None)?;
                    f.stack.push(v);
                }
                Op::SuperCall(argc) => {
                    let start = f.stack.len() - argc as usize;
                    let args = f.stack.split_off(start);
                    let env = f.env.clone();
                    let v = self.super_call(&env, &args)?;
                    f.stack.push(v);
                }
                Op::SuperCallSpread => {
                    let arr = Self::pop(f);
                    let args = self.array_values(&arr);
                    let env = f.env.clone();
                    let v = self.super_call(&env, &args)?;
                    f.stack.push(v);
                }
                Op::Jump(t) => {
                    if (t as usize) < f.pc {
                        self.tick()?;
                    }
                    f.pc = t as usize;
                }
                Op::JumpIfFalse(t) => {
                    if !Self::pop(f).truthy() {
                        f.pc = t as usize;
                    }
                }
                Op::JumpIfTrue(t) => {
                    if Self::pop(f).truthy() {
                        if (t as usize) < f.pc {
                            self.tick()?;
                        }
                        f.pc = t as usize;
                    }
                }
                Op::JumpIfFalseKeep(t) => {
                    if !f.stack.last().is_some_and(|v| v.truthy()) {
                        f.pc = t as usize;
                    } else {
                        f.stack.pop();
                    }
                }
                Op::JumpIfTrueKeep(t) => {
                    if f.stack.last().is_some_and(|v| v.truthy()) {
                        f.pc = t as usize;
                    } else {
                        f.stack.pop();
                    }
                }
                Op::JumpIfNotNullishKeep(t) => {
                    if f.stack.last().is_some_and(|v| !v.is_nullish()) {
                        f.pc = t as usize;
                    } else {
                        f.stack.pop();
                    }
                }
                Op::JumpIfNullishShort(t) => {
                    if f.stack.last().is_none_or(|v| v.is_nullish()) {
                        f.stack.pop();
                        f.stack.push(Value::Undefined);
                        f.pc = t as usize;
                    }
                }
                Op::JumpIfNullishShort2(t) => {
                    if f.stack.last().is_none_or(|v| v.is_nullish()) {
                        f.stack.pop();
                        f.stack.pop();
                        f.stack.push(Value::Undefined);
                        f.pc = t as usize;
                    }
                }
                Op::JumpIfNotUndef(t) => {
                    if f.stack.last().is_some_and(|v| !v.is_undefined()) {
                        f.pc = t as usize;
                    } else {
                        f.stack.pop();
                    }
                }
                Op::Throw => {
                    let v = Self::pop(f);
                    return Err(Ctl::Throw(v));
                }
                Op::ThrowError(kind, s) => {
                    let msg = code.strs[s as usize].clone();
                    return Err(self.error_ctl(ERROR_KINDS[kind as usize], &msg));
                }
                Op::TryCatch(t) | Op::TryFinally(t) => {
                    f.handlers.push(Handler { target: t, finally: matches!(op, Op::TryFinally(_)), stack: f.stack.len() as u32, scopes: f.scopes.len() as u32, completions: f.completions.len() as u32 });
                }
                Op::PopHandler => {
                    f.handlers.pop();
                }
                Op::NormalCompletion => f.completions.push(Completion::Normal),
                Op::EndFinally => match f.completions.pop().unwrap_or(Completion::Normal) {
                    Completion::Normal => {}
                    Completion::Throw(e) => return Err(Ctl::Throw(e)),
                    Completion::Return(v) => {
                        if let Some(v) = Self::do_return(f, v) {
                            return Ok(Step::Ret(v));
                        }
                    }
                    Completion::Jump(t, packed) => Self::do_leave(f, t, packed),
                },
                Op::Leave(t, packed) => Self::do_leave(f, t, packed),
                Op::Return => {
                    let v = Self::pop(f);
                    if let Some(v) = Self::do_return(f, v) {
                        return Ok(Step::Ret(v));
                    }
                }
                Op::GetIter(r) => {
                    let v = Self::pop(f);
                    self.iter_init(f, r as usize, v)?;
                }
                Op::GetAsyncIter(r) => {
                    let v = Self::pop(f);
                    let r = r as usize;
                    let sym = self.wk().async_iterator.clone();
                    let m = self.get_v(&v, &Key::Y(sym.0.id))?;
                    if matches!(&m, Value::Obj(o) if o.is_callable()) {
                        let it = self.call(&m, v, &[])?;
                        let next = self.get_v(&it, &self.names.next.clone())?;
                        f.regs[r] = it;
                        f.regs[r + 1] = next;
                    } else {
                        // sync iterables: values are awaited by the loop
                        let it = self.get_iterator(&v, false)?;
                        let next = self.get_v(&it, &self.names.next.clone())?;
                        f.regs[r] = it;
                        f.regs[r + 1] = next;
                    }
                }
                Op::IterNext(r, t) => match self.iter_step_regs(f, r as usize)? {
                    Some(v) => f.stack.push(v),
                    None => f.pc = t as usize,
                },
                Op::IterNextOrUndef(r) => {
                    let v = self.iter_step_regs(f, r as usize)?.unwrap_or(Value::Undefined);
                    f.stack.push(v);
                }
                Op::IterRest(r) => {
                    let mut out = Vec::new();
                    while let Some(v) = self.iter_step_regs(f, r as usize)? {
                        out.push(v);
                        if out.len() > 50_000_000 {
                            return Err(self.error_ctl("RangeError", "Invalid array length"));
                        }
                    }
                    let a = self.new_array(out);
                    f.stack.push(a);
                }
                Op::IterClose(r) => self.iter_close_regs(f, r as usize),
                Op::IterCloseCompletion(r) | Op::AsyncIterCloseCompletion(r) => {
                    if !matches!(f.completions.last(), Some(Completion::Normal) | None) {
                        self.iter_close_regs(f, r as usize);
                    }
                }
                Op::AsyncIterNextCall(r) => {
                    let r = r as usize;
                    let next = f.regs[r + 1].clone();
                    if next.is_undefined() {
                        let done = self.make_iter_result(Value::Undefined, true);
                        f.stack.push(done);
                    } else {
                        let it = f.regs[r].clone();
                        let res = self.call(&next, it, &[])?;
                        f.stack.push(res);
                    }
                }
                Op::IterResult(r, t) => {
                    let res = Self::pop(f);
                    if !matches!(res, Value::Obj(_)) {
                        return Err(self.error_ctl("TypeError", "Iterator result is not an object"));
                    }
                    let done = self.get_v(&res, &self.names.done.clone())?;
                    if done.truthy() {
                        f.regs[r as usize + 1] = Value::Undefined;
                        f.pc = t as usize;
                    } else {
                        let v = self.get_v(&res, &self.names.value.clone())?;
                        f.stack.push(v);
                    }
                }
                Op::ForIn(r) => {
                    let o = Self::pop(f);
                    let keys = if o.is_nullish() { Vec::new() } else { self.for_in_keys_pub(&o)? };
                    let r = r as usize;
                    f.regs[r] = self.new_array(keys.into_iter().map(Value::Str).collect());
                    f.regs[r + 1] = Value::Num(0.0);
                }
                Op::ForInNext(r, t) => {
                    let r = r as usize;
                    let i = match f.regs[r + 1] {
                        Value::Num(n) => n as usize,
                        _ => usize::MAX,
                    };
                    let v = match &f.regs[r] {
                        Value::Obj(o) => match &o.borrow().class {
                            Class::Array(a) => a.get(i).cloned(),
                            _ => None,
                        },
                        _ => None,
                    };
                    match v {
                        Some(v) => {
                            f.regs[r + 1] = Value::Num((i + 1) as f64);
                            f.stack.push(v);
                        }
                        None => {
                            f.regs[r] = Value::Undefined;
                            f.pc = t as usize;
                        }
                    }
                }
                Op::GenStart => return Ok(Step::Yield(Value::Undefined)),
                Op::Yield => {
                    let v = Self::pop(f);
                    return Ok(Step::Yield(v));
                }
                Op::YieldStar(r, t) => {
                    let r = r as usize;
                    let (mode, received) = match f.pending_resume.take() {
                        Some((m, v)) => (m, v),
                        None => (Resume::Next, Self::pop(f)),
                    };
                    let iter = f.regs[r].clone();
                    let res = match mode {
                        Resume::Next => match f.regs[r + 1].clone() {
                            Value::Num(_) | Value::Undefined => {
                                // array fast path state
                                match self.iter_step_regs(f, r)? {
                                    Some(v) => self.make_iter_result(v, false),
                                    None => self.make_iter_result(Value::Undefined, true),
                                }
                            }
                            next => self.call(&next, iter, &[received])?,
                        },
                        Resume::Throw => {
                            let m = if matches!(iter, Value::Obj(_)) && !matches!(f.regs[r + 1], Value::Num(_)) { self.get_v(&iter, &Key::str("throw"))? } else { Value::Undefined };
                            if m.is_nullish() {
                                self.iter_close_regs(f, r);
                                return Err(self.error_ctl("TypeError", "The iterator does not provide a 'throw' method"));
                            }
                            self.call(&m, iter, &[received])?
                        }
                        Resume::Return => {
                            let m = if matches!(iter, Value::Obj(_)) && !matches!(f.regs[r + 1], Value::Num(_)) { self.get_v(&iter, &Key::str("return"))? } else { Value::Undefined };
                            if m.is_nullish() {
                                if let Some(v) = Self::do_return(f, received) {
                                    return Ok(Step::Ret(v));
                                }
                                continue;
                            }
                            let res = self.call(&m, iter, &[received])?;
                            if !matches!(res, Value::Obj(_)) {
                                return Err(self.error_ctl("TypeError", "Iterator result is not an object"));
                            }
                            let done = self.get_v(&res, &self.names.done.clone())?;
                            if done.truthy() {
                                let v = self.get_v(&res, &self.names.value.clone())?;
                                if let Some(v) = Self::do_return(f, v) {
                                    return Ok(Step::Ret(v));
                                }
                                continue;
                            }
                            f.pc -= 1;
                            f.yield_raw = true;
                            return Ok(Step::Yield(res));
                        }
                    };
                    if !matches!(res, Value::Obj(_)) {
                        return Err(self.error_ctl("TypeError", "Iterator result is not an object"));
                    }
                    let done = self.get_v(&res, &self.names.done.clone())?;
                    if done.truthy() {
                        let v = self.get_v(&res, &self.names.value.clone())?;
                        f.stack.push(v);
                        f.pc = t as usize;
                    } else {
                        f.pc -= 1;
                        f.yield_raw = true;
                        return Ok(Step::Yield(res));
                    }
                }
                Op::Await => {
                    let v = Self::pop(f);
                    return Ok(Step::Await(v));
                }
                Op::ImportCall => {
                    let s = Self::pop(f);
                    let v = self.dynamic_import_value(&s)?;
                    f.stack.push(v);
                }
                Op::RequireObjectCoercible => {
                    if let Some(v) = f.stack.last() {
                        if v.is_nullish() {
                            let d = self.describe(v);
                            return Err(self.error_ctl("TypeError", &format!("Cannot destructure '{d}' as it is {d}.")));
                        }
                    }
                }
            }
        }
    }

    fn op_call(&mut self, f: &mut Frame, code: &Code, argc: u32, d: Option<u32>) -> Result<Option<Step>, Ctl> {
        let start = f.stack.len() - argc as usize;
        let args = f.stack.split_off(start);
        let func = Self::pop(f);
        let this = Self::pop(f);
        if !matches!(&func, Value::Obj(o) if o.is_callable()) {
            let desc = match d {
                Some(d) => self.callee_text(code, d),
                None => String::from("eval"),
            };
            return Err(self.error_ctl("TypeError", &format!("{desc} is not a function")));
        }
        if d.is_none() && self.is_intrinsic_eval(&func) {
            let v = self.direct_eval(f, args.first().cloned().unwrap_or(Value::Undefined))?;
            f.stack.push(v);
            return Ok(None);
        }
        match self.inline_call(&func, this, args)? {
            Ok(frame) => Ok(Some(Step::Call(frame))),
            Err((this, args)) => {
                let v = self.call(&func, this, &args)?;
                f.stack.push(v);
                Ok(None)
            }
        }
    }

    fn get_elem(&mut self, o: &Value, k: &Value) -> Res {
        if let (Value::Obj(ob), Value::Num(n)) = (o, k) {
            if let Class::Array(a) = &ob.borrow().class {
                let i = *n as usize;
                if i as f64 == *n && i < a.len() && !is_hole(&a[i]) {
                    return Ok(a[i].clone());
                }
            }
        }
        if o.is_nullish() {
            let ks = self.to_str(k).unwrap_or_else(|_| Rc::from("?"));
            return Err(self.error_ctl("TypeError", &format!("Cannot read properties of {} (reading '{}')", self.describe(o), ks)));
        }
        self.get_member(o, k)
    }

    pub fn is_intrinsic_eval(&self, f: &Value) -> bool {
        match (f, &self.eval_fn) {
            (Value::Obj(a), Some(b)) => a.ptr_eq(b),
            _ => false,
        }
    }

    /// Direct `eval(src)` inside a function: runs in the caller's scope.
    fn direct_eval(&mut self, f: &mut Frame, src: Value) -> Res {
        let Value::Str(s) = src else { return Ok(src) };
        let strict = f.code.strict;
        let prog = match crate::js::parser::parse_program(&s, &mut self.interner, false) {
            Ok(p) => p,
            Err(e) => return Err(self.error_ctl("SyntaxError", e.trim_start_matches("SyntaxError: "))),
        };
        let env = if strict { self.new_scope(&f.env, None) } else { f.env.clone() };
        for &n in &prog.var_names {
            if !strict {
                // var declarations land in the function scope
                if !f.fenv.vars.borrow().contains_key(&n) {
                    self.declare(&f.fenv, n, 0, Value::Undefined, true);
                }
            } else {
                self.declare(&env, n, 0, Value::Undefined, true);
            }
        }
        let scope = self.new_scope(&env, None);
        self.hoist_block(&prog.body, &scope, false)?;
        let mut last = Value::Undefined;
        for st in &prog.body.stmts {
            if let crate::js::ast::Stmt::Expr(e) = st {
                last = self.eval(e, &scope)?;
            } else {
                self.exec(st, &scope, &mut Vec::new())?;
            }
        }
        Ok(last)
    }
}
