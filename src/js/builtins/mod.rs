//! Standard library: creates the realm (prototypes, global object, constructors).
use super::{
    ast::Name,
    interp::*,
    parser::Interner,
    value::*,
};
use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::RefCell;

pub mod array;
pub mod binary;
pub mod collections;
pub mod date;
pub mod json;
pub mod misc;
pub mod number;
pub mod object;
pub mod promise;
pub mod string;

fn noop(_: &mut Interp, _: &Call) -> Res {
    Ok(Value::Undefined)
}

pub fn new_interp() -> Interp {
    let object = Obj::new(Object::new(None, Class::Plain));
    let plain = |proto: &Obj| Obj::new(Object::new(Some(proto.clone()), Class::Plain));
    let function = Obj::new(Object::new(Some(object.clone()), Class::Function(Func::Native { f: noop, data: Value::Undefined, ctor: false })));
    let array = Obj::new(Object::new(Some(object.clone()), Class::Array(Vec::new())));
    let string = Obj::new(Object::new(Some(object.clone()), Class::Str(Rc::from(""))));
    let number = Obj::new(Object::new(Some(object.clone()), Class::Num(0.0)));
    let boolean = Obj::new(Object::new(Some(object.clone()), Class::Bool(false)));
    let error = plain(&object);
    let iterator = plain(&object);
    let mut error_kinds = BTreeMap::new();
    for k in ["Error", "TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError", "URIError", "AggregateError"] {
        error_kinds.insert(k, if k == "Error" { error.clone() } else { plain(&error) });
    }
    let event_target = plain(&object);
    let node = plain(&event_target);
    let element = plain(&node);
    let html_element = plain(&element);
    let protos = Protos {
        symbol: plain(&object),
        date: plain(&object),
        regexp: plain(&object),
        map: plain(&object),
        set: plain(&object),
        weakmap: plain(&object),
        weakset: plain(&object),
        promise: plain(&object),
        generator: plain(&iterator),
        array_iterator: plain(&iterator),
        text: plain(&node),
        document: plain(&node),
        event: plain(&object),
        event_target,
        node,
        element,
        html_element,
        iterator,
        error_kinds,
        extra: BTreeMap::new(),
        object: object.clone(),
        function,
        array,
        string,
        number,
        boolean,
        error,
    };
    let global = Obj::new(Object::new(Some(object.clone()), Class::Window));
    let global_env = Rc::new(Scope {
        vars: RefCell::new(VarMap::new()),
        parent: None,
        with_obj: None,
        func: Some(FuncCtx { this: RefCell::new(Some(Value::Obj(global.clone()))), callee: None, new_target: Value::Undefined, home: None, is_global: true, strict: false }),
    });
    let mk = |id: u32, d: &str| Sym(Rc::new(SymData { id, desc: Some(Rc::from(d)) }));
    let wk = WellKnown {
        iterator: mk(1, "Symbol.iterator"),
        async_iterator: mk(2, "Symbol.asyncIterator"),
        to_primitive: mk(3, "Symbol.toPrimitive"),
        has_instance: mk(4, "Symbol.hasInstance"),
        to_string_tag: mk(5, "Symbol.toStringTag"),
        species: mk(6, "Symbol.species"),
    };
    let mut interner = Interner::default();
    let names = Names {
        length: Key::str("length"),
        prototype: Key::str("prototype"),
        constructor: Key::str("constructor"),
        name: Key::str("name"),
        message: Key::str("message"),
        stack: Key::str("stack"),
        then: Key::str("then"),
        next: Key::str("next"),
        done: Key::str("done"),
        value: Key::str("value"),
        get: Key::str("get"),
        set: Key::str("set"),
        to_string: Key::str("toString"),
        value_of: Key::str("valueOf"),
        to_json: Key::str("toJSON"),
        this_name: interner.intern("this"),
        arguments: interner.intern("arguments"),
        undefined: interner.intern("undefined"),
    };
    let mut it = Interp {
        interner,
        global,
        global_env,
        protos: Some(protos),
        wk: Some(wk),
        sym_counter: 6,
        symbol_registry: BTreeMap::new(),
        depth: 0,
        max_depth: 120,
        stack_base: 0,
        max_stack: 1 << 20,
        ticks: 0,
        tick_limit: 400_000_000,
        microtasks: VecDeque::new(),
        timers: Vec::new(),
        timer_seq: 0,
        now: 0.0,
        doc: core::ptr::null_mut(),
        node_cache: BTreeMap::new(),
        dom_gc_next: 20_000,
        pinned_nodes: Vec::new(),
        gen_stack: Vec::new(),
        logs: Vec::new(),
        names,
        rng: 0x2545F4914F6CDD1D,
        unhandled: Vec::new(),
        draining: 0,
        host: super::dom::HostSlot::default(),
        dom: super::dom::DomState::default(),
        modules: BTreeMap::new(),
        module_sources: BTreeMap::new(),
        module_urls: Vec::new(),
        import_map: BTreeMap::new(),
        array_values_native: true,
        join_stack: Vec::new(),
        console_indent: 0,
        loaded_links: Vec::new(),
        last_prevented: false,
        dispatch_count: 0,
        history_state: BTreeMap::new(),
        parsing_html_scripts: false,
        executed_scripts: Vec::new(),
        current_script: None,
        custom_elements: Vec::new(),
        template_content: BTreeMap::new(),
        shadow_closed: Vec::new(),
        upgrade_target: None,
        upgraded: Default::default(),
        when_defined: Vec::new(),
        counters: BTreeMap::new(),
        dom_symbols: BTreeMap::new(),
        tier: super::vm::Tier::Bytecode,
        vm_stats: Default::default(),
        vm_frames: 0,
        max_frames: 10_000,
        max_task_allocs: 8_000_000,
        max_string_len: 256 << 20,
        frame_pool: Vec::new(),
        source_urls: BTreeMap::new(),
        interrupt: None,
        next_poll: 0,
        trace_errors: false,
        heap: Default::default(),
        eval_fn: None,
        #[cfg(feature = "jit")]
        jit: Default::default(),
    };
    object::setup(&mut it);
    array::setup(&mut it);
    string::setup(&mut it);
    number::setup(&mut it);
    number::setup_bigint(&mut it);
    json::setup(&mut it);
    date::setup(&mut it);
    collections::setup(&mut it);
    promise::setup(&mut it);
    misc::setup(&mut it);
    super::dom_setup::setup_all(&mut it);
    super::events::setup(&mut it);
    super::window::setup(&mut it);
    setup_eval(&mut it);
    it
}

/// Global `eval`. Called indirectly it runs in the global scope; direct calls inside VM functions
/// are recognised by identity and run in the caller's scope.
fn setup_eval(it: &mut Interp) {
    let f = it.new_native("eval", 1, |it, c| {
        let Value::Str(src) = c.arg(0) else { return Ok(c.arg(0)) };
        let prog = match super::parser::parse_program(&src, &mut it.interner, false) {
            Ok(p) => p,
            Err(e) => return Err(it.error_ctl("SyntaxError", e.trim_start_matches("SyntaxError: "))),
        };
        it.run_program(&prog)
    });
    let g = it.global.clone();
    it.define_data(&g, Key::str("eval"), Value::Obj(f.clone()), HIDDEN);
    it.eval_fn = Some(f);
}

// ---------------------------------------------------------------------------
// Shared helpers for builtin modules
// ---------------------------------------------------------------------------

impl Interp {
    /// Create a constructor function bound to `proto`, and register it on the global object.
    pub fn make_ctor(&mut self, name: &str, len: u32, f: NativeFn, proto: &Obj) -> Obj {
        let c = self.new_native_data(name, len, f, Value::Undefined, true);
        self.define_data(&c, self.names.prototype.clone(), Value::Obj(proto.clone()), 0);
        self.define_data(proto, self.names.constructor.clone(), Value::Obj(c.clone()), HIDDEN);
        let g = self.global.clone();
        self.define_data(&g, Key::str(name), Value::Obj(c.clone()), HIDDEN);
        c
    }

    pub fn global_fn(&mut self, name: &str, len: u32, f: NativeFn) {
        let g = self.global.clone();
        self.def_method(&g, name, len, f);
    }

    pub fn this_obj(&mut self, c: &Call, what: &str) -> Res<Obj> {
        match &c.this {
            Value::Obj(o) => Ok(o.clone()),
            _ => Err(self.error_ctl("TypeError", &alloc::format!("{what} called on non-object"))),
        }
    }

    pub fn arg_str(&mut self, c: &Call, i: usize) -> Res<JsStr> {
        self.to_str(&c.arg(i))
    }

    pub fn arg_num(&mut self, c: &Call, i: usize) -> Res<f64> {
        self.to_number(&c.arg(i))
    }

    /// Prototype for an object created by a native constructor (honours subclassing).
    pub fn proto_from_call(&mut self, c: &Call, default: &Obj) -> Res<Obj> {
        match &c.new_target {
            Some(nt) if !nt.ptr_eq(c.callee) => self.proto_for_default(nt, default),
            _ => Ok(default.clone()),
        }
    }

    fn proto_for_default(&mut self, nt: &Obj, default: &Obj) -> Res<Obj> {
        let p = self.get_obj(nt, &self.names.prototype.clone(), &Value::Obj(nt.clone()))?;
        Ok(match p {
            Value::Obj(o) => o,
            _ => default.clone(),
        })
    }

    pub fn array_like_to_vec(&mut self, v: &Value) -> Res<Vec<Value>> {
        if let Value::Obj(o) = v {
            if let Class::Array(a) = &o.borrow().class {
                return Ok(unhole_vec(a.clone()));
            }
            let len = self.get_v(v, &self.names.length.clone())?;
            let n = self.to_number(&len)?;
            let n = if n.is_nan() || n < 0.0 { 0 } else { (n as usize).min(50_000_000) };
            let mut out = Vec::with_capacity(n.min(1 << 20));
            for i in 0..n {
                out.push(self.get_v(v, &Key::S(Rc::from(i.to_string())))?);
            }
            return Ok(out);
        }
        if let Value::Str(s) = v {
            return Ok(s.chars().map(|c| Value::string(c.to_string())).collect());
        }
        Ok(Vec::new())
    }

    pub fn make_iter_result(&mut self, value: Value, done: bool) -> Value {
        let o = self.new_object();
        self.define_data(&o, self.names.value.clone(), value, ALL);
        self.define_data(&o, self.names.done.clone(), Value::Bool(done), ALL);
        Value::Obj(o)
    }

    pub fn intern(&mut self, s: &str) -> Name {
        self.interner.intern(s)
    }

    pub fn next_random(&mut self) -> f64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545F4914F6CDD1D);
        (r >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Create a list iterator object backed by a snapshot of values.
    pub fn make_list_iterator(&mut self, items: Vec<Value>) -> Value {
        let proto = self.protos().array_iterator.clone();
        let o = self.alloc(Object::new(Some(proto), Class::Plain));
        let state = self.new_array(items);
        self.define_data(&o, Key::str("__items"), state, 0);
        self.define_data(&o, Key::str("__pos"), Value::Num(0.0), WRITABLE);
        Value::Obj(o)
    }
}

pub fn list_iterator_next(it: &mut Interp, c: &Call) -> Res {
    let Value::Obj(o) = &c.this else { return it.type_error("next called on non-iterator") };
    let items = it.get_v(&c.this, &Key::str("__items"))?;
    let pos = match it.get_v(&c.this, &Key::str("__pos"))? {
        Value::Num(n) => n as usize,
        _ => 0,
    };
    let v = match &items {
        Value::Obj(a) => match &a.borrow().class {
            Class::Array(a) => a.get(pos).cloned().map(unhole),
            _ => None,
        },
        _ => None,
    };
    match v {
        Some(v) => {
            it.define_data(o, Key::str("__pos"), Value::Num((pos + 1) as f64), WRITABLE);
            Ok(it.make_iter_result(v, false))
        }
        None => Ok(it.make_iter_result(Value::Undefined, true)),
    }
}

pub fn boxed<T>(t: T) -> Box<T> {
    Box::new(t)
}

pub fn jstr(s: &str) -> JsStr {
    Rc::from(s)
}

pub fn to_string_vec(it: &mut Interp, vals: &[Value]) -> Res<Vec<String>> {
    let mut out = Vec::new();
    for v in vals {
        out.push(it.to_str(v)?.to_string());
    }
    Ok(out)
}
