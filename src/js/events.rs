//! Events: Event classes, EventTarget, dispatch with capture/bubble, MutationObserver plumbing.
use super::{builtins::*, dom::*, interp::*, value::*};
use crate::dom::{NodeId, NodeKind};
use alloc::{
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

/// Event classes: (constructor name, default init properties).
const EVENT_CLASSES: &[(&str, &[(&str, f64)])] = &[
    ("Event", &[]),
    ("UIEvent", &[("detail", 0.0)]),
    ("MouseEvent", &[("detail", 0.0), ("clientX", 0.0), ("clientY", 0.0), ("screenX", 0.0), ("screenY", 0.0), ("pageX", 0.0), ("pageY", 0.0), ("offsetX", 0.0), ("offsetY", 0.0), ("button", 0.0), ("buttons", 0.0)]),
    ("PointerEvent", &[("detail", 0.0), ("clientX", 0.0), ("clientY", 0.0), ("pointerId", 1.0), ("button", 0.0), ("buttons", 0.0)]),
    ("WheelEvent", &[("deltaX", 0.0), ("deltaY", 0.0), ("deltaZ", 0.0), ("deltaMode", 0.0), ("clientX", 0.0), ("clientY", 0.0)]),
    ("KeyboardEvent", &[("keyCode", 0.0), ("which", 0.0), ("charCode", 0.0), ("location", 0.0)]),
    ("InputEvent", &[]),
    ("FocusEvent", &[]),
    ("TouchEvent", &[]),
    ("DragEvent", &[]),
    ("ClipboardEvent", &[]),
    ("CustomEvent", &[]),
    ("ErrorEvent", &[("lineno", 0.0), ("colno", 0.0)]),
    ("MessageEvent", &[]),
    ("HashChangeEvent", &[]),
    ("PopStateEvent", &[]),
    ("StorageEvent", &[]),
    ("TransitionEvent", &[("elapsedTime", 0.0)]),
    ("AnimationEvent", &[("elapsedTime", 0.0)]),
    ("ProgressEvent", &[("loaded", 0.0), ("total", 0.0)]),
    ("SubmitEvent", &[]),
    ("PageTransitionEvent", &[]),
    ("CompositionEvent", &[]),
    ("PromiseRejectionEvent", &[]),
];

const BOOL_DEFAULTS: &[&str] = &["bubbles", "cancelable", "composed", "ctrlKey", "shiftKey", "altKey", "metaKey", "repeat", "isComposing", "lengthComputable"];

pub fn setup(it: &mut Interp) {
    let ep = it.protos().event.clone();
    it.def_method(&ep, "preventDefault", 0, |it, c| {
        if let Value::Obj(o) = &c.this {
            if it.get_str(&c.this, "cancelable")?.truthy() {
                it.define_data(o, Key::str("defaultPrevented"), Value::Bool(true), HIDDEN);
            }
        }
        Ok(Value::Undefined)
    });
    it.def_method(&ep, "stopPropagation", 0, |it, c| {
        if let Value::Obj(o) = &c.this {
            it.define_data(o, Key::str("__stop"), Value::Bool(true), HIDDEN);
            it.define_data(o, Key::str("cancelBubble"), Value::Bool(true), HIDDEN);
        }
        Ok(Value::Undefined)
    });
    it.def_method(&ep, "stopImmediatePropagation", 0, |it, c| {
        if let Value::Obj(o) = &c.this {
            it.define_data(o, Key::str("__stop"), Value::Bool(true), HIDDEN);
            it.define_data(o, Key::str("__stopi"), Value::Bool(true), HIDDEN);
        }
        Ok(Value::Undefined)
    });
    it.def_method(&ep, "composedPath", 0, |it, c| {
        let p = it.get_str(&c.this, "__path")?;
        Ok(if p.is_undefined() { it.new_array(Vec::new()) } else { p })
    });
    it.def_method(&ep, "initEvent", 3, |it, c| {
        if let Value::Obj(o) = &c.this {
            let t = it.arg_str(c, 0)?;
            it.define_data(o, Key::str("type"), Value::Str(t), HIDDEN);
            it.define_data(o, Key::str("bubbles"), Value::Bool(c.arg(1).truthy()), HIDDEN);
            it.define_data(o, Key::str("cancelable"), Value::Bool(c.arg(2).truthy()), HIDDEN);
        }
        Ok(Value::Undefined)
    });
    for (i, n) in ["NONE", "CAPTURING_PHASE", "AT_TARGET", "BUBBLING_PHASE"].iter().enumerate() {
        it.define_data(&ep, Key::str(n), Value::Num(i as f64), 0);
    }
    // constructors, one prototype per class chained through Event
    let mut base = ep.clone();
    for (name, _defaults) in EVENT_CLASSES {
        let proto = if *name == "Event" { ep.clone() } else { Obj::new(Object::new(Some(ep.clone()), Class::Plain)) };
        if *name == "UIEvent" {
            base = proto.clone();
        }
        let ctor = it.make_ctor(name, 2, event_ctor, &proto);
        it.protos.as_mut().unwrap().extra.insert(name, proto.clone());
        let _ = ctor;
    }
    // MouseEvent/KeyboardEvent... inherit UIEvent
    for name in ["MouseEvent", "KeyboardEvent", "FocusEvent", "InputEvent", "TouchEvent", "CompositionEvent"] {
        let p = it.protos().extra.get(name).cloned().unwrap();
        p.borrow_mut().proto = Some(base.clone());
    }
    for name in ["PointerEvent", "WheelEvent", "DragEvent"] {
        let p = it.protos().extra.get(name).cloned().unwrap();
        let mouse = it.protos().extra.get("MouseEvent").cloned().unwrap();
        p.borrow_mut().proto = Some(mouse);
    }

    // EventTarget
    let et = it.protos().event_target.clone();
    it.make_ctor("EventTarget", 0, |it, c| {
        let proto = it.protos().event_target.clone();
        let p = it.proto_from_call(c, &proto)?;
        Ok(Value::Obj(Obj::new(Object::new(Some(p), Class::Plain))))
    }, &et);
    for target in [et.clone(), it.global.clone()] {
        it.def_method(&target, "addEventListener", 2, add_listener);
        it.def_method(&target, "removeEventListener", 2, remove_listener);
        it.def_method(&target, "dispatchEvent", 1, |it, c| {
            let Value::Obj(ev) = c.arg(0) else { return it.type_error("parameter 1 is not of type 'Event'") };
            let r = it.dispatch_event_obj(&c.this, &ev)?;
            Ok(Value::Bool(r))
        });
    }
    setup_mutation_observer(it);
}

fn event_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_none() {
        return it.type_error("Failed to construct 'Event': Please use the 'new' operator");
    }
    let name = match it.get_str(&Value::Obj(c.callee.clone()), "name")? {
        Value::Str(s) => s.to_string(),
        _ => "Event".to_string(),
    };
    let ty = it.arg_str(c, 0)?;
    let default = it.protos().extra.get(name.as_str()).cloned().unwrap_or_else(|| it.protos().event.clone());
    let proto = it.proto_from_call(c, &default)?;
    let ev = Obj::new(Object::new(Some(proto), Class::Plain));
    let ts = it.now + 0.0;
    it.init_event(&ev, &ty, &name, ts);
    if let Value::Obj(init) = c.arg(1) {
        for k in it.own_keys(&init)? {
            if matches!(k, Key::S(_)) && it.own_prop(&init, &k).is_some_and(|p| p.flags & ENUMERABLE != 0) {
                let v = it.get_obj(&init, &k, &Value::Obj(init.clone()))?;
                it.define_data(&ev, k, v, ALL);
            }
        }
    }
    Ok(Value::Obj(ev))
}

impl Interp {
    pub fn init_event(&mut self, ev: &Obj, ty: &str, class: &str, ts: f64) {
        for (k, v) in [("type", Value::str(ty)), ("target", Value::Null), ("currentTarget", Value::Null), ("eventPhase", Value::Num(0.0)), ("defaultPrevented", Value::Bool(false)), ("isTrusted", Value::Bool(false)), ("timeStamp", Value::Num(ts)), ("cancelBubble", Value::Bool(false)), ("srcElement", Value::Null), ("returnValue", Value::Bool(true))] {
            self.define_data(ev, Key::str(k), v, ALL);
        }
        for b in BOOL_DEFAULTS {
            self.define_data(ev, Key::str(b), Value::Bool(false), ALL);
        }
        if let Some((_, defaults)) = EVENT_CLASSES.iter().find(|(n, _)| *n == class) {
            for (k, v) in defaults.iter() {
                self.define_data(ev, Key::str(k), Value::Num(*v), ALL);
            }
        }
        match class {
            "CustomEvent" => self.define_data(ev, Key::str("detail"), Value::Null, ALL),
            "MessageEvent" | "InputEvent" | "ErrorEvent" | "PopStateEvent" | "ClipboardEvent" | "PromiseRejectionEvent" => {
                for k in ["data", "origin", "lastEventId", "source", "state", "inputType", "message", "error", "reason", "promise", "clipboardData"] {
                    self.define_data(ev, Key::str(k), if k == "inputType" || k == "origin" || k == "lastEventId" { Value::str("") } else { Value::Null }, ALL);
                }
            }
            "KeyboardEvent" => {
                self.define_data(ev, Key::str("key"), Value::str(""), ALL);
                self.define_data(ev, Key::str("code"), Value::str(""), ALL);
            }
            "FocusEvent" | "MouseEvent" | "PointerEvent" => self.define_data(ev, Key::str("relatedTarget"), Value::Null, ALL),
            _ => {}
        }
    }

    pub fn make_event(&mut self, ty: &str, class: &str, bubbles: bool, cancelable: bool, props: &[(&str, Value)]) -> Obj {
        let proto = self.protos().extra.get(class).cloned().unwrap_or_else(|| self.protos().event.clone());
        let ev = Obj::new(Object::new(Some(proto), Class::Plain));
        let ts = self.now;
        self.init_event(&ev, ty, class, ts);
        self.define_data(&ev, Key::str("bubbles"), Value::Bool(bubbles), ALL);
        self.define_data(&ev, Key::str("cancelable"), Value::Bool(cancelable), ALL);
        self.define_data(&ev, Key::str("isTrusted"), Value::Bool(true), ALL);
        for (k, v) in props {
            self.define_data(&ev, Key::str(k), v.clone(), ALL);
        }
        ev
    }

    /// Identity of an event target for the listener table.
    fn listener_key(&mut self, v: &Value) -> Option<usize> {
        match v {
            Value::Obj(o) => match o.borrow().class {
                Class::Node(id) => Some(id),
                _ => Some(o.id()),
            },
            _ => None,
        }
    }

    pub fn fire_ready_state_change(&mut self) -> Res<()> {
        let root = self.dom()?.root;
        let t = self.wrap(root);
        let ev = self.make_event("readystatechange", "Event", false, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        Ok(())
    }

    pub fn fire_node_event(&mut self, id: NodeId, ty: &str) -> Res<()> {
        let ev = self.make_event(ty, "Event", false, false, &[]);
        let t = self.wrap(id);
        self.dispatch_event_obj(&t, &ev)?;
        Ok(())
    }

    /// Dispatch `ev` at `target` through capture → target → bubble phases. Returns `!defaultPrevented`.
    pub fn dispatch_event_obj(&mut self, target: &Value, ev: &Obj) -> Res<bool> {
        let ev_v = Value::Obj(ev.clone());
        let ty: Rc<str> = match self.get_str(&ev_v, "type")? {
            Value::Str(s) => s,
            v => self.to_str(&v)?,
        };
        self.define_data(ev, Key::str("target"), target.clone(), ALL);
        self.define_data(ev, Key::str("srcElement"), target.clone(), ALL);
        let bubbles = self.get_str(&ev_v, "bubbles")?.truthy();
        // path: (key, value) from target upwards
        let mut path: Vec<(usize, Value)> = Vec::new();
        let tkey = self.listener_key(target).unwrap_or(0);
        path.push((tkey, target.clone()));
        let is_node = matches!(target, Value::Obj(o) if matches!(o.borrow().class, Class::Node(_)));
        if is_node {
            let mut cur = self.node_id(target)?;
            loop {
                let next = match self.dom()?.nodes[cur].parent {
                    Some(p) => Some(p),
                    // a shadow root continues the path at its host
                    None => crate::shadow::host_of(self.dom()?, cur).filter(|&h| h != cur),
                };
                let Some(p) = next else { break };
                let w = self.wrap(p);
                path.push((p, w));
                cur = p;
            }
            if self.dom()?.nodes[cur].kind == NodeKind::Document {
                let g = Value::Obj(self.global.clone());
                path.push((self.global.id(), g));
            }
        }
        let composed: Vec<Value> = path.iter().map(|p| p.1.clone()).collect();
        let arr = self.new_array(composed);
        self.define_data(ev, Key::str("__path"), arr, 0);
        let cancel = |it: &mut Interp| -> bool { it.get_str(&Value::Obj(ev.clone()), "__stop").map(|v| v.truthy()).unwrap_or(false) };
        // capture
        for (key, val) in path.iter().skip(1).rev() {
            self.define_data(ev, Key::str("eventPhase"), Value::Num(1.0), ALL);
            self.invoke_listeners(*key, val, ev, &ty, Some(true))?;
            if cancel(self) {
                break;
            }
        }
        if !cancel(self) {
            self.define_data(ev, Key::str("eventPhase"), Value::Num(2.0), ALL);
            self.invoke_listeners(path[0].0, &path[0].1, ev, &ty, Some(true))?;
            if !self.get_str(&ev_v, "__stopi")?.truthy() {
                self.invoke_handler_property(&path[0].1, ev, &ty)?;
                self.invoke_listeners(path[0].0, &path[0].1, ev, &ty, Some(false))?;
            }
        }
        if bubbles {
            for (key, val) in path.iter().skip(1) {
                if cancel(self) {
                    break;
                }
                self.define_data(ev, Key::str("eventPhase"), Value::Num(3.0), ALL);
                self.invoke_handler_property(val, ev, &ty)?;
                if self.get_str(&ev_v, "__stopi")?.truthy() {
                    continue;
                }
                self.invoke_listeners(*key, val, ev, &ty, Some(false))?;
            }
        }
        self.define_data(ev, Key::str("eventPhase"), Value::Num(0.0), ALL);
        self.define_data(ev, Key::str("currentTarget"), Value::Null, ALL);
        let prevented = self.get_str(&ev_v, "defaultPrevented")?.truthy();
        Ok(!prevented)
    }

    fn invoke_listeners(&mut self, key: usize, current: &Value, ev: &Obj, ty: &str, capture: Option<bool>) -> Res<()> {
        let snapshot: Vec<(u32, Value, bool, bool)> = self
            .dom
            .listeners
            .iter()
            .filter(|l| l.key == key && !l.removed && &*l.ty == ty && capture.is_none_or(|c| l.capture == c))
            .map(|l| (l.id, l.cb.clone(), l.once, l.passive))
            .collect();
        let ev_v = Value::Obj(ev.clone());
        for (id, cb, once, _passive) in snapshot {
            if !self.dom.listeners.iter().any(|l| l.id == id) {
                continue;
            }
            if once {
                self.dom.listeners.retain(|l| l.id != id);
            }
            self.define_data(ev, Key::str("currentTarget"), current.clone(), ALL);
            self.dispatch_count += 1;
            let r = if cb.as_obj().is_some_and(|o| o.is_callable()) {
                self.call(&cb, current.clone(), &[ev_v.clone()])
            } else if let Value::Obj(_) = &cb {
                let h = self.get_str(&cb, "handleEvent")?;
                if h.as_obj().is_some_and(|o| o.is_callable()) { self.call(&h, cb.clone(), &[ev_v.clone()]) } else { Ok(Value::Undefined) }
            } else {
                Ok(Value::Undefined)
            };
            match r {
                Ok(_) => {}
                Err(Ctl::Throw(e)) => self.report_error(&e),
                Err(e) => return Err(e),
            }
            if self.get_str(&ev_v, "__stopi")?.truthy() {
                break;
            }
        }
        Ok(())
    }

    /// `onclick = fn` properties and inline `onclick="..."` attributes.
    fn invoke_handler_property(&mut self, current: &Value, ev: &Obj, ty: &str) -> Res<()> {
        let prop = format!("on{ty}");
        let mut handler = if let Value::Obj(o) = current { self.get_obj(o, &Key::S(Rc::from(prop.as_str())), current)? } else { Value::Undefined };
        if handler.is_nullish() {
            if let Value::Obj(o) = current {
                let nid = if let Class::Node(id) = o.borrow().class { Some(id) } else { None };
                if let Some(id) = nid {
                    if let Some(src) = self.attr(id, &prop) {
                        let def = super::parser::parse_function_source("event", &src, &mut self.interner);
                        if let Ok(def) = def {
                            let env = self.global_env.clone();
                            let f = self.make_closure(def, &env, None, Some(Rc::from(prop.as_str())));
                            handler = Value::Obj(f);
                            self.define_data(o, Key::S(Rc::from(prop.as_str())), handler.clone(), ALL);
                        }
                    }
                }
            }
        }
        if handler.as_obj().is_some_and(|o| o.is_callable()) {
            let ev_v = Value::Obj(ev.clone());
            self.define_data(ev, Key::str("currentTarget"), current.clone(), ALL);
            self.dispatch_count += 1;
            match self.call(&handler, current.clone(), &[ev_v]) {
                Ok(Value::Bool(false)) => {
                    if self.get_str(&Value::Obj(ev.clone()), "cancelable")?.truthy() {
                        self.define_data(ev, Key::str("defaultPrevented"), Value::Bool(true), ALL);
                    }
                }
                Ok(_) => {}
                Err(Ctl::Throw(e)) => self.report_error(&e),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Mutation records
    // -----------------------------------------------------------------------

    fn observers_for(&mut self, node: NodeId, kind: u8, attr: Option<&str>) -> Vec<usize> {
        // kind: 0 childList, 1 attributes, 2 characterData
        let mut out = Vec::new();
        if self.dom.observers.is_empty() {
            return out;
        }
        let Ok(doc) = self.dom() else { return out };
        for (oi, o) in self.dom.observers.iter().enumerate() {
            for (target, child, attrs, chars, subtree, filter) in &o.targets {
                let applies = match kind {
                    0 => *child,
                    1 => *attrs && (filter.is_empty() || attr.is_some_and(|a| filter.iter().any(|f| f == a))),
                    _ => *chars,
                };
                if !applies {
                    continue;
                }
                let mut hit = *target == node;
                if !hit && *subtree {
                    let mut cur = doc.nodes[node].parent;
                    while let Some(p) = cur {
                        if p == *target {
                            hit = true;
                            break;
                        }
                        cur = doc.nodes[p].parent;
                    }
                }
                if hit {
                    out.push(oi);
                    break;
                }
            }
        }
        out
    }

    fn push_record(&mut self, observers: Vec<usize>, rec: Value) {
        for oi in observers {
            let o = &mut self.dom.observers[oi];
            o.queue.push(rec.clone());
            if !o.scheduled {
                o.scheduled = true;
                let id = o.id;
                let f = self.new_native_data("", 0, deliver_records, Value::Num(id as f64), false);
                self.microtasks.push_back(Job::Call(Value::Obj(f), Value::Undefined, Vec::new()));
            }
        }
    }

    pub fn note_children(&mut self, parent: NodeId, added: Vec<NodeId>, removed: Vec<NodeId>) {
        let obs = self.observers_for(parent, 0, None);
        if obs.is_empty() {
            return;
        }
        let rec = self.new_object();
        let t = self.wrap(parent);
        let a = self.node_list(&added);
        let r = self.node_list(&removed);
        for (k, v) in [("type", Value::str("childList")), ("target", t), ("addedNodes", a), ("removedNodes", r), ("previousSibling", Value::Null), ("nextSibling", Value::Null), ("attributeName", Value::Null), ("oldValue", Value::Null)] {
            self.define_data(&rec, Key::str(k), v, ALL);
        }
        self.push_record(obs, Value::Obj(rec));
    }

    pub fn note_attr(&mut self, id: NodeId, name: &str, old: Option<String>) {
        let obs = self.observers_for(id, 1, Some(name));
        if obs.is_empty() {
            return;
        }
        let rec = self.new_object();
        let t = self.wrap(id);
        let empty = self.new_node_list_empty();
        for (k, v) in [
            ("type", Value::str("attributes")),
            ("target", t),
            ("attributeName", Value::str(name)),
            ("oldValue", old.map_or(Value::Null, Value::string)),
            ("addedNodes", empty.clone()),
            ("removedNodes", empty),
        ] {
            self.define_data(&rec, Key::str(k), v, ALL);
        }
        self.push_record(obs, Value::Obj(rec));
    }

    pub fn note_char(&mut self, id: NodeId) {
        let obs = self.observers_for(id, 2, None);
        if obs.is_empty() {
            return;
        }
        let rec = self.new_object();
        let t = self.wrap(id);
        for (k, v) in [("type", Value::str("characterData")), ("target", t)] {
            self.define_data(&rec, Key::str(k), v, ALL);
        }
        self.push_record(obs, Value::Obj(rec));
    }

    fn new_node_list_empty(&mut self) -> Value {
        self.node_list(&[])
    }
}

fn deliver_records(it: &mut Interp, c: &Call) -> Res {
    let Value::Num(id) = it.native_data(c) else { return Ok(Value::Undefined) };
    let id = id as u32;
    let Some(pos) = it.dom.observers.iter().position(|o| o.id == id) else { return Ok(Value::Undefined) };
    let (cb, obj, queue) = {
        let o = &mut it.dom.observers[pos];
        o.scheduled = false;
        (o.callback.clone(), o.object.clone(), core::mem::take(&mut o.queue))
    };
    if queue.is_empty() {
        return Ok(Value::Undefined);
    }
    let arr = it.new_array(queue);
    it.call(&cb, Value::Obj(obj.clone()), &[arr, Value::Obj(obj)])?;
    Ok(Value::Undefined)
}

fn setup_mutation_observer(it: &mut Interp) {
    let proto = it.new_object();
    it.make_ctor("MutationObserver", 1, |it, c| {
        if c.new_target.is_none() {
            return it.type_error("Constructor MutationObserver requires 'new'");
        }
        let cb = c.arg(0);
        let proto = it.protos().extra.get("mutation_observer").cloned().unwrap();
        let p = it.proto_from_call(c, &proto)?;
        let o = Obj::new(Object::new(Some(p), Class::Plain));
        it.dom.observer_seq += 1;
        let id = it.dom.observer_seq;
        it.define_data(&o, Key::str("__mid"), Value::Num(id as f64), 0);
        it.dom.observers.push(MutationObs { id, callback: cb, object: o.clone(), targets: Vec::new(), queue: Vec::new(), scheduled: false });
        Ok(Value::Obj(o))
    }, &proto);
    it.def_method(&proto, "observe", 2, |it, c| {
        let id = match it.get_str(&c.this, "__mid")? {
            Value::Num(n) => n as u32,
            _ => return Ok(Value::Undefined),
        };
        let node = it.node_id(&c.arg(0))?;
        let opts = c.arg(1);
        let flag = |it: &mut Interp, k: &str| -> Res<Option<bool>> {
            let v = it.get_str(&opts, k)?;
            Ok(if v.is_undefined() { None } else { Some(v.truthy()) })
        };
        let attr_filter = it.get_str(&opts, "attributeFilter")?;
        let mut filter = Vec::new();
        if !attr_filter.is_undefined() {
            for v in it.array_like_to_vec(&attr_filter)? {
                filter.push(it.to_str(&v)?.to_string());
            }
        }
        let child = flag(it, "childList")?.unwrap_or(false);
        let attrs_flag = flag(it, "attributes")?;
        let attrs = attrs_flag.unwrap_or(false) || (attrs_flag.is_none() && !filter.is_empty()) || flag(it, "attributeOldValue")?.unwrap_or(false);
        let chars = flag(it, "characterData")?.unwrap_or(false);
        let subtree = flag(it, "subtree")?.unwrap_or(false);
        if let Some(o) = it.dom.observers.iter_mut().find(|o| o.id == id) {
            o.targets.retain(|t| t.0 != node);
            o.targets.push((node, child, attrs, chars, subtree, filter));
        }
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "disconnect", 0, |it, c| {
        if let Value::Num(n) = it.get_str(&c.this, "__mid")? {
            if let Some(o) = it.dom.observers.iter_mut().find(|o| o.id == n as u32) {
                o.targets.clear();
                o.queue.clear();
            }
        }
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "takeRecords", 0, |it, c| {
        let mut recs = Vec::new();
        if let Value::Num(n) = it.get_str(&c.this, "__mid")? {
            if let Some(o) = it.dom.observers.iter_mut().find(|o| o.id == n as u32) {
                recs = core::mem::take(&mut o.queue);
            }
        }
        Ok(it.new_array(recs))
    });
    it.protos.as_mut().unwrap().extra.insert("mutation_observer", proto);
}

fn parse_listener_opts(it: &mut Interp, v: &Value) -> Res<(bool, bool, bool, Option<Value>)> {
    match v {
        Value::Obj(_) => {
            let capture = it.get_str(v, "capture")?.truthy();
            let once = it.get_str(v, "once")?.truthy();
            let passive = it.get_str(v, "passive")?.truthy();
            let signal = it.get_str(v, "signal")?;
            Ok((capture, once, passive, if signal.is_nullish() { None } else { Some(signal) }))
        }
        v => Ok((v.truthy(), false, false, None)),
    }
}

fn add_listener(it: &mut Interp, c: &Call) -> Res {
    let Some(key) = it.listener_key(&c.this) else { return Ok(Value::Undefined) };
    let ty = it.arg_str(c, 0)?;
    let cb = c.arg(1);
    if !matches!(cb, Value::Obj(_)) {
        return Ok(Value::Undefined);
    }
    let (capture, once, passive, signal) = parse_listener_opts(it, &c.arg(2))?;
    if it.dom.listeners.iter().any(|l| l.key == key && !l.removed && l.ty == ty && l.capture == capture && strict_equals(&l.cb, &cb)) {
        return Ok(Value::Undefined);
    }
    it.dom.listener_seq += 1;
    let id = it.dom.listener_seq;
    it.dom.listeners.push(Listener { id, key, ty, cb: cb.clone(), capture, once, passive, removed: false });
    if let Some(sig) = signal {
        // remove when the signal aborts
        let add = it.get_str(&sig, "addEventListener")?;
        let remover = it.new_native_data("", 0, |it, c| {
            if let Value::Num(n) = it.native_data(c) {
                let id = n as u32;
                it.dom.listeners.retain(|l| l.id != id);
            }
            Ok(Value::Undefined)
        }, Value::Num(id as f64), false);
        if add.as_obj().is_some() {
            it.call(&add, sig, &[Value::str("abort"), Value::Obj(remover)])?;
        }
    }
    Ok(Value::Undefined)
}

fn remove_listener(it: &mut Interp, c: &Call) -> Res {
    let Some(key) = it.listener_key(&c.this) else { return Ok(Value::Undefined) };
    let ty = it.arg_str(c, 0)?;
    let cb = c.arg(1);
    let (capture, _, _, _) = parse_listener_opts(it, &c.arg(2))?;
    it.dom.listeners.retain(|l| !(l.key == key && l.ty == ty && l.capture == capture && strict_equals(&l.cb, &cb)));
    Ok(Value::Undefined)
}

#[allow(dead_code)]
fn unused(_: String) -> Vec<i32> {
    vec![]
}
