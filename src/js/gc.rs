//! Cycle collector for the reference-counted heap.
//!
//! Objects and scopes are `Rc`s, so acyclic garbage is freed immediately; only cycles leak
//! (closure ↔ scope, prototype ↔ constructor, parent ↔ child structures). This collector finds
//! them with the trial-deletion algorithm CPython uses:
//!
//! 1. every allocation is registered as a `Weak`;
//! 2. for each live node, references coming from other tracked nodes are subtracted from its
//!    strong count — what remains are references held by the engine itself (frames, DOM, timers,
//!    caches, Rust locals): those nodes are roots;
//! 3. everything reachable from a root is live; the rest is cyclic garbage and gets its contents
//!    cleared, which lets `Rc` free the whole cycle.
//!
//! The scheme is conservative: an edge the collector does not know about only makes its target a
//! root, so a live object can never be freed. It runs between tasks, when no object is borrowed.
use super::{
    interp::{Env, Interp, Scope, VarMap},
    value::*,
    vm::Frame,
};
use alloc::{
    collections::BTreeMap,
    rc::{Rc, Weak},
    vec::Vec,
};
use core::cell::{Cell, RefCell};

/// Registry of collectable allocations.
pub struct Heap {
    objs: RefCell<Vec<Weak<RefCell<Object>>>>,
    scopes: RefCell<Vec<Weak<Scope>>>,
    /// Allocations since the last collection.
    since: Cell<usize>,
    /// Allocations between collections (0 disables automatic collection).
    pub threshold: usize,
    pub stats: GcStats,
}

#[derive(Default, Clone, Debug)]
pub struct GcStats {
    pub collections: usize,
    pub freed_objects: usize,
    pub freed_scopes: usize,
    /// Tracked nodes alive after the last collection.
    pub live: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Heap { objs: RefCell::new(Vec::new()), scopes: RefCell::new(Vec::new()), since: Cell::new(0), threshold: 200_000, stats: GcStats::default() }
    }
}

impl Heap {
    fn note(&self) {
        self.since.set(self.since.get() + 1);
    }

    /// Drop registry entries whose node is already gone.
    fn compact(&self) {
        self.objs.borrow_mut().retain(|w| w.strong_count() > 0);
        self.scopes.borrow_mut().retain(|w| w.strong_count() > 0);
    }
}

enum Node {
    O(Rc<RefCell<Object>>),
    S(Rc<Scope>),
}

impl Node {
    fn id(&self) -> usize {
        match self {
            Node::O(o) => Rc::as_ptr(o) as *const u8 as usize,
            Node::S(s) => Rc::as_ptr(s) as *const u8 as usize,
        }
    }
    fn strong(&self) -> usize {
        match self {
            Node::O(o) => Rc::strong_count(o),
            Node::S(s) => Rc::strong_count(s),
        }
    }
}

/// Collects the identities of the `Rc`s held directly by a node.
struct Edges<'a> {
    out: &'a mut Vec<usize>,
}

impl Edges<'_> {
    fn obj(&mut self, o: &Obj) {
        self.out.push(Rc::as_ptr(&o.0) as *const u8 as usize);
    }
    fn env(&mut self, e: &Env) {
        self.out.push(Rc::as_ptr(e) as *const u8 as usize);
    }
    fn val(&mut self, v: &Value) {
        if let Value::Obj(o) = v {
            self.obj(o);
        }
    }
    fn frame(&mut self, f: &Frame) {
        for v in f.stack.iter().chain(f.regs.iter()).chain(f.args.iter()) {
            self.val(v);
        }
        self.env(&f.env);
        for s in &f.scopes {
            self.env(s);
        }
        self.env(&f.fenv);
        if let Some(c) = &f.callee {
            self.obj(c);
        }
        if let Some(p) = &f.promise {
            self.obj(p);
        }
        for c in &f.completions {
            match c {
                super::vm::Completion::Throw(v) | super::vm::Completion::Return(v) => self.val(v),
                _ => {}
            }
        }
        if let Some((_, v)) = &f.pending_resume {
            self.val(v);
        }
    }
    fn object(&mut self, o: &Object) {
        if let Some(p) = &o.proto {
            self.obj(p);
        }
        for (_, p) in o.props.iter() {
            match &p.slot {
                Slot::Data(v) => self.val(v),
                Slot::Accessor(g, s) => {
                    if let Some(g) = g {
                        self.obj(g);
                    }
                    if let Some(s) = s {
                        self.obj(s);
                    }
                }
            }
        }
        match &o.class {
            Class::Array(a) => a.iter().for_each(|v| self.val(v)),
            Class::Function(Func::Closure(c)) => {
                self.env(&c.env);
                if let Some(h) = &c.home {
                    self.obj(h);
                }
                // field initialisers are shared (`Rc<Vec<..>>`): only count them when unshared
                if Rc::strong_count(&c.fields) == 1 {
                    for f in c.fields.iter() {
                        self.env(&f.env);
                        self.obj(&f.home);
                    }
                }
            }
            Class::Function(Func::Native { data, .. }) => self.val(data),
            Class::Function(Func::Bound { target, this, args }) => {
                self.obj(target);
                self.val(this);
                args.iter().for_each(|v| self.val(v));
            }
            Class::Map(m) | Class::Set(m) | Class::WeakMap(m) | Class::WeakSet(m) => {
                for (k, v) in m.entries.iter().flatten() {
                    self.val(k);
                    self.val(v);
                }
            }
            Class::Promise(p) => {
                match &p.state {
                    PromiseState::Fulfilled(v) | PromiseState::Rejected(v) => self.val(v),
                    PromiseState::Pending => {}
                }
                for r in &p.reactions {
                    if let Some(v) = &r.on_fulfilled {
                        self.val(v);
                    }
                    if let Some(v) = &r.on_rejected {
                        self.val(v);
                    }
                    if let Some(o) = &r.result {
                        self.obj(o);
                    }
                }
            }
            Class::Generator(g) => {
                g.values.iter().for_each(|v| self.val(v));
                self.val(&g.ret);
                if let Some((_, env)) = &g.pending {
                    self.env(env);
                }
                if let Some(co) = &g.co {
                    if let Some(f) = &co.frame {
                        self.frame(f);
                    }
                    for (_, v, p) in &co.queue {
                        self.val(v);
                        self.obj(p);
                    }
                }
            }
            Class::Proxy { target, handler } => {
                self.obj(target);
                self.obj(handler);
            }
            Class::ModuleEnv(e) => self.env(e),
            Class::Typed(v) | Class::DataView(v) => self.obj(&v.buffer),
            _ => {}
        }
    }
    fn scope(&mut self, s: &Scope) {
        if let Ok(vars) = s.vars.try_borrow() {
            for (_, v) in vars.entries() {
                self.val(&v.v);
            }
        }
        if let Some(p) = &s.parent {
            self.env(p);
        }
        if let Some(f) = &s.func {
            if let Ok(t) = f.this.try_borrow() {
                if let Some(v) = &*t {
                    self.val(v);
                }
            }
            if let Some(c) = &f.callee {
                self.obj(c);
            }
            self.val(&f.new_target);
            if let Some(h) = &f.home {
                self.obj(h);
            }
        }
        if let Some(w) = &s.with_obj {
            self.obj(w);
        }
    }
}

impl Interp {
    /// Allocate a collectable object.
    pub fn alloc(&self, o: Object) -> Obj {
        let obj = Obj::new(o);
        self.heap.objs.borrow_mut().push(Rc::downgrade(&obj.0));
        self.heap.note();
        obj
    }

    /// Register a scope with the collector.
    pub fn track_scope(&self, s: Env) -> Env {
        self.heap.scopes.borrow_mut().push(Rc::downgrade(&s));
        self.heap.note();
        s
    }

    /// Collect if enough has been allocated since the last run. Call only between tasks.
    pub fn maybe_gc(&mut self) {
        let since = self.heap.since.get();
        if self.heap.threshold == 0 || since < self.heap.threshold {
            return;
        }
        // registries hold dead weak entries too; compact cheaply before deciding on a full pass
        let before = self.heap.objs.borrow().len() + self.heap.scopes.borrow().len();
        self.heap.compact();
        let after = self.heap.objs.borrow().len() + self.heap.scopes.borrow().len();
        if after * 4 < before && self.heap.threshold >= 1000 {
            // most allocations were short-lived and are gone already
            self.heap.since.set(0);
            return;
        }
        self.gc();
    }

    /// Full cycle collection. Returns the number of freed nodes.
    pub fn gc(&mut self) -> usize {
        self.heap.since.set(0);
        let mut nodes: Vec<Node> = Vec::new();
        nodes.extend(self.heap.objs.borrow().iter().filter_map(|w| w.upgrade()).map(Node::O));
        nodes.extend(self.heap.scopes.borrow().iter().filter_map(|w| w.upgrade()).map(Node::S));
        let index: BTreeMap<usize, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id(), i)).collect();
        // our own upgrade adds one strong reference to each node
        let mut refs: Vec<isize> = nodes.iter().map(|n| n.strong() as isize - 1).collect();
        let mut edges: Vec<Vec<u32>> = Vec::with_capacity(nodes.len());
        let mut busy = alloc::vec![false; nodes.len()];
        let mut buf = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            buf.clear();
            match n {
                Node::O(o) => match o.try_borrow() {
                    Ok(b) => Edges { out: &mut buf }.object(&b),
                    Err(_) => busy[i] = true,
                },
                Node::S(s) => {
                    if s.vars.try_borrow_mut().is_err() {
                        busy[i] = true;
                    }
                    Edges { out: &mut buf }.scope(s)
                }
            }
            let mut list = Vec::with_capacity(buf.len());
            for id in &buf {
                if let Some(&j) = index.get(id) {
                    refs[j] -= 1;
                    list.push(j as u32);
                }
            }
            edges.push(list);
        }
        // roots: referenced from outside the tracked graph (or currently borrowed)
        let mut live = alloc::vec![false; nodes.len()];
        let mut stack: Vec<usize> = (0..nodes.len()).filter(|&i| refs[i] > 0 || busy[i]).collect();
        for &i in &stack {
            live[i] = true;
        }
        while let Some(i) = stack.pop() {
            for &j in &edges[i] {
                let j = j as usize;
                if !live[j] {
                    live[j] = true;
                    stack.push(j);
                }
            }
        }
        // clear garbage; dropped contents are collected after all borrows end
        let mut graveyard: Vec<(Option<Obj>, PropMap, Class)> = Vec::new();
        let mut dead_vars: Vec<VarMap> = Vec::new();
        let mut dead_this: Vec<Option<Value>> = Vec::new();
        let (mut fo, mut fs) = (0, 0);
        for (i, n) in nodes.iter().enumerate() {
            if live[i] {
                continue;
            }
            match n {
                Node::O(o) => {
                    if let Ok(mut b) = o.try_borrow_mut() {
                        let proto = b.proto.take();
                        let props = core::mem::take(&mut b.props);
                        let class = core::mem::replace(&mut b.class, Class::Plain);
                        graveyard.push((proto, props, class));
                        fo += 1;
                    }
                }
                Node::S(s) => {
                    if let Ok(mut v) = s.vars.try_borrow_mut() {
                        dead_vars.push(core::mem::replace(&mut *v, VarMap::new()));
                        fs += 1;
                    }
                    if let Some(f) = &s.func {
                        if let Ok(mut t) = f.this.try_borrow_mut() {
                            dead_this.push(t.take());
                        }
                    }
                }
            }
        }
        drop(graveyard);
        drop(dead_vars);
        drop(dead_this);
        drop(edges);
        let total = nodes.len();
        drop(nodes);
        self.heap.compact();
        self.heap.stats.collections += 1;
        self.heap.stats.freed_objects += fo;
        self.heap.stats.freed_scopes += fs;
        self.heap.stats.live = total - fo - fs;
        fo + fs
    }

    pub fn gc_stats(&self) -> GcStats {
        self.heap.stats.clone()
    }
}
