//! Bytecode tier of the JS engine.
//!
//! Functions are compiled lazily from the AST into a compact stack bytecode ([`Code`]) and run by
//! the loop in `exec.rs`. Frames live on the heap, so:
//! * generators and async functions really suspend (`yield`/`await` save the frame), with the
//!   spec's microtask ordering instead of draining the event loop;
//! * JS → JS calls do not recurse on the Rust stack, so deep recursion cannot overflow the host
//!   stack (important on hosts with small thread stacks);
//! * hot code can be handed to the optional native tier (see [`crate::js::jit`]).
//!
//! Locals that no closure can observe live in frame registers; everything else uses the same
//! [`Scope`](super::interp::Scope) objects as the AST interpreter, so both tiers interoperate:
//! a construct the compiler does not support makes that one function fall back to the AST walker.
pub mod compile;
pub mod exec;

use super::{
    ast::*,
    interp::{Env, FuncCtx, Interp},
    value::*,
};
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::cell::{Cell, RefCell};

/// One bytecode instruction. Operands index the tables of the owning [`Code`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Nop,
    // constants and stack shuffling
    Undef,
    Null,
    True,
    False,
    Int(i32),
    Const(u32),
    Pop,
    Dup,
    Dup2,
    Swap,
    /// `a b c` → `c a b`
    Rot3,
    /// `a b c d` → `d a b c`
    Rot4,
    // registers (frame locals no closure can see)
    GetReg(u32),
    SetReg(u32),
    PutReg(u32),
    /// `r++` / `r--` as a statement (ToNumber then ±1).
    IncReg(u32),
    DecReg(u32),
    // named bindings (scope chain, `with`, global object)
    GetName(Name),
    TypeofName(Name),
    /// Assign to an existing binding (or the global object); keeps the value.
    SetName(Name),
    /// Create an uninitialised lexical binding in the current scope (kind 1 let, 2 const).
    DeclLet(Name, u8),
    /// Pop and initialise a lexical binding in the current scope.
    InitLet(Name, u8),
    /// Declare `var` in the current scope if absent.
    DeclVar(Name),
    /// Pop and store with `var` semantics (assign through the chain).
    InitVar(Name),
    /// Pop and assign a function-scope `var` (Annex B block functions).
    SetVarOuter(Name),
    /// Pop and define a global `var`/function (top-level scripts).
    InitGlobal(Name),
    DeleteName(Name),
    PushScope,
    PopScope,
    /// Replace the current scope with a copy (fresh per-iteration `let` bindings).
    CopyScope,
    PushWith,
    // function context
    This,
    NewTarget,
    ImportMeta,
    LoadArg(u32),
    /// Rest parameter: array of arguments from index n.
    ArgsFrom(u32),
    Arguments,
    /// Function object being executed (named function expression self-reference).
    Callee,
    Closure(u32),
    /// Named function expression (binds its own name).
    FuncExpr(u32),
    /// Closure with an inferred name (interned name).
    ClosureNamed(u32, Name),
    /// Closure with an inferred name (`strs` index).
    ClosureStrName(u32, u32),
    /// Class definition; second operand is the inferred name or `u32::MAX`.
    Class(u32, u32),
    Regex(u32),
    TemplateObj(u32),
    /// Convert the top of stack with ToString (template literal parts).
    ToStr,
    /// Concatenate n strings.
    Concat(u32),
    // properties
    GetProp(u32),
    /// `obj` → `obj obj.key` (method call receiver stays below).
    GetMethod(u32),
    GetElem,
    GetElemMethod,
    /// `obj v` → `v`
    SetProp(u32),
    /// `obj k v` → `v`
    SetElem,
    GetSuper(u32),
    GetSuperElem,
    /// pushes `this` then the super property (method call).
    GetSuperMethod(u32),
    SetSuper(u32),
    SetSuperElem,
    DelProp(u32),
    DelElem,
    // operators
    Bin(BinOp),
    Neg,
    Pos,
    /// ToNumeric: numbers and BigInts pass, everything else becomes a number (`x++`).
    ToNumeric,
    Not,
    BitNot,
    TypeOf,
    Inc,
    Dec,
    ToKey,
    In,
    // literals
    NewObject,
    /// `obj v` → `obj`
    InitProp(u32),
    /// `obj k v` → `obj`
    InitElem,
    /// `obj k` → `obj`; kind 0 method, 1 getter, 2 setter.
    InitMethod(u32, u8),
    /// `obj src` → `obj`
    CopyProps,
    /// `obj src` → `obj`, excluding the keys in the array below src: `obj keys src` → `obj`.
    CopyPropsExcept,
    SetProtoLit,
    NewArray,
    ArrPush,
    ArrHole,
    ArrSpread,
    // calls; second operand indexes `strs` for the callee description (error messages)
    Call(u32, u32),
    CallSpread(u32),
    New(u32, u32),
    NewSpread(u32),
    SuperCall(u32),
    SuperCallSpread,
    CallEval(u32),
    // control flow
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    JumpIfFalseKeep(u32),
    JumpIfTrueKeep(u32),
    JumpIfNotNullishKeep(u32),
    /// Optional chain: if the top is nullish, replace it with undefined and jump.
    JumpIfNullishShort(u32),
    /// Optional call: if the function (top) is nullish, drop it and the receiver, push undefined, jump.
    JumpIfNullishShort2(u32),
    /// Defaults: if the top is not undefined jump keeping it, else pop it.
    JumpIfNotUndef(u32),
    // exceptions and completions
    Throw,
    /// kind index into ERROR_KINDS, message in `strs`.
    ThrowError(u8, u32),
    TryCatch(u32),
    TryFinally(u32),
    PopHandler,
    NormalCompletion,
    EndFinally,
    /// Jump out of protected regions: target, (handler depth << 16 | scope depth).
    Leave(u32, u32),
    Return,
    // iteration (state in two consecutive registers)
    GetIter(u32),
    GetAsyncIter(u32),
    IterNext(u32, u32),
    IterNextOrUndef(u32),
    IterRest(u32),
    IterClose(u32),
    /// Close the iterator unless the pending completion is normal (for-of `finally`).
    IterCloseCompletion(u32),
    AsyncIterCloseCompletion(u32),
    AsyncIterNextCall(u32),
    IterResult(u32, u32),
    ForIn(u32),
    ForInNext(u32, u32),
    // suspension
    GenStart,
    Yield,
    /// `yield*` step; register pair holds the inner iterator; jumps to target when done.
    YieldStar(u32, u32),
    Await,
    ImportCall,
    RequireObjectCoercible,
    Debugger,
}

pub const ERROR_KINDS: [&str; 4] = ["TypeError", "ReferenceError", "SyntaxError", "RangeError"];

/// A compiled function body (or script).
pub struct Code {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
    pub keys: Vec<Key>,
    pub strs: Vec<JsStr>,
    pub funcs: Vec<Rc<FuncDef>>,
    pub classes: Vec<Rc<ClassDef>>,
    pub templates: Vec<Rc<TemplateObj>>,
    pub regexes: Vec<(JsStr, JsStr)>,
    pub nregs: u32,
    /// The function needs its own runtime scope (closures, `with`/`eval`, scope-stored bindings,
    /// `super`/`new.target`, suspension). When false, `this` lives in the frame and calls skip
    /// the scope allocation entirely.
    pub needs_scope: bool,
    pub strict: bool,
    pub is_generator: bool,
    pub is_async: bool,
    /// Entry counter for the native tier.
    pub hot: Cell<u32>,
    #[cfg(feature = "jit")]
    pub native: RefCell<super::jit::NativeSlot>,
    pub name: JsStr,
    /// Source text and span of the function (diagnostics).
    pub source: Option<(JsStr, (usize, usize))>,
    /// Template objects are created once per site (`lit-html` relies on their identity).
    pub template_cache: RefCell<Vec<Option<Value>>>,
}

impl Code {
    pub fn op_count(&self) -> usize {
        self.ops.len()
    }
}

/// Pending completion of a `finally` block.
#[derive(Clone)]
pub enum Completion {
    Normal,
    Throw(Value),
    Return(Value),
    Jump(u32, u32),
}

#[derive(Clone)]
pub struct Handler {
    pub target: u32,
    pub finally: bool,
    pub stack: u32,
    pub scopes: u32,
    pub completions: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameKind {
    Normal,
    /// `new`: a non-object result is replaced by `this`.
    Construct,
    Async,
    Generator,
    AsyncGenerator,
}

pub struct Frame {
    pub code: Rc<Code>,
    pub pc: usize,
    pub stack: Vec<Value>,
    pub regs: Vec<Value>,
    pub env: Env,
    pub scopes: Vec<Env>,
    /// Function scope (holds `this`, callee, new.target).
    pub fenv: Env,
    pub handlers: Vec<Handler>,
    pub completions: Vec<Completion>,
    pub args: Vec<Value>,
    pub callee: Option<Obj>,
    pub kind: FrameKind,
    /// Promise of an async function.
    pub promise: Option<Obj>,
    /// `throw`/`return` requested by the resumer, applied when the loop restarts.
    pub pending_resume: Option<(Resume, Value)>,
    /// The last yield delivered an inner iterator result as-is (`yield*`).
    pub yield_raw: bool,
    /// `this` of a function that runs without its own scope.
    pub this: Option<Value>,
}

/// How a run of the loop ended.
pub enum Outcome {
    Return(Value),
    Throw(Value),
    Yield(Value, Box<Frame>),
    Await(Value, Box<Frame>),
}

/// How a suspended frame is resumed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    Next,
    Throw,
    Return,
}

/// State of a VM generator / async function stored in a `Class::Generator` object.
pub struct CoState {
    pub frame: Option<Box<Frame>>,
    pub running: bool,
    pub done: bool,
    pub kind: FrameKind,
    /// Async generators: queued requests (mode, value, promise).
    pub queue: alloc::collections::VecDeque<(Resume, Value, Obj)>,
    /// Async generators: awaiting the operand of a `yield` before delivering it.
    pub yield_await: bool,
    /// Async generators: awaiting the operand of `return` (completion requested by the caller).
    pub return_await: bool,
}

impl CoState {
    pub fn new(kind: FrameKind) -> CoState {
        CoState { frame: None, running: false, done: false, kind, queue: Default::default(), yield_await: false, return_await: false }
    }
}

/// How functions are executed. Chosen per runtime by [`crate::config::EngineConfig`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// AST walker only (reference/debug mode).
    Ast,
    /// Bytecode interpreter (portable; no executable memory needed).
    Bytecode,
    /// Bytecode plus native compilation of hot functions when the platform allows it.
    Native,
}

impl Interp {
    /// Compiled code for a function, compiling on first use. `None` when the function uses
    /// something the bytecode compiler does not handle (the AST walker runs it instead).
    pub fn vm_code(&mut self, def: &Rc<FuncDef>) -> Option<Rc<Code>> {
        if self.tier == Tier::Ast {
            return None;
        }
        if let Some(c) = def.code.get() {
            return c.clone();
        }
        let compiled = compile::compile_function(self, def).ok().map(Rc::new);
        if compiled.is_none() {
            self.vm_stats.fallbacks += 1;
        } else {
            self.vm_stats.compiled += 1;
        }
        let _ = def.code.set(compiled.clone());
        compiled
    }

    /// Build the frame for a closure call. `this` is `None` for derived constructors until `super()`.
    #[allow(clippy::too_many_arguments)]
    pub fn vm_frame(&mut self, code: Rc<Code>, fo: &Obj, def: &Rc<FuncDef>, env: &Env, home: Option<Obj>, this: Option<Value>, args: &[Value], new_target: Option<Obj>) -> Frame {
        let mut own_this = None;
        if !code.needs_scope && (def.is_arrow || this.is_some()) {
            if !def.is_arrow {
                let t = this.expect("checked");
                own_this = Some(if def.strict {
                    t
                } else {
                    match &t {
                        Value::Undefined | Value::Null => Value::Obj(self.global.clone()),
                        Value::Obj(_) => t,
                        prim => self.to_object(prim).map(Value::Obj).unwrap_or(Value::Undefined),
                    }
                });
            }
            let kind = if new_target.is_some() && !def.is_arrow { FrameKind::Construct } else { FrameKind::Normal };
            let nregs = code.nregs as usize;
            let _ = home;
            return Frame {
                code,
                pc: 0,
                stack: Vec::with_capacity(8),
                regs: alloc::vec![Value::Undefined; nregs],
                env: env.clone(),
                scopes: Vec::new(),
                fenv: env.clone(),
                handlers: Vec::new(),
                completions: Vec::new(),
                args: args.to_vec(),
                callee: Some(fo.clone()),
                kind,
                promise: None,
                pending_resume: None,
                yield_raw: false,
                this: own_this,
            };
        }
        let func_ctx = if def.is_arrow {
            None
        } else {
            let this_val = match this {
                None => None,
                Some(t) if def.strict => Some(t),
                Some(t) => Some(match &t {
                    Value::Undefined | Value::Null => Value::Obj(self.global.clone()),
                    Value::Obj(_) => t,
                    prim => self.to_object(prim).map(Value::Obj).unwrap_or(Value::Undefined),
                }),
            };
            Some(FuncCtx { this: RefCell::new(this_val), callee: Some(fo.clone()), new_target: new_target.clone().map_or(Value::Undefined, Value::Obj), home, is_global: false, strict: def.strict })
        };
        let scope = self.new_scope(env, func_ctx);
        let kind = match (def.is_async, def.is_generator) {
            (true, true) => FrameKind::AsyncGenerator,
            (false, true) => FrameKind::Generator,
            (true, false) => FrameKind::Async,
            _ if new_target.is_some() && !def.is_arrow => FrameKind::Construct,
            _ => FrameKind::Normal,
        };
        let nregs = code.nregs as usize;
        Frame {
            code,
            pc: 0,
            stack: Vec::with_capacity(8),
            regs: alloc::vec![Value::Undefined; nregs],
            env: scope.clone(),
            scopes: Vec::new(),
            fenv: scope,
            handlers: Vec::new(),
            completions: Vec::new(),
            args: args.to_vec(),
            callee: Some(fo.clone()),
            kind,
            promise: None,
            pending_resume: None,
            yield_raw: false,
            this: own_this,
        }
    }

    /// Run a closure call on the VM (from native code or the AST walker).
    pub fn vm_invoke(&mut self, mut frame: Frame) -> Res {
        match frame.kind {
            FrameKind::Generator | FrameKind::AsyncGenerator => {
                let kind = frame.kind;
                // parameters are bound now; the body starts at the first `next()`
                match self.vm_run(frame) {
                    Outcome::Yield(_, f) => Ok(Value::Obj(self.make_generator(f, kind))),
                    Outcome::Throw(e) => Err(Ctl::Throw(e)),
                    Outcome::Return(_) | Outcome::Await(..) => Err(self.error_ctl("Error", "generator prologue did not suspend")),
                }
            }
            FrameKind::Async => {
                let p = self.new_promise();
                frame.promise = Some(p.clone());
                let out = self.vm_run(frame);
                self.vm_settle_async(out);
                Ok(Value::Obj(p))
            }
            _ => match self.vm_run(frame) {
                Outcome::Return(v) => Ok(v),
                Outcome::Throw(e) => Err(Ctl::Throw(e)),
                Outcome::Yield(..) | Outcome::Await(..) => Err(self.error_ctl("SyntaxError", "unexpected suspension")),
            },
        }
    }

    fn make_generator(&mut self, frame: Box<Frame>, kind: FrameKind) -> Obj {
        let fallback = if kind == FrameKind::AsyncGenerator {
            self.protos().extra.get("async_generator").cloned().unwrap_or_else(|| self.protos().generator.clone())
        } else {
            self.protos().generator.clone()
        };
        let proto = match &frame.callee {
            Some(fo) => match self.get_obj(fo, &self.names.prototype.clone(), &Value::Obj(fo.clone())) {
                Ok(Value::Obj(o)) => o,
                _ => fallback,
            },
            None => fallback,
        };
        let mut st = CoState::new(kind);
        st.frame = Some(frame);
        let g = GenData { values: Vec::new(), pos: 0, started: true, ret: Value::Undefined, pending: None, co: Some(Box::new(st)) };
        self.alloc(Object::new(Some(proto), Class::Generator(Box::new(g))))
    }

    fn co_take(&mut self, g: &Obj) -> Option<(Box<Frame>, FrameKind)> {
        let mut b = g.borrow_mut();
        let Class::Generator(d) = &mut b.class else { return None };
        let co = d.co.as_mut()?;
        let f = co.frame.take()?;
        co.running = true;
        Some((f, co.kind))
    }

    fn co_store(&mut self, g: &Obj, frame: Option<Box<Frame>>) {
        if let Class::Generator(d) = &mut g.borrow_mut().class {
            if let Some(co) = d.co.as_mut() {
                co.running = false;
                co.done = frame.is_none();
                co.frame = frame;
            }
        }
    }

    /// Is this a VM generator object? Returns (running, done).
    pub fn vm_gen_state(&self, g: &Obj) -> Option<(bool, bool, FrameKind)> {
        match &g.borrow().class {
            Class::Generator(d) => d.co.as_ref().map(|c| (c.running, c.done, c.kind)),
            _ => None,
        }
    }

    /// `generator.next/throw/return` for a VM generator.
    pub fn vm_gen_resume(&mut self, g: &Obj, mode: Resume, v: Value) -> Res {
        let Some((running, done, _)) = self.vm_gen_state(g) else { return Err(self.error_ctl("TypeError", "not a generator")) };
        if running {
            return Err(self.error_ctl("TypeError", "Generator is already running"));
        }
        if done {
            return match mode {
                Resume::Next => Ok(self.make_iter_result(Value::Undefined, true)),
                Resume::Return => Ok(self.make_iter_result(v, true)),
                Resume::Throw => Err(Ctl::Throw(v)),
            };
        }
        let (mut frame, _) = self.co_take(g).expect("suspended generator");
        // a generator that never started completes immediately on return/throw
        let fresh = frame.code.ops.get(frame.pc.wrapping_sub(1)) == Some(&Op::GenStart);
        if fresh && mode != Resume::Next {
            self.co_store(g, None);
            return match mode {
                Resume::Return => Ok(self.make_iter_result(v, true)),
                _ => Err(Ctl::Throw(v)),
            };
        }
        frame.resume_with(mode, v);
        let out = self.vm_run_resumed(frame);
        match out {
            Outcome::Yield(y, mut f) => {
                let raw = core::mem::take(&mut f.yield_raw);
                self.co_store(g, Some(f));
                Ok(if raw { y } else { self.make_iter_result(y, false) })
            }
            Outcome::Return(r) => {
                self.co_store(g, None);
                Ok(self.make_iter_result(r, true))
            }
            Outcome::Throw(e) => {
                self.co_store(g, None);
                Err(Ctl::Throw(e))
            }
            Outcome::Await(_, f) => {
                self.co_store(g, None);
                drop(f);
                Err(self.error_ctl("SyntaxError", "await in a generator"))
            }
        }
    }

    // -----------------------------------------------------------------------
    // async functions
    // -----------------------------------------------------------------------

    /// Settle or park an async function after a run of its frame.
    pub fn vm_settle_async(&mut self, out: Outcome) {
        match out {
            Outcome::Return(_) | Outcome::Throw(_) => {}
            Outcome::Await(v, frame) => self.vm_park(v, frame, None),
            Outcome::Yield(_, _) => {}
        }
    }

    /// Wait for `v` and resume `frame` (an async function, or an async generator `g`).
    pub fn vm_park(&mut self, v: Value, frame: Box<Frame>, g: Option<Obj>) {
        let holder = match g {
            Some(g) => {
                self.co_store(&g, Some(frame));
                if let Class::Generator(d) = &mut g.borrow_mut().class {
                    if let Some(co) = d.co.as_mut() {
                        co.running = true;
                    }
                }
                g
            }
            None => {
                let mut st = CoState::new(FrameKind::Async);
                st.frame = Some(frame);
                st.running = true;
                let gd = GenData { values: Vec::new(), pos: 0, started: true, ret: Value::Undefined, pending: None, co: Some(Box::new(st)) };
                self.alloc(Object::new(None, Class::Generator(Box::new(gd))))
            }
        };
        let p = match self.promise_resolved(v) {
            Ok(p) => p,
            Err(Ctl::Throw(e)) => {
                let p = self.new_promise();
                self.reject_promise(&p, e);
                p
            }
            Err(_) => self.new_promise(),
        };
        let on_f = self.new_native_data("", 1, resume_fulfilled, Value::Obj(holder.clone()), false);
        let on_r = self.new_native_data("", 1, resume_rejected, Value::Obj(holder), false);
        self.promise_then_internal(&p, Value::Obj(on_f), Value::Obj(on_r));
    }

    fn vm_resume_holder(&mut self, holder: &Obj, mode: Resume, v: Value) {
        let taken = {
            let mut b = holder.borrow_mut();
            match &mut b.class {
                Class::Generator(d) => d.co.as_mut().and_then(|c| c.frame.take().map(|f| (f, c.kind))),
                _ => None,
            }
        };
        let Some((mut frame, kind)) = taken else { return };
        if kind == FrameKind::AsyncGenerator {
            self.async_gen_continue(holder, frame, mode, v);
            return;
        }
        frame.resume_with(mode, v);
        let out = self.vm_run_resumed(frame);
        self.vm_settle_async(out);
    }

    // -----------------------------------------------------------------------
    // async generators
    // -----------------------------------------------------------------------

    /// `next/throw/return` on an async generator: queue the request and run if idle.
    pub fn vm_async_gen_request(&mut self, g: &Obj, mode: Resume, v: Value) -> Res {
        let p = self.new_promise();
        let idle = {
            let mut b = g.borrow_mut();
            let Class::Generator(d) = &mut b.class else {
                drop(b);
                let e = self.make_error("TypeError", "not an async generator");
                self.reject_promise(&p, e);
                return Ok(Value::Obj(p));
            };
            let Some(co) = d.co.as_mut() else { return Ok(Value::Obj(p)) };
            co.queue.push_back((mode, v, p.clone()));
            !co.running
        };
        if idle {
            self.async_gen_drain(g);
        }
        Ok(Value::Obj(p))
    }

    /// Serve queued requests while the generator is idle.
    fn async_gen_drain(&mut self, g: &Obj) {
        loop {
            let (req, frame, done) = {
                let mut b = g.borrow_mut();
                let Class::Generator(d) = &mut b.class else { return };
                let Some(co) = d.co.as_mut() else { return };
                if co.running {
                    return;
                }
                let Some(req) = co.queue.front().map(|(m, v, _)| (*m, v.clone())) else { return };
                (req, co.frame.take(), co.done)
            };
            let (mode, v) = req;
            if done || frame.is_none() {
                // completed generator: answer immediately
                let (_, _, p) = self.async_gen_pop(g).expect("queued request");
                match mode {
                    Resume::Next => {
                        let r = self.make_iter_result(Value::Undefined, true);
                        let _ = self.resolve_promise(&p, r);
                    }
                    Resume::Return => {
                        // await the value, then deliver {value, done: true}
                        self.async_gen_mark(g, |co| {
                            co.running = true;
                            co.return_await = true;
                        });
                        self.vm_park_value(v, g);
                        return;
                    }
                    Resume::Throw => self.reject_promise(&p, v),
                }
                continue;
            }
            let mut frame = frame.expect("frame");
            let fresh = frame.code.ops.get(frame.pc.wrapping_sub(1)) == Some(&Op::GenStart);
            if fresh && mode != Resume::Next {
                self.async_gen_mark(g, |co| co.done = true);
                drop(frame);
                continue;
            }
            self.async_gen_mark(g, |co| co.running = true);
            frame.resume_with(mode, v);
            let out = self.vm_run_resumed(frame);
            self.async_gen_outcome(g, out);
        }
    }

    fn async_gen_mark(&mut self, g: &Obj, f: impl FnOnce(&mut CoState)) {
        if let Class::Generator(d) = &mut g.borrow_mut().class {
            if let Some(co) = d.co.as_mut() {
                f(co);
            }
        }
    }

    fn async_gen_pop(&mut self, g: &Obj) -> Option<(Resume, Value, Obj)> {
        match &mut g.borrow_mut().class {
            Class::Generator(d) => d.co.as_mut().and_then(|c| c.queue.pop_front()),
            _ => None,
        }
    }

    /// Await `v` for an async generator without a frame to resume (yield/return delivery).
    fn vm_park_value(&mut self, v: Value, g: &Obj) {
        let p = match self.promise_resolved(v) {
            Ok(p) => p,
            Err(Ctl::Throw(e)) => {
                let p = self.new_promise();
                self.reject_promise(&p, e);
                p
            }
            Err(_) => self.new_promise(),
        };
        let on_f = self.new_native_data("", 1, resume_fulfilled, Value::Obj(g.clone()), false);
        let on_r = self.new_native_data("", 1, resume_rejected, Value::Obj(g.clone()), false);
        self.promise_then_internal(&p, Value::Obj(on_f), Value::Obj(on_r));
    }

    fn async_gen_outcome(&mut self, g: &Obj, out: Outcome) {
        match out {
            Outcome::Yield(v, f) => {
                // `yield v` awaits v first, then resolves the head request
                self.co_store(g, Some(f));
                self.async_gen_mark(g, |co| {
                    co.running = true;
                    co.yield_await = true;
                });
                self.vm_park_value(v, g);
            }
            Outcome::Await(v, f) => {
                self.vm_park(v, f, Some(g.clone()));
            }
            Outcome::Return(v) => {
                self.co_store(g, None);
                if let Some((_, _, p)) = self.async_gen_pop(g) {
                    let r = self.make_iter_result(v, true);
                    let _ = self.resolve_promise(&p, r);
                }
                self.async_gen_drain(g);
            }
            Outcome::Throw(e) => {
                self.co_store(g, None);
                if let Some((_, _, p)) = self.async_gen_pop(g) {
                    self.reject_promise(&p, e);
                }
                self.async_gen_drain(g);
            }
        }
    }

    /// An await inside an async generator (or a yield/return delivery) settled.
    fn async_gen_continue(&mut self, g: &Obj, frame: Box<Frame>, mode: Resume, v: Value) {
        let (yield_await, return_await) = match &g.borrow().class {
            Class::Generator(d) => d.co.as_ref().map_or((false, false), |c| (c.yield_await, c.return_await)),
            _ => (false, false),
        };
        if yield_await {
            self.async_gen_mark(g, |co| co.yield_await = false);
            if mode == Resume::Throw {
                // the awaited yield operand rejected: throw at the yield
                let mut frame = frame;
                frame.resume_with(Resume::Throw, v);
                let out = self.vm_run_resumed(frame);
                self.async_gen_outcome(g, out);
                return;
            }
            self.co_store(g, Some(frame));
            if let Some((_, _, p)) = self.async_gen_pop(g) {
                let r = self.make_iter_result(v, false);
                let _ = self.resolve_promise(&p, r);
            }
            self.async_gen_drain(g);
            return;
        }
        let _ = return_await;
        let mut frame = frame;
        frame.resume_with(mode, v);
        let out = self.vm_run_resumed(frame);
        self.async_gen_outcome(g, out);
    }

    /// `then` without a user-visible derived promise result.
    pub fn promise_then_internal(&mut self, p: &Obj, on_f: Value, on_r: Value) {
        let _ = self.promise_then(p, Some(on_f), Some(on_r));
    }
}

impl Frame {
    /// Prepare a suspended frame to continue after `yield`/`await`.
    pub fn resume_with(&mut self, mode: Resume, v: Value) {
        match mode {
            Resume::Next => self.stack.push(v),
            _ => self.pending_resume = Some((mode, v)),
        }
    }
}

fn holder_of(it: &Interp, c: &Call) -> Option<Obj> {
    match it.native_data(c) {
        Value::Obj(o) => Some(o),
        _ => None,
    }
}

fn resume_fulfilled(it: &mut Interp, c: &Call) -> Res {
    if let Some(h) = holder_of(it, c) {
        // async generator delivering `return v`
        let ret = match &h.borrow().class {
            Class::Generator(d) => d.co.as_ref().is_some_and(|co| co.return_await),
            _ => false,
        };
        if ret {
            it.async_gen_return_settled(&h, c.arg(0), false);
            return Ok(Value::Undefined);
        }
        it.vm_resume_holder(&h, Resume::Next, c.arg(0));
    }
    Ok(Value::Undefined)
}

fn resume_rejected(it: &mut Interp, c: &Call) -> Res {
    if let Some(h) = holder_of(it, c) {
        let ret = match &h.borrow().class {
            Class::Generator(d) => d.co.as_ref().is_some_and(|co| co.return_await),
            _ => false,
        };
        if ret {
            it.async_gen_return_settled(&h, c.arg(0), true);
            return Ok(Value::Undefined);
        }
        it.vm_resume_holder(&h, Resume::Throw, c.arg(0));
    }
    Ok(Value::Undefined)
}

impl Interp {
    fn async_gen_return_settled(&mut self, g: &Obj, v: Value, rejected: bool) {
        self.async_gen_mark(g, |co| {
            co.return_await = false;
            co.running = false;
        });
        if let Some((_, _, p)) = self.async_gen_pop(g) {
            if rejected {
                self.reject_promise(&p, v);
            } else {
                let r = self.make_iter_result(v, true);
                let _ = self.resolve_promise(&p, r);
            }
        }
        self.async_gen_drain(g);
    }
}

/// Counters exposed for diagnostics and benchmarks.
#[derive(Default, Clone, Debug)]
pub struct VmStats {
    pub compiled: usize,
    pub fallbacks: usize,
    pub native_compiled: usize,
    /// Native compilation attempts that failed (unsupported code, no executable memory).
    pub native_failed: usize,
    /// Hot functions skipped because the runtime has no executable memory.
    pub native_no_memory: usize,
}
