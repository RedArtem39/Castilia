//! The tree-walking evaluator.
use super::{ast::*, parser::Interner, value::*};
use crate::dom::Document;
use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::cell::RefCell;

pub struct Var {
    pub v: Value,
    pub kind: u8, // 0 var, 1 let, 2 const
    pub init: bool,
}

pub struct FuncCtx {
    pub this: RefCell<Option<Value>>,
    pub callee: Option<Obj>,
    pub new_target: Value,
    pub home: Option<Obj>,
    pub is_global: bool,
    pub strict: bool,
}

pub struct Scope {
    pub vars: RefCell<BTreeMap<Name, Var>>,
    pub parent: Option<Env>,
    pub func: Option<FuncCtx>,
    /// Object environment record introduced by a `with` statement.
    pub with_obj: Option<Obj>,
}

pub type Env = Rc<Scope>;

pub struct FieldInit {
    pub key: Key,
    pub class: Rc<ClassDef>,
    pub idx: usize,
    pub env: Env,
    pub home: Obj,
}

pub struct Protos {
    pub object: Obj,
    pub function: Obj,
    pub array: Obj,
    pub string: Obj,
    pub number: Obj,
    pub boolean: Obj,
    pub symbol: Obj,
    pub error: Obj,
    pub date: Obj,
    pub regexp: Obj,
    pub map: Obj,
    pub set: Obj,
    pub weakmap: Obj,
    pub weakset: Obj,
    pub promise: Obj,
    pub generator: Obj,
    pub iterator: Obj,
    pub array_iterator: Obj,
    pub node: Obj,
    pub element: Obj,
    pub html_element: Obj,
    pub text: Obj,
    pub document: Obj,
    pub event: Obj,
    pub event_target: Obj,
    pub error_kinds: BTreeMap<&'static str, Obj>,
    pub extra: BTreeMap<&'static str, Obj>,
}

pub struct WellKnown {
    pub iterator: Sym,
    pub async_iterator: Sym,
    pub to_primitive: Sym,
    pub has_instance: Sym,
    pub to_string_tag: Sym,
    pub species: Sym,
}

pub struct Timer {
    pub id: u32,
    pub due: f64,
    pub interval: Option<f64>,
    pub callback: Value,
    pub args: Vec<Value>,
    pub raf: bool,
}

pub enum Job {
    Call(Value, Value, Vec<Value>),
    /// Run a promise reaction: (handler, argument, derived promise, is_rejection, passthrough)
    Reaction { handler: Option<Value>, arg: Value, result: Option<Obj>, rejected: bool },
    ResolveThenable { promise: Obj, thenable: Value, then: Value },
}

pub struct LogEntry {
    pub level: &'static str,
    pub text: String,
}

pub struct Interp {
    pub interner: Interner,
    pub global: Obj,
    pub global_env: Env,
    pub protos: Option<Protos>,
    pub wk: Option<WellKnown>,
    pub sym_counter: u32,
    pub symbol_registry: BTreeMap<String, Sym>,
    pub depth: usize,
    pub max_depth: usize,
    pub ticks: u64,
    pub tick_limit: u64,
    pub microtasks: VecDeque<Job>,
    pub timers: Vec<Timer>,
    pub timer_seq: u32,
    pub now: f64,
    pub doc: *mut Document,
    pub node_cache: BTreeMap<usize, Obj>,
    pub gen_stack: Vec<Vec<Value>>,
    pub logs: Vec<LogEntry>,
    pub names: Names,
    pub rng: u64,
    pub unhandled: Vec<Value>,
    pub draining: usize,
    pub host: super::dom::HostSlot,
    pub dom: super::dom::DomState,
    pub modules: BTreeMap<String, Obj>,
    pub module_sources: BTreeMap<String, String>,
    pub module_urls: Vec<String>,
    pub import_map: BTreeMap<String, String>,
    pub array_values_native: bool,
    pub join_stack: Vec<usize>,
    pub console_indent: usize,
    pub loaded_links: Vec<usize>,
    pub last_prevented: bool,
    pub dispatch_count: usize,
    pub history_state: BTreeMap<usize, Value>,
    pub parsing_html_scripts: bool,
    pub executed_scripts: Vec<usize>,
    pub current_script: Option<usize>,
    pub custom_elements: Vec<(String, Obj)>,
    pub template_content: BTreeMap<usize, usize>,
    pub shadow_roots: BTreeMap<usize, usize>,
    pub counters: BTreeMap<String, u32>,
    pub dom_symbols: BTreeMap<u32, Sym>,
}

pub struct Names {
    pub length: Key,
    pub prototype: Key,
    pub constructor: Key,
    pub name: Key,
    pub message: Key,
    pub stack: Key,
    pub then: Key,
    pub next: Key,
    pub done: Key,
    pub value: Key,
    pub get: Key,
    pub set: Key,
    pub to_string: Key,
    pub value_of: Key,
    pub to_json: Key,
    pub this_name: Name,
    pub arguments: Name,
    pub undefined: Name,
}

const GEN_LIMIT: usize = 20000;

impl Interp {
    pub fn doc(&self) -> Option<&mut Document> {
        if self.doc.is_null() {
            None
        } else {
            // SAFETY: the pointer is installed by `Runtime::with_document` for the duration of a
            // call and never outlives it; the engine is single threaded and callers never hold
            // two overlapping borrows of the document.
            Some(unsafe { &mut *self.doc })
        }
    }

    pub fn protos(&self) -> &Protos {
        self.protos.as_ref().expect("realm initialised")
    }

    pub fn wk(&self) -> &WellKnown {
        self.wk.as_ref().expect("realm initialised")
    }

    pub fn tick(&mut self) -> Res<()> {
        self.ticks += 1;
        if self.ticks > self.tick_limit {
            return Err(self.error_ctl("Error", "script execution limit reached"));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Scopes
    // -----------------------------------------------------------------------

    pub fn new_scope(&self, parent: &Env, func: Option<FuncCtx>) -> Env {
        Rc::new(Scope { vars: RefCell::new(BTreeMap::new()), parent: Some(parent.clone()), func, with_obj: None })
    }

    pub fn declare(&self, env: &Env, name: Name, kind: u8, v: Value, init: bool) {
        env.vars.borrow_mut().insert(name, Var { v, kind, init });
    }

    fn with_has(&mut self, cur: &Env, name: Name) -> Res<Option<Obj>> {
        if let Some(o) = &cur.with_obj {
            let key = Key::S(self.interner.get(name).clone());
            if self.has_property(o, &key)? {
                return Ok(Some(o.clone()));
            }
        }
        Ok(None)
    }

    pub fn lookup(&mut self, name: Name, env: &Env) -> Res {
        let mut cur = env;
        loop {
            if let Some(o) = self.with_has(cur, name)? {
                let key = Key::S(self.interner.get(name).clone());
                return self.get_obj(&o, &key, &Value::Obj(o.clone()));
            }
            if let Some(var) = cur.vars.borrow().get(&name) {
                if !var.init {
                    let n = self.interner.get(name).clone();
                    drop(var);
                    return Err(self.error_ctl("ReferenceError", &format!("Cannot access '{n}' before initialization")));
                }
                return Ok(var.v.clone());
            }
            match &cur.parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        // global object
        let key = Key::S(self.interner.get(name).clone());
        let g = self.global.clone();
        if self.has_property(&g, &key)? {
            return self.get_obj(&g, &key, &Value::Obj(g.clone()));
        }
        let n = self.interner.get(name).clone();
        Err(self.error_ctl("ReferenceError", &format!("{n} is not defined")))
    }

    pub fn lookup_typeof(&mut self, name: Name, env: &Env) -> Res {
        let mut cur = env;
        loop {
            if let Some(o) = self.with_has(cur, name)? {
                let key = Key::S(self.interner.get(name).clone());
                return self.get_obj(&o, &key, &Value::Obj(o.clone()));
            }
            if let Some(var) = cur.vars.borrow().get(&name) {
                if !var.init {
                    return Ok(Value::Undefined);
                }
                return Ok(var.v.clone());
            }
            match &cur.parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        let key = Key::S(self.interner.get(name).clone());
        let g = self.global.clone();
        if self.has_property(&g, &key)? {
            return self.get_obj(&g, &key, &Value::Obj(g.clone()));
        }
        Ok(Value::Undefined)
    }

    pub fn assign_var(&mut self, name: Name, v: Value, env: &Env) -> Res<()> {
        let mut cur = env;
        loop {
            if let Some(o) = self.with_has(cur, name)? {
                let key = Key::S(self.interner.get(name).clone());
                return self.set_v(&Value::Obj(o), key, v);
            }
            {
                let mut vars = cur.vars.borrow_mut();
                if let Some(var) = vars.get_mut(&name) {
                    if !var.init {
                        let n = self.interner.get(name).clone();
                        drop(vars);
                        return Err(self.error_ctl("ReferenceError", &format!("Cannot access '{n}' before initialization")));
                    }
                    if var.kind == 2 {
                        drop(vars);
                        return Err(self.error_ctl("TypeError", "Assignment to constant variable."));
                    }
                    var.v = v;
                    return Ok(());
                }
            }
            match &cur.parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        let key = Key::S(self.interner.get(name).clone());
        let g = Value::Obj(self.global.clone());
        self.set_v(&g, key, v)
    }

    /// Initialise a `let`/`const`/`var` binding declared in (or inserted into) `env`.
    pub fn init_binding(&mut self, name: Name, v: Value, env: &Env, kind: VarKind) -> Res<()> {
        match kind {
            VarKind::Var => self.assign_var(name, v, env),
            k => {
                let kind = if k == VarKind::Const { 2 } else { 1 };
                env.vars.borrow_mut().insert(name, Var { v, kind, init: true });
                Ok(())
            }
        }
    }

    pub fn this_value(&mut self, env: &Env) -> Res {
        let mut cur = env;
        loop {
            if let Some(f) = &cur.func {
                return match &*f.this.borrow() {
                    Some(v) => Ok(v.clone()),
                    None => Err(self.error_ctl("ReferenceError", "Must call super constructor in derived class before accessing 'this'")),
                };
            }
            match &cur.parent {
                Some(p) => cur = p,
                None => return Ok(Value::Undefined),
            }
        }
    }

    fn func_ctx<'a>(&self, env: &'a Env) -> Option<&'a FuncCtx> {
        let mut cur = env;
        loop {
            if let Some(f) = &cur.func {
                return Some(f);
            }
            match &cur.parent {
                Some(p) => cur = p,
                None => return None,
            }
        }
    }

    // -----------------------------------------------------------------------
    // Programs
    // -----------------------------------------------------------------------

    /// Run a parsed program in the global scope; returns the completion value of the last expression statement.
    pub fn run_program(&mut self, prog: &Program) -> Res {
        let env = self.global_env.clone();
        // var declarations become global object properties
        for &n in &prog.var_names {
            let key = Key::S(self.interner.get(n).clone());
            let g = self.global.clone();
            if !self.has_own(&g, &key) {
                self.define_data(&g, key, Value::Undefined, ALL);
            }
        }
        self.hoist_block(&prog.body, &env, true)?;
        let mut last = Value::Undefined;
        for s in &prog.body.stmts {
            if let Stmt::Expr(e) = s {
                last = self.eval(e, &env)?;
            } else {
                self.exec(s, &env, &mut Vec::new())?;
            }
        }
        Ok(last)
    }

    pub(crate) fn hoist_block(&mut self, b: &Block, env: &Env, global: bool) -> Res<()> {
        for (n, k) in &b.lets {
            if global {
                // top-level let/const/class live in the global lexical scope
                self.declare(env, *n, if *k == VarKind::Const { 2 } else { 1 }, Value::Undefined, false);
            } else {
                self.declare(env, *n, if *k == VarKind::Const { 2 } else { 1 }, Value::Undefined, false);
            }
        }
        for f in &b.funcs {
            let fo = self.make_closure(f.clone(), env, None, f.name.map(|n| self.interner.get(n).clone()));
            let name = f.name.expect("declared function has a name");
            if global {
                let key = Key::S(self.interner.get(name).clone());
                let g = self.global.clone();
                self.define_data(&g, key, Value::Obj(fo), ALL);
            } else {
                self.declare(env, name, 0, Value::Obj(fo), true);
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Functions
    // -----------------------------------------------------------------------

    pub fn make_closure(&mut self, def: Rc<FuncDef>, env: &Env, home: Option<Obj>, name: Option<JsStr>) -> Obj {
        let key = match (def.is_async, def.is_generator) {
            (true, false) => Some("async_function"),
            (false, true) => Some("generator_function"),
            (true, true) => Some("async_generator_function"),
            _ => None,
        };
        let proto = key.and_then(|k| self.protos().extra.get(k).cloned()).unwrap_or_else(|| self.protos().function.clone());
        let clo = Closure { def, env: env.clone(), home, is_class_ctor: false, derived: false, fields: Rc::new(Vec::new()) };
        let o = Obj::new(Object::new(Some(proto), Class::Function(Func::Closure(Box::new(clo)))));
        if let Some(n) = name {
            self.define_data(&o, self.names.name.clone(), Value::Str(n), CONFIGURABLE);
        }
        o
    }

    pub fn call(&mut self, f: &Value, this: Value, args: &[Value]) -> Res {
        let Value::Obj(fo) = f else {
            return Err(self.error_ctl("TypeError", &format!("{} is not a function", self.describe(f))));
        };
        self.call_obj(fo, this, args, None)
    }

    pub fn call_obj(&mut self, fo: &Obj, this: Value, args: &[Value], new_target: Option<Obj>) -> Res {
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth -= 1;
            return Err(self.error_ctl("RangeError", "Maximum call stack size exceeded"));
        }
        let r = self.call_inner(fo, this, args, new_target);
        self.depth -= 1;
        r
    }

    fn call_inner(&mut self, fo: &Obj, this: Value, args: &[Value], new_target: Option<Obj>) -> Res {
        self.tick()?;
        enum Kind {
            Closure(Rc<FuncDef>, Env, Option<Obj>, bool, bool),
            Native(NativeFn),
            Bound(Obj, Value, Vec<Value>),
            Proxy(Obj, Obj),
            None,
        }
        let kind = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) => Kind::Closure(c.def.clone(), c.env.clone(), c.home.clone(), c.is_class_ctor, c.derived),
            Class::Function(Func::Native { f, .. }) => Kind::Native(*f),
            Class::Function(Func::Bound { target, this, args }) => Kind::Bound(target.clone(), this.clone(), args.clone()),
            Class::Proxy { target, handler } => Kind::Proxy(target.clone(), handler.clone()),
            _ => Kind::None,
        };
        match kind {
            Kind::Native(f) => {
                let call = Call { this, args, callee: fo, new_target };
                f(self, &call)
            }
            Kind::Closure(def, env, home, is_class, derived) => {
                if is_class && new_target.is_none() {
                    return Err(self.error_ctl("TypeError", "Class constructor cannot be invoked without 'new'"));
                }
                self.call_closure(fo, def, env, home, this, args, new_target, derived)
            }
            Kind::Bound(target, bthis, mut bargs) => {
                bargs.extend_from_slice(args);
                let nt = new_target.map(|n| if n.ptr_eq(fo) { target.clone() } else { n });
                if nt.is_some() {
                    return self.construct(&Value::Obj(target), &bargs, nt);
                }
                self.call_obj(&target, bthis, &bargs, None)
            }
            Kind::Proxy(target, handler) => {
                let trap = self.get_obj(&handler, &Key::str("apply"), &Value::Obj(handler.clone()))?;
                if trap.is_undefined() {
                    return self.call_obj(&target, this, args, None);
                }
                let arr = self.new_array(args.to_vec());
                self.call(&trap, Value::Obj(handler), &[Value::Obj(target), this, arr])
            }
            Kind::None => Err(self.error_ctl("TypeError", "value is not a function")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn call_closure(&mut self, fo: &Obj, def: Rc<FuncDef>, env: Env, home: Option<Obj>, this: Value, args: &[Value], new_target: Option<Obj>, derived: bool) -> Res {
        let func_ctx = if def.is_arrow {
            None
        } else {
            let this_val = if derived && new_target.is_some() {
                None
            } else if def.strict {
                Some(this.clone())
            } else {
                Some(match &this {
                    Value::Undefined | Value::Null => Value::Obj(self.global.clone()),
                    Value::Obj(_) => this.clone(),
                    prim => self.to_object(prim).map(Value::Obj).unwrap_or(Value::Undefined),
                })
            };
            Some(FuncCtx {
                this: RefCell::new(this_val),
                callee: Some(fo.clone()),
                new_target: new_target.clone().map_or(Value::Undefined, Value::Obj),
                home,
                is_global: false,
                strict: def.strict,
            })
        };
        let scope = self.new_scope(&env, func_ctx);
        self.bind_params(&def, &scope, args, fo)?;
        if def.is_generator {
            let gp = self.protos().generator.clone();
            let proto = {
                let p = self.get_obj(fo, &self.names.prototype.clone(), &Value::Obj(fo.clone()))?;
                match p {
                    Value::Obj(o) => o,
                    _ => gp,
                }
            };
            let g = GenData { values: Vec::new(), pos: 0, started: false, ret: Value::Undefined, pending: Some((def.clone(), scope)) };
            return Ok(Value::Obj(Obj::new(Object::new(Some(proto), Class::Generator(Box::new(g))))));
        }
        if def.is_async {
            return self.run_async(&def, &scope);
        }
        let r = self.run_body(&def, &scope)?;
        if let (Some(nt), true) = (&new_target, !def.is_arrow) {
            // constructors: object results replace `this`
            let _ = nt;
            if let Value::Obj(_) = &r {
                return Ok(r);
            }
            return self.this_value(&scope);
        }
        Ok(r)
    }

    pub fn run_async(&mut self, def: &Rc<FuncDef>, scope: &Env) -> Res {
        let promise = self.new_promise();
        match self.run_body(def, scope) {
            Ok(v) => self.resolve_promise(&promise, v)?,
            Err(Ctl::Throw(e)) => self.reject_promise(&promise, e),
            Err(Ctl::GenStop) => {}
            Err(other) => return Err(other),
        }
        Ok(Value::Obj(promise))
    }

    /// Execute the function body in `scope` and return its completion value.
    pub fn run_body(&mut self, def: &Rc<FuncDef>, scope: &Env) -> Res {
        match &def.body {
            FuncBody::Expr(e) => self.eval(e, scope),
            FuncBody::Block(b) => {
                for &n in &def.var_names {
                    if !scope.vars.borrow().contains_key(&n) {
                        self.declare(scope, n, 0, Value::Undefined, true);
                    }
                }
                self.hoist_block(b, scope, false)?;
                let mut labels = Vec::new();
                for s in &b.stmts {
                    match self.exec(s, scope, &mut labels) {
                        Ok(()) => {}
                        Err(Ctl::Return(v)) => return Ok(v),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Undefined)
            }
        }
    }

    fn bind_params(&mut self, def: &Rc<FuncDef>, scope: &Env, args: &[Value], fo: &Obj) -> Res<()> {
        if def.uses_args && !def.is_arrow {
            let argobj = self.new_array(args.to_vec());
            if let Value::Obj(o) = &argobj {
                let op = self.protos().object.clone();
                o.borrow_mut().proto = Some(op);
                let it = self.wk().iterator.clone();
                let values = self.get_obj(&self.protos().array.clone(), &Key::str("values"), &Value::Undefined).unwrap_or(Value::Undefined);
                self.define_data(o, Key::Y(it.0.id), values, HIDDEN);
                self.define_data(o, Key::str("callee"), Value::Obj(fo.clone()), HIDDEN);
            }
            let an = self.names.arguments;
            self.declare(scope, an, 0, argobj, true);
        }
        if def.simple_params {
            let mut vars = scope.vars.borrow_mut();
            for (i, p) in def.params.iter().enumerate() {
                if let Pat::Ident(n) = &p.pat {
                    vars.insert(*n, Var { v: args.get(i).cloned().unwrap_or(Value::Undefined), kind: 0, init: true });
                }
            }
            return Ok(());
        }
        for (i, p) in def.params.iter().enumerate() {
            if p.rest {
                let rest = self.new_array(args.get(i..).map(|s| s.to_vec()).unwrap_or_default());
                self.bind_pattern(&p.pat, rest, scope, Some(VarKind::Var))?;
                continue;
            }
            let mut v = args.get(i).cloned().unwrap_or(Value::Undefined);
            if v.is_undefined() {
                if let Some(d) = &p.default {
                    v = self.eval_named_pat(d, &p.pat, scope)?;
                }
            }
            if let Pat::Ident(n) = &p.pat {
                self.declare(scope, *n, 0, v, true);
            } else {
                // declare names first so defaults can refer to earlier params
                let mut names = Vec::new();
                super::parser::pat_names(&p.pat, &mut names);
                for n in names {
                    if !scope.vars.borrow().contains_key(&n) {
                        self.declare(scope, n, 0, Value::Undefined, true);
                    }
                }
                self.bind_pattern(&p.pat, v, scope, Some(VarKind::Var))?;
            }
        }
        Ok(())
    }

    pub fn construct(&mut self, f: &Value, args: &[Value], new_target: Option<Obj>) -> Res {
        let Value::Obj(fo) = f else {
            return Err(self.error_ctl("TypeError", &format!("{} is not a constructor", self.describe(f))));
        };
        let nt = new_target.unwrap_or_else(|| fo.clone());
        enum K {
            Closure(Rc<FuncDef>, bool, bool, Rc<Vec<FieldInit>>),
            Native(NativeFn, bool),
            Bound,
            Proxy(Obj, Obj),
            None,
        }
        let k = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) => K::Closure(c.def.clone(), c.is_class_ctor, c.derived, c.fields.clone()),
            Class::Function(Func::Native { f, ctor, .. }) => K::Native(*f, *ctor),
            Class::Function(Func::Bound { .. }) => K::Bound,
            Class::Proxy { target, handler } => K::Proxy(target.clone(), handler.clone()),
            _ => K::None,
        };
        match k {
            K::Native(nf, ctor) => {
                if !ctor {
                    return Err(self.error_ctl("TypeError", "not a constructor"));
                }
                self.depth += 1;
                if self.depth > self.max_depth {
                    self.depth -= 1;
                    return Err(self.error_ctl("RangeError", "Maximum call stack size exceeded"));
                }
                let call = Call { this: Value::Undefined, args, callee: fo, new_target: Some(nt) };
                let r = nf(self, &call);
                self.depth -= 1;
                r
            }
            K::Closure(def, is_class, derived, fields) => {
                if def.is_arrow || def.is_async || def.is_generator || matches!(def.kind, FnKind::Method | FnKind::Getter | FnKind::Setter) && !is_class {
                    return Err(self.error_ctl("TypeError", &format!("{} is not a constructor", self.describe(f))));
                }
                if derived {
                    return self.construct_derived(fo, &nt, args, &fields);
                }
                let proto = self.proto_for(&nt, false)?;
                let this = Obj::new(Object::new(Some(proto), Class::Plain));
                self.init_fields(&this, &fields)?;
                let r = self.call_obj(fo, Value::Obj(this.clone()), args, Some(nt))?;
                let _ = is_class;
                Ok(match r {
                    Value::Obj(_) => r,
                    _ => Value::Obj(this),
                })
            }
            K::Bound => self.call_obj(fo, Value::Undefined, args, Some(nt)),
            K::Proxy(target, handler) => {
                let trap = self.get_obj(&handler, &Key::str("construct"), &Value::Obj(handler.clone()))?;
                if trap.is_undefined() {
                    return self.construct(&Value::Obj(target), args, None);
                }
                let arr = self.new_array(args.to_vec());
                self.call(&trap, Value::Obj(handler), &[Value::Obj(target), arr, Value::Obj(nt)])
            }
            K::None => Err(self.error_ctl("TypeError", &format!("{} is not a constructor", self.describe(f)))),
        }
    }

    /// `new_target.prototype` or the fallback.
    pub fn proto_for(&mut self, nt: &Obj, array: bool) -> Res<Obj> {
        let p = self.get_obj(nt, &self.names.prototype.clone(), &Value::Obj(nt.clone()))?;
        Ok(match p {
            Value::Obj(o) => o,
            _ => {
                if array {
                    self.protos().array.clone()
                } else {
                    self.protos().object.clone()
                }
            }
        })
    }

    fn construct_derived(&mut self, fo: &Obj, nt: &Obj, args: &[Value], fields: &Rc<Vec<FieldInit>>) -> Res {
        // default constructor behaviour: forward to the parent
        let def = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) => c.def.clone(),
            _ => unreachable!(),
        };
        let is_default = matches!(&def.body, FuncBody::Block(b) if b.stmts.is_empty()) && def.params.is_empty() && def.span.0 == def.span.1;
        if is_default {
            let parent = fo.borrow().proto.clone();
            let Some(p) = parent else {
                return Err(self.error_ctl("TypeError", "Super constructor is not a constructor"));
            };
            let this = self.construct(&Value::Obj(p), args, Some(nt.clone()))?;
            if let Value::Obj(o) = &this {
                self.init_fields(o, fields)?;
            }
            return Ok(this);
        }
        // `this` is bound when super(...) runs; the closure call returns it
        let env = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) => c.env.clone(),
            _ => unreachable!(),
        };
        let home = match &fo.borrow().class {
            Class::Function(Func::Closure(c)) => c.home.clone(),
            _ => None,
        };
        let ctx = FuncCtx {
            this: RefCell::new(None),
            callee: Some(fo.clone()),
            new_target: Value::Obj(nt.clone()),
            home,
            is_global: false,
            strict: true,
        };
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth -= 1;
            return Err(self.error_ctl("RangeError", "Maximum call stack size exceeded"));
        }
        let scope = self.new_scope(&env, Some(ctx));
        let r = (|| {
            self.bind_params(&def, &scope, args, fo)?;
            let ret = self.run_body(&def, &scope)?;
            if let Value::Obj(_) = ret {
                return Ok(ret);
            }
            self.this_value(&scope)
        })();
        self.depth -= 1;
        r
    }

    pub fn init_fields(&mut self, this: &Obj, fields: &Rc<Vec<FieldInit>>) -> Res<()> {
        for f in fields.iter() {
            let ClassMemberKind::Field(init) = &f.class.members[f.idx].kind else { continue };
            let ctx = FuncCtx {
                this: RefCell::new(Some(Value::Obj(this.clone()))),
                callee: None,
                new_target: Value::Undefined,
                home: Some(f.home.clone()),
                is_global: false,
                strict: true,
            };
            let scope = self.new_scope(&f.env, Some(ctx));
            let v = match init {
                Some(e) => self.eval(e, &scope)?,
                None => Value::Undefined,
            };
            let is_private = matches!(&f.key, Key::S(s) if s.starts_with('#'));
            self.define_data(this, f.key.clone(), v, if is_private { HIDDEN } else { ALL });
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn exec_block(&mut self, b: &Block, env: &Env) -> Res<()> {
        let scope = if b.lets.is_empty() && b.funcs.is_empty() {
            env.clone()
        } else {
            let s = self.new_scope(env, None);
            self.hoist_block(b, &s, false)?;
            s
        };
        let mut labels = Vec::new();
        for st in &b.stmts {
            self.exec(st, &scope, &mut labels)?;
        }
        Ok(())
    }

    pub fn exec(&mut self, s: &Stmt, env: &Env, labels: &mut Vec<Name>) -> Res<()> {
        self.tick()?;
        match s {
            Stmt::Expr(e) => {
                self.eval(e, env)?;
                Ok(())
            }
            Stmt::Var(kind, decls) => {
                for (pat, init) in decls {
                    match init {
                        Some(e) => {
                            let v = self.eval_named_pat(e, pat, env)?;
                            self.bind_pattern(pat, v, env, Some(*kind))?;
                        }
                        None => {
                            if *kind != VarKind::Var {
                                self.bind_pattern(pat, Value::Undefined, env, Some(*kind))?;
                            }
                        }
                    }
                }
                Ok(())
            }
            Stmt::Func(_) | Stmt::Empty => Ok(()),
            Stmt::Class(c) => {
                let v = self.eval_class(c, env, None)?;
                if let Some(n) = c.name {
                    self.init_binding(n, v, env, VarKind::Let)?;
                }
                Ok(())
            }
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Undefined,
                };
                Err(Ctl::Return(v))
            }
            Stmt::If(t, c, a) => {
                if self.eval(t, env)?.truthy() {
                    self.exec(c, env, &mut Vec::new())
                } else if let Some(a) = a {
                    self.exec(a, env, &mut Vec::new())
                } else {
                    Ok(())
                }
            }
            Stmt::Block(b) => self.exec_block(b, env),
            Stmt::For { .. } | Stmt::ForIn { .. } | Stmt::ForOf { .. } | Stmt::While(..) | Stmt::DoWhile(..) => {
                let ls = core::mem::take(labels);
                self.exec_loop(s, env, &ls)
            }
            Stmt::With(obj, body) => {
                let v = self.eval(obj, env)?;
                let o = self.to_object(&v)?;
                let scope = Rc::new(Scope { vars: RefCell::new(BTreeMap::new()), parent: Some(env.clone()), func: None, with_obj: Some(o) });
                self.exec(body, &scope, &mut Vec::new())
            }
            Stmt::Break(l) => Err(Ctl::Break(*l)),
            Stmt::Continue(l) => Err(Ctl::Continue(*l)),
            Stmt::Throw(e) => {
                let v = self.eval(e, env)?;
                Err(Ctl::Throw(v))
            }
            Stmt::Try { block, param, handler, finalizer } => {
                let mut result = self.exec_block(block, env);
                if let (Err(Ctl::Throw(exc)), Some(h)) = (&result, handler) {
                    let exc = exc.clone();
                    let scope = self.new_scope(env, None);
                    result = (|| {
                        if let Some(p) = param {
                            let mut names = Vec::new();
                            super::parser::pat_names(p, &mut names);
                            for n in names {
                                self.declare(&scope, n, 1, Value::Undefined, true);
                            }
                            self.bind_pattern(p, exc, &scope, Some(VarKind::Let))?;
                        }
                        self.exec_block(h, &scope)
                    })();
                }
                if let Some(f) = finalizer {
                    // GenStop must not be intercepted by finally blocks
                    if !matches!(result, Err(Ctl::GenStop)) {
                        self.exec_block(f, env)?;
                    }
                }
                result
            }
            Stmt::Switch { disc, cases, lets, funcs } => {
                let d = self.eval(disc, env)?;
                let scope = if lets.is_empty() && funcs.is_empty() {
                    env.clone()
                } else {
                    let s = self.new_scope(env, None);
                    let b = Block { stmts: Vec::new(), lets: lets.clone(), funcs: funcs.clone() };
                    self.hoist_block(&b, &s, false)?;
                    s
                };
                let mut matched: Option<usize> = None;
                for (i, c) in cases.iter().enumerate() {
                    if let Some(t) = &c.test {
                        let tv = self.eval(t, &scope)?;
                        if strict_equals(&d, &tv) {
                            matched = Some(i);
                            break;
                        }
                    }
                }
                if matched.is_none() {
                    matched = cases.iter().position(|c| c.test.is_none());
                }
                if let Some(start) = matched {
                    'outer: for c in &cases[start..] {
                        for st in &c.body {
                            match self.exec(st, &scope, &mut Vec::new()) {
                                Ok(()) => {}
                                Err(Ctl::Break(None)) => break 'outer,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
                Ok(())
            }
            Stmt::Labeled(l, inner) => {
                labels.push(*l);
                let is_loop = matches!(**inner, Stmt::For { .. } | Stmt::ForIn { .. } | Stmt::ForOf { .. } | Stmt::While(..) | Stmt::DoWhile(..) | Stmt::Labeled(..));
                let r = if is_loop {
                    self.exec(inner, env, labels)
                } else {
                    labels.clear();
                    self.exec(inner, env, &mut Vec::new())
                };
                match r {
                    Err(Ctl::Break(Some(b))) if b == *l => Ok(()),
                    other => other,
                }
            }
            Stmt::Import { .. } | Stmt::ExportNamed { .. } | Stmt::ExportAll(..) => Ok(()),
            Stmt::ExportDecl(inner, _) => self.exec(inner, env, labels),
            Stmt::ExportDefault(e) => {
                let v = self.eval(e, env)?;
                let n = self.interner.intern("*default*");
                env.vars.borrow_mut().insert(n, Var { v, kind: 1, init: true });
                Ok(())
            }
        }
    }

    /// Returns Ok(true) when the loop must stop because of `break` aimed at it.
    fn loop_ctl(r: Res<()>, labels: &[Name]) -> Res<bool> {
        match r {
            Ok(()) => Ok(false),
            Err(Ctl::Break(None)) => Ok(true),
            Err(Ctl::Break(Some(l))) if labels.contains(&l) => Ok(true),
            Err(Ctl::Continue(None)) => Ok(false),
            Err(Ctl::Continue(Some(l))) if labels.contains(&l) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn exec_loop(&mut self, s: &Stmt, env: &Env, labels: &[Name]) -> Res<()> {
        match s {
            Stmt::While(t, body) => {
                while self.eval(t, env)?.truthy() {
                    self.tick()?;
                    if Self::loop_ctl(self.exec(body, env, &mut Vec::new()), labels)? {
                        break;
                    }
                }
                Ok(())
            }
            Stmt::DoWhile(body, t) => {
                loop {
                    self.tick()?;
                    if Self::loop_ctl(self.exec(body, env, &mut Vec::new()), labels)? {
                        break;
                    }
                    if !self.eval(t, env)?.truthy() {
                        break;
                    }
                }
                Ok(())
            }
            Stmt::For { init, test, update, body, lets } => {
                let mut scope = if lets.is_empty() { env.clone() } else { self.new_scope(env, None) };
                match init {
                    Some(ForInit::Var(kind, decls)) => {
                        for n in lets {
                            let k = if *kind == VarKind::Const { 2 } else { 1 };
                            self.declare(&scope, *n, k, Value::Undefined, false);
                        }
                        for (pat, i) in decls {
                            let v = match i {
                                Some(e) => self.eval_named_pat(e, pat, &scope)?,
                                None => Value::Undefined,
                            };
                            if i.is_some() || *kind != VarKind::Var {
                                self.bind_pattern(pat, v, &scope, Some(*kind))?;
                            }
                        }
                    }
                    Some(ForInit::Expr(e)) => {
                        self.eval(e, &scope)?;
                    }
                    None => {}
                }
                loop {
                    self.tick()?;
                    if let Some(t) = test {
                        if !self.eval(t, &scope)?.truthy() {
                            break;
                        }
                    }
                    if Self::loop_ctl(self.exec(body, &scope, &mut Vec::new()), labels)? {
                        break;
                    }
                    // fresh per-iteration copy of the loop variables (closures capture each iteration)
                    if !lets.is_empty() {
                        let next = self.new_scope(env, None);
                        {
                            let old = scope.vars.borrow();
                            let mut nv = next.vars.borrow_mut();
                            for (k, v) in old.iter() {
                                nv.insert(*k, Var { v: v.v.clone(), kind: v.kind, init: v.init });
                            }
                        }
                        scope = next;
                    }
                    if let Some(u) = update {
                        self.eval(u, &scope)?;
                    }
                }
                Ok(())
            }
            Stmt::ForIn { head, obj, body } => {
                let o = self.eval(obj, env)?;
                if o.is_nullish() {
                    return Ok(());
                }
                let keys = self.for_in_keys(&o)?;
                for k in keys {
                    self.tick()?;
                    let scope = self.new_scope(env, None);
                    self.bind_head(head, Value::Str(k), &scope)?;
                    if Self::loop_ctl(self.exec(body, &scope, &mut Vec::new()), labels)? {
                        break;
                    }
                }
                Ok(())
            }
            Stmt::ForOf { head, iter, body, is_await } => {
                let it = self.eval(iter, env)?;
                // arrays: iterate live by index
                if let Value::Obj(o) = &it {
                    if matches!(o.borrow().class, Class::Array(_)) && self.array_iter_pristine(o) {
                        let mut i = 0;
                        loop {
                            let v = match &o.borrow().class {
                                Class::Array(a) => a.get(i).cloned(),
                                _ => None,
                            };
                            let Some(mut v) = v else { break };
                            i += 1;
                            if *is_await {
                                v = self.await_value(v)?;
                            }
                            self.tick()?;
                            let scope = self.new_scope(env, None);
                            self.bind_head(head, v, &scope)?;
                            if Self::loop_ctl(self.exec(body, &scope, &mut Vec::new()), labels)? {
                                break;
                            }
                        }
                        return Ok(());
                    }
                }
                let iter_obj = self.get_iterator(&it, *is_await)?;
                loop {
                    self.tick()?;
                    let step = self.iter_step(&iter_obj)?;
                    let Some(mut v) = step else { break };
                    if *is_await {
                        v = self.await_value(v)?;
                    }
                    let scope = self.new_scope(env, None);
                    self.bind_head(head, v, &scope)?;
                    match Self::loop_ctl(self.exec(body, &scope, &mut Vec::new()), labels) {
                        Ok(false) => {}
                        Ok(true) => {
                            self.iter_close(&iter_obj);
                            break;
                        }
                        Err(e) => {
                            self.iter_close(&iter_obj);
                            return Err(e);
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn array_iter_pristine(&mut self, o: &Obj) -> bool {
        // own Symbol.iterator override or modified prototype iterator would break the fast path
        let it = Key::Y(self.wk().iterator.0.id);
        if o.borrow().props.get(&it).is_some() {
            return false;
        }
        let proto = o.borrow().proto.clone();
        match proto {
            Some(p) => p.ptr_eq(&self.protos().array) && self.array_values_native,
            None => false,
        }
    }

    fn bind_head(&mut self, head: &ForHead, v: Value, scope: &Env) -> Res<()> {
        match head {
            ForHead::Var(kind, pat) => {
                if *kind != VarKind::Var {
                    let mut names = Vec::new();
                    super::parser::pat_names(pat, &mut names);
                    for n in names {
                        self.declare(scope, n, 1, Value::Undefined, false);
                    }
                }
                self.bind_pattern(pat, v, scope, Some(*kind))
            }
            ForHead::Pat(pat) => self.bind_pattern(pat, v, scope, None),
        }
    }

    // -----------------------------------------------------------------------
    // Destructuring
    // -----------------------------------------------------------------------

    /// Bind `v` to `pat`. `kind` is Some for declarations and None for plain assignment.
    pub fn bind_pattern(&mut self, pat: &Pat, v: Value, env: &Env, kind: Option<VarKind>) -> Res<()> {
        match pat {
            Pat::Ident(n) => match kind {
                Some(k) => self.init_binding(*n, v, env, k),
                None => self.assign_var(*n, v, env),
            },
            Pat::Expr(e) => self.assign_to_expr(e, v, env),
            Pat::Array(elems, rest) => {
                if v.is_nullish() {
                    return Err(self.error_ctl("TypeError", &format!("{} is not iterable", self.describe(&v))));
                }
                let items = self.iter_to_vec_limit(&v, if rest.is_some() { usize::MAX } else { elems.len() })?;
                for (i, el) in elems.iter().enumerate() {
                    let Some(el) = el else { continue };
                    let mut item = items.get(i).cloned().unwrap_or(Value::Undefined);
                    if item.is_undefined() {
                        if let Some(d) = &el.default {
                            item = self.eval_named_pat(d, &el.pat, env)?;
                        }
                    }
                    self.bind_pattern(&el.pat, item, env, kind)?;
                }
                if let Some(r) = rest {
                    let tail = items.get(elems.len()..).map(|s| s.to_vec()).unwrap_or_default();
                    let arr = self.new_array(tail);
                    self.bind_pattern(r, arr, env, kind)?;
                }
                Ok(())
            }
            Pat::Object(props, rest) => {
                if v.is_nullish() {
                    return Err(self.error_ctl("TypeError", &format!("Cannot destructure '{}' as it is {}.", self.describe(&v), self.describe(&v))));
                }
                let mut used: Vec<Key> = Vec::new();
                for p in props {
                    let key = self.eval_prop_key(&p.key, env)?;
                    used.push(key.clone());
                    let mut item = self.get_v(&v, &key)?;
                    if item.is_undefined() {
                        if let Some(d) = &p.default {
                            item = self.eval_named_pat(d, &p.pat, env)?;
                        }
                    }
                    self.bind_pattern(&p.pat, item, env, kind)?;
                }
                if let Some(r) = rest {
                    let out = self.new_object();
                    if let Value::Obj(src) = &v {
                        for k in self.own_keys(src)? {
                            if used.contains(&k) {
                                continue;
                            }
                            if let Some(p) = self.own_prop_mut(src, &k)? {
                                if p.flags & ENUMERABLE != 0 {
                                    let val = self.get_obj(src, &k, &v)?;
                                    self.define_data(&out, k, val, ALL);
                                }
                            }
                        }
                    }
                    self.bind_pattern(r, Value::Obj(out), env, kind)?;
                }
                Ok(())
            }
        }
    }

    pub fn eval_prop_key(&mut self, k: &PropKey, env: &Env) -> Res<Key> {
        match k {
            PropKey::Name(n) => Ok(Key::S(n.clone())),
            PropKey::Private(n) => Ok(Key::S(Rc::from(format!("#{n}")))),
            PropKey::Computed(e) => {
                let v = self.eval(e, env)?;
                self.to_key(&v)
            }
        }
    }

    fn eval_named_pat(&mut self, e: &Expr, pat: &Pat, env: &Env) -> Res {
        match pat {
            Pat::Ident(n) => {
                let name = self.interner.get(*n).clone();
                self.eval_named(e, Some(name), env)
            }
            _ => self.eval(e, env),
        }
    }

    /// Evaluate with function-name inference for anonymous functions and classes.
    pub fn eval_named(&mut self, e: &Expr, name: Option<JsStr>, env: &Env) -> Res {
        match e {
            Expr::Function(f) if f.name.is_none() => {
                let o = self.make_closure(f.clone(), env, None, name);
                Ok(Value::Obj(o))
            }
            Expr::Class(c) if c.name.is_none() => self.eval_class(c, env, name),
            Expr::Paren(inner) => self.eval_named(inner, name, env),
            _ => self.eval(e, env),
        }
    }

    fn assign_to_expr(&mut self, target: &Expr, v: Value, env: &Env) -> Res<()> {
        match target {
            Expr::Ident(n) => self.assign_var(*n, v, env),
            Expr::Dot { obj, name, .. } => {
                let o = self.eval(obj, env)?;
                self.set_v(&o, Key::S(name.clone()), v)
            }
            Expr::Member { obj, prop, .. } => {
                let o = self.eval(obj, env)?;
                let k = self.eval(prop, env)?;
                self.set_member(&o, &k, v)
            }
            Expr::PrivateMember { obj, name, .. } => {
                let o = self.eval(obj, env)?;
                self.set_v(&o, Key::S(Rc::from(format!("#{name}"))), v)
            }
            Expr::SuperDot(name) => {
                let this = self.this_value(env)?;
                self.set_v(&this, Key::S(name.clone()), v)
            }
            Expr::Paren(e) => self.assign_to_expr(e, v, env),
            _ => Err(self.error_ctl("SyntaxError", "Invalid assignment target")),
        }
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    pub fn eval(&mut self, e: &Expr, env: &Env) -> Res {
        match e {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::This => self.this_value(env),
            Expr::Ident(n) => {
                if *n == self.names.undefined {
                    // `undefined` is a global binding but shadowing it is vanishingly rare
                    let mut cur = env;
                    loop {
                        if cur.vars.borrow().contains_key(n) {
                            break;
                        }
                        match &cur.parent {
                            Some(p) => cur = p,
                            None => return Ok(Value::Undefined),
                        }
                    }
                }
                self.lookup(*n, env)
            }
            Expr::Paren(inner) => self.eval(inner, env),
            Expr::Template(strs, exprs) => {
                let mut out = String::new();
                for (i, s) in strs.iter().enumerate() {
                    out.push_str(s);
                    if let Some(e) = exprs.get(i) {
                        let v = self.eval(e, env)?;
                        let sv = self.to_str(&v)?;
                        out.push_str(&sv);
                    }
                }
                Ok(Value::string(out))
            }
            Expr::Tagged(tag, tobj, exprs) => {
                let (f, this) = self.eval_callee(tag, env)?;
                let strings = self.template_object(tobj);
                let mut args = vec![strings];
                for e in exprs {
                    args.push(self.eval(e, env)?);
                }
                self.call(&f, this, &args)
            }
            Expr::Regex(p, f) => self.new_regexp(p, f),
            Expr::Array(elems) => {
                let mut out = Vec::with_capacity(elems.len());
                for el in elems {
                    match el {
                        None => out.push(Value::Undefined),
                        Some(ArgElem::Expr(e)) => out.push(self.eval(e, env)?),
                        Some(ArgElem::Spread(e)) => {
                            let v = self.eval(e, env)?;
                            out.extend(self.iter_to_vec(&v)?);
                        }
                    }
                }
                Ok(self.new_array(out))
            }
            Expr::Object(props) => self.eval_object(props, env),
            Expr::Function(f) => {
                let name = f.name.map(|n| self.interner.get(n).clone());
                if f.name.is_some() && !f.is_arrow {
                    // named function expressions can refer to themselves
                    let scope = self.new_scope(env, None);
                    let o = self.make_closure(f.clone(), &scope, None, name);
                    self.declare(&scope, f.name.expect("named"), 0, Value::Obj(o.clone()), true);
                    return Ok(Value::Obj(o));
                }
                Ok(Value::Obj(self.make_closure(f.clone(), env, None, name)))
            }
            Expr::Class(c) => self.eval_class(c, env, None),
            Expr::Unary(op, x) => self.eval_unary(*op, x, env),
            Expr::Update { inc, prefix, target } => {
                let old = match &**target {
                    Expr::Ident(_) | Expr::Dot { .. } | Expr::Member { .. } | Expr::Paren(_) | Expr::PrivateMember { .. } => self.eval(target, env)?,
                    _ => return Err(self.error_ctl("SyntaxError", "Invalid left-hand side expression in postfix operation")),
                };
                let n = self.to_number(&old)?;
                let new = if *inc { n + 1.0 } else { n - 1.0 };
                self.assign_to_expr(target, Value::Num(new), env)?;
                Ok(Value::Num(if *prefix { new } else { n }))
            }
            Expr::Binary(op, a, b) => {
                let l = self.eval(a, env)?;
                let r = self.eval(b, env)?;
                self.binary_op(*op, &l, &r)
            }
            Expr::Logical(op, a, b) => {
                let l = self.eval(a, env)?;
                let short = match op {
                    LogOp::And => !l.truthy(),
                    LogOp::Or => l.truthy(),
                    LogOp::Nullish => !l.is_nullish(),
                };
                if short { Ok(l) } else { self.eval(b, env) }
            }
            Expr::Assign(op, target, value) => self.eval_assign(*op, target, value, env),
            Expr::Cond(t, a, b) => {
                if self.eval(t, env)?.truthy() {
                    self.eval(a, env)
                } else {
                    self.eval(b, env)
                }
            }
            Expr::Seq(list) => {
                let mut last = Value::Undefined;
                for e in list {
                    last = self.eval(e, env)?;
                }
                Ok(last)
            }
            Expr::OptChain(inner) => match self.eval(inner, env) {
                Err(Ctl::OptShort) => Ok(Value::Undefined),
                other => other,
            },
            Expr::Dot { obj, name, optional } => {
                let o = self.eval(obj, env)?;
                if *optional && o.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                if o.is_nullish() {
                    return Err(self.error_ctl("TypeError", &format!("Cannot read properties of {} (reading '{}')", self.describe(&o), name)));
                }
                self.get_v(&o, &Key::S(name.clone()))
            }
            Expr::PrivateMember { obj, name, optional } => {
                let o = self.eval(obj, env)?;
                if *optional && o.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                self.get_v(&o, &Key::S(Rc::from(format!("#{name}"))))
            }
            Expr::Member { obj, prop, optional } => {
                let o = self.eval(obj, env)?;
                if *optional && o.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                let k = self.eval(prop, env)?;
                if o.is_nullish() {
                    let ks = self.to_str(&k).unwrap_or_else(|_| Rc::from("?"));
                    return Err(self.error_ctl("TypeError", &format!("Cannot read properties of {} (reading '{}')", self.describe(&o), ks)));
                }
                self.get_member(&o, &k)
            }
            Expr::Call { callee, args, optional } => {
                let (f, this) = self.eval_callee(callee, env)?;
                if *optional && f.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                let argv = self.eval_args(args, env)?;
                if !matches!(&f, Value::Obj(o) if o.is_callable()) {
                    let desc = self.callee_desc(callee);
                    return Err(self.error_ctl("TypeError", &format!("{desc} is not a function")));
                }
                self.call(&f, this, &argv)
            }
            Expr::New(callee, args) => {
                let f = self.eval(callee, env)?;
                let argv = self.eval_args(args, env)?;
                if !matches!(&f, Value::Obj(o) if o.is_callable()) {
                    let desc = self.callee_desc(callee);
                    return Err(self.error_ctl("TypeError", &format!("{desc} is not a constructor")));
                }
                self.construct(&f, &argv, None)
            }
            Expr::NewTarget => Ok(self.func_ctx(env).map_or(Value::Undefined, |f| f.new_target.clone())),
            Expr::SuperCall(args) => {
                let argv = self.eval_args(args, env)?;
                let (callee, nt) = match self.func_ctx(env) {
                    Some(f) => (f.callee.clone(), f.new_target.clone()),
                    None => (None, Value::Undefined),
                };
                let Some(callee) = callee else {
                    return Err(self.error_ctl("SyntaxError", "'super' keyword unexpected here"));
                };
                let parent = callee.borrow().proto.clone();
                let Some(parent) = parent else {
                    return Err(self.error_ctl("TypeError", "Super constructor null of anonymous class is not a constructor"));
                };
                let nt_obj = match nt {
                    Value::Obj(o) => Some(o),
                    _ => None,
                };
                let this = self.construct(&Value::Obj(parent), &argv, nt_obj)?;
                // bind `this`
                let mut cur = env;
                loop {
                    if let Some(f) = &cur.func {
                        *f.this.borrow_mut() = Some(this.clone());
                        break;
                    }
                    match &cur.parent {
                        Some(p) => cur = p,
                        None => break,
                    }
                }
                let fields = match &callee.borrow().class {
                    Class::Function(Func::Closure(c)) => c.fields.clone(),
                    _ => Rc::new(Vec::new()),
                };
                if let Value::Obj(o) = &this {
                    self.init_fields(o, &fields)?;
                }
                Ok(this)
            }
            Expr::SuperDot(name) => {
                let this = self.this_value(env)?;
                let home = self.func_ctx(env).and_then(|f| f.home.clone());
                self.super_get(home, &Key::S(name.clone()), &this)
            }
            Expr::SuperMember(p) => {
                let this = self.this_value(env)?;
                let k = self.eval(p, env)?;
                let key = self.to_key(&k)?;
                let home = self.func_ctx(env).and_then(|f| f.home.clone());
                self.super_get(home, &key, &this)
            }
            Expr::Await(x) => {
                let v = self.eval(x, env)?;
                self.await_value(v)
            }
            Expr::Yield(arg, delegate) => {
                let v = match arg {
                    Some(a) => self.eval(a, env)?,
                    None => Value::Undefined,
                };
                if self.gen_stack.is_empty() {
                    return Err(self.error_ctl("SyntaxError", "yield outside generator"));
                }
                if *delegate {
                    let items = self.iter_to_vec(&v)?;
                    for it in items {
                        self.gen_push(it)?;
                    }
                    return Ok(Value::Undefined);
                }
                self.gen_push(v)?;
                Ok(Value::Undefined)
            }
            Expr::ImportCall(spec) => {
                let s = self.eval(spec, env)?;
                self.dynamic_import_value(&s)
            }
            Expr::ImportMeta => {
                let o = self.new_object();
                let url = self.current_module_url();
                self.define_data(&o, Key::str("url"), Value::string(url), ALL);
                Ok(Value::Obj(o))
            }
        }
    }

    fn gen_push(&mut self, v: Value) -> Res<()> {
        let buf = self.gen_stack.last_mut().expect("generator buffer");
        buf.push(v);
        if buf.len() >= GEN_LIMIT {
            return Err(Ctl::GenStop);
        }
        Ok(())
    }

    fn callee_desc(&self, e: &Expr) -> String {
        match e {
            Expr::Ident(n) => self.interner.get(*n).to_string(),
            Expr::Dot { obj, name, .. } => format!("{}.{}", self.callee_desc(obj), name),
            Expr::This => "this".to_string(),
            Expr::Paren(i) => self.callee_desc(i),
            Expr::Member { obj, .. } => format!("{}[...]", self.callee_desc(obj)),
            Expr::Call { callee, .. } => format!("{}(...)", self.callee_desc(callee)),
            _ => "expression".to_string(),
        }
    }

    fn template_object(&mut self, t: &Rc<TemplateObj>) -> Value {
        let strs: Vec<Value> = t.cooked.iter().map(|c| c.clone().map_or(Value::Undefined, Value::Str)).collect();
        let raw: Vec<Value> = t.raw.iter().map(|r| Value::Str(r.clone())).collect();
        let arr = self.new_array(strs);
        let rawv = self.new_array(raw);
        if let Value::Obj(o) = &arr {
            self.define_data(o, Key::str("raw"), rawv, 0);
        }
        arr
    }

    pub fn eval_args(&mut self, args: &[ArgElem], env: &Env) -> Res<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            match a {
                ArgElem::Expr(e) => out.push(self.eval(e, env)?),
                ArgElem::Spread(e) => {
                    let v = self.eval(e, env)?;
                    out.extend(self.iter_to_vec(&v)?);
                }
            }
        }
        Ok(out)
    }

    /// Evaluate a callee expression, returning the function and the `this` value to call it with.
    fn eval_callee(&mut self, callee: &Expr, env: &Env) -> Res<(Value, Value)> {
        match callee {
            Expr::Dot { obj, name, optional } => {
                let o = self.eval(obj, env)?;
                if *optional && o.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                if o.is_nullish() {
                    return Err(self.error_ctl("TypeError", &format!("Cannot read properties of {} (reading '{}')", self.describe(&o), name)));
                }
                let f = self.get_v(&o, &Key::S(name.clone()))?;
                Ok((f, o))
            }
            Expr::Member { obj, prop, optional } => {
                let o = self.eval(obj, env)?;
                if *optional && o.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                let k = self.eval(prop, env)?;
                if o.is_nullish() {
                    return Err(self.error_ctl("TypeError", &format!("Cannot read properties of {}", self.describe(&o))));
                }
                let f = self.get_member(&o, &k)?;
                Ok((f, o))
            }
            Expr::PrivateMember { obj, name, optional } => {
                let o = self.eval(obj, env)?;
                if *optional && o.is_nullish() {
                    return Err(Ctl::OptShort);
                }
                let f = self.get_v(&o, &Key::S(Rc::from(format!("#{name}"))))?;
                Ok((f, o))
            }
            Expr::SuperDot(name) => {
                let this = self.this_value(env)?;
                let home = self.func_ctx(env).and_then(|f| f.home.clone());
                let f = self.super_get(home, &Key::S(name.clone()), &this)?;
                Ok((f, this))
            }
            Expr::Paren(inner) if matches!(**inner, Expr::Dot { .. } | Expr::Member { .. }) => self.eval_callee(inner, env),
            Expr::OptChain(inner) => match self.eval_callee(inner, env) {
                Err(Ctl::OptShort) => Ok((Value::Undefined, Value::Undefined)),
                other => other,
            },
            other => Ok((self.eval(other, env)?, Value::Undefined)),
        }
    }

    fn super_get(&mut self, home: Option<Obj>, key: &Key, this: &Value) -> Res {
        let Some(home) = home else {
            return Err(self.error_ctl("SyntaxError", "'super' keyword unexpected here"));
        };
        let proto = home.borrow().proto.clone();
        match proto {
            Some(p) => self.get_obj(&p, key, this),
            None => Ok(Value::Undefined),
        }
    }

    fn eval_unary(&mut self, op: UnOp, x: &Expr, env: &Env) -> Res {
        match op {
            UnOp::TypeOf => {
                if let Expr::Ident(n) = x {
                    let v = self.lookup_typeof(*n, env)?;
                    return Ok(Value::str(v.type_of()));
                }
                let v = self.eval(x, env)?;
                Ok(Value::str(v.type_of()))
            }
            UnOp::Delete => match x {
                Expr::Dot { obj, name, .. } => {
                    let o = self.eval(obj, env)?;
                    self.delete_v(&o, &Key::S(name.clone()))
                }
                Expr::Member { obj, prop, .. } => {
                    let o = self.eval(obj, env)?;
                    let k = self.eval(prop, env)?;
                    let key = self.to_key(&k)?;
                    self.delete_v(&o, &key)
                }
                Expr::OptChain(inner) => match self.eval_unary(UnOp::Delete, inner, env) {
                    Err(Ctl::OptShort) => Ok(Value::Bool(true)),
                    other => other,
                },
                Expr::Paren(inner) => self.eval_unary(UnOp::Delete, inner, env),
                _ => Ok(Value::Bool(true)),
            },
            UnOp::Void => {
                self.eval(x, env)?;
                Ok(Value::Undefined)
            }
            UnOp::Not => Ok(Value::Bool(!self.eval(x, env)?.truthy())),
            UnOp::Neg => {
                let v = self.eval(x, env)?;
                Ok(Value::Num(-self.to_number(&v)?))
            }
            UnOp::Plus => {
                let v = self.eval(x, env)?;
                Ok(Value::Num(self.to_number(&v)?))
            }
            UnOp::BitNot => {
                let v = self.eval(x, env)?;
                let n = self.to_number(&v)?;
                Ok(Value::Num(!to_int32(n) as f64))
            }
        }
    }

    fn eval_assign(&mut self, op: AssignOp, target: &Pat, value: &Expr, env: &Env) -> Res {
        match op {
            AssignOp::Assign => {
                let v = match target {
                    Pat::Ident(n) => {
                        let name = self.interner.get(*n).clone();
                        self.eval_named(value, Some(name), env)?
                    }
                    Pat::Expr(e) => {
                        // evaluate the target's object before the value
                        match &**e {
                            Expr::Dot { obj, name, .. } => {
                                let o = self.eval(obj, env)?;
                                let v = self.eval(value, env)?;
                                self.set_v(&o, Key::S(name.clone()), v.clone())?;
                                return Ok(v);
                            }
                            Expr::Member { obj, prop, .. } => {
                                let o = self.eval(obj, env)?;
                                let k = self.eval(prop, env)?;
                                let v = self.eval(value, env)?;
                                self.set_member(&o, &k, v.clone())?;
                                return Ok(v);
                            }
                            _ => self.eval(value, env)?,
                        }
                    }
                    _ => self.eval(value, env)?,
                };
                self.bind_pattern(target, v.clone(), env, None)?;
                Ok(v)
            }
            AssignOp::Bin(bop) => {
                let (base, key) = self.eval_target_ref(target, env)?;
                let old = self.read_ref(target, &base, &key, env)?;
                let r = self.eval(value, env)?;
                let nv = self.binary_op(bop, &old, &r)?;
                self.write_ref(target, &base, &key, nv.clone(), env)?;
                Ok(nv)
            }
            AssignOp::Logical(lop) => {
                let (base, key) = self.eval_target_ref(target, env)?;
                let old = self.read_ref(target, &base, &key, env)?;
                let short = match lop {
                    LogOp::And => !old.truthy(),
                    LogOp::Or => old.truthy(),
                    LogOp::Nullish => !old.is_nullish(),
                };
                if short {
                    return Ok(old);
                }
                let nv = match target {
                    Pat::Ident(n) => {
                        let name = self.interner.get(*n).clone();
                        self.eval_named(value, Some(name), env)?
                    }
                    _ => self.eval(value, env)?,
                };
                self.write_ref(target, &base, &key, nv.clone(), env)?;
                Ok(nv)
            }
        }
    }

    fn eval_target_ref(&mut self, target: &Pat, env: &Env) -> Res<(Value, Value)> {
        match target {
            Pat::Ident(_) => Ok((Value::Undefined, Value::Undefined)),
            Pat::Expr(e) => match &**e {
                Expr::Dot { obj, name, .. } => Ok((self.eval(obj, env)?, Value::Str(name.clone()))),
                Expr::Member { obj, prop, .. } => {
                    let o = self.eval(obj, env)?;
                    let k = self.eval(prop, env)?;
                    Ok((o, k))
                }
                Expr::PrivateMember { obj, name, .. } => Ok((self.eval(obj, env)?, Value::string(format!("#{name}")))),
                Expr::SuperDot(name) => Ok((self.this_value(env)?, Value::Str(name.clone()))),
                _ => Err(self.error_ctl("SyntaxError", "Invalid assignment target")),
            },
            _ => Err(self.error_ctl("SyntaxError", "Invalid assignment target")),
        }
    }

    fn read_ref(&mut self, target: &Pat, base: &Value, key: &Value, env: &Env) -> Res {
        match target {
            Pat::Ident(n) => self.lookup(*n, env),
            _ => {
                if base.is_nullish() {
                    return Err(self.error_ctl("TypeError", &format!("Cannot read properties of {}", self.describe(base))));
                }
                self.get_member(base, key)
            }
        }
    }

    fn write_ref(&mut self, target: &Pat, base: &Value, key: &Value, v: Value, env: &Env) -> Res<()> {
        match target {
            Pat::Ident(n) => self.assign_var(*n, v, env),
            _ => self.set_member(base, key, v),
        }
    }

    fn eval_object(&mut self, props: &[PropDef], env: &Env) -> Res {
        let obj = self.new_object();
        for p in props {
            match &p.value {
                PropValue::Spread(e) => {
                    let v = self.eval(e, env)?;
                    self.copy_data_properties(&obj, &v, &[])?;
                }
                PropValue::Value(e) => {
                    let key = self.eval_prop_key(&p.key, env)?;
                    let name = match &key {
                        Key::S(s) => Some(s.clone()),
                        _ => None,
                    };
                    let v = self.eval_named(e, name, env)?;
                    if matches!(&key, Key::S(s) if &**s == "__proto__") && matches!(p.key, PropKey::Name(_)) {
                        match &v {
                            Value::Obj(o) => obj.borrow_mut().proto = Some(o.clone()),
                            Value::Null => obj.borrow_mut().proto = None,
                            _ => {}
                        }
                        continue;
                    }
                    self.define_data(&obj, key, v, ALL);
                }
                PropValue::ShorthandDefault(..) => return Err(self.error_ctl("SyntaxError", "Invalid shorthand property initializer")),
                PropValue::Method(f) => {
                    let key = self.eval_prop_key(&p.key, env)?;
                    let name = key_to_name(&key);
                    let fo = self.make_closure(f.clone(), env, Some(obj.clone()), Some(name));
                    self.define_data(&obj, key, Value::Obj(fo), ALL);
                }
                PropValue::Getter(f) | PropValue::Setter(f) => {
                    let is_get = matches!(p.value, PropValue::Getter(_));
                    let key = self.eval_prop_key(&p.key, env)?;
                    let fo = self.make_closure(f.clone(), env, Some(obj.clone()), None);
                    self.define_accessor_part(&obj, key, Some(fo), is_get, ENUMERABLE | CONFIGURABLE);
                }
            }
        }
        Ok(Value::Obj(obj))
    }

    pub fn copy_data_properties(&mut self, target: &Obj, src: &Value, exclude: &[Key]) -> Res<()> {
        match src {
            Value::Obj(s) => {
                for k in self.own_keys(s)? {
                    if exclude.contains(&k) {
                        continue;
                    }
                    if let Some(p) = self.own_prop_mut(s, &k)? {
                        if p.flags & ENUMERABLE != 0 {
                            let v = self.get_obj(s, &k, src)?;
                            self.define_data(target, k, v, ALL);
                        }
                    }
                }
            }
            Value::Str(s) => {
                for (i, c) in s.chars().enumerate() {
                    self.define_data(target, Key::S(Rc::from(i.to_string())), Value::string(c.to_string()), ALL);
                }
            }
            _ => {}
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Classes
    // -----------------------------------------------------------------------

    pub fn eval_class(&mut self, c: &Rc<ClassDef>, env: &Env, inferred: Option<JsStr>) -> Res {
        let scope = self.new_scope(env, None);
        if let Some(n) = c.name {
            self.declare(&scope, n, 2, Value::Undefined, false);
        }
        let (proto_parent, ctor_parent) = match &c.superclass {
            None => (Some(self.protos().object.clone()), self.protos().function.clone()),
            Some(e) => {
                let sv = self.eval(e, &scope)?;
                match &sv {
                    Value::Null => (None, self.protos().function.clone()),
                    Value::Obj(so) if so.is_callable() => {
                        let pp = self.get_obj(so, &self.names.prototype.clone(), &sv)?;
                        let pp = match pp {
                            Value::Obj(o) => Some(o),
                            Value::Null => None,
                            _ => return Err(self.error_ctl("TypeError", "Class extends value does not have valid prototype property")),
                        };
                        (pp, so.clone())
                    }
                    _ => return Err(self.error_ctl("TypeError", &format!("Class extends value {} is not a constructor or null", self.describe(&sv)))),
                }
            }
        };
        let proto = Obj::new(Object::new(proto_parent, Class::Plain));
        let derived = c.superclass.is_some();
        // constructor
        let ctor_def = match &c.ctor {
            Some(d) => d.clone(),
            None => Rc::new(FuncDef {
                name: c.name,
                params: Vec::new(),
                body: FuncBody::Block(Block { stmts: Vec::new(), lets: Vec::new(), funcs: Vec::new() }),
                is_arrow: false,
                is_async: false,
                is_generator: false,
                kind: if derived { FnKind::DerivedCtor } else { FnKind::Ctor },
                var_names: Vec::new(),
                uses_args: false,
                simple_params: true,
                strict: true,
                source: c.source.clone(),
                span: (0, 0),
                length: 0,
            }),
        };
        let clo = Closure { def: ctor_def, env: scope.clone(), home: Some(proto.clone()), is_class_ctor: true, derived, fields: Rc::new(Vec::new()) };
        let ctor = Obj::new(Object::new(Some(ctor_parent), Class::Function(Func::Closure(Box::new(clo)))));
        let name = c.name.map(|n| self.interner.get(n).clone()).or(inferred).unwrap_or_else(|| Rc::from(""));
        self.define_data(&ctor, self.names.name.clone(), Value::Str(name), CONFIGURABLE);
        self.define_data(&ctor, self.names.prototype.clone(), Value::Obj(proto.clone()), 0);
        self.define_data(&proto, self.names.constructor.clone(), Value::Obj(ctor.clone()), HIDDEN);
        // remember the class source for toString
        self.define_data(&ctor, Key::str("__class_src"), Value::str(&c.source[c.span.0..c.span.1.min(c.source.len())]), 0);
        let mut fields: Vec<FieldInit> = Vec::new();
        let mut statics: Vec<(usize, Key)> = Vec::new();
        for (idx, m) in c.members.iter().enumerate() {
            let target = if m.is_static { ctor.clone() } else { proto.clone() };
            match &m.kind {
                ClassMemberKind::Method(f) => {
                    let key = self.eval_prop_key(&m.key, &scope)?;
                    let fo = self.make_closure(f.clone(), &scope, Some(target.clone()), Some(key_to_name(&key)));
                    self.define_data(&target, key, Value::Obj(fo), HIDDEN);
                }
                ClassMemberKind::Getter(f) | ClassMemberKind::Setter(f) => {
                    let is_get = matches!(m.kind, ClassMemberKind::Getter(_));
                    let key = self.eval_prop_key(&m.key, &scope)?;
                    let fo = self.make_closure(f.clone(), &scope, Some(target.clone()), None);
                    self.define_accessor_part(&target, key, Some(fo), is_get, CONFIGURABLE);
                }
                ClassMemberKind::Field(_) => {
                    let key = self.eval_prop_key(&m.key, &scope)?;
                    if m.is_static {
                        statics.push((idx, key));
                    } else {
                        fields.push(FieldInit { key, class: c.clone(), idx, env: scope.clone(), home: proto.clone() });
                    }
                }
                ClassMemberKind::StaticBlock(_) => statics.push((idx, Key::str(""))),
            }
        }
        if let Class::Function(Func::Closure(cl)) = &mut ctor.borrow_mut().class {
            cl.fields = Rc::new(fields);
        }
        if let Some(n) = c.name {
            self.init_binding(n, Value::Obj(ctor.clone()), &scope, VarKind::Const).ok();
            scope.vars.borrow_mut().insert(n, Var { v: Value::Obj(ctor.clone()), kind: 2, init: true });
        }
        // static fields and blocks run with this = constructor
        for (idx, key) in statics {
            let ctx = FuncCtx { this: RefCell::new(Some(Value::Obj(ctor.clone()))), callee: None, new_target: Value::Undefined, home: Some(ctor.clone()), is_global: false, strict: true };
            let s = self.new_scope(&scope, Some(ctx));
            match &c.members[idx].kind {
                ClassMemberKind::Field(init) => {
                    let v = match init {
                        Some(e) => self.eval(e, &s)?,
                        None => Value::Undefined,
                    };
                    let private = matches!(&key, Key::S(k) if k.starts_with('#'));
                    self.define_data(&ctor, key, v, if private { HIDDEN } else { ALL });
                }
                ClassMemberKind::StaticBlock(b) => {
                    self.exec_block(b, &s)?;
                }
                _ => {}
            }
        }
        Ok(Value::Obj(ctor))
    }

    // -----------------------------------------------------------------------
    // Generators and async
    // -----------------------------------------------------------------------

    /// Run a pending generator body to completion (bounded) collecting its yields.
    pub fn gen_fill(&mut self, g: &Obj) -> Res<()> {
        let pending = match &mut g.borrow_mut().class {
            Class::Generator(d) if !d.started => {
                d.started = true;
                d.pending.take()
            }
            _ => None,
        };
        let Some((def, scope)) = pending else { return Ok(()) };
        self.gen_stack.push(Vec::new());
        let r = self.run_body(&def, &scope);
        let values = self.gen_stack.pop().unwrap_or_default();
        let ret = match r {
            Ok(v) => v,
            Err(Ctl::GenStop) => Value::Undefined,
            Err(e) => {
                if let Class::Generator(d) = &mut g.borrow_mut().class {
                    d.values = values;
                }
                return Err(e);
            }
        };
        if let Class::Generator(d) = &mut g.borrow_mut().class {
            d.values = values;
            d.ret = ret;
        }
        Ok(())
    }

    /// Wait for a value: drains the event loop until a promise settles. Never suspends the
    /// caller: a promise that can never settle aborts the async function silently.
    pub fn await_value(&mut self, v: Value) -> Res {
        let p = match &v {
            Value::Obj(o) if matches!(o.borrow().class, Class::Promise(_)) => o.clone(),
            Value::Obj(_) => {
                // thenable?
                let then = self.get_v(&v, &self.names.then.clone())?;
                if matches!(&then, Value::Obj(t) if t.is_callable()) {
                    let p = self.new_promise();
                    self.resolve_promise(&p, v)?;
                    p
                } else {
                    return Ok(v);
                }
            }
            _ => return Ok(v),
        };
        let mut spins = 0;
        loop {
            let state = match &p.borrow().class {
                Class::Promise(d) => match &d.state {
                    PromiseState::Pending => 0,
                    PromiseState::Fulfilled(_) => 1,
                    PromiseState::Rejected(_) => 2,
                },
                _ => 1,
            };
            match state {
                1 | 2 => {
                    let (val, rejected) = match &mut p.borrow_mut().class {
                        Class::Promise(d) => {
                            d.handled = true;
                            match &d.state {
                                PromiseState::Fulfilled(v) => (v.clone(), false),
                                PromiseState::Rejected(v) => (v.clone(), true),
                                PromiseState::Pending => unreachable!(),
                            }
                        }
                        _ => (Value::Undefined, false),
                    };
                    return if rejected { Err(Ctl::Throw(val)) } else { Ok(val) };
                }
                _ => {}
            }
            spins += 1;
            if spins > 100_000 {
                return Err(Ctl::GenStop);
            }
            if self.run_one_microtask()? {
                continue;
            }
            if !self.run_next_timer()? {
                return Err(Ctl::GenStop);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Iteration helpers
    // -----------------------------------------------------------------------

    pub fn get_iterator(&mut self, v: &Value, is_async: bool) -> Res<Value> {
        let sym = if is_async { self.wk().async_iterator.clone() } else { self.wk().iterator.clone() };
        let mut method = self.get_v(v, &Key::Y(sym.0.id))?;
        if method.is_nullish() && is_async {
            let s2 = self.wk().iterator.clone();
            method = self.get_v(v, &Key::Y(s2.0.id))?;
        }
        if !matches!(&method, Value::Obj(o) if o.is_callable()) {
            return Err(self.error_ctl("TypeError", &format!("{} is not iterable", self.describe(v))));
        }
        let it = self.call(&method, v.clone(), &[])?;
        if !matches!(it, Value::Obj(_)) {
            return Err(self.error_ctl("TypeError", "Result of the Symbol.iterator method is not an object"));
        }
        Ok(it)
    }

    /// Advance an iterator; None when done.
    pub fn iter_step(&mut self, it: &Value) -> Res<Option<Value>> {
        let next = self.get_v(it, &self.names.next.clone())?;
        let r = self.call(&next, it.clone(), &[])?;
        if !matches!(r, Value::Obj(_)) {
            return Err(self.error_ctl("TypeError", "Iterator result is not an object"));
        }
        let done = self.get_v(&r, &self.names.done.clone())?;
        if done.truthy() {
            return Ok(None);
        }
        Ok(Some(self.get_v(&r, &self.names.value.clone())?))
    }

    pub fn iter_close(&mut self, it: &Value) {
        if let Ok(ret) = self.get_v(it, &Key::str("return")) {
            if matches!(&ret, Value::Obj(o) if o.is_callable()) {
                let _ = self.call(&ret, it.clone(), &[]);
            }
        }
    }

    pub fn iter_to_vec(&mut self, v: &Value) -> Res<Vec<Value>> {
        self.iter_to_vec_limit(v, usize::MAX)
    }

    pub fn iter_to_vec_limit(&mut self, v: &Value, limit: usize) -> Res<Vec<Value>> {
        if let Value::Obj(o) = v {
            if let Class::Array(a) = &o.borrow().class {
                if self.array_iter_pristine_ref(o) {
                    return Ok(a.clone());
                }
            }
        }
        if let Value::Str(s) = v {
            return Ok(s.chars().map(|c| Value::string(c.to_string())).collect());
        }
        let it = self.get_iterator(v, false)?;
        let mut out = Vec::new();
        while out.len() < limit {
            match self.iter_step(&it)? {
                Some(x) => out.push(x),
                None => return Ok(out),
            }
        }
        self.iter_close(&it);
        Ok(out)
    }

    fn array_iter_pristine_ref(&self, o: &Obj) -> bool {
        let it = Key::Y(self.wk().iterator.0.id);
        if o.borrow().props.get(&it).is_some() {
            return false;
        }
        match &o.borrow().proto {
            Some(p) => p.ptr_eq(&self.protos().array) && self.array_values_native,
            None => false,
        }
    }

    fn for_in_keys(&mut self, v: &Value) -> Res<Vec<JsStr>> {
        let mut out: Vec<JsStr> = Vec::new();
        let mut seen: Vec<Key> = Vec::new();
        let mut cur = match v {
            Value::Obj(o) => Some(o.clone()),
            Value::Str(s) => {
                for i in 0..u16_len(s) {
                    out.push(Rc::from(i.to_string()));
                }
                None
            }
            _ => None,
        };
        while let Some(o) = cur {
            for k in self.own_keys(&o)? {
                if let Key::S(s) = &k {
                    if seen.contains(&k) {
                        continue;
                    }
                    seen.push(k.clone());
                    if let Some(p) = self.own_prop_mut(&o, &k)? {
                        if p.flags & ENUMERABLE != 0 {
                            out.push(s.clone());
                        }
                    }
                }
            }
            cur = o.borrow().proto.clone();
        }
        Ok(out)
    }
}

pub fn key_to_name(k: &Key) -> JsStr {
    match k {
        Key::S(s) => s.clone(),
        Key::Y(_) => Rc::from(""),
    }
}
