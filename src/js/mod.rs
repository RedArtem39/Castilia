//! ECMAScript engine: lexer, parser, tree-walking interpreter and the DOM bindings.
pub mod ast;
pub mod builtins;
pub mod dom;
pub mod dom_extra;
pub mod dom_setup;
pub mod events;
pub mod gc;
pub mod window;
pub mod interp;
#[cfg(feature = "jit")]
pub mod jit;
pub mod lexer;
pub mod modules;
pub mod ops;
pub mod parser;
pub mod regex;
pub mod url;
pub mod value;
pub mod vm;

use crate::dom::{Document, NodeId};
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};

pub use dom::{Host, HostResponse};

/// Property value for synthesized events.
pub enum Prop<'a> {
    Num(f64),
    Str(&'a str),
    Bool(bool),
}

/// A script runtime bound to one page.
pub struct Runtime {
    pub interp: interp::Interp,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self::with_config(&crate::config::EngineConfig::default())
    }

    pub fn with_config(config: &crate::config::EngineConfig) -> Self {
        let mut rt = Runtime { interp: builtins::new_interp() };
        rt.configure(config);
        rt
    }

    /// Apply execution mode and limits (may be called again before scripts run).
    pub fn configure(&mut self, config: &crate::config::EngineConfig) {
        let it = &mut self.interp;
        it.tier = config.tier();
        it.max_depth = config.max_native_depth;
        it.tick_limit = config.task_budget;
        it.max_frames = config.max_frames;
        it.heap.threshold = config.gc_threshold;
        #[cfg(feature = "jit")]
        {
            it.jit.threshold = config.jit_threshold;
        }
    }

    /// Executable memory for the native tier; without it `ExecMode::Jit` runs interpreted.
    pub fn set_code_memory(&mut self, mem: Box<dyn crate::platform::CodeMemory>) {
        #[cfg(feature = "jit")]
        {
            self.interp.jit.memory = Some(mem);
        }
        #[cfg(not(feature = "jit"))]
        drop(mem);
    }

    /// Run the cycle collector now; returns freed objects + scopes.
    pub fn collect_garbage(&mut self) -> usize {
        self.interp.gc()
    }

    pub fn gc_stats(&self) -> gc::GcStats {
        self.interp.gc_stats()
    }

    /// Bytecode/native tier counters.
    pub fn vm_stats(&self) -> vm::VmStats {
        self.interp.vm_stats.clone()
    }

    fn with<R>(&mut self, doc: &mut Document, f: impl FnOnce(&mut interp::Interp) -> R) -> R {
        self.interp.doc = doc as *mut Document;
        let r = f(&mut self.interp);
        self.interp.maybe_gc();
        self.interp.doc = core::ptr::null_mut();
        r
    }

    pub fn set_host(&mut self, host: Box<dyn Host>) {
        self.interp.host.host = Some(host);
    }

    pub fn set_url(&mut self, url: &str) {
        self.interp.dom.base_url = String::from(url);
        self.interp.dom.history = alloc::vec![String::from(url)];
        self.interp.dom.history_pos = 0;
    }

    pub fn url(&self) -> &str {
        &self.interp.dom.base_url
    }

    pub fn clear_host(&mut self) {
        self.interp.host.host = None;
    }

    pub fn set_document_base(&mut self, base: &str) {
        self.interp.dom.doc_base = Some(String::from(base));
    }

    /// Stylesheets for `<link>` elements inserted by scripts since the last call.
    pub fn take_new_sheets(&mut self) -> Vec<(NodeId, String)> {
        core::mem::take(&mut self.interp.dom.new_sheets)
    }

    pub fn set_viewport(&mut self, w: f32, h: f32) {
        self.interp.dom.viewport = (w, h);
    }

    pub fn set_layout(&mut self, rects: BTreeMap<usize, [f32; 4]>) {
        self.interp.dom.rects = rects;
    }

    pub fn set_computed(&mut self, computed: BTreeMap<usize, Vec<(String, String)>>) {
        self.interp.dom.computed = computed;
    }

    /// Run a classic script. `node` is the `<script>` element (for `document.currentScript`).
    pub fn run_script(&mut self, source: &str, node: Option<NodeId>, document: &mut Document) -> Result<(), String> {
        self.with(document, |it| {
            it.ticks = 0;
            let prev = it.current_script;
            it.current_script = node;
            if let Some(n) = node {
                it.executed_scripts.push(n);
            }
            let r = it.eval_script(source);
            it.current_script = prev;
            it.run_microtasks();
            r
        })
    }

    /// Run an ES module entry point (dependencies are fetched through the host or preloaded).
    pub fn run_module(&mut self, url: &str, source: Option<&str>, document: &mut Document) -> Result<(), String> {
        self.with(document, |it| {
            it.ticks = 0;
            let r = match source {
                Some(src) => {
                    it.module_sources.insert(String::from(url), String::from(src));
                    it.load_module(url).map(|_| ())
                }
                None => it.load_module(url).map(|_| ()),
            };
            it.run_microtasks();
            match r {
                Ok(()) => Ok(()),
                Err(value::Ctl::Throw(e)) => Err(it.error_text(&e)),
                Err(_) => Ok(()),
            }
        })
    }

    pub fn add_import_map(&mut self, json: &str, document: &mut Document) {
        self.with(document, |it| {
            let code = alloc::format!("__setImportMap(JSON.parse({}))", {
                let mut q = String::new();
                builtins::json::quote(json, &mut q);
                q
            });
            let _ = it.eval_script(&code);
        });
    }

    /// Make a module's source available without fetching it.
    pub fn preload_module(&mut self, url: &str, source: &str) {
        self.interp.module_sources.insert(String::from(url), String::from(source));
    }

    pub fn execute(&mut self, source: &str, document: &mut Document) -> Result<(), String> {
        self.run_script(source, None, document)
    }

    /// Dispatch a synthesized event; returns how many listeners ran.
    pub fn dispatch_event(&mut self, document: &mut Document, target: NodeId, ty: &str, class: &str, bubbles: bool, cancelable: bool, props: &[(&str, Prop)]) -> Result<usize, String> {
        self.with(document, |it| {
            it.ticks = 0;
            let before = it.dispatch_count;
            let mut vals = Vec::new();
            for (k, p) in props {
                let v = match p {
                    Prop::Num(n) => value::Value::Num(*n),
                    Prop::Str(s) => value::Value::str(s),
                    Prop::Bool(b) => value::Value::Bool(*b),
                };
                vals.push((*k, v));
            }
            let ev = it.make_event(ty, class, bubbles, cancelable, &vals);
            let t = it.wrap(target);
            let result = match it.dispatch_event_obj(&t, &ev) {
                Ok(not_prevented) => {
                    it.last_prevented = !not_prevented;
                    if not_prevented && ty == "click" {
                        it.run_default_action(target).map_err(|_| String::from("default action failed"))?;
                    }
                    Ok(it.dispatch_count - before)
                }
                Err(value::Ctl::Throw(e)) => Err(it.error_text(&e)),
                Err(_) => Ok(0),
            };
            it.run_microtasks();
            result
        })
    }

    /// Run timers and microtasks until `ms` of virtual time have elapsed.
    pub fn advance(&mut self, document: &mut Document, ms: f64, max_fires: usize) {
        self.with(document, |it| {
            it.ticks = 0;
            let until = it.now + ms;
            it.run_timers_until(until, max_fires);
        });
    }

    pub fn has_pending_timers(&self) -> bool {
        !self.interp.timers.is_empty()
    }

    fn fire(it: &mut interp::Interp, target: value::Value, ty: &str, bubbles: bool) {
        let ev = it.make_event(ty, "Event", bubbles, false, &[]);
        let _ = it.dispatch_event_obj(&target, &ev);
    }

    pub fn dom_content_loaded(&mut self, document: &mut Document) {
        self.with(document, |it| {
            it.dom.ready_state = "interactive";
            let _ = it.fire_ready_state_change();
            let root = it.dom().map(|d| d.root).unwrap_or(0);
            let d = it.wrap(root);
            Self::fire(it, d, "DOMContentLoaded", true);
            it.run_microtasks();
        });
    }

    pub fn load(&mut self, document: &mut Document) {
        self.with(document, |it| {
            it.dom.ready_state = "complete";
            let _ = it.fire_ready_state_change();
            let g = value::Value::Obj(it.global.clone());
            Self::fire(it, g.clone(), "load", false);
            Self::fire(it, g, "pageshow", false);
            it.run_microtasks();
        });
    }

    pub fn last_event_prevented(&self) -> bool {
        self.interp.last_prevented
    }

    /// Insert text at the caret of an input/textarea/contenteditable (typing).
    pub fn insert_text(&mut self, document: &mut Document, target: NodeId, text: &str) {
        self.with(document, |it| {
            let _ = it.input_insert(target, text);
            it.run_microtasks();
        });
    }

    /// Default action of a key press that was not cancelled.
    pub fn key_default(&mut self, document: &mut Document, target: NodeId, key: &str) {
        self.with(document, |it| {
            let _ = it.key_default(target, key);
            it.run_microtasks();
        });
    }

    pub fn take_navigation(&mut self) -> Option<String> {
        self.interp.dom.navigate_to.take()
    }

    pub fn take_dirty(&mut self) -> bool {
        core::mem::take(&mut self.interp.dom.dirty)
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.interp.dom.focused
    }

    pub fn focus(&mut self, document: &mut Document, node: Option<NodeId>) {
        self.with(document, |it| {
            match node {
                Some(n) => {
                    let _ = it.focus_node(n);
                }
                None => {
                    if let Some(f) = it.dom.focused {
                        let _ = it.blur_node(f);
                    }
                }
            }
            it.run_microtasks();
        });
    }

    pub fn logs(&self) -> Vec<String> {
        self.interp.logs.iter().map(|l| alloc::format!("{}: {}", l.level, l.text)).collect()
    }

    pub fn clear_logs(&mut self) {
        self.interp.logs.clear();
    }

    pub fn eval_in_page(&mut self, document: &mut Document, source: &str) -> Result<String, String> {
        self.with(document, |it| {
            let prog = parser::parse_program(source, &mut it.interner, false).map_err(|e| alloc::format!("SyntaxError: {e}"))?;
            match it.run_program(&prog) {
                Ok(v) => {
                    it.run_microtasks();
                    Ok(it.to_string_lossy(&v))
                }
                Err(value::Ctl::Throw(v)) => Err(it.error_text(&v)),
                Err(_) => Ok(String::new()),
            }
        })
    }

    /// Fill the `NodeId -> [x, y, w, h]` table from hit regions.
    pub fn rects_from_hits(document: &Document, hits: &[crate::layout::HitRegion]) -> BTreeMap<usize, [f32; 4]> {
        let mut r: BTreeMap<usize, [f32; 4]> = BTreeMap::new();
        let mut merge = |id: usize, x0: f32, y0: f32, x1: f32, y1: f32, r: &mut BTreeMap<usize, [f32; 4]>| {
            let e = r.entry(id).or_insert([x0, y0, x1 - x0, y1 - y0]);
            let (ex1, ey1) = (e[0] + e[2], e[1] + e[3]);
            let nx0 = e[0].min(x0);
            let ny0 = e[1].min(y0);
            *e = [nx0, ny0, ex1.max(x1) - nx0, ey1.max(y1) - ny0];
        };
        for h in hits {
            // block boxes replace, inline fragments accumulate (both handled by union)
            merge(h.node, h.rect.x, h.rect.y, h.rect.x + h.rect.width, h.rect.y + h.rect.height, &mut r);
        }
        // propagate to ancestors lacking their own region
        let ids: Vec<usize> = r.keys().copied().collect();
        for id in ids {
            let b = r[&id];
            let mut cur = document.nodes.get(id).and_then(|n| n.parent);
            let mut depth = 0;
            while let Some(p) = cur {
                if depth > 64 {
                    break;
                }
                depth += 1;
                if !hits.iter().any(|h| h.node == p) || !r.contains_key(&p) {
                    merge(p, b[0], b[1], b[0] + b[2], b[1] + b[3], &mut r);
                } else {
                    break;
                }
                cur = document.nodes[p].parent;
            }
        }
        r
    }
}

impl interp::Interp {
    /// Parse and run a classic script in the global scope.
    pub fn eval_script(&mut self, source: &str) -> Result<(), String> {
        let prog = parser::parse_program(source, &mut self.interner, false).map_err(|e| alloc::format!("SyntaxError: {e}"))?;
        match self.run_program(&prog) {
            Ok(_) => Ok(()),
            Err(value::Ctl::Throw(v)) => Err(self.error_text(&v)),
            Err(_) => Ok(()),
        }
    }

    pub fn new_regexp(&mut self, pattern: &str, flags: &str) -> value::Res {
        builtins::string::new_regexp(self, pattern, flags)
    }
}

impl core::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Runtime")
    }
}

#[cfg(test)]
mod tests;
