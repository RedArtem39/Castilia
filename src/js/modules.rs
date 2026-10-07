//! ES module loading: instantiation, evaluation, namespaces, dynamic import and import maps.
use super::{ast::*, builtins::*, interp::*, url::Url, value::*};
use alloc::{
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::RefCell;

impl Interp {
    /// Resolve a module specifier against the importing module's URL and the import map.
    pub fn resolve_specifier(&mut self, spec: &str, referrer: &str) -> Res<String> {
        // import map: exact match, then longest prefix ending in '/'
        if let Some(t) = self.import_map.get(spec) {
            return Ok(t.clone());
        }
        let mut best: Option<(&String, &String)> = None;
        for (k, v) in &self.import_map {
            if k.ends_with('/') && spec.starts_with(k.as_str()) && best.is_none_or(|(bk, _)| k.len() > bk.len()) {
                best = Some((k, v));
            }
        }
        if let Some((k, v)) = best {
            let full = format!("{}{}", v, &spec[k.len()..]);
            return Ok(full);
        }
        if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') || Url::parse(spec, None).is_some() {
            let base = Url::parse(referrer, None);
            return match Url::parse(spec, base.as_ref()) {
                Some(u) => Ok(u.href()),
                None => Err(self.error_ctl("TypeError", &format!("Failed to resolve module specifier '{spec}'"))),
            };
        }
        Err(self.error_ctl("TypeError", &format!("Failed to resolve module specifier '{spec}'. Relative references must start with either \"/\", \"./\", or \"../\".")))
    }

    fn module_source(&mut self, url: &str) -> Option<String> {
        if let Some(s) = self.module_sources.get(url) {
            return Some(s.clone());
        }
        let r = self.host_fetch("GET", url, &[], None)?;
        if r.status >= 400 {
            return None;
        }
        Some(r.body)
    }

    /// Load (and evaluate on first use) the module at `url`; returns its namespace object.
    pub fn load_module(&mut self, url: &str) -> Res<Obj> {
        if let Some(ns) = self.modules.get(url) {
            return Ok(ns.clone());
        }
        let Some(source) = self.module_source(url) else {
            return Err(self.error_ctl("TypeError", &format!("Failed to fetch module: {url}")));
        };
        self.instantiate_module(url, &source)
    }

    pub fn instantiate_module(&mut self, url: &str, source: &str) -> Res<Obj> {
        let prog = match super::parser::parse_program(source, &mut self.interner, true) {
            Ok(p) => p,
            Err(e) => return Err(self.error_ctl("SyntaxError", &format!("{url}: {}", e.trim_start_matches("SyntaxError: ")))),
        };
        // namespace first (cycles see the partially populated object)
        let ns = Obj::new(Object::new(None, Class::Plain));
        let tag = Key::Y(self.wk().to_string_tag.0.id);
        self.define_data(&ns, tag, Value::str("Module"), 0);
        self.modules.insert(url.to_string(), ns.clone());
        let env = Rc::new(Scope {
            vars: RefCell::new(BTreeMap::new()),
            parent: Some(self.global_env.clone()),
            func: Some(FuncCtx { this: RefCell::new(Some(Value::Undefined)), callee: None, new_target: Value::Undefined, home: None, is_global: false, strict: true }),
            with_obj: None,
        });
        let holder = Obj::new(Object::new(None, Class::ModuleEnv(env.clone())));
        let r = self.run_module_body(url, &prog, &env, &holder, &ns);
        if r.is_err() {
            self.modules.remove(url);
        }
        r?;
        Ok(ns)
    }

    fn export_getter(&mut self, ns: &Obj, exported: &str, holder: &Obj, local: &str) {
        let data = self.new_array(alloc::vec![Value::Obj(holder.clone()), Value::str(local)]);
        let g = self.new_native_data(exported, 0, module_export_get, data, false);
        self.define_accessor_part(ns, Key::str(exported), Some(g), true, ENUMERABLE);
    }

    fn reexport_getter(&mut self, ns: &Obj, exported: &str, dep: &Obj, imported: &str) {
        let data = self.new_array(alloc::vec![Value::Obj(dep.clone()), Value::str(imported)]);
        let g = self.new_native_data(exported, 0, module_reexport_get, data, false);
        self.define_accessor_part(ns, Key::str(exported), Some(g), true, ENUMERABLE);
    }

    fn run_module_body(&mut self, url: &str, prog: &Program, env: &Env, holder: &Obj, ns: &Obj) -> Res<()> {
        self.module_urls.push(url.to_string());
        let r = self.run_module_inner(url, prog, env, holder, ns);
        self.module_urls.pop();
        r
    }

    fn run_module_inner(&mut self, url: &str, prog: &Program, env: &Env, holder: &Obj, ns: &Obj) -> Res<()> {
        for &n in &prog.var_names {
            self.declare(env, n, 0, Value::Undefined, true);
        }
        self.hoist_block(&prog.body, env, false)?;
        // exports of local bindings are defined up front (live through the environment)
        for s in &prog.body.stmts {
            match s {
                Stmt::ExportDecl(_, names) => {
                    for (local, exported) in names {
                        let l = self.interner.get(*local).to_string();
                        self.export_getter(ns, exported, holder, &l);
                    }
                }
                Stmt::ExportDefault(_) => self.export_getter(ns, "default", holder, "*default*"),
                Stmt::ExportNamed { specs, from: None } => {
                    for (local, exported) in specs {
                        let l = self.interner.get(*local).to_string();
                        self.export_getter(ns, exported, holder, &l);
                    }
                }
                _ => {}
            }
        }
        // link dependencies (imports are hoisted)
        for s in &prog.body.stmts {
            match s {
                Stmt::Import { source, specs } => {
                    let dep_url = self.resolve_specifier(source, url)?;
                    let dep = self.load_module(&dep_url)?;
                    for spec in specs {
                        match spec {
                            ImportSpec::Default(n) => {
                                let v = self.get_obj(&dep, &Key::str("default"), &Value::Obj(dep.clone()))?;
                                self.declare(env, *n, 2, v, true);
                            }
                            ImportSpec::Namespace(n) => self.declare(env, *n, 2, Value::Obj(dep.clone()), true),
                            ImportSpec::Named(imported, local) => {
                                let key = Key::S(imported.clone());
                                if !self.has_property(&dep, &key)? && self.modules.contains_key(&dep_url) && !self.module_in_progress(&dep_url) {
                                    return Err(self.error_ctl("SyntaxError", &format!("The requested module '{source}' does not provide an export named '{imported}'")));
                                }
                                let v = self.get_obj(&dep, &key, &Value::Obj(dep.clone()))?;
                                self.declare(env, *local, 2, v, true);
                            }
                        }
                    }
                }
                Stmt::ExportNamed { specs, from: Some(src) } => {
                    let dep_url = self.resolve_specifier(src, url)?;
                    let dep = self.load_module(&dep_url)?;
                    for (local, exported) in specs {
                        let l = self.interner.get(*local).to_string();
                        self.reexport_getter(ns, exported, &dep, &l);
                    }
                }
                Stmt::ExportAll(src, alias) => {
                    let dep_url = self.resolve_specifier(src, url)?;
                    let dep = self.load_module(&dep_url)?;
                    match alias {
                        Some(a) => {
                            let a = a.to_string();
                            self.define_data(ns, Key::S(Rc::from(a)), Value::Obj(dep), ENUMERABLE | CONFIGURABLE);
                        }
                        None => {
                            let keys = self.own_keys(&dep)?;
                            for k in keys {
                                if let Key::S(s) = &k {
                                    if &**s != "default" && !self.has_own(ns, &k) {
                                        self.reexport_getter(ns, s, &dep, s);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // evaluate
        for s in &prog.body.stmts {
            if matches!(s, Stmt::Import { .. } | Stmt::ExportNamed { .. } | Stmt::ExportAll(..)) {
                continue;
            }
            self.exec(s, env, &mut Vec::new())?;
        }
        Ok(())
    }

    fn module_in_progress(&self, url: &str) -> bool {
        self.module_urls.iter().any(|u| u == url)
    }

    pub fn dynamic_import_value(&mut self, spec: &Value) -> Res {
        let p = self.new_promise();
        let referrer = self.module_urls.last().cloned().unwrap_or_else(|| self.dom.base_url.clone());
        let r = (|| -> Res<Obj> {
            let s = self.to_str(spec)?;
            let url = self.resolve_specifier(&s, &referrer)?;
            self.load_module(&url)
        })();
        match r {
            Ok(ns) => self.resolve_promise(&p, Value::Obj(ns))?,
            Err(Ctl::Throw(e)) => self.reject_promise(&p, e),
            Err(e) => return Err(e),
        }
        Ok(Value::Obj(p))
    }

    pub fn current_module_url(&self) -> String {
        self.module_urls.last().cloned().unwrap_or_else(|| self.dom.base_url.clone())
    }
}

fn module_export_get(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(d) = it.native_data(c) else { return Ok(Value::Undefined) };
    let (holder, name) = match &d.borrow().class {
        Class::Array(a) => (a[0].clone(), a[1].clone()),
        _ => return Ok(Value::Undefined),
    };
    let Value::Obj(h) = holder else { return Ok(Value::Undefined) };
    let env = match &h.borrow().class {
        Class::ModuleEnv(e) => e.clone(),
        _ => return Ok(Value::Undefined),
    };
    let n = it.to_string_lossy(&name);
    let id = it.interner.intern(&n);
    let v = env.vars.borrow().get(&id).map(|v| (v.v.clone(), v.init));
    match v {
        Some((v, true)) => Ok(v),
        Some((_, false)) => Err(it.error_ctl("ReferenceError", &format!("Cannot access '{n}' before initialization"))),
        None => Ok(Value::Undefined),
    }
}

fn module_reexport_get(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(d) = it.native_data(c) else { return Ok(Value::Undefined) };
    let (dep, name) = match &d.borrow().class {
        Class::Array(a) => (a[0].clone(), a[1].clone()),
        _ => return Ok(Value::Undefined),
    };
    let key = it.to_key(&name)?;
    it.get_v(&dep, &key)
}

pub fn set_import_map(it: &mut Interp, c: &Call) -> Res {
    let map = it.get_str(&c.arg(0), "imports")?;
    if let Value::Obj(o) = &map {
        for k in it.own_keys(o)? {
            if let Key::S(ks) = &k {
                let v = it.get_obj(o, &k, &map)?;
                if let Value::Str(s) = v {
                    let base = it.dom.base_url.clone();
                    let resolved = Url::parse(&s, Url::parse(&base, None).as_ref()).map(|u| u.href()).unwrap_or_else(|| s.to_string());
                    it.import_map.insert(ks.to_string(), resolved);
                }
            }
        }
    }
    Ok(Value::Undefined)
}
