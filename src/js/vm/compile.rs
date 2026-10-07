//! AST → bytecode compiler.
//!
//! Scope handling: a binding lives in a frame register unless a nested function or class may
//! refer to it (or the function uses `with`/direct `eval`), in which case it lives in a runtime
//! [`Scope`](crate::js::interp::Scope) exactly like in the AST interpreter.
use super::{Code, Op};
use crate::js::{ast::*, interp::Interp, value::*};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    vec::Vec,
};
use core::cell::Cell;

/// The function uses something the compiler does not translate; the AST walker runs it.
#[derive(Debug)]
pub struct Unsupported;

type CResult<T = ()> = Result<T, Unsupported>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Store {
    Reg(u32),
    Env,
}

struct Binding {
    name: Name,
    store: Store,
    /// 0 var/function/param, 1 let/class, 2 const
    kind: u8,
}

struct CScope {
    bindings: Vec<Binding>,
}

struct JumpTarget {
    labels: Vec<Name>,
    is_loop: bool,
    breaks: Vec<usize>,
    continues: Vec<usize>,
    /// Handler / runtime scope depth outside the construct (break) and inside it (continue).
    break_depth: (u32, u32),
    continue_depth: (u32, u32),
}

/// Names referenced inside nested functions/classes, and whether dynamic scoping is in play.
struct Analysis {
    captured: BTreeSet<Name>,
    dynamic: bool,
    eval_name: Name,
}

struct Compiler {
    ops: Vec<Op>,
    consts: Vec<Value>,
    keys: Vec<Key>,
    key_index: BTreeMap<Key, u32>,
    strs: Vec<JsStr>,
    funcs: Vec<Rc<FuncDef>>,
    classes: Vec<Rc<ClassDef>>,
    templates: Vec<Rc<TemplateObj>>,
    regexes: Vec<(JsStr, JsStr)>,
    nregs: u32,
    free_temps: Vec<u32>,
    scopes: Vec<CScope>,
    an: Analysis,
    handler_depth: u32,
    scope_depth: u32,
    jumps: Vec<JumpTarget>,
    pending_labels: Vec<Name>,
    /// Patch lists of the optional chains being compiled.
    chains: Vec<Vec<usize>>,
    strict: bool,
    is_async: bool,
    is_gen: bool,
    undefined_name: Name,
    arguments_name: Name,
    /// Annex B: block-level functions also assign a function-scope var.
    annex_b: BTreeSet<Name>,
}

pub fn compile_function(it: &mut Interp, def: &Rc<FuncDef>) -> CResult<Code> {
    let eval_name = it.interner.intern("eval");
    let mut an = Analysis { captured: BTreeSet::new(), dynamic: false, eval_name };
    for p in &def.params {
        an.pat(&p.pat, false);
        if let Some(d) = &p.default {
            an.expr(d, false);
        }
    }
    match &def.body {
        FuncBody::Block(b) => an.block(b, false),
        FuncBody::Expr(e) => an.expr(e, false),
    }
    let mut c = Compiler {
        ops: Vec::new(),
        consts: Vec::new(),
        keys: Vec::new(),
        key_index: BTreeMap::new(),
        strs: Vec::new(),
        funcs: Vec::new(),
        classes: Vec::new(),
        templates: Vec::new(),
        regexes: Vec::new(),
        nregs: 0,
        free_temps: Vec::new(),
        scopes: Vec::new(),
        an,
        handler_depth: 0,
        scope_depth: 0,
        jumps: Vec::new(),
        pending_labels: Vec::new(),
        chains: Vec::new(),
        strict: def.strict,
        is_async: def.is_async,
        is_gen: def.is_generator,
        undefined_name: it.names.undefined,
        arguments_name: it.names.arguments,
        annex_b: BTreeSet::new(),
    };
    c.function_prologue(def)?;
    match &def.body {
        FuncBody::Expr(e) => {
            c.expr(e)?;
            c.emit(Op::Return);
        }
        FuncBody::Block(b) => {
            for s in &b.stmts {
                c.stmt(s)?;
            }
            c.emit(Op::Undef);
            c.emit(Op::Return);
        }
    }
    let name = def.name.map(|n| it.interner.get(n).clone()).unwrap_or_else(|| Rc::from(""));
    let mut code = c.finish(name, def.is_generator, def.is_async);
    code.source = Some((def.source.clone(), def.span));
    Ok(code)
}

// ---------------------------------------------------------------------------
// Capture analysis
// ---------------------------------------------------------------------------

impl Analysis {
    fn name(&mut self, n: Name, nested: bool) {
        if nested {
            self.captured.insert(n);
        }
    }

    fn func(&mut self, f: &FuncDef) {
        for p in &f.params {
            self.pat(&p.pat, true);
            if let Some(d) = &p.default {
                self.expr(d, true);
            }
        }
        match &f.body {
            FuncBody::Block(b) => self.block(b, true),
            FuncBody::Expr(e) => self.expr(e, true),
        }
    }

    fn class(&mut self, c: &ClassDef) {
        if let Some(n) = c.name {
            self.captured.insert(n);
        }
        if let Some(s) = &c.superclass {
            self.expr(s, true);
        }
        if let Some(f) = &c.ctor {
            self.func(f);
        }
        for m in &c.members {
            self.prop_key(&m.key, true);
            match &m.kind {
                ClassMemberKind::Method(f) | ClassMemberKind::Getter(f) | ClassMemberKind::Setter(f) => self.func(f),
                ClassMemberKind::Field(Some(e)) => self.expr(e, true),
                ClassMemberKind::Field(None) => {}
                ClassMemberKind::StaticBlock(b) => self.block(b, true),
            }
        }
    }

    fn prop_key(&mut self, k: &PropKey, nested: bool) {
        if let PropKey::Computed(e) = k {
            self.expr(e, nested);
        }
    }

    fn block(&mut self, b: &Block, nested: bool) {
        for f in &b.funcs {
            self.func(f);
        }
        for s in &b.stmts {
            self.stmt(s, nested);
        }
    }

    fn pat(&mut self, p: &Pat, nested: bool) {
        match p {
            Pat::Ident(n) => self.name(*n, nested),
            Pat::Expr(e) => self.expr(e, nested),
            Pat::Array(elems, rest) => {
                for el in elems.iter().flatten() {
                    self.pat(&el.pat, nested);
                    if let Some(d) = &el.default {
                        self.expr(d, nested);
                    }
                }
                if let Some(r) = rest {
                    self.pat(r, nested);
                }
            }
            Pat::Object(props, rest) => {
                for pr in props {
                    self.prop_key(&pr.key, nested);
                    self.pat(&pr.pat, nested);
                    if let Some(d) = &pr.default {
                        self.expr(d, nested);
                    }
                }
                if let Some(r) = rest {
                    self.pat(r, nested);
                }
            }
        }
    }

    fn stmt(&mut self, s: &Stmt, nested: bool) {
        match s {
            Stmt::Expr(e) | Stmt::Throw(e) | Stmt::ExportDefault(e) => self.expr(e, nested),
            Stmt::Var(_, decls) => {
                for (p, i) in decls {
                    self.pat(p, nested);
                    if let Some(i) = i {
                        self.expr(i, nested);
                    }
                }
            }
            Stmt::Func(_) => {}
            Stmt::Class(c) => self.class(c),
            Stmt::Return(e) => {
                if let Some(e) = e {
                    self.expr(e, nested);
                }
            }
            Stmt::If(t, a, b) => {
                self.expr(t, nested);
                self.stmt(a, nested);
                if let Some(b) = b {
                    self.stmt(b, nested);
                }
            }
            Stmt::Block(b) => self.block(b, nested),
            Stmt::For { init, test, update, body, .. } => {
                match init {
                    Some(ForInit::Var(_, decls)) => {
                        for (p, i) in decls {
                            self.pat(p, nested);
                            if let Some(i) = i {
                                self.expr(i, nested);
                            }
                        }
                    }
                    Some(ForInit::Expr(e)) => self.expr(e, nested),
                    None => {}
                }
                if let Some(t) = test {
                    self.expr(t, nested);
                }
                if let Some(u) = update {
                    self.expr(u, nested);
                }
                self.stmt(body, nested);
            }
            Stmt::ForIn { head, obj, body } => {
                self.head(head, nested);
                self.expr(obj, nested);
                self.stmt(body, nested);
            }
            Stmt::ForOf { head, iter, body, .. } => {
                self.head(head, nested);
                self.expr(iter, nested);
                self.stmt(body, nested);
            }
            Stmt::While(t, b) | Stmt::DoWhile(b, t) => {
                self.expr(t, nested);
                self.stmt(b, nested);
            }
            Stmt::With(o, b) => {
                self.dynamic = true;
                self.expr(o, nested);
                self.stmt(b, nested);
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Empty => {}
            Stmt::Try { block, param, handler, finalizer } => {
                self.block(block, nested);
                if let Some(p) = param {
                    self.pat(p, nested);
                }
                if let Some(h) = handler {
                    self.block(h, nested);
                }
                if let Some(f) = finalizer {
                    self.block(f, nested);
                }
            }
            Stmt::Switch { disc, cases, funcs, .. } => {
                self.expr(disc, nested);
                for f in funcs {
                    self.func(f);
                }
                for c in cases {
                    if let Some(t) = &c.test {
                        self.expr(t, nested);
                    }
                    for s in &c.body {
                        self.stmt(s, nested);
                    }
                }
            }
            Stmt::Labeled(_, s) | Stmt::ExportDecl(s, _) => self.stmt(s, nested),
            Stmt::Import { .. } | Stmt::ExportNamed { .. } | Stmt::ExportAll(..) => {}
        }
    }

    fn head(&mut self, h: &ForHead, nested: bool) {
        match h {
            ForHead::Var(_, p) | ForHead::Pat(p) => self.pat(p, nested),
        }
    }

    fn args(&mut self, args: &[ArgElem], nested: bool) {
        for a in args {
            match a {
                ArgElem::Expr(e) | ArgElem::Spread(e) => self.expr(e, nested),
            }
        }
    }

    fn expr(&mut self, e: &Expr, nested: bool) {
        match e {
            Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Null | Expr::This | Expr::Regex(..) | Expr::NewTarget | Expr::SuperDot(_) | Expr::ImportMeta => {}
            Expr::Ident(n) => self.name(*n, nested),
            Expr::Template(_, xs) => {
                for x in xs {
                    self.expr(x, nested);
                }
            }
            Expr::Tagged(t, _, xs) => {
                self.expr(t, nested);
                for x in xs {
                    self.expr(x, nested);
                }
            }
            Expr::Array(items) => {
                for a in items.iter().flatten() {
                    match a {
                        ArgElem::Expr(e) | ArgElem::Spread(e) => self.expr(e, nested),
                    }
                }
            }
            Expr::Object(props) => {
                for p in props {
                    self.prop_key(&p.key, nested);
                    match &p.value {
                        PropValue::Value(e) | PropValue::Spread(e) => self.expr(e, nested),
                        PropValue::Getter(f) | PropValue::Setter(f) | PropValue::Method(f) => self.func(f),
                        PropValue::ShorthandDefault(n, d) => {
                            self.name(*n, nested);
                            self.expr(d, nested);
                        }
                    }
                }
            }
            Expr::Function(f) => self.func(f),
            Expr::Class(c) => self.class(c),
            Expr::Unary(_, x) | Expr::Await(x) | Expr::Paren(x) | Expr::OptChain(x) | Expr::SuperMember(x) | Expr::ImportCall(x) => self.expr(x, nested),
            Expr::Update { target, .. } => self.expr(target, nested),
            Expr::Binary(_, a, b) | Expr::Logical(_, a, b) => {
                self.expr(a, nested);
                self.expr(b, nested);
            }
            Expr::Assign(_, p, v) => {
                self.pat(p, nested);
                self.expr(v, nested);
            }
            Expr::Cond(a, b, c) => {
                self.expr(a, nested);
                self.expr(b, nested);
                self.expr(c, nested);
            }
            Expr::Call { callee, args, .. } => {
                if matches!(&**callee, Expr::Ident(n) if *n == self.eval_name) {
                    self.dynamic = true;
                }
                self.expr(callee, nested);
                self.args(args, nested);
            }
            Expr::New(c, args) => {
                self.expr(c, nested);
                self.args(args, nested);
            }
            Expr::Member { obj, prop, .. } => {
                self.expr(obj, nested);
                self.expr(prop, nested);
            }
            Expr::Dot { obj, .. } | Expr::PrivateMember { obj, .. } => self.expr(obj, nested),
            Expr::Seq(xs) => {
                for x in xs {
                    self.expr(x, nested);
                }
            }
            Expr::Yield(x, _) => {
                if let Some(x) = x {
                    self.expr(x, nested);
                }
            }
            Expr::SuperCall(args) => self.args(args, nested),
        }
    }
}

// ---------------------------------------------------------------------------
// Emission helpers
// ---------------------------------------------------------------------------

impl Compiler {
    fn finish(self, name: JsStr, is_generator: bool, is_async: bool) -> Code {
        Code {
            ops: self.ops,
            consts: self.consts,
            keys: self.keys,
            strs: self.strs,
            funcs: self.funcs,
            classes: self.classes,
            templates: self.templates,
            regexes: self.regexes,
            nregs: self.nregs,
            strict: self.strict,
            is_generator,
            is_async,
            hot: Cell::new(0),
            #[cfg(feature = "jit")]
            native: core::cell::RefCell::new(Default::default()),
            name,
            source: None,
            template_cache: core::cell::RefCell::new(Vec::new()),
        }
    }

    fn emit(&mut self, op: Op) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    fn here(&self) -> u32 {
        self.ops.len() as u32
    }

    fn patch(&mut self, at: usize, target: u32) {
        self.ops[at] = match self.ops[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(target),
            Op::JumpIfFalseKeep(_) => Op::JumpIfFalseKeep(target),
            Op::JumpIfTrueKeep(_) => Op::JumpIfTrueKeep(target),
            Op::JumpIfNotNullishKeep(_) => Op::JumpIfNotNullishKeep(target),
            Op::JumpIfNullishShort(_) => Op::JumpIfNullishShort(target),
            Op::JumpIfNullishShort2(_) => Op::JumpIfNullishShort2(target),
            Op::JumpIfNotUndef(_) => Op::JumpIfNotUndef(target),
            Op::TryCatch(_) => Op::TryCatch(target),
            Op::TryFinally(_) => Op::TryFinally(target),
            Op::Leave(_, d) => Op::Leave(target, d),
            Op::IterNext(r, _) => Op::IterNext(r, target),
            Op::IterResult(r, _) => Op::IterResult(r, target),
            Op::ForInNext(r, _) => Op::ForInNext(r, target),
            Op::YieldStar(r, _) => Op::YieldStar(r, target),
            other => other,
        };
    }

    fn patch_here(&mut self, at: usize) {
        let h = self.here();
        self.patch(at, h);
    }

    fn key(&mut self, k: Key) -> u32 {
        if let Some(&i) = self.key_index.get(&k) {
            return i;
        }
        let i = self.keys.len() as u32;
        self.keys.push(k.clone());
        self.key_index.insert(k, i);
        i
    }

    fn str_key(&mut self, s: &JsStr) -> u32 {
        self.key(Key::S(s.clone()))
    }

    fn private_key(&mut self, s: &JsStr) -> u32 {
        self.key(Key::S(Rc::from(alloc::format!("#{s}"))))
    }

    fn str_const(&mut self, s: &JsStr) -> u32 {
        if let Some(i) = self.strs.iter().position(|x| Rc::ptr_eq(x, s) || **x == **s) {
            return i as u32;
        }
        self.strs.push(s.clone());
        (self.strs.len() - 1) as u32
    }

    fn push_const(&mut self, v: Value) {
        let i = match &v {
            Value::Str(s) => self.consts.iter().position(|c| matches!(c, Value::Str(x) if x == s)),
            Value::Num(n) => self.consts.iter().position(|c| matches!(c, Value::Num(x) if x.to_bits() == n.to_bits())),
            _ => None,
        };
        let i = match i {
            Some(i) => i,
            None => {
                self.consts.push(v);
                self.consts.len() - 1
            }
        };
        self.emit(Op::Const(i as u32));
    }

    fn push_str(&mut self, s: &JsStr) {
        self.push_const(Value::Str(s.clone()));
    }

    fn alloc_reg(&mut self) -> u32 {
        self.nregs += 1;
        self.nregs - 1
    }

    fn temp(&mut self) -> u32 {
        self.free_temps.pop().unwrap_or_else(|| self.alloc_reg())
    }

    /// Two consecutive registers (iterator state).
    fn temp_pair(&mut self) -> u32 {
        let r = self.nregs;
        self.nregs += 2;
        r
    }

    fn free(&mut self, r: u32) {
        self.free_temps.push(r);
    }

    fn func_index(&mut self, f: &Rc<FuncDef>) -> u32 {
        if let Some(i) = self.funcs.iter().position(|x| Rc::ptr_eq(x, f)) {
            return i as u32;
        }
        self.funcs.push(f.clone());
        (self.funcs.len() - 1) as u32
    }

    fn depth_pair(&self) -> (u32, u32) {
        (self.handler_depth, self.scope_depth)
    }

    fn leave(&mut self, depth: (u32, u32)) -> usize {
        if depth.0 > 0xffff || depth.1 > 0xffff {
            // absurd nesting; still encode something valid
            return self.emit(Op::Leave(0, 0));
        }
        self.emit(Op::Leave(0, (depth.0 << 16) | depth.1))
    }

    fn throw_error(&mut self, kind: u8, msg: &str) {
        let s = self.str_const(&Rc::from(msg));
        self.emit(Op::ThrowError(kind, s));
    }

    // -----------------------------------------------------------------------
    // Bindings
    // -----------------------------------------------------------------------

    fn store_for(&mut self, n: Name) -> Store {
        if self.an.dynamic || self.an.captured.contains(&n) { Store::Env } else { Store::Reg(self.alloc_reg()) }
    }

    fn resolve(&self, n: Name) -> Option<(Store, u8)> {
        for s in self.scopes.iter().rev() {
            if let Some(b) = s.bindings.iter().rev().find(|b| b.name == n) {
                return Some((b.store, b.kind));
            }
        }
        None
    }

    /// Open a lexical scope for `lets`/function declarations; returns whether a runtime scope was pushed.
    fn open_scope(&mut self, lets: &[(Name, VarKind)], funcs: &[Rc<FuncDef>]) -> CResult<bool> {
        let mut bindings = Vec::new();
        let mut env = false;
        for (n, k) in lets {
            let store = self.store_for(*n);
            env |= store == Store::Env;
            bindings.push(Binding { name: *n, store, kind: if *k == VarKind::Const { 2 } else { 1 } });
        }
        for f in funcs {
            let Some(n) = f.name else { return Err(Unsupported) };
            if bindings.iter().any(|b| b.name == n) {
                continue;
            }
            let store = self.store_for(n);
            env |= store == Store::Env;
            bindings.push(Binding { name: n, store, kind: 0 });
        }
        if env {
            self.emit(Op::PushScope);
            self.scope_depth += 1;
            for b in &bindings {
                if b.store == Store::Env {
                    if b.kind == 0 {
                        self.emit(Op::DeclVar(b.name));
                    } else {
                        self.emit(Op::DeclLet(b.name, b.kind));
                    }
                }
            }
        }
        self.scopes.push(CScope { bindings });
        for f in funcs {
            let n = f.name.expect("declared function has a name");
            let i = self.func_index(f);
            self.emit(Op::Closure(i));
            if self.annex_b.contains(&n) && self.scopes.len() > 1 {
                self.emit(Op::Dup);
                self.store_var_outer(n)?;
            }
            self.store_decl(n, VarKind::Var)?;
        }
        Ok(env)
    }

    fn close_scope(&mut self, pushed: bool) {
        self.scopes.pop();
        if pushed {
            self.emit(Op::PopScope);
            self.scope_depth -= 1;
        }
    }

    /// Annex B: store the top of stack into the function-level var `n` (skipping the innermost scope).
    fn store_var_outer(&mut self, n: Name) -> CResult {
        let fs = &self.scopes[0];
        match fs.bindings.iter().find(|b| b.name == n).map(|b| b.store) {
            Some(Store::Reg(r)) => {
                self.emit(Op::SetReg(r));
            }
            _ => {
                // the var lives in the function scope object; walk past the block binding
                self.emit(Op::SetVarOuter(n));
            }
        }
        Ok(())
    }

    /// Pop the top of stack into a declaration of `n`.
    fn store_decl(&mut self, n: Name, kind: VarKind) -> CResult {
        match self.resolve(n) {
            Some((Store::Reg(r), _)) => {
                self.emit(Op::SetReg(r));
            }
            Some((Store::Env, _)) | None => {
                if kind == VarKind::Var {
                    self.emit(Op::InitVar(n));
                } else {
                    self.emit(Op::InitLet(n, if kind == VarKind::Const { 2 } else { 1 }));
                }
            }
        }
        Ok(())
    }

    /// Assign the top of stack to `n`, keeping it on the stack.
    fn store_assign_keep(&mut self, n: Name) {
        match self.resolve(n) {
            Some((Store::Reg(_), 2)) => {
                self.throw_error(0, "Assignment to constant variable.");
            }
            Some((Store::Reg(r), _)) => {
                self.emit(Op::PutReg(r));
            }
            _ => {
                self.emit(Op::SetName(n));
            }
        }
    }

    fn load_name(&mut self, n: Name) {
        match self.resolve(n) {
            Some((Store::Reg(r), _)) => {
                self.emit(Op::GetReg(r));
            }
            Some((Store::Env, _)) => {
                self.emit(Op::GetName(n));
            }
            None => {
                if n == self.undefined_name {
                    self.emit(Op::Undef);
                } else {
                    self.emit(Op::GetName(n));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Functions
    // -----------------------------------------------------------------------

    fn function_prologue(&mut self, def: &Rc<FuncDef>) -> CResult {
        let FuncBody::Block(body) = &def.body else {
            // concise arrow: parameters only
            let mut bindings = Vec::new();
            let mut names = Vec::new();
            for p in &def.params {
                crate::js::parser::pat_names(&p.pat, &mut names);
            }
            for n in names {
                if !bindings.iter().any(|b: &Binding| b.name == n) {
                    let store = self.store_for(n);
                    bindings.push(Binding { name: n, store, kind: 0 });
                }
            }
            for b in &bindings {
                if b.store == Store::Env {
                    self.emit(Op::DeclVar(b.name));
                }
            }
            self.scopes.push(CScope { bindings });
            return self.bind_params(def);
        };
        // Annex B (sloppy mode): functions declared in nested blocks are also function-scope vars
        if !def.strict {
            let mut found = Vec::new();
            for s in &body.stmts {
                collect_block_funcs(s, &mut found);
            }
            for n in found {
                self.annex_b.insert(n);
            }
        }
        let mut bindings: Vec<Binding> = Vec::new();
        let add = |c: &mut Compiler, bindings: &mut Vec<Binding>, n: Name, kind: u8| {
            if bindings.iter().any(|b| b.name == n) {
                return;
            }
            let store = c.store_for(n);
            bindings.push(Binding { name: n, store, kind });
        };
        let mut pnames = Vec::new();
        for p in &def.params {
            crate::js::parser::pat_names(&p.pat, &mut pnames);
        }
        for n in pnames {
            add(self, &mut bindings, n, 0);
        }
        if def.uses_args && !def.is_arrow {
            add(self, &mut bindings, self.arguments_name, 0);
        }
        for &n in &def.var_names {
            add(self, &mut bindings, n, 0);
        }
        let annex: Vec<Name> = self.annex_b.iter().copied().collect();
        for n in annex {
            if !body.lets.iter().any(|(l, _)| *l == n) {
                add(self, &mut bindings, n, 0);
            }
        }
        for f in &body.funcs {
            let Some(n) = f.name else { return Err(Unsupported) };
            add(self, &mut bindings, n, 0);
        }
        for (n, k) in &body.lets {
            add(self, &mut bindings, *n, if *k == VarKind::Const { 2 } else { 1 });
        }
        for b in &bindings {
            if b.store == Store::Env {
                if b.kind == 0 {
                    self.emit(Op::DeclVar(b.name));
                } else {
                    self.emit(Op::DeclLet(b.name, b.kind));
                }
            }
        }
        self.scopes.push(CScope { bindings });
        if def.uses_args && !def.is_arrow {
            self.emit(Op::Arguments);
            self.store_decl(self.arguments_name, VarKind::Var)?;
        }
        self.bind_params(def)?;
        for f in &body.funcs {
            let i = self.func_index(f);
            self.emit(Op::Closure(i));
            self.store_decl(f.name.expect("named"), VarKind::Var)?;
        }
        if def.is_generator {
            self.emit(Op::GenStart);
        }
        Ok(())
    }

    fn bind_params(&mut self, def: &Rc<FuncDef>) -> CResult {
        for (i, p) in def.params.iter().enumerate() {
            if p.rest {
                self.emit(Op::ArgsFrom(i as u32));
            } else {
                self.emit(Op::LoadArg(i as u32));
                if let Some(d) = &p.default {
                    let j = self.emit(Op::JumpIfNotUndef(0));
                    self.named_expr(d, &p.pat)?;
                    self.patch_here(j);
                }
            }
            self.bind_pattern(&p.pat, Some(VarKind::Var))?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Patterns: the value to bind is on top of the stack and is consumed.
    // -----------------------------------------------------------------------

    fn bind_pattern(&mut self, pat: &Pat, kind: Option<VarKind>) -> CResult {
        match pat {
            Pat::Ident(n) => match kind {
                Some(k) => self.store_decl(*n, k),
                None => {
                    self.store_assign_keep(*n);
                    self.emit(Op::Pop);
                    Ok(())
                }
            },
            Pat::Expr(e) => self.assign_to_target(e),
            Pat::Array(elems, rest) => {
                let r = self.temp_pair();
                self.emit(Op::GetIter(r));
                for el in elems {
                    self.emit(Op::IterNextOrUndef(r));
                    match el {
                        None => {
                            self.emit(Op::Pop);
                        }
                        Some(el) => {
                            if let Some(d) = &el.default {
                                let j = self.emit(Op::JumpIfNotUndef(0));
                                self.named_expr(d, &el.pat)?;
                                self.patch_here(j);
                            }
                            self.bind_pattern(&el.pat, kind)?;
                        }
                    }
                }
                if let Some(rp) = rest {
                    self.emit(Op::IterRest(r));
                    self.bind_pattern(rp, kind)?;
                }
                self.emit(Op::IterClose(r));
                Ok(())
            }
            Pat::Object(props, rest) => {
                self.emit(Op::RequireObjectCoercible);
                let src = self.temp();
                self.emit(Op::SetReg(src));
                let keys = if rest.is_some() {
                    let k = self.temp();
                    self.emit(Op::NewArray);
                    self.emit(Op::SetReg(k));
                    Some(k)
                } else {
                    None
                };
                for p in props {
                    self.emit(Op::GetReg(src));
                    match &p.key {
                        PropKey::Name(n) => {
                            if let Some(k) = keys {
                                self.emit(Op::GetReg(k));
                                self.push_str(n);
                                self.emit(Op::ArrPush);
                                self.emit(Op::Pop);
                            }
                            let ki = self.str_key(n);
                            self.emit(Op::GetProp(ki));
                        }
                        PropKey::Private(n) => {
                            let ki = self.private_key(n);
                            self.emit(Op::GetProp(ki));
                        }
                        PropKey::Computed(e) => {
                            self.expr(e)?;
                            self.emit(Op::ToKey);
                            if let Some(k) = keys {
                                self.emit(Op::Dup);
                                self.emit(Op::GetReg(k));
                                self.emit(Op::Swap);
                                self.emit(Op::ArrPush);
                                self.emit(Op::Pop);
                            }
                            self.emit(Op::GetElem);
                        }
                    }
                    if let Some(d) = &p.default {
                        let j = self.emit(Op::JumpIfNotUndef(0));
                        self.named_expr(d, &p.pat)?;
                        self.patch_here(j);
                    }
                    self.bind_pattern(&p.pat, kind)?;
                }
                if let (Some(rp), Some(k)) = (rest, keys) {
                    self.emit(Op::NewObject);
                    self.emit(Op::GetReg(k));
                    self.emit(Op::GetReg(src));
                    self.emit(Op::CopyPropsExcept);
                    self.bind_pattern(rp, kind)?;
                    self.free(k);
                }
                self.emit(Op::Undef);
                self.emit(Op::SetReg(src));
                self.free(src);
                Ok(())
            }
        }
    }

    /// Value on the stack → store into a member/identifier expression target (consumes it).
    fn assign_to_target(&mut self, e: &Expr) -> CResult {
        match e {
            Expr::Ident(n) => {
                self.store_assign_keep(*n);
                self.emit(Op::Pop);
            }
            Expr::Paren(inner) => return self.assign_to_target(inner),
            Expr::Dot { obj, name, .. } => {
                self.expr(obj)?;
                self.emit(Op::Swap);
                let k = self.str_key(name);
                self.emit(Op::SetProp(k));
                self.emit(Op::Pop);
            }
            Expr::PrivateMember { obj, name, .. } => {
                self.expr(obj)?;
                self.emit(Op::Swap);
                let k = self.private_key(name);
                self.emit(Op::SetProp(k));
                self.emit(Op::Pop);
            }
            Expr::Member { obj, prop, .. } => {
                let t = self.temp();
                self.emit(Op::SetReg(t));
                self.expr(obj)?;
                self.expr(prop)?;
                self.emit(Op::GetReg(t));
                self.emit(Op::SetElem);
                self.emit(Op::Pop);
                self.free(t);
            }
            Expr::SuperDot(name) => {
                let k = self.str_key(name);
                self.emit(Op::SetSuper(k));
                self.emit(Op::Pop);
            }
            _ => {
                self.throw_error(2, "Invalid assignment target");
            }
        }
        Ok(())
    }

    fn named_expr(&mut self, e: &Expr, pat: &Pat) -> CResult {
        match pat {
            Pat::Ident(n) => self.expr_named(e, Some(*n)),
            _ => self.expr(e),
        }
    }

    /// Evaluate with name inference for anonymous functions/classes.
    fn expr_named(&mut self, e: &Expr, name: Option<Name>) -> CResult {
        let Some(n) = name else { return self.expr(e) };
        match e {
            Expr::Function(f) if f.name.is_none() => {
                let i = self.func_index(f);
                self.emit(Op::ClosureNamed(i, n));
                Ok(())
            }
            Expr::Class(c) if c.name.is_none() => {
                self.classes.push(c.clone());
                let i = (self.classes.len() - 1) as u32;
                self.emit(Op::Class(i, n));
                Ok(())
            }
            Expr::Paren(inner) => self.expr_named(inner, name),
            _ => self.expr(e),
        }
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn block(&mut self, b: &Block) -> CResult {
        let pushed = self.open_scope(&b.lets, &b.funcs)?;
        for s in &b.stmts {
            self.stmt(s)?;
        }
        self.close_scope(pushed);
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> CResult {
        let labels = core::mem::take(&mut self.pending_labels);
        match s {
            Stmt::Expr(e) => {
                self.expr(e)?;
                self.emit(Op::Pop);
            }
            Stmt::Var(kind, decls) => {
                for (pat, init) in decls {
                    match init {
                        Some(e) => {
                            self.named_expr(e, pat)?;
                            self.bind_pattern(pat, Some(*kind))?;
                        }
                        None => {
                            if *kind != VarKind::Var {
                                self.emit(Op::Undef);
                                self.bind_pattern(pat, Some(*kind))?;
                            }
                        }
                    }
                }
            }
            Stmt::Func(_) | Stmt::Empty => {}
            Stmt::Class(c) => {
                self.classes.push(c.clone());
                let i = (self.classes.len() - 1) as u32;
                self.emit(Op::Class(i, u32::MAX));
                match c.name {
                    Some(n) => self.store_decl(n, VarKind::Let)?,
                    None => {
                        self.emit(Op::Pop);
                    }
                }
            }
            Stmt::Return(e) => {
                match e {
                    Some(e) => self.expr(e)?,
                    None => {
                        self.emit(Op::Undef);
                    }
                }
                self.emit(Op::Return);
            }
            Stmt::If(t, a, b) => {
                self.expr(t)?;
                let j = self.emit(Op::JumpIfFalse(0));
                self.stmt(a)?;
                match b {
                    Some(b) => {
                        let j2 = self.emit(Op::Jump(0));
                        self.patch_here(j);
                        self.stmt(b)?;
                        self.patch_here(j2);
                    }
                    None => self.patch_here(j),
                }
            }
            Stmt::Block(b) => {
                if labels.is_empty() {
                    self.block(b)?;
                } else {
                    self.labeled_block(labels, |c| c.block(b))?;
                }
            }
            Stmt::Labeled(l, inner) => {
                let mut ls = labels;
                ls.push(*l);
                let is_loop = matches!(**inner, Stmt::For { .. } | Stmt::ForIn { .. } | Stmt::ForOf { .. } | Stmt::While(..) | Stmt::DoWhile(..) | Stmt::Labeled(..));
                if is_loop {
                    self.pending_labels = ls;
                    self.stmt(inner)?;
                } else {
                    self.labeled_block(ls, |c| c.stmt(inner))?;
                }
            }
            Stmt::While(t, body) => {
                let top = self.here();
                self.expr(t)?;
                let exit = self.emit(Op::JumpIfFalse(0));
                self.enter_loop(labels);
                self.stmt(body)?;
                let cont = self.here();
                self.emit(Op::Jump(top));
                let end = self.here();
                self.patch(exit, end);
                self.exit_loop(cont, end);
            }
            Stmt::DoWhile(body, t) => {
                let top = self.here();
                self.enter_loop(labels);
                self.stmt(body)?;
                let cont = self.here();
                self.expr(t)?;
                self.emit(Op::JumpIfTrue(top));
                let end = self.here();
                self.exit_loop(cont, end);
            }
            Stmt::For { init, test, update, body, lets } => self.for_loop(labels, init, test, update, body, lets)?,
            Stmt::ForIn { head, obj, body } => self.for_in(labels, head, obj, body)?,
            Stmt::ForOf { head, iter, body, is_await } => self.for_of(labels, head, iter, body, *is_await)?,
            Stmt::With(obj, body) => {
                self.expr(obj)?;
                self.emit(Op::PushWith);
                self.scope_depth += 1;
                self.scopes.push(CScope { bindings: Vec::new() });
                self.stmt(body)?;
                self.scopes.pop();
                self.emit(Op::PopScope);
                self.scope_depth -= 1;
            }
            Stmt::Break(l) => self.break_continue(*l, false)?,
            Stmt::Continue(l) => self.break_continue(*l, true)?,
            Stmt::Throw(e) => {
                self.expr(e)?;
                self.emit(Op::Throw);
            }
            Stmt::Try { block, param, handler, finalizer } => self.try_stmt(block, param, handler, finalizer)?,
            Stmt::Switch { disc, cases, lets, funcs } => self.switch(labels, disc, cases, lets, funcs)?,
            Stmt::Import { .. } | Stmt::ExportNamed { .. } | Stmt::ExportAll(..) | Stmt::ExportDecl(..) | Stmt::ExportDefault(_) => return Err(Unsupported),
        }
        Ok(())
    }

    fn labeled_block(&mut self, labels: Vec<Name>, f: impl FnOnce(&mut Compiler) -> CResult) -> CResult {
        let d = self.depth_pair();
        self.jumps.push(JumpTarget { labels, is_loop: false, breaks: Vec::new(), continues: Vec::new(), break_depth: d, continue_depth: d });
        f(self)?;
        let jt = self.jumps.pop().expect("jump target");
        let end = self.here();
        for b in jt.breaks {
            self.patch(b, end);
        }
        Ok(())
    }

    fn enter_loop(&mut self, labels: Vec<Name>) {
        let d = self.depth_pair();
        self.jumps.push(JumpTarget { labels, is_loop: true, breaks: Vec::new(), continues: Vec::new(), break_depth: d, continue_depth: d });
    }

    /// Like `enter_loop` but `break` leaves to an outer depth.
    fn enter_loop_at(&mut self, labels: Vec<Name>, break_depth: (u32, u32)) {
        let d = self.depth_pair();
        self.jumps.push(JumpTarget { labels, is_loop: true, breaks: Vec::new(), continues: Vec::new(), break_depth, continue_depth: d });
    }

    fn exit_loop(&mut self, cont: u32, end: u32) {
        let jt = self.jumps.pop().expect("loop");
        for b in jt.breaks {
            self.patch(b, end);
        }
        for c in jt.continues {
            self.patch(c, cont);
        }
    }

    fn break_continue(&mut self, label: Option<Name>, is_continue: bool) -> CResult {
        let idx = self.jumps.iter().rposition(|j| match label {
            Some(l) => j.labels.contains(&l) && (!is_continue || j.is_loop),
            None => j.is_loop || (!is_continue && j.labels.is_empty()),
        });
        let Some(idx) = idx else { return Err(Unsupported) };
        let depth = if is_continue { self.jumps[idx].continue_depth } else { self.jumps[idx].break_depth };
        let at = self.leave(depth);
        if is_continue {
            self.jumps[idx].continues.push(at);
        } else {
            self.jumps[idx].breaks.push(at);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn for_loop(&mut self, labels: Vec<Name>, init: &Option<ForInit>, test: &Option<Expr>, update: &Option<Expr>, body: &Stmt, lets: &[Name]) -> CResult {
        let outer = self.depth_pair();
        let mut pushed = false;
        if !lets.is_empty() {
            let kind = match init {
                Some(ForInit::Var(k, _)) => *k,
                _ => VarKind::Let,
            };
            let list: Vec<(Name, VarKind)> = lets.iter().map(|n| (*n, kind)).collect();
            pushed = self.open_scope(&list, &[])?;
        }
        match init {
            Some(ForInit::Var(kind, decls)) => {
                for (pat, i) in decls {
                    match i {
                        Some(e) => {
                            self.named_expr(e, pat)?;
                            self.bind_pattern(pat, Some(*kind))?;
                        }
                        None if *kind != VarKind::Var => {
                            self.emit(Op::Undef);
                            self.bind_pattern(pat, Some(*kind))?;
                        }
                        None => {}
                    }
                }
            }
            Some(ForInit::Expr(e)) => {
                self.expr(e)?;
                self.emit(Op::Pop);
            }
            None => {}
        }
        let top = self.here();
        let exit = match test {
            Some(t) => {
                self.expr(t)?;
                Some(self.emit(Op::JumpIfFalse(0)))
            }
            None => None,
        };
        self.enter_loop_at(labels, outer);
        self.stmt(body)?;
        let cont = self.here();
        if pushed {
            self.emit(Op::CopyScope);
        }
        if let Some(u) = update {
            self.expr(u)?;
            self.emit(Op::Pop);
        }
        self.emit(Op::Jump(top));
        let end_inner = self.here();
        if let Some(x) = exit {
            self.patch(x, end_inner);
        }
        if !lets.is_empty() {
            self.close_scope(pushed);
        }
        let end = self.here();
        // `break` leaves to the outer depth, which already dropped the loop scope
        let jt = self.jumps.pop().expect("loop");
        for b in jt.breaks {
            self.patch(b, end);
        }
        for c in jt.continues {
            self.patch(c, cont);
        }
        Ok(())
    }

    /// Bind a for-in/of head inside a fresh per-iteration scope; returns whether a scope was pushed.
    fn bind_head(&mut self, head: &ForHead) -> CResult<bool> {
        match head {
            ForHead::Var(kind, pat) if *kind != VarKind::Var => {
                let mut names = Vec::new();
                crate::js::parser::pat_names(pat, &mut names);
                let list: Vec<(Name, VarKind)> = names.into_iter().map(|n| (n, *kind)).collect();
                // the value is on the stack: park it while the scope opens
                let t = self.temp();
                self.emit(Op::SetReg(t));
                let pushed = self.open_scope(&list, &[])?;
                self.emit(Op::GetReg(t));
                self.emit(Op::Undef);
                self.emit(Op::SetReg(t));
                self.free(t);
                self.bind_pattern(pat, Some(*kind))?;
                Ok(pushed)
            }
            ForHead::Var(kind, pat) => {
                self.bind_pattern(pat, Some(*kind))?;
                self.scopes.push(CScope { bindings: Vec::new() });
                Ok(false)
            }
            ForHead::Pat(pat) => {
                self.bind_pattern(pat, None)?;
                self.scopes.push(CScope { bindings: Vec::new() });
                Ok(false)
            }
        }
    }

    fn for_in(&mut self, labels: Vec<Name>, head: &ForHead, obj: &Expr, body: &Stmt) -> CResult {
        let outer = self.depth_pair();
        self.expr(obj)?;
        let r = self.temp_pair();
        self.emit(Op::ForIn(r));
        let top = self.here();
        let exit = self.emit(Op::ForInNext(r, 0));
        let pushed = self.bind_head(head)?;
        self.enter_loop_at(labels, outer);
        self.stmt(body)?;
        let cont = self.here();
        self.close_scope(pushed);
        self.emit(Op::Jump(top));
        let end = self.here();
        self.patch(exit, end);
        let jt = self.jumps.pop().expect("loop");
        for b in jt.breaks {
            self.patch(b, end);
        }
        for c in jt.continues {
            self.patch(c, cont);
        }
        Ok(())
    }

    fn for_of(&mut self, labels: Vec<Name>, head: &ForHead, iter: &Expr, body: &Stmt, is_await: bool) -> CResult {
        let outer = self.depth_pair();
        self.expr(iter)?;
        let r = self.temp_pair();
        self.emit(if is_await { Op::GetAsyncIter(r) } else { Op::GetIter(r) });
        let fin = self.emit(Op::TryFinally(0));
        self.handler_depth += 1;
        let top = self.here();
        let exit = if is_await {
            self.emit(Op::AsyncIterNextCall(r));
            self.emit(Op::Await);
            self.emit(Op::IterResult(r, 0))
        } else {
            self.emit(Op::IterNext(r, 0))
        };
        let pushed = self.bind_head(head)?;
        self.enter_loop_at(labels, outer);
        self.stmt(body)?;
        let cont = self.here();
        self.close_scope(pushed);
        self.emit(Op::Jump(top));
        let done = self.here();
        self.patch(exit, done);
        self.emit(Op::PopHandler);
        self.handler_depth -= 1;
        self.emit(Op::NormalCompletion);
        self.patch_here(fin);
        if is_await {
            self.emit(Op::AsyncIterCloseCompletion(r));
        } else {
            self.emit(Op::IterCloseCompletion(r));
        }
        self.emit(Op::EndFinally);
        let end = self.here();
        let jt = self.jumps.pop().expect("loop");
        for b in jt.breaks {
            self.patch(b, end);
        }
        for c in jt.continues {
            self.patch(c, cont);
        }
        Ok(())
    }

    fn try_stmt(&mut self, block: &Block, param: &Option<Pat>, handler: &Option<Block>, finalizer: &Option<Block>) -> CResult {
        let fin = if finalizer.is_some() {
            let f = self.emit(Op::TryFinally(0));
            self.handler_depth += 1;
            Some(f)
        } else {
            None
        };
        if let Some(h) = handler {
            let c = self.emit(Op::TryCatch(0));
            self.handler_depth += 1;
            self.block(block)?;
            self.emit(Op::PopHandler);
            self.handler_depth -= 1;
            let skip = self.emit(Op::Jump(0));
            self.patch_here(c);
            // exception value on the stack
            match param {
                Some(p) => {
                    let mut names = Vec::new();
                    crate::js::parser::pat_names(p, &mut names);
                    let list: Vec<(Name, VarKind)> = names.into_iter().map(|n| (n, VarKind::Let)).collect();
                    let t = self.temp();
                    self.emit(Op::SetReg(t));
                    let pushed = self.open_scope(&list, &[])?;
                    self.emit(Op::GetReg(t));
                    self.emit(Op::Undef);
                    self.emit(Op::SetReg(t));
                    self.free(t);
                    self.bind_pattern(p, Some(VarKind::Let))?;
                    self.block(h)?;
                    self.close_scope(pushed);
                }
                None => {
                    self.emit(Op::Pop);
                    self.block(h)?;
                }
            }
            self.patch_here(skip);
        } else {
            self.block(block)?;
        }
        if let (Some(f), Some(fb)) = (fin, finalizer) {
            self.emit(Op::PopHandler);
            self.handler_depth -= 1;
            self.emit(Op::NormalCompletion);
            self.patch_here(f);
            self.block(fb)?;
            self.emit(Op::EndFinally);
        }
        Ok(())
    }

    fn switch(&mut self, labels: Vec<Name>, disc: &Expr, cases: &[SwitchCase], lets: &[(Name, VarKind)], funcs: &[Rc<FuncDef>]) -> CResult {
        let outer = self.depth_pair();
        self.expr(disc)?;
        let d = self.temp();
        self.emit(Op::SetReg(d));
        let pushed = self.open_scope(lets, funcs)?;
        let mut jumps = Vec::new();
        for c in cases {
            if let Some(t) = &c.test {
                self.emit(Op::GetReg(d));
                self.expr(t)?;
                self.emit(Op::Bin(BinOp::StrictEq));
                jumps.push(Some(self.emit(Op::JumpIfTrue(0))));
            } else {
                jumps.push(None);
            }
        }
        let to_default = self.emit(Op::Jump(0));
        self.jumps.push(JumpTarget { labels, is_loop: false, breaks: Vec::new(), continues: Vec::new(), break_depth: outer, continue_depth: outer });
        let mut default_at = None;
        for (i, c) in cases.iter().enumerate() {
            let here = self.here();
            match jumps[i] {
                Some(j) => self.patch(j, here),
                None => default_at = Some(here),
            }
            for s in &c.body {
                self.stmt(s)?;
            }
        }
        self.close_scope(pushed);
        let end = self.here();
        self.patch(to_default, default_at.unwrap_or(end));
        let jt = self.jumps.pop().expect("switch");
        for b in jt.breaks {
            self.patch(b, end);
        }
        self.free(d);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Expressions: each pushes exactly one value.
    // -----------------------------------------------------------------------

    fn args(&mut self, args: &[ArgElem]) -> CResult<Option<u32>> {
        if args.iter().any(|a| matches!(a, ArgElem::Spread(_))) {
            self.emit(Op::NewArray);
            for a in args {
                match a {
                    ArgElem::Expr(e) => {
                        self.expr(e)?;
                        self.emit(Op::ArrPush);
                    }
                    ArgElem::Spread(e) => {
                        self.expr(e)?;
                        self.emit(Op::ArrSpread);
                    }
                }
            }
            return Ok(None);
        }
        for a in args {
            if let ArgElem::Expr(e) = a {
                self.expr(e)?;
            }
        }
        Ok(Some(args.len() as u32))
    }

    fn callee_desc(&mut self, e: &Expr) -> u32 {
        // names are resolved lazily at error time; store the interned ids as a marker string
        let s = describe_expr(e);
        self.str_const(&Rc::from(s))
    }

    /// Push `this` and the function for a call.
    fn callee(&mut self, callee: &Expr) -> CResult {
        match callee {
            Expr::Dot { obj, name, optional } => {
                self.expr(obj)?;
                if *optional {
                    self.chain_short();
                }
                let k = self.str_key(name);
                self.emit(Op::GetMethod(k));
            }
            Expr::Member { obj, prop, optional } => {
                self.expr(obj)?;
                if *optional {
                    self.chain_short();
                }
                self.expr(prop)?;
                self.emit(Op::GetElemMethod);
            }
            Expr::PrivateMember { obj, name, optional } => {
                self.expr(obj)?;
                if *optional {
                    self.chain_short();
                }
                let k = self.private_key(name);
                self.emit(Op::GetMethod(k));
            }
            Expr::SuperDot(name) => {
                let k = self.str_key(name);
                self.emit(Op::GetSuperMethod(k));
            }
            Expr::Paren(inner) if matches!(**inner, Expr::Dot { .. } | Expr::Member { .. } | Expr::PrivateMember { .. }) => self.callee(inner)?,
            other => {
                self.emit(Op::Undef);
                self.expr(other)?;
            }
        }
        Ok(())
    }

    fn chain_short(&mut self) {
        let j = self.emit(Op::JumpIfNullishShort(0));
        match self.chains.last_mut() {
            Some(c) => c.push(j),
            None => {
                // optional link outside a chain wrapper: short-circuit to right after this op
                self.ops[j] = Op::Nop;
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> CResult {
        match e {
            Expr::Num(n) => {
                let n = *n;
                if crate::math::trunc(n) == n && n > -1e9 && n < 1e9 && !(n == 0.0 && n.is_sign_negative()) {
                    self.emit(Op::Int(n as i32));
                } else {
                    self.push_const(Value::Num(n));
                }
            }
            Expr::Str(s) => self.push_str(s),
            Expr::Bool(b) => {
                self.emit(if *b { Op::True } else { Op::False });
            }
            Expr::Null => {
                self.emit(Op::Null);
            }
            Expr::This => {
                self.emit(Op::This);
            }
            Expr::Ident(n) => self.load_name(*n),
            Expr::Paren(inner) => self.expr(inner)?,
            Expr::Template(strs, exprs) => {
                let mut parts = 0;
                for (i, s) in strs.iter().enumerate() {
                    if !s.is_empty() || (i == 0 && exprs.is_empty()) {
                        self.push_str(s);
                        parts += 1;
                    }
                    if let Some(x) = exprs.get(i) {
                        self.expr(x)?;
                        self.emit(Op::ToStr);
                        parts += 1;
                    }
                }
                if parts == 0 {
                    self.push_str(&Rc::from(""));
                } else if parts > 1 {
                    self.emit(Op::Concat(parts));
                } else if exprs.is_empty() {
                    // single literal part, already a string
                }
            }
            Expr::Tagged(tag, tobj, exprs) => {
                self.callee(tag)?;
                self.templates.push(tobj.clone());
                let ti = (self.templates.len() - 1) as u32;
                self.emit(Op::TemplateObj(ti));
                for x in exprs {
                    self.expr(x)?;
                }
                let d = self.callee_desc(tag);
                self.emit(Op::Call(exprs.len() as u32 + 1, d));
            }
            Expr::Regex(p, f) => {
                self.regexes.push((p.clone(), f.clone()));
                let i = (self.regexes.len() - 1) as u32;
                self.emit(Op::Regex(i));
            }
            Expr::Array(elems) => {
                self.emit(Op::NewArray);
                for el in elems {
                    match el {
                        None => {
                            self.emit(Op::ArrHole);
                        }
                        Some(ArgElem::Expr(x)) => {
                            self.expr(x)?;
                            self.emit(Op::ArrPush);
                        }
                        Some(ArgElem::Spread(x)) => {
                            self.expr(x)?;
                            self.emit(Op::ArrSpread);
                        }
                    }
                }
            }
            Expr::Object(props) => self.object(props)?,
            Expr::Function(f) => {
                let i = self.func_index(f);
                if f.name.is_some() && !f.is_arrow {
                    self.emit(Op::FuncExpr(i));
                } else {
                    self.emit(Op::Closure(i));
                }
            }
            Expr::Class(c) => {
                self.classes.push(c.clone());
                let i = (self.classes.len() - 1) as u32;
                self.emit(Op::Class(i, u32::MAX));
            }
            Expr::Unary(op, x) => self.unary(*op, x)?,
            Expr::Update { inc, prefix, target } => self.update(*inc, *prefix, target)?,
            Expr::Binary(op, a, b) => {
                self.expr(a)?;
                self.expr(b)?;
                self.emit(match op {
                    BinOp::In => Op::In,
                    o => Op::Bin(*o),
                });
            }
            Expr::Logical(op, a, b) => {
                self.expr(a)?;
                let j = self.emit(match op {
                    LogOp::And => Op::JumpIfFalseKeep(0),
                    LogOp::Or => Op::JumpIfTrueKeep(0),
                    LogOp::Nullish => Op::JumpIfNotNullishKeep(0),
                });
                self.expr(b)?;
                self.patch_here(j);
            }
            Expr::Assign(op, target, value) => self.assign(*op, target, value)?,
            Expr::Cond(t, a, b) => {
                self.expr(t)?;
                let j = self.emit(Op::JumpIfFalse(0));
                self.expr(a)?;
                let j2 = self.emit(Op::Jump(0));
                self.patch_here(j);
                self.expr(b)?;
                self.patch_here(j2);
            }
            Expr::Seq(list) => {
                for (i, x) in list.iter().enumerate() {
                    self.expr(x)?;
                    if i + 1 < list.len() {
                        self.emit(Op::Pop);
                    }
                }
                if list.is_empty() {
                    self.emit(Op::Undef);
                }
            }
            Expr::OptChain(inner) => {
                self.chains.push(Vec::new());
                self.expr(inner)?;
                let list = self.chains.pop().expect("chain");
                let end = self.here();
                for j in list {
                    self.patch(j, end);
                }
            }
            Expr::Dot { obj, name, optional } => {
                self.expr(obj)?;
                if *optional {
                    self.chain_short();
                }
                let k = self.str_key(name);
                self.emit(Op::GetProp(k));
            }
            Expr::PrivateMember { obj, name, optional } => {
                self.expr(obj)?;
                if *optional {
                    self.chain_short();
                }
                let k = self.private_key(name);
                self.emit(Op::GetProp(k));
            }
            Expr::Member { obj, prop, optional } => {
                self.expr(obj)?;
                if *optional {
                    self.chain_short();
                }
                self.expr(prop)?;
                self.emit(Op::GetElem);
            }
            Expr::Call { callee, args, optional } => {
                let direct_eval = matches!(&**callee, Expr::Ident(n) if *n == self.an.eval_name && self.resolve(*n).is_none());
                self.callee(callee)?;
                if *optional {
                    let j = self.emit(Op::JumpIfNullishShort2(0));
                    match self.chains.last_mut() {
                        Some(c) => c.push(j),
                        None => self.ops[j] = Op::Nop,
                    }
                }
                let d = self.callee_desc(callee);
                match self.args(args)? {
                    Some(n) if direct_eval => {
                        self.emit(Op::CallEval(n));
                    }
                    Some(n) => {
                        self.emit(Op::Call(n, d));
                    }
                    None => {
                        self.emit(Op::CallSpread(d));
                    }
                }
            }
            Expr::New(callee, args) => {
                self.expr(callee)?;
                let d = self.callee_desc(callee);
                match self.args(args)? {
                    Some(n) => self.emit(Op::New(n, d)),
                    None => self.emit(Op::NewSpread(d)),
                };
            }
            Expr::NewTarget => {
                self.emit(Op::NewTarget);
            }
            Expr::SuperCall(args) => {
                match self.args(args)? {
                    Some(n) => self.emit(Op::SuperCall(n)),
                    None => self.emit(Op::SuperCallSpread),
                };
            }
            Expr::SuperDot(name) => {
                let k = self.str_key(name);
                self.emit(Op::GetSuper(k));
            }
            Expr::SuperMember(p) => {
                self.expr(p)?;
                self.emit(Op::GetSuperElem);
            }
            Expr::Await(x) => {
                if !self.is_async {
                    return Err(Unsupported);
                }
                self.expr(x)?;
                self.emit(Op::Await);
            }
            Expr::Yield(arg, delegate) => {
                if !self.is_gen {
                    return Err(Unsupported);
                }
                match arg {
                    Some(a) => self.expr(a)?,
                    None => {
                        self.emit(Op::Undef);
                    }
                }
                if *delegate {
                    if self.is_async {
                        return Err(Unsupported);
                    }
                    let r = self.temp_pair();
                    self.emit(Op::GetIter(r));
                    self.emit(Op::Undef);
                    let at = self.emit(Op::YieldStar(r, 0));
                    self.patch_here(at);
                } else {
                    self.emit(Op::Yield);
                }
            }
            Expr::ImportCall(spec) => {
                self.expr(spec)?;
                self.emit(Op::ImportCall);
            }
            Expr::ImportMeta => {
                self.emit(Op::ImportMeta);
            }
        }
        Ok(())
    }

    fn object(&mut self, props: &[PropDef]) -> CResult {
        self.emit(Op::NewObject);
        for p in props {
            match &p.value {
                PropValue::Spread(e) => {
                    self.expr(e)?;
                    self.emit(Op::CopyProps);
                }
                PropValue::Value(e) => match &p.key {
                    PropKey::Name(n) => {
                        if &**n == "__proto__" {
                            self.expr(e)?;
                            self.emit(Op::SetProtoLit);
                        } else {
                            match e {
                                Expr::Function(f) if f.name.is_none() => {
                                    let i = self.func_index(f);
                                    let s = self.str_const(n);
                                    self.emit(Op::ClosureStrName(i, s));
                                }
                                Expr::Class(_) | Expr::Paren(_) => self.expr(e)?,
                                _ => self.expr(e)?,
                            }
                            let k = self.str_key(n);
                            self.emit(Op::InitProp(k));
                        }
                    }
                    PropKey::Private(_) => return Err(Unsupported),
                    PropKey::Computed(k) => {
                        self.expr(k)?;
                        self.emit(Op::ToKey);
                        self.expr(e)?;
                        self.emit(Op::InitElem);
                    }
                },
                PropValue::ShorthandDefault(..) => {
                    self.throw_error(2, "Invalid shorthand property initializer");
                }
                PropValue::Method(f) | PropValue::Getter(f) | PropValue::Setter(f) => {
                    match &p.key {
                        PropKey::Name(n) => self.push_str(n),
                        PropKey::Computed(k) => {
                            self.expr(k)?;
                            self.emit(Op::ToKey);
                        }
                        PropKey::Private(_) => return Err(Unsupported),
                    }
                    let kind = match &p.value {
                        PropValue::Method(_) => 0,
                        PropValue::Getter(_) => 1,
                        _ => 2,
                    };
                    let i = self.func_index(f);
                    self.emit(Op::InitMethod(i, kind));
                }
            }
        }
        Ok(())
    }

    fn unary(&mut self, op: UnOp, x: &Expr) -> CResult {
        match op {
            UnOp::TypeOf => match x {
                Expr::Ident(n) => match self.resolve(*n) {
                    Some((Store::Reg(r), _)) => {
                        self.emit(Op::GetReg(r));
                        self.emit(Op::TypeOf);
                    }
                    _ => {
                        self.emit(Op::TypeofName(*n));
                    }
                },
                _ => {
                    self.expr(x)?;
                    self.emit(Op::TypeOf);
                }
            },
            UnOp::Delete => match x {
                Expr::Dot { obj, name, .. } => {
                    self.expr(obj)?;
                    let k = self.str_key(name);
                    self.emit(Op::DelProp(k));
                }
                Expr::Member { obj, prop, .. } => {
                    self.expr(obj)?;
                    self.expr(prop)?;
                    self.emit(Op::DelElem);
                }
                Expr::Paren(inner) => self.unary(UnOp::Delete, inner)?,
                Expr::OptChain(inner) => {
                    self.chains.push(Vec::new());
                    self.unary(UnOp::Delete, inner)?;
                    let list = self.chains.pop().expect("chain");
                    if !list.is_empty() {
                        let skip = self.emit(Op::Jump(0));
                        let short = self.here();
                        for j in list {
                            self.patch(j, short);
                        }
                        self.emit(Op::Pop);
                        self.emit(Op::True);
                        self.patch_here(skip);
                    }
                }
                Expr::Ident(n) => match self.resolve(*n) {
                    Some(_) => {
                        self.emit(Op::False);
                    }
                    None => {
                        self.emit(Op::DeleteName(*n));
                    }
                },
                _ => {
                    self.expr(x)?;
                    self.emit(Op::Pop);
                    self.emit(Op::True);
                }
            },
            UnOp::Void => {
                self.expr(x)?;
                self.emit(Op::Pop);
                self.emit(Op::Undef);
            }
            UnOp::Not => {
                self.expr(x)?;
                self.emit(Op::Not);
            }
            UnOp::Neg => {
                self.expr(x)?;
                self.emit(Op::Neg);
            }
            UnOp::Plus => {
                self.expr(x)?;
                self.emit(Op::Pos);
            }
            UnOp::BitNot => {
                self.expr(x)?;
                self.emit(Op::BitNot);
            }
        }
        Ok(())
    }

    fn update(&mut self, inc: bool, prefix: bool, target: &Expr) -> CResult {
        let step = if inc { Op::Inc } else { Op::Dec };
        match target {
            Expr::Paren(inner) => return self.update(inc, prefix, inner),
            Expr::Ident(n) => {
                self.load_name(*n);
                self.emit(Op::Pos);
                if !prefix {
                    self.emit(Op::Dup);
                }
                self.emit(step);
                self.store_assign_keep(*n);
                if !prefix {
                    self.emit(Op::Pop);
                }
            }
            Expr::Dot { obj, name, .. } | Expr::PrivateMember { obj, name, .. } => {
                let k = if matches!(target, Expr::PrivateMember { .. }) { self.private_key(name) } else { self.str_key(name) };
                self.expr(obj)?;
                self.emit(Op::Dup);
                self.emit(Op::GetProp(k));
                self.emit(Op::Pos);
                if !prefix {
                    self.emit(Op::Dup);
                    self.emit(Op::Rot3);
                }
                self.emit(step);
                self.emit(Op::SetProp(k));
                if !prefix {
                    self.emit(Op::Pop);
                }
            }
            Expr::Member { obj, prop, .. } => {
                self.expr(obj)?;
                self.expr(prop)?;
                self.emit(Op::ToKey);
                self.emit(Op::Dup2);
                self.emit(Op::GetElem);
                self.emit(Op::Pos);
                if !prefix {
                    self.emit(Op::Dup);
                    self.emit(Op::Rot4);
                }
                self.emit(step);
                self.emit(Op::SetElem);
                if !prefix {
                    self.emit(Op::Pop);
                }
            }
            _ => {
                self.throw_error(2, "Invalid left-hand side expression in postfix operation");
                self.emit(Op::Undef);
            }
        }
        Ok(())
    }

    fn assign(&mut self, op: AssignOp, target: &Pat, value: &Expr) -> CResult {
        match op {
            AssignOp::Assign => match target {
                Pat::Ident(n) => {
                    self.expr_named(value, Some(*n))?;
                    self.store_assign_keep(*n);
                }
                Pat::Expr(e) => match &**e {
                    Expr::Dot { obj, name, .. } => {
                        self.expr(obj)?;
                        self.expr(value)?;
                        let k = self.str_key(name);
                        self.emit(Op::SetProp(k));
                    }
                    Expr::PrivateMember { obj, name, .. } => {
                        self.expr(obj)?;
                        self.expr(value)?;
                        let k = self.private_key(name);
                        self.emit(Op::SetProp(k));
                    }
                    Expr::Member { obj, prop, .. } => {
                        self.expr(obj)?;
                        self.expr(prop)?;
                        self.expr(value)?;
                        self.emit(Op::SetElem);
                    }
                    Expr::SuperDot(name) => {
                        self.expr(value)?;
                        let k = self.str_key(name);
                        self.emit(Op::SetSuper(k));
                    }
                    Expr::SuperMember(p) => {
                        self.expr(p)?;
                        self.expr(value)?;
                        self.emit(Op::SetSuperElem);
                    }
                    Expr::Ident(n) => {
                        self.expr_named(value, Some(*n))?;
                        self.store_assign_keep(*n);
                    }
                    Expr::Paren(inner) => {
                        let p = Pat::Expr(alloc::boxed::Box::new(clone_simple_target(inner).ok_or(Unsupported)?));
                        return self.assign(op, &p, value);
                    }
                    _ => {
                        self.expr(value)?;
                        self.throw_error(2, "Invalid assignment target");
                    }
                },
                _ => {
                    self.expr(value)?;
                    self.emit(Op::Dup);
                    self.bind_pattern(target, None)?;
                }
            },
            AssignOp::Bin(bop) => {
                let binop = |c: &mut Compiler| {
                    c.emit(match bop {
                        BinOp::In => Op::In,
                        o => Op::Bin(o),
                    });
                };
                match target {
                    Pat::Ident(n) => {
                        self.load_name(*n);
                        self.expr(value)?;
                        binop(self);
                        self.store_assign_keep(*n);
                    }
                    Pat::Expr(e) => match &**e {
                        Expr::Ident(n) => {
                            self.load_name(*n);
                            self.expr(value)?;
                            binop(self);
                            self.store_assign_keep(*n);
                        }
                        Expr::Dot { obj, name, .. } | Expr::PrivateMember { obj, name, .. } => {
                            let k = if matches!(&**e, Expr::PrivateMember { .. }) { self.private_key(name) } else { self.str_key(name) };
                            self.expr(obj)?;
                            self.emit(Op::Dup);
                            self.emit(Op::GetProp(k));
                            self.expr(value)?;
                            binop(self);
                            self.emit(Op::SetProp(k));
                        }
                        Expr::Member { obj, prop, .. } => {
                            self.expr(obj)?;
                            self.expr(prop)?;
                            self.emit(Op::ToKey);
                            self.emit(Op::Dup2);
                            self.emit(Op::GetElem);
                            self.expr(value)?;
                            binop(self);
                            self.emit(Op::SetElem);
                        }
                        Expr::SuperDot(name) => {
                            let k = self.str_key(name);
                            self.emit(Op::GetSuper(k));
                            self.expr(value)?;
                            binop(self);
                            self.emit(Op::SetSuper(k));
                        }
                        Expr::Paren(inner) => {
                            let p = Pat::Expr(alloc::boxed::Box::new(clone_simple_target(inner).ok_or(Unsupported)?));
                            return self.assign(op, &p, value);
                        }
                        _ => return Err(Unsupported),
                    },
                    _ => return Err(Unsupported),
                }
            }
            AssignOp::Logical(lop) => {
                let jump = |c: &mut Compiler| {
                    c.emit(match lop {
                        LogOp::And => Op::JumpIfFalseKeep(0),
                        LogOp::Or => Op::JumpIfTrueKeep(0),
                        LogOp::Nullish => Op::JumpIfNotNullishKeep(0),
                    })
                };
                let ident = match target {
                    Pat::Ident(n) => Some(*n),
                    Pat::Expr(e) => match &**e {
                        Expr::Ident(n) => Some(*n),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(n) = ident {
                    self.load_name(n);
                    let j = jump(self);
                    self.expr_named(value, Some(n))?;
                    self.store_assign_keep(n);
                    self.patch_here(j);
                    return Ok(());
                }
                let Pat::Expr(e) = target else { return Err(Unsupported) };
                match &**e {
                    Expr::Dot { obj, name, .. } | Expr::PrivateMember { obj, name, .. } => {
                        let k = if matches!(&**e, Expr::PrivateMember { .. }) { self.private_key(name) } else { self.str_key(name) };
                        self.expr(obj)?;
                        self.emit(Op::Dup);
                        self.emit(Op::GetProp(k));
                        let j = jump(self);
                        self.expr(value)?;
                        self.emit(Op::SetProp(k));
                        let done = self.emit(Op::Jump(0));
                        self.patch_here(j);
                        // short-circuit: [obj old] → [old]
                        self.emit(Op::Swap);
                        self.emit(Op::Pop);
                        self.patch_here(done);
                    }
                    Expr::Member { obj, prop, .. } => {
                        self.expr(obj)?;
                        self.expr(prop)?;
                        self.emit(Op::ToKey);
                        self.emit(Op::Dup2);
                        self.emit(Op::GetElem);
                        let j = jump(self);
                        self.expr(value)?;
                        self.emit(Op::SetElem);
                        let done = self.emit(Op::Jump(0));
                        self.patch_here(j);
                        // [obj key old] → [old]
                        self.emit(Op::Rot3);
                        self.emit(Op::Pop);
                        self.emit(Op::Pop);
                        self.patch_here(done);
                    }
                    _ => return Err(Unsupported),
                }
            }
        }
        Ok(())
    }
}

/// Re-express a parenthesised assignment target (`(a.b) = 1`).
fn clone_simple_target(e: &Expr) -> Option<Expr> {
    match e {
        Expr::Ident(n) => Some(Expr::Ident(*n)),
        Expr::Paren(i) => clone_simple_target(i),
        _ => None,
    }
}

/// Function declarations in nested blocks (Annex B candidates).
fn collect_block_funcs(s: &Stmt, out: &mut Vec<Name>) {
    fn in_block(b: &Block, out: &mut Vec<Name>) {
        for f in &b.funcs {
            if let Some(n) = f.name {
                if !f.is_async && !f.is_generator {
                    out.push(n);
                }
            }
        }
        for s in &b.stmts {
            collect_block_funcs(s, out);
        }
    }
    match s {
        Stmt::Block(b) => in_block(b, out),
        Stmt::If(_, a, b) => {
            collect_block_funcs(a, out);
            if let Some(b) = b {
                collect_block_funcs(b, out);
            }
        }
        Stmt::For { body, .. } | Stmt::ForIn { body, .. } | Stmt::ForOf { body, .. } | Stmt::While(_, body) | Stmt::DoWhile(body, _) | Stmt::Labeled(_, body) | Stmt::With(_, body) => collect_block_funcs(body, out),
        Stmt::Try { block, handler, finalizer, .. } => {
            in_block(block, out);
            if let Some(h) = handler {
                in_block(h, out);
            }
            if let Some(f) = finalizer {
                in_block(f, out);
            }
        }
        Stmt::Switch { cases, funcs, .. } => {
            for f in funcs {
                if let Some(n) = f.name {
                    out.push(n);
                }
            }
            for c in cases {
                for s in &c.body {
                    collect_block_funcs(s, out);
                }
            }
        }
        _ => {}
    }
}

/// Callee description for "x is not a function" messages; identifiers are stored as `\u{1}id\u{1}`
/// markers and expanded through the interner when the error is raised.
fn describe_expr(e: &Expr) -> alloc::string::String {
    use alloc::string::ToString;
    match e {
        Expr::Ident(n) => alloc::format!("\u{1}{n}\u{1}"),
        Expr::Dot { obj, name, .. } => alloc::format!("{}.{}", describe_expr(obj), name),
        Expr::PrivateMember { obj, name, .. } => alloc::format!("{}.#{}", describe_expr(obj), name),
        Expr::This => "this".to_string(),
        Expr::Paren(i) | Expr::OptChain(i) => describe_expr(i),
        Expr::Member { obj, .. } => alloc::format!("{}[...]", describe_expr(obj)),
        Expr::Call { callee, .. } => alloc::format!("{}(...)", describe_expr(callee)),
        Expr::SuperDot(n) => alloc::format!("super.{n}"),
        _ => "expression".to_string(),
    }
}

