//! DOM bindings: node wrappers, Node/Element/Document APIs, classList/dataset/style.
use super::{builtins::*, interp::*, url::Url, value::*};
use crate::{
    dom::{Document, NodeId, NodeKind},
    html,
    selector::{self, Complex},
};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

// ---------------------------------------------------------------------------
// Host interface and shared state
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct HostSlot {
    pub host: Option<Box<dyn Host>>,
}

pub struct HostResponse {
    pub status: u16,
    pub status_text: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Services the embedder may provide to scripts. Every method has a conservative default so
/// a port only implements what its platform offers.
pub trait Host {
    /// Perform a request synchronously. `None` means the network is unavailable.
    fn fetch(&mut self, _method: &str, _url: &str, _headers: &[(String, String)], _body: Option<&str>) -> Option<HostResponse> {
        None
    }
    fn alert(&mut self, _message: &str) {}
    fn confirm(&mut self, _message: &str) -> bool {
        false
    }
    fn prompt(&mut self, _message: &str, default: &str) -> Option<String> {
        Some(default.to_string())
    }
    /// A script requested navigation (location.href = ..., form submit, window.open).
    fn navigate(&mut self, _url: &str) {}
}

pub struct Listener {
    pub id: u32,
    pub key: usize,
    pub ty: Rc<str>,
    pub cb: Value,
    pub capture: bool,
    pub once: bool,
    pub passive: bool,
    pub removed: bool,
}

pub struct MutationObs {
    pub id: u32,
    pub callback: Value,
    pub object: Obj,
    pub targets: Vec<(NodeId, bool, bool, bool, bool, Vec<String>)>, // node, childList, attributes, characterData, subtree, filter
    pub queue: Vec<Value>,
    pub scheduled: bool,
}

pub struct DomState {
    pub base_url: String,
    /// Wall-clock origin (ms since the Unix epoch) corresponding to virtual time 0.
    pub epoch_ms: f64,
    pub listeners: Vec<Listener>,
    pub listener_seq: u32,
    pub selector_cache: BTreeMap<String, Rc<Vec<Complex>>>,
    pub cookie: String,
    pub rects: BTreeMap<usize, [f32; 4]>,
    pub viewport: (f32, f32),
    pub scroll: (f32, f32),
    pub focused: Option<NodeId>,
    pub focus_value: Option<String>,
    pub ready_state: &'static str,
    pub local_storage: BTreeMap<String, String>,
    pub session_storage: BTreeMap<String, String>,
    pub history: Vec<String>,
    pub history_pos: usize,
    pub navigate_to: Option<String>,
    pub observers: Vec<MutationObs>,
    pub observer_seq: u32,
    pub computed: BTreeMap<usize, Vec<(String, String)>>,
    pub user_agent: String,
    pub dirty: bool,
    pub doc_base: Option<String>,
    pub new_sheets: Vec<(NodeId, String)>,
}

impl Default for DomState {
    fn default() -> Self {
        DomState {
            base_url: String::from("about:blank"),
            epoch_ms: 1_767_225_600_000.0,
            listeners: Vec::new(),
            listener_seq: 0,
            selector_cache: BTreeMap::new(),
            cookie: String::new(),
            rects: BTreeMap::new(),
            viewport: (1024.0, 768.0),
            scroll: (0.0, 0.0),
            focused: None,
            focus_value: None,
            ready_state: "loading",
            local_storage: BTreeMap::new(),
            session_storage: BTreeMap::new(),
            history: Vec::new(),
            history_pos: 0,
            navigate_to: None,
            observers: Vec::new(),
            observer_seq: 0,
            computed: BTreeMap::new(),
            user_agent: String::from("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Castilia/0.1"),
            dirty: false,
            doc_base: None,
            new_sheets: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Basic helpers
// ---------------------------------------------------------------------------

pub fn kebab(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for ch in s.chars() {
        if ch == '-' {
            up = true;
        } else if up {
            out.push(ch.to_ascii_uppercase());
            up = false;
        } else {
            out.push(ch);
        }
    }
    out
}

impl Interp {
    pub fn dom<'a>(&mut self) -> Res<&'a mut Document> {
        if self.doc.is_null() {
            return self.type_error("document is not available");
        }
        // SAFETY: see `Interp::doc`; callers keep the borrow short and never across a JS call.
        Ok(unsafe { &mut *self.doc })
    }

    pub fn node_id(&mut self, v: &Value) -> Res<NodeId> {
        if let Value::Obj(o) = v {
            if let Class::Node(id) = o.borrow().class {
                return Ok(id);
            }
        }
        let what = match v {
            Value::Obj(o) => {
                let ctor = self.get_v(v, &self.names.constructor.clone()).ok().and_then(|c| self.get_str(&c, "name").ok()).map(|n| self.to_string_lossy(&n)).unwrap_or_default();
                let proxy = matches!(o.borrow().class, Class::Proxy { .. });
                alloc::format!("{}{}", if proxy { "Proxy of " } else { "" }, if ctor.is_empty() { "object" } else { &ctor })
            }
            other => self.describe(other),
        };
        self.type_error(&alloc::format!("Illegal invocation: not a DOM node ({what})"))
    }

    pub fn this_node(&mut self, c: &Call) -> Res<NodeId> {
        self.node_id(&c.this)
    }

    pub fn opt_node(&mut self, v: &Value) -> Res<Option<NodeId>> {
        if v.is_nullish() { Ok(None) } else { Ok(Some(self.node_id(v)?)) }
    }

    pub fn wrap(&mut self, id: NodeId) -> Value {
        if let Some(o) = self.node_cache.get(&id) {
            return Value::Obj(o.clone());
        }
        let proto = match self.doc().map(|d| d.nodes[id].kind.clone()) {
            Some(NodeKind::Element { tag, .. }) => self.proto_for_tag(&tag),
            Some(NodeKind::Text(_)) => self.protos().text.clone(),
            Some(NodeKind::Comment(_)) => self.protos().extra.get("comment").cloned().unwrap_or_else(|| self.protos().text.clone()),
            Some(NodeKind::Document) => self.protos().document.clone(),
            Some(NodeKind::Fragment) => self.protos().extra.get("fragment").cloned().unwrap_or_else(|| self.protos().node.clone()),
            None => self.protos().node.clone(),
        };
        let class = if matches!(self.doc().map(|d| &d.nodes[id].kind), Some(NodeKind::Document)) { Class::Node(id) } else { Class::Node(id) };
        let o = self.alloc(Object::new(Some(proto), class));
        self.node_cache.insert(id, o.clone());
        Value::Obj(o)
    }

    pub fn proto_for_tag(&self, tag: &str) -> Obj {
        let key = match tag {
            "a" => "a",
            "img" => "img",
            "input" => "input",
            "button" => "button",
            "form" => "form",
            "select" => "select",
            "textarea" => "textarea",
            "option" => "option",
            "canvas" => "canvas",
            "script" => "script",
            "style" => "style",
            "link" => "link",
            "iframe" => "iframe",
            "div" => "div",
            "span" => "span",
            "p" => "p",
            "table" => "table",
            "tr" => "tr",
            "td" | "th" => "td",
            "ul" => "ul",
            "ol" => "ol",
            "li" => "li",
            "label" => "label",
            "video" => "video",
            "audio" => "audio",
            "body" => "body",
            "head" => "head",
            "html" => "html",
            "svg" => "svg",
            "template" => "template",
            "meta" => "meta",
            "title" => "title",
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
            _ => "",
        };
        if !key.is_empty() {
            if let Some(p) = self.protos().extra.get(key) {
                return p.clone();
            }
        }
        if tag.contains('-') {
            return self.protos().html_element.clone();
        }
        self.protos().html_element.clone()
    }

    pub fn node_list(&mut self, ids: &[NodeId]) -> Value {
        let items: Vec<Value> = ids.iter().map(|&i| self.wrap(i)).collect();
        let proto = self.protos().extra.get("nodelist").cloned().unwrap_or_else(|| self.protos().array.clone());
        Value::Obj(self.alloc(Object::new(Some(proto), Class::Array(items))))
    }

    pub fn attr(&mut self, id: NodeId, name: &str) -> Option<String> {
        self.doc().and_then(|d| d.nodes[id].attribute(name).map(|s| s.to_string()))
    }

    pub fn tag_name(&mut self, id: NodeId) -> String {
        self.doc().and_then(|d| d.nodes[id].tag().map(|s| s.to_string())).unwrap_or_default()
    }

    pub fn is_element(&mut self, id: NodeId) -> bool {
        matches!(self.doc().map(|d| &d.nodes[id].kind), Some(NodeKind::Element { .. }))
    }

    /// Parse and cache a selector list; throws SyntaxError for invalid selectors.
    pub fn selector(&mut self, s: &str) -> Res<Rc<Vec<Complex>>> {
        if let Some(l) = self.dom.selector_cache.get(s) {
            return Ok(l.clone());
        }
        match selector::parse_list(s) {
            Some(l) => {
                if self.dom.selector_cache.len() > 2000 {
                    self.dom.selector_cache.clear();
                }
                let rc = Rc::new(l);
                self.dom.selector_cache.insert(s.to_string(), rc.clone());
                Ok(rc)
            }
            None => Err(self.error_ctl("SyntaxError", &format!("'{s}' is not a valid selector"))),
        }
    }

    pub fn query(&mut self, root: NodeId, sel: &str, first: bool) -> Res<Vec<NodeId>> {
        let list = self.selector(sel)?;
        let doc = self.dom()?;
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = doc.nodes[root].children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if matches!(doc.nodes[id].kind, NodeKind::Element { .. }) && selector::matches_list(doc, id, &list) {
                out.push(id);
                if first {
                    break;
                }
            }
            if doc.nodes[id].tag() != Some("template") {
                stack.extend(doc.nodes[id].children.iter().rev().copied());
            }
        }
        Ok(out)
    }

    pub fn url_resolve(&self, rel: &str) -> String {
        let base = Url::parse(self.dom.doc_base.as_deref().unwrap_or(&self.dom.base_url), None);
        match Url::parse(rel, base.as_ref()) {
            Some(u) => u.href(),
            None => rel.to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // Mutation primitives (all DOM edits flow through these so observers see them)
    // -----------------------------------------------------------------------

    pub fn dom_insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) -> Res<()> {
        let doc = self.dom()?;
        if matches!(doc.nodes[child].kind, NodeKind::Fragment) {
            let kids = doc.nodes[child].children.clone();
            for k in kids {
                self.dom_insert(parent, k, before)?;
            }
            return Ok(());
        }
        let old_parent = doc.nodes[child].parent;
        if let Some(op) = old_parent {
            if op != parent || before.is_some() {
                self.note_children(op, Vec::new(), vec![child]);
            }
        }
        if !doc.insert_before(parent, child, before) {
            return Err(self.error_ctl("HierarchyRequestError", "The new child element contains the parent."));
        }
        self.dom.dirty = true;
        self.note_children(parent, vec![child], Vec::new());
        self.after_insert(child)?;
        if self.dom()?.is_connected(child) {
            self.custom_element_callbacks(child, "connectedCallback");
        }
        Ok(())
    }

    pub fn dom_remove(&mut self, child: NodeId) -> Res<()> {
        let doc = self.dom()?;
        if let Some(p) = doc.nodes[child].parent {
            doc.remove(child);
            self.dom.dirty = true;
            self.custom_element_callbacks(child, "disconnectedCallback");
            self.note_children(p, Vec::new(), vec![child]);
        }
        Ok(())
    }

    pub fn dom_set_attr(&mut self, id: NodeId, name: &str, value: &str) -> Res<()> {
        let doc = self.dom()?;
        let name = name.to_ascii_lowercase();
        let old = doc.nodes[id].attribute(&name).map(|s| s.to_string());
        if old.as_deref() == Some(value) {
            return Ok(());
        }
        doc.set_attribute(id, &name, value);
        self.dom.dirty = true;
        self.note_attr(id, &name, old);
        if name == "src" && self.tag_name(id) == "script" {
            self.maybe_run_script(id)?;
        }
        Ok(())
    }

    pub fn dom_remove_attr(&mut self, id: NodeId, name: &str) -> Res<()> {
        let doc = self.dom()?;
        let name = name.to_ascii_lowercase();
        if let NodeKind::Element { attributes, .. } = &mut doc.nodes[id].kind {
            if let Some(pos) = attributes.iter().position(|(k, _)| *k == name) {
                let (_, old) = attributes.remove(pos);
                self.dom.dirty = true;
                self.note_attr(id, &name, Some(old));
            }
        }
        Ok(())
    }

    pub fn dom_set_text(&mut self, id: NodeId, text: &str) -> Res<()> {
        let doc = self.dom()?;
        let is_char = matches!(doc.nodes[id].kind, NodeKind::Text(_) | NodeKind::Comment(_));
        let removed = doc.nodes[id].children.clone();
        doc.set_text_content(id, text);
        self.dom.dirty = true;
        if is_char {
            self.note_char(id);
        } else {
            let added = self.dom()?.nodes[id].children.clone();
            self.note_children(id, added, removed);
        }
        Ok(())
    }

    pub fn dom_set_inner_html(&mut self, id: NodeId, src: &str) -> Res<()> {
        let doc = self.dom()?;
        let removed = core::mem::take(&mut doc.nodes[id].children);
        for &c in &removed {
            doc.nodes[c].parent = None;
        }
        // template content lives in the element itself
        html::parse_fragment(doc, id, src);
        let added = doc.nodes[id].children.clone();
        self.dom.dirty = true;
        self.note_children(id, added.clone(), removed);
        for a in added {
            self.after_insert(a)?;
        }
        Ok(())
    }

    /// Hook after a node entered the tree: run dynamically inserted scripts.
    pub fn after_insert(&mut self, id: NodeId) -> Res<()> {
        let doc = self.dom()?;
        if !doc.is_connected(id) {
            return Ok(());
        }
        let mut stack = vec![id];
        let mut scripts = Vec::new();
        while let Some(n) = stack.pop() {
            if doc.nodes[n].tag() == Some("script") {
                scripts.push(n);
            }
            stack.extend(doc.nodes[n].children.iter().rev().copied());
        }
        let mut links = Vec::new();
        let mut st = vec![id];
        while let Some(n) = st.pop() {
            if doc.nodes[n].tag() == Some("link") && doc.nodes[n].attribute("rel").is_some_and(|r| r.split_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet"))) {
                links.push(n);
            }
            st.extend(doc.nodes[n].children.iter().rev().copied());
        }
        for l in links {
            self.load_stylesheet_link(l);
        }
        // scripts created through innerHTML never execute (per spec); only DOM-API inserted ones do.
        if self.parsing_html_scripts {
            return Ok(());
        }
        for s in scripts {
            self.maybe_run_script(s)?;
        }
        Ok(())
    }

    /// Fetch a script-inserted `<link rel=stylesheet>` through the host and queue it for the page.
    pub fn load_stylesheet_link(&mut self, id: NodeId) {
        let Some(href) = self.attr(id, "href") else { return };
        if self.loaded_links.contains(&id) {
            return;
        }
        self.loaded_links.push(id);
        let url = self.url_resolve(&href);
        let Some(r) = self.host_fetch("GET", &url, &[], None) else { return };
        if r.status >= 400 {
            return;
        }
        let css = {
            let mut fetch = |u: &str| self.host_fetch("GET", u, &[], None).filter(|r| r.status < 400).map(|r| r.body);
            crate::cssload::process(&r.body, &url, &mut fetch)
        };
        self.dom.new_sheets.push((id, css));
        self.dom.dirty = true;
    }

    pub fn maybe_run_script(&mut self, id: NodeId) -> Res<()> {
        if self.executed_scripts.contains(&id) {
            return Ok(());
        }
        let doc = self.dom()?;
        if !doc.is_connected(id) {
            return Ok(());
        }
        let ty = doc.nodes[id].attribute("type").unwrap_or("").to_ascii_lowercase();
        let runnable = ty.is_empty() || matches!(ty.as_str(), "text/javascript" | "application/javascript" | "module" | "text/ecmascript" | "application/ecmascript");
        if !runnable {
            return Ok(());
        }
        let src = doc.nodes[id].attribute("src").map(|s| s.to_string());
        let code = match &src {
            Some(u) => {
                let url = self.url_resolve(u);
                let r = self.host_fetch("GET", &url, &[], None);
                match r {
                    Some(r) if r.status < 400 => r.body,
                    _ => {
                        self.fire_node_event(id, "error")?;
                        return Ok(());
                    }
                }
            }
            None => doc.text_content(id).unwrap_or_default(),
        };
        if code.trim().is_empty() && src.is_none() {
            return Ok(());
        }
        self.executed_scripts.push(id);
        let prev = self.current_script;
        self.current_script = Some(id);
        let r = self.eval_script(&code);
        self.current_script = prev;
        if let Err(e) = r {
            self.logs.push(LogEntry { level: "error", text: format!("Uncaught {e}") });
        }
        if src.is_some() {
            self.fire_node_event(id, "load")?;
        }
        Ok(())
    }

    pub fn host_fetch(&mut self, method: &str, url: &str, headers: &[(String, String)], body: Option<&str>) -> Option<HostResponse> {
        if url.starts_with("data:") {
            return Some(data_url_response(url));
        }
        let h = self.host.host.as_mut()?;
        h.fetch(method, url, headers, body)
    }
}

fn data_url_response(url: &str) -> HostResponse {
    let rest = &url[5..];
    let (meta, data) = rest.split_once(',').unwrap_or(("", rest));
    let body = if meta.ends_with(";base64") {
        let mut vals = Vec::new();
        const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for ch in data.chars() {
            if let Some(v) = B64.iter().position(|b| *b as char == ch) {
                vals.push(v as u32);
            }
        }
        let mut bytes = Vec::new();
        for chunk in vals.chunks(4) {
            let mut n = 0u32;
            for (i, v) in chunk.iter().enumerate() {
                n |= v << (18 - 6 * i);
            }
            bytes.push((n >> 16) as u8);
            if chunk.len() > 2 {
                bytes.push((n >> 8) as u8);
            }
            if chunk.len() > 3 {
                bytes.push(n as u8);
            }
        }
        String::from_utf8_lossy(&bytes).to_string()
    } else {
        percent_decode(data)
    };
    HostResponse { status: 200, status_text: "OK".into(), url: url.to_string(), headers: vec![("content-type".into(), meta.split(';').next().unwrap_or("text/plain").to_string())], body }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = core::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
