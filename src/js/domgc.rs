//! Collection of unreachable DOM nodes.
//!
//! The document is an arena (`Vec<Node>`) addressed by `NodeId`. Scripts that rebuild markup
//! (`innerHTML = ...` in a timer, list re-renders) would otherwise grow it without bound. A node
//! is kept when it is in the document, or in the same tree (shadow and template contents
//! included) as a node whose JS wrapper is referenced from somewhere other than the wrapper
//! cache. Everything else becomes a tombstone whose id `Document::push` reuses.
use super::interp::Interp;
use crate::dom::{Node, NodeId, NodeKind};
use alloc::{collections::BTreeMap, vec, vec::Vec};

/// New nodes since the last collection before another one is considered.
const MIN_GROWTH: usize = 20_000;

impl Interp {
    /// Collect garbage DOM nodes when the document grew enough. Only at task boundaries: no JS
    /// frame may hold a bare `NodeId` while nodes are reused.
    pub fn maybe_collect_dom(&mut self) {
        if self.depth != 0 || self.vm_frames != 0 {
            return;
        }
        self.collect_dom_if_grown();
    }

    /// Collection point inside a task, for natives called straight from script code (one native
    /// level): JS frames keep their nodes alive through wrappers, and the caller's own node is
    /// held by its `this` wrapper.
    pub fn maybe_collect_dom_in_native(&mut self) {
        if self.depth > 1 {
            return;
        }
        self.collect_dom_if_grown();
    }

    fn collect_dom_if_grown(&mut self) {
        let Ok(doc) = self.dom() else { return };
        let live = doc.nodes.len() - doc.free.len();
        if live < self.dom_gc_next {
            return;
        }
        let kept = self.collect_dom();
        self.dom_gc_next = (kept * 2).max(kept + MIN_GROWTH);
    }

    /// Run a DOM collection; returns the number of live nodes.
    pub fn collect_dom(&mut self) -> usize {
        let Ok(doc) = self.dom() else { return 0 };
        let len = doc.nodes.len();
        let mut free = vec![false; len];
        for &f in &doc.free {
            if f < len {
                free[f] = true;
            }
        }
        // links that make a parentless node part of a larger tree
        let mut owner: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        for (&host, &root) in &doc.shadow_roots {
            owner.insert(root, host);
        }
        for (&tpl, &content) in &self.template_content {
            owner.insert(content, tpl);
        }
        let mut roots: Vec<NodeId> = vec![doc.root];
        for (&id, o) in &self.node_cache {
            // the cache itself holds one reference
            if alloc::rc::Rc::strong_count(&o.0) > 1 {
                roots.push(id);
            }
        }
        roots.extend(self.current_script);
        roots.extend(self.pinned_nodes.iter().copied());
        roots.extend(self.pending_scripts.iter().copied());
        roots.extend(self.upgrade_target);
        roots.extend(self.dom.focused);
        for o in &self.dom.observers {
            // observers are reachable from JS through their own object; their targets stay
            if alloc::rc::Rc::strong_count(&o.object.0) > 1 {
                roots.extend(o.targets.iter().map(|t| t.0));
            }
        }
        let mut marked = vec![false; len];
        let mut stack: Vec<NodeId> = Vec::new();
        for r in roots {
            if r >= len || free[r] || marked[r] {
                continue;
            }
            // climb to the top of this tree (through shadow hosts and templates)
            let mut top = r;
            let mut hops = 0;
            loop {
                hops += 1;
                if hops > len + 8 {
                    break;
                }
                match doc.nodes[top].parent {
                    Some(p) if p < len => top = p,
                    _ => match owner.get(&top) {
                        Some(&o) if o < len => top = o,
                        _ => break,
                    },
                }
            }
            stack.push(top);
            while let Some(n) = stack.pop() {
                if n >= len || marked[n] {
                    continue;
                }
                marked[n] = true;
                stack.extend(doc.nodes[n].children.iter().copied());
                if let Some(&sr) = doc.shadow_roots.get(&n) {
                    stack.push(sr);
                }
                if let Some(&tc) = self.template_content.get(&n) {
                    stack.push(tc);
                }
            }
        }
        let mut freed: Vec<NodeId> = Vec::new();
        for id in 0..len {
            if !marked[id] && !free[id] {
                freed.push(id);
            }
        }
        if freed.is_empty() {
            return marked.iter().filter(|m| **m).count();
        }
        let is_freed = |id: &NodeId| *id < len && !marked[*id] && !free[*id];
        for &id in &freed {
            doc.nodes[id] = Node { kind: NodeKind::Comment(alloc::string::String::new()), parent: None, children: Vec::new() };
        }
        doc.free.extend(freed.iter().copied());
        doc.shadow_roots.retain(|h, r| !is_freed(h) && !is_freed(r));
        doc.adopted_css.retain(|k, _| !is_freed(k));
        doc.sheet_override.retain(|k, _| !is_freed(k));
        // runtime tables keyed by node
        self.node_cache.retain(|k, _| !is_freed(k));
        self.template_content.retain(|k, v| !is_freed(k) && !is_freed(v));
        self.executed_scripts.retain(|k| !is_freed(k));
        self.loaded_links.retain(|k| !is_freed(k));
        self.shadow_closed.retain(|k| !is_freed(k));
        self.upgraded.retain(|k| !is_freed(k));
        self.dom.listeners.retain(|l| !is_freed(&l.key));
        for o in &mut self.dom.observers {
            o.targets.retain(|t| !is_freed(&t.0));
        }
        self.dom.rects.retain(|k, _| !is_freed(k));
        self.dom.computed.retain(|k, _| !is_freed(k));
        self.dom.style_cache = None;
        self.dom.version += 1;
        len - doc.free.len()
    }
}
