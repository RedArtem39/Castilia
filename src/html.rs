//! HTML parser: a pragmatic implementation of the HTML5 tree construction algorithm
//! (implied tags, tables, formatting elements, foreign content, raw text, entities).
use crate::dom::{Document, NodeId, NodeKind};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

pub fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    let mut b = Builder::new(&mut doc, None);
    b.run(source);
    wrap_templates(&mut doc, 0);
    doc
}

/// `<template>` keeps its parsed children inside a single `Fragment` child (its content), so that
/// queries and traversal of the live document do not see them.
fn wrap_templates(doc: &mut Document, from: usize) {
    for id in from..doc.nodes.len() {
        if doc.nodes[id].tag() != Some("template") || doc.nodes[id].children.is_empty() {
            continue;
        }
        if doc.nodes[id].children.len() == 1 && doc.nodes[doc.nodes[id].children[0]].kind == NodeKind::Fragment {
            continue;
        }
        let frag = doc.push(NodeKind::Fragment, None);
        let kids = core::mem::take(&mut doc.nodes[id].children);
        for k in kids {
            doc.nodes[k].parent = Some(frag);
            doc.nodes[frag].children.push(k);
        }
        doc.nodes[frag].parent = Some(id);
        doc.nodes[id].children.push(frag);
    }
}

/// Parse `source` as if it were the content of `context` (used for `innerHTML`) and append the
/// resulting nodes to it.
pub fn parse_fragment(doc: &mut Document, context: NodeId, source: &str) {
    let from = doc.nodes.len();
    let mut b = Builder::new(doc, Some(context));
    b.run(source);
    wrap_templates(doc, from);
}

pub fn is_void(tag: &str) -> bool {
    matches!(
        tag,
        "area" | "base" | "basefont" | "bgsound" | "br" | "col" | "embed" | "frame" | "hr" | "img" | "input" | "keygen" | "link"
            | "meta" | "param" | "source" | "track" | "wbr"
    )
}

fn is_special(tag: &str) -> bool {
    matches!(
        tag,
        "address" | "applet" | "area" | "article" | "aside" | "base" | "basefont" | "bgsound" | "blockquote" | "body" | "br"
            | "button" | "caption" | "center" | "col" | "colgroup" | "dd" | "details" | "dir" | "div" | "dl" | "dt" | "embed"
            | "fieldset" | "figcaption" | "figure" | "footer" | "form" | "frame" | "frameset" | "h1" | "h2" | "h3" | "h4"
            | "h5" | "h6" | "head" | "header" | "hgroup" | "hr" | "html" | "iframe" | "img" | "input" | "keygen" | "li"
            | "link" | "listing" | "main" | "marquee" | "menu" | "meta" | "nav" | "noembed" | "noframes" | "noscript"
            | "object" | "ol" | "p" | "param" | "plaintext" | "pre" | "script" | "search" | "section" | "select" | "source"
            | "style" | "summary" | "table" | "tbody" | "td" | "template" | "textarea" | "tfoot" | "th" | "thead" | "title"
            | "tr" | "track" | "ul" | "wbr" | "xmp"
    )
}

fn is_formatting(tag: &str) -> bool {
    matches!(tag, "a" | "b" | "big" | "code" | "em" | "font" | "i" | "nobr" | "s" | "small" | "strike" | "strong" | "tt" | "u")
}

fn implied_end(tag: &str) -> bool {
    matches!(tag, "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc")
}

fn is_heading(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

const SVG_TAGS: &[(&str, &str)] = &[
    ("altglyph", "altGlyph"), ("altglyphdef", "altGlyphDef"), ("altglyphitem", "altGlyphItem"), ("animatecolor", "animateColor"),
    ("animatemotion", "animateMotion"), ("animatetransform", "animateTransform"), ("clippath", "clipPath"), ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"), ("fecomponenttransfer", "feComponentTransfer"), ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"), ("fediffuselighting", "feDiffuseLighting"), ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"), ("fedropshadow", "feDropShadow"), ("feflood", "feFlood"), ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"), ("fefuncg", "feFuncG"), ("fefuncr", "feFuncR"), ("fegaussianblur", "feGaussianBlur"), ("feimage", "feImage"),
    ("femerge", "feMerge"), ("femergenode", "feMergeNode"), ("femorphology", "feMorphology"), ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"), ("fespecularlighting", "feSpecularLighting"), ("fespotlight", "feSpotLight"), ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"), ("foreignobject", "foreignObject"), ("glyphref", "glyphRef"), ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"), ("textpath", "textPath"),
];

const SVG_ATTRS: &[(&str, &str)] = &[
    ("attributename", "attributeName"), ("attributetype", "attributeType"), ("basefrequency", "baseFrequency"), ("baseprofile", "baseProfile"),
    ("calcmode", "calcMode"), ("clippathunits", "clipPathUnits"), ("diffuseconstant", "diffuseConstant"), ("edgemode", "edgeMode"),
    ("filterunits", "filterUnits"), ("glyphref", "glyphRef"), ("gradienttransform", "gradientTransform"), ("gradientunits", "gradientUnits"),
    ("kernelmatrix", "kernelMatrix"), ("kernelunitlength", "kernelUnitLength"), ("keypoints", "keyPoints"), ("keysplines", "keySplines"),
    ("keytimes", "keyTimes"), ("lengthadjust", "lengthAdjust"), ("limitingconeangle", "limitingConeAngle"), ("markerheight", "markerHeight"),
    ("markerunits", "markerUnits"), ("markerwidth", "markerWidth"), ("maskcontentunits", "maskContentUnits"), ("maskunits", "maskUnits"),
    ("numoctaves", "numOctaves"), ("pathlength", "pathLength"), ("patterncontentunits", "patternContentUnits"),
    ("patterntransform", "patternTransform"), ("patternunits", "patternUnits"), ("pointsatx", "pointsAtX"), ("pointsaty", "pointsAtY"),
    ("pointsatz", "pointsAtZ"), ("preservealpha", "preserveAlpha"), ("preserveaspectratio", "preserveAspectRatio"),
    ("primitiveunits", "primitiveUnits"), ("refx", "refX"), ("refy", "refY"), ("repeatcount", "repeatCount"), ("repeatdur", "repeatDur"),
    ("requiredextensions", "requiredExtensions"), ("requiredfeatures", "requiredFeatures"), ("specularconstant", "specularConstant"),
    ("specularexponent", "specularExponent"), ("spreadmethod", "spreadMethod"), ("startoffset", "startOffset"), ("stddeviation", "stdDeviation"),
    ("stitchtiles", "stitchTiles"), ("surfacescale", "surfaceScale"), ("systemlanguage", "systemLanguage"), ("tablevalues", "tableValues"),
    ("targetx", "targetX"), ("targety", "targetY"), ("textlength", "textLength"), ("viewbox", "viewBox"), ("viewtarget", "viewTarget"),
    ("xchannelselector", "xChannelSelector"), ("ychannelselector", "yChannelSelector"), ("zoomandpan", "zoomAndPan"),
];

type Attrs = Vec<(String, String)>;

struct Builder<'a> {
    doc: &'a mut Document,
    stack: Vec<NodeId>,
    /// Active formatting elements: (node, tag, attributes). `None` marks a scope boundary.
    active: Vec<Option<(NodeId, String, Attrs)>>,
    body_started: bool,
    head_closed: bool,
    form_open: bool,
    skip_newline: bool,
    fragment: bool,
    /// Raw-text element requested by the last start tag: (name, decode entities).
    pending_raw: Option<(String, bool)>,
    /// Set while inserting table structure elements, which are never foster parented.
    no_foster: bool,
}

type StartTag<'s> = (String, Attrs, &'s str, bool);

impl<'a> Builder<'a> {
    fn new(doc: &'a mut Document, context: Option<NodeId>) -> Self {
        let (stack, body_started, fragment) = match context {
            Some(c) => (Vec::from([c]), true, true),
            None => (Vec::from([doc.nodes[doc.root].children[0]]), false, false),
        };
        Builder { doc, stack, active: Vec::new(), body_started, head_closed: false, form_open: false, skip_newline: false, fragment, pending_raw: None, no_foster: false }
    }

    fn html(&self) -> NodeId {
        self.doc.nodes[self.doc.root].children[0]
    }

    fn current(&self) -> NodeId {
        *self.stack.last().unwrap_or(&self.doc.body)
    }

    fn tag_of(&self, id: NodeId) -> &str {
        self.doc.nodes[id].tag().unwrap_or("")
    }

    fn in_foreign(&self) -> bool {
        for &id in self.stack.iter().rev() {
            match self.tag_of(id) {
                "svg" | "math" => return true,
                "foreignObject" | "desc" | "title" | "annotation-xml" | "html" | "body" => return false,
                _ => {}
            }
        }
        false
    }

    // ---- scope helpers ----

    fn scope_stop(tag: &str, extra: u8) -> bool {
        let base = matches!(tag, "applet" | "caption" | "html" | "table" | "td" | "th" | "marquee" | "object" | "template" | "foreignObject" | "desc");
        match extra {
            0 => base,
            1 => base || tag == "button",
            2 => base || tag == "ol" || tag == "ul",
            _ => matches!(tag, "html" | "table" | "template"),
        }
    }

    /// `extra`: 0 default scope, 1 button scope, 2 list-item scope, 3 table scope
    fn in_scope(&self, tag: &str, extra: u8) -> bool {
        self.in_scope_any(|t| t == tag, extra)
    }

    fn in_scope_any(&self, f: impl Fn(&str) -> bool, extra: u8) -> bool {
        for &id in self.stack.iter().rev() {
            let t = self.tag_of(id);
            if f(t) {
                return true;
            }
            if Self::scope_stop(t, extra) {
                return false;
            }
        }
        false
    }

    fn pop_until(&mut self, pred: impl Fn(&str) -> bool) {
        while self.stack.len() > 1 {
            let id = self.stack.pop().expect("non-empty");
            if pred(self.tag_of(id)) {
                break;
            }
        }
    }

    fn generate_implied_end(&mut self, except: Option<&str>) {
        while self.stack.len() > 1 {
            let t = self.tag_of(self.current());
            if implied_end(t) && Some(t) != except {
                self.stack.pop();
            } else {
                break;
            }
        }
    }

    fn close_p(&mut self) {
        if self.in_scope("p", 1) {
            self.generate_implied_end(Some("p"));
            self.pop_until(|t| t == "p");
        }
    }

    // ---- node creation ----

    fn make_element(&mut self, tag: &str, attrs: Attrs) -> NodeId {
        self.doc.push(NodeKind::Element { tag: tag.to_string(), attributes: attrs }, None)
    }

    fn insert_at(&mut self, parent: NodeId, id: NodeId) {
        self.doc.nodes[id].parent = Some(parent);
        self.doc.nodes[parent].children.push(id);
    }

    /// Foster parenting: content directly inside a table goes before the table.
    fn insertion_parent(&self) -> (NodeId, Option<NodeId>) {
        let cur = self.current();
        if self.no_foster {
            return (cur, None);
        }
        if matches!(self.tag_of(cur), "table" | "tbody" | "tfoot" | "thead" | "tr")
            && let Some(&table) = self.stack.iter().rev().find(|&&i| self.tag_of(i) == "table")
            && let Some(parent) = self.doc.nodes[table].parent
        {
            return (parent, Some(table));
        }
        (cur, None)
    }

    fn add_node(&mut self, id: NodeId) {
        let (parent, before) = self.insertion_parent();
        match before {
            Some(table) => {
                let pos = self.doc.nodes[parent].children.iter().position(|&c| c == table).unwrap_or(self.doc.nodes[parent].children.len());
                self.doc.nodes[id].parent = Some(parent);
                self.doc.nodes[parent].children.insert(pos, id);
            }
            None => self.insert_at(parent, id),
        }
    }

    fn insert_element(&mut self, tag: &str, attrs: Attrs) -> NodeId {
        let id = self.make_element(tag, attrs);
        self.add_node(id);
        self.stack.push(id);
        id
    }

    fn insert_void(&mut self, tag: &str, attrs: Attrs) -> NodeId {
        let id = self.make_element(tag, attrs);
        self.add_node(id);
        id
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let (parent, before) = self.insertion_parent();
        let idx = match before {
            Some(t) => self.doc.nodes[parent].children.iter().position(|&c| c == t).unwrap_or(self.doc.nodes[parent].children.len()),
            None => self.doc.nodes[parent].children.len(),
        };
        if idx > 0 {
            let prev = self.doc.nodes[parent].children[idx - 1];
            if let NodeKind::Text(t) = &mut self.doc.nodes[prev].kind {
                t.push_str(text);
                return;
            }
        }
        let id = self.doc.push(NodeKind::Text(text.to_string()), None);
        self.doc.nodes[id].parent = Some(parent);
        self.doc.nodes[parent].children.insert(idx, id);
    }

    // ---- active formatting elements ----

    fn push_active(&mut self, id: NodeId, tag: &str, attrs: &[(String, String)]) {
        // Noah's Ark clause: at most three identical entries after the last marker.
        let same: Vec<usize> = self
            .active
            .iter()
            .enumerate()
            .rev()
            .take_while(|(_, e)| e.is_some())
            .filter(|(_, e)| e.as_ref().is_some_and(|(_, t, a)| t == tag && a == attrs))
            .map(|(i, _)| i)
            .collect();
        if same.len() >= 3 {
            self.active.remove(*same.last().unwrap_or(&0));
        }
        self.active.push(Some((id, tag.to_string(), attrs.to_vec())));
    }

    fn reconstruct_active(&mut self) {
        let Some(last) = self.active.len().checked_sub(1) else { return };
        match &self.active[last] {
            None => return,
            Some((id, _, _)) if self.stack.contains(id) => return,
            _ => {}
        }
        let mut i = last;
        while i > 0 {
            match &self.active[i - 1] {
                None => break,
                Some((id, _, _)) if self.stack.contains(id) => break,
                _ => i -= 1,
            }
        }
        for k in i..self.active.len() {
            if let Some((_, tag, attrs)) = self.active[k].clone() {
                let id = self.insert_element(&tag, attrs.clone());
                self.active[k] = Some((id, tag, attrs));
            }
        }
    }

    fn remove_active(&mut self, id: NodeId) {
        if let Some(p) = self.active.iter().position(|e| e.as_ref().is_some_and(|(n, _, _)| *n == id)) {
            self.active.remove(p);
        }
    }

    fn clear_active_to_marker(&mut self) {
        while let Some(e) = self.active.pop() {
            if e.is_none() {
                break;
            }
        }
    }

    /// Simplified adoption agency algorithm.
    fn adoption(&mut self, tag: &str) {
        let Some(pos) = self.active.iter().rposition(|e| e.as_ref().is_some_and(|(_, t, _)| t == tag)) else {
            self.any_other_end(tag);
            return;
        };
        let (fid, ftag, fattrs) = self.active[pos].clone().expect("checked");
        let Some(spos) = self.stack.iter().position(|&n| n == fid) else {
            self.active.remove(pos);
            return;
        };
        let furthest = self.stack[spos + 1..].iter().position(|&n| is_special(self.tag_of(n))).map(|p| p + spos + 1);
        let Some(fb) = furthest else {
            self.stack.truncate(spos);
            self.active.remove(pos);
            return;
        };
        let common = if spos > 0 { self.stack[spos - 1] } else { self.html() };
        let fb_node = self.stack[fb];
        // move the furthest block under the common ancestor
        if let Some(old) = self.doc.nodes[fb_node].parent {
            self.doc.nodes[old].children.retain(|&c| c != fb_node);
        }
        self.insert_at(common, fb_node);
        // clone the formatting element around the furthest block's children
        let clone = self.make_element(&ftag, fattrs.clone());
        let kids = core::mem::take(&mut self.doc.nodes[fb_node].children);
        for k in kids {
            self.doc.nodes[k].parent = Some(clone);
            self.doc.nodes[clone].children.push(k);
        }
        self.insert_at(fb_node, clone);
        self.active[pos] = Some((clone, ftag, fattrs));
        self.stack.remove(spos);
        let fb_idx = self.stack.iter().position(|&n| n == fb_node).unwrap_or(self.stack.len() - 1);
        self.stack.insert(fb_idx + 1, clone);
    }

    fn any_other_end(&mut self, tag: &str) {
        let mut i = self.stack.len();
        while i > 1 {
            i -= 1;
            let t = self.tag_of(self.stack[i]).to_string();
            if t == tag {
                self.generate_implied_end(Some(tag));
                self.stack.truncate(i);
                return;
            }
            if is_special(&t) {
                return;
            }
        }
    }

    // ---- structure ----

    fn ensure_body(&mut self) {
        if self.body_started {
            return;
        }
        self.body_started = true;
        self.head_closed = true;
        self.stack = Vec::from([self.html(), self.doc.body]);
    }

    fn in_table_context(&self) -> bool {
        matches!(self.tag_of(self.current()), "table" | "tbody" | "tfoot" | "thead" | "tr" | "colgroup")
    }

    fn clear_to(&mut self, tags: &[&str]) {
        while self.stack.len() > 1 {
            let t = self.tag_of(self.current());
            if tags.contains(&t) || t == "html" || t == "template" {
                break;
            }
            self.stack.pop();
        }
    }

    fn merge_attrs(&mut self, id: NodeId, attrs: Attrs) {
        if let NodeKind::Element { attributes, .. } = &mut self.doc.nodes[id].kind {
            for (k, v) in attrs {
                if !attributes.iter().any(|(ek, _)| *ek == k) {
                    attributes.push((k, v));
                }
            }
        }
    }

    fn process_text(&mut self, text: &str) {
        let mut text = text;
        if self.skip_newline {
            self.skip_newline = false;
            if let Some(t) = text.strip_prefix('\n') {
                text = t;
            }
        }
        if text.is_empty() {
            return;
        }
        let ws_only = text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r'));
        if !self.body_started {
            if ws_only {
                return;
            }
            self.ensure_body();
        }
        if !ws_only || self.in_table_context() && !ws_only {
            self.reconstruct_active();
        }
        self.insert_text(text);
    }

    fn head_insert(&mut self, name: &str, attrs: Attrs, void: bool) {
        let head = self.doc.head;
        self.stack = Vec::from([self.html(), head]);
        if void {
            self.insert_void(name, attrs);
        } else {
            self.insert_element(name, attrs);
        }
    }

    fn process_start(&mut self, name: String, attrs: Attrs, self_closing: bool) {
        let name = if name == "image" { "img".to_string() } else { name };
        // foreign (svg / math) content
        if self.in_foreign() && self.body_started {
            let breaks_out = matches!(
                name.as_str(),
                "b" | "big" | "blockquote" | "body" | "br" | "center" | "code" | "dd" | "div" | "dl" | "dt" | "em" | "embed" | "h1" | "h2"
                    | "h3" | "h4" | "h5" | "h6" | "head" | "hr" | "i" | "img" | "li" | "listing" | "menu" | "meta" | "nobr" | "ol" | "p"
                    | "pre" | "ruby" | "s" | "small" | "span" | "strong" | "strike" | "sub" | "sup" | "table" | "tt" | "u" | "ul" | "var"
            );
            if !breaks_out {
                let tag = SVG_TAGS.iter().find(|(l, _)| *l == name).map_or(name.clone(), |(_, c)| c.to_string());
                let attrs: Attrs = attrs
                    .into_iter()
                    .map(|(k, v)| (SVG_ATTRS.iter().find(|(l, _)| *l == k).map_or(k, |(_, c)| c.to_string()), v))
                    .collect();
                let id = self.make_element(&tag, attrs);
                self.add_node(id);
                if !self_closing {
                    self.stack.push(id);
                }
                return;
            }
            while self.stack.len() > 1 && self.in_foreign() {
                self.stack.pop();
            }
        }
        match name.as_str() {
            "html" => {
                let h = self.html();
                self.merge_attrs(h, attrs);
                return;
            }
            "head" => {
                if !self.body_started && !self.head_closed {
                    let head = self.doc.head;
                    self.merge_attrs(head, attrs);
                    self.stack = Vec::from([self.html(), head]);
                }
                return;
            }
            "body" => {
                if self.fragment {
                    return;
                }
                let body = self.doc.body;
                self.merge_attrs(body, attrs);
                self.ensure_body();
                return;
            }
            "title" | "style" | "script" | "noscript" | "noframes" | "template" if !self.body_started => {
                self.head_insert(&name, attrs, false);
                self.pending_raw = match name.as_str() {
                    "title" => Some((name, true)),
                    "template" => None,
                    _ => Some((name, false)),
                };
                return;
            }
            "meta" | "link" | "base" | "basefont" | "bgsound" if !self.body_started => {
                self.head_insert(&name, attrs, true);
                return;
            }
            _ => {}
        }
        self.ensure_body();
        if matches!(name.as_str(), "caption" | "colgroup" | "col" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th") {
            self.table_start(&name, attrs);
            return;
        }
        match name.as_str() {
            "table" => {
                if !self.doc.quirks {
                    self.close_p();
                }
                self.reconstruct_active();
                self.insert_element("table", attrs);
            }
            "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog" | "dir" | "div" | "dl" | "fieldset"
            | "figcaption" | "figure" | "footer" | "header" | "hgroup" | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section"
            | "summary" | "ul" => {
                self.close_p();
                self.insert_element(&name, attrs);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_p();
                if is_heading(self.tag_of(self.current())) {
                    self.stack.pop();
                }
                self.insert_element(&name, attrs);
            }
            "pre" | "listing" => {
                self.close_p();
                self.insert_element(&name, attrs);
                self.skip_newline = true;
            }
            "form" => {
                if self.form_open {
                    return;
                }
                self.close_p();
                self.form_open = true;
                self.insert_element("form", attrs);
            }
            "li" => {
                let mut i = self.stack.len();
                while i > 1 {
                    i -= 1;
                    let t = self.tag_of(self.stack[i]).to_string();
                    if t == "li" {
                        self.generate_implied_end(Some("li"));
                        self.pop_until(|t| t == "li");
                        break;
                    }
                    if is_special(&t) && !matches!(t.as_str(), "address" | "div" | "p") {
                        break;
                    }
                }
                self.close_p();
                self.insert_element("li", attrs);
            }
            "dd" | "dt" => {
                let mut i = self.stack.len();
                while i > 1 {
                    i -= 1;
                    let t = self.tag_of(self.stack[i]).to_string();
                    if t == "dd" || t == "dt" {
                        self.generate_implied_end(Some(&t));
                        self.pop_until(|x| x == t);
                        break;
                    }
                    if is_special(&t) && !matches!(t.as_str(), "address" | "div" | "p") {
                        break;
                    }
                }
                self.close_p();
                self.insert_element(&name, attrs);
            }
            "plaintext" => {
                self.close_p();
                self.insert_element("plaintext", attrs);
                self.pending_raw = Some(("plaintext".to_string(), false));
            }
            "button" => {
                if self.in_scope("button", 0) {
                    self.generate_implied_end(None);
                    self.pop_until(|t| t == "button");
                }
                self.reconstruct_active();
                self.insert_element("button", attrs);
            }
            "a" => {
                let open_a = self.active.iter().rev().take_while(|e| e.is_some()).find_map(|e| match e {
                    Some((id, t, _)) if t == "a" => Some(*id),
                    _ => None,
                });
                if let Some(id) = open_a {
                    self.adoption("a");
                    self.remove_active(id);
                    if let Some(p) = self.stack.iter().position(|&n| n == id) {
                        self.stack.remove(p);
                    }
                }
                self.reconstruct_active();
                let id = self.insert_element("a", attrs.clone());
                self.push_active(id, "a", &attrs);
            }
            n if is_formatting(n) => {
                self.reconstruct_active();
                let id = self.insert_element(n, attrs.clone());
                self.push_active(id, n, &attrs);
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct_active();
                self.insert_element(&name, attrs);
                self.active.push(None);
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" | "input" | "param" | "source" | "track" => {
                self.reconstruct_active();
                self.insert_void(&name, attrs);
            }
            "hr" => {
                self.close_p();
                self.insert_void("hr", attrs);
            }
            "textarea" => {
                self.reconstruct_active();
                self.insert_element("textarea", attrs);
                self.skip_newline = true;
                self.pending_raw = Some(("textarea".to_string(), true));
            }
            "xmp" => {
                self.close_p();
                self.reconstruct_active();
                self.insert_element("xmp", attrs);
                self.pending_raw = Some(("xmp".to_string(), false));
            }
            "iframe" | "noembed" | "noframes" | "noscript" => {
                self.reconstruct_active();
                self.insert_element(&name, attrs);
                self.pending_raw = Some((name.clone(), false));
            }
            "script" | "style" => {
                self.insert_element(&name, attrs);
                self.pending_raw = Some((name.clone(), false));
            }
            "title" => {
                self.insert_element("title", attrs);
                self.pending_raw = Some(("title".to_string(), true));
            }
            "select" => {
                self.reconstruct_active();
                self.insert_element("select", attrs);
            }
            "optgroup" | "option" => {
                if self.tag_of(self.current()) == "option" {
                    self.stack.pop();
                }
                self.reconstruct_active();
                self.insert_element(&name, attrs);
            }
            "svg" | "math" => {
                self.reconstruct_active();
                let attrs: Attrs = if name == "svg" {
                    attrs.into_iter().map(|(k, v)| (SVG_ATTRS.iter().find(|(l, _)| *l == k).map_or(k, |(_, c)| c.to_string()), v)).collect()
                } else {
                    attrs
                };
                let id = self.make_element(&name, attrs);
                self.add_node(id);
                if !self_closing {
                    self.stack.push(id);
                }
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" => {
                self.insert_void(&name, attrs);
            }
            "frame" | "frameset" | "head" => {}
            "template" => {
                self.insert_element("template", attrs);
            }
            _ => {
                self.reconstruct_active();
                self.insert_element(&name, attrs);
            }
        }
    }

    fn table_start(&mut self, name: &str, attrs: Attrs) {
        let in_table = self.stack.iter().any(|&i| self.tag_of(i) == "table");
        if !in_table {
            // table parts outside a table are ignored
            return;
        }
        self.no_foster = true;
        self.table_start_inner(name, attrs);
        self.no_foster = false;
    }

    fn table_start_inner(&mut self, name: &str, attrs: Attrs) {
        let in_cell = self
            .stack
            .iter()
            .rev()
            .find(|&&i| matches!(self.tag_of(i), "td" | "th" | "table"))
            .is_some_and(|&i| self.tag_of(i) != "table");
        match name {
            "caption" => {
                self.clear_to(&["table"]);
                self.insert_element("caption", attrs);
                self.active.push(None);
            }
            "colgroup" => {
                self.clear_to(&["table"]);
                self.insert_element("colgroup", attrs);
            }
            "col" => {
                self.clear_to(&["table", "colgroup"]);
                if self.tag_of(self.current()) == "table" {
                    self.insert_element("colgroup", Vec::new());
                }
                self.insert_void("col", attrs);
            }
            "tbody" | "tfoot" | "thead" => {
                if in_cell {
                    self.close_cell();
                }
                self.clear_to(&["table"]);
                self.insert_element(name, attrs);
            }
            "tr" => {
                if in_cell {
                    self.close_cell();
                }
                self.clear_to(&["table", "tbody", "thead", "tfoot"]);
                if self.tag_of(self.current()) == "table" {
                    self.insert_element("tbody", Vec::new());
                }
                self.insert_element("tr", attrs);
            }
            _ => {
                if in_cell {
                    self.close_cell();
                }
                self.clear_to(&["table", "tbody", "thead", "tfoot", "tr"]);
                if self.tag_of(self.current()) == "table" {
                    self.insert_element("tbody", Vec::new());
                }
                if matches!(self.tag_of(self.current()), "tbody" | "thead" | "tfoot") {
                    self.insert_element("tr", Vec::new());
                }
                self.insert_element(name, attrs);
                self.active.push(None);
            }
        }
    }

    fn close_cell(&mut self) {
        if self.in_scope_any(|t| matches!(t, "td" | "th"), 3) {
            self.generate_implied_end(None);
            self.pop_until(|t| matches!(t, "td" | "th"));
            self.clear_active_to_marker();
        }
    }

    fn process_end(&mut self, name: String) {
        if self.in_foreign() && self.body_started {
            let mut i = self.stack.len();
            while i > 1 {
                i -= 1;
                let t = self.tag_of(self.stack[i]).to_ascii_lowercase();
                if t == name {
                    self.stack.truncate(i);
                    return;
                }
                if t == "svg" || t == "math" {
                    break;
                }
            }
            if !matches!(name.as_str(), "p" | "br") {
                return;
            }
        }
        match name.as_str() {
            "html" | "body" => {}
            "head" => {
                if !self.body_started {
                    self.head_closed = true;
                    self.stack = Vec::from([self.html()]);
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details" | "dialog" | "dir" | "div" | "dl"
            | "fieldset" | "figcaption" | "figure" | "footer" | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol"
            | "pre" | "search" | "section" | "summary" | "ul" => {
                if self.in_scope(&name, 0) {
                    self.generate_implied_end(None);
                    self.pop_until(|t| t == name);
                }
            }
            "form" => {
                self.form_open = false;
                if self.in_scope("form", 0) {
                    self.generate_implied_end(None);
                    self.pop_until(|t| t == "form");
                }
            }
            "p" => {
                if !self.in_scope("p", 1) {
                    self.ensure_body();
                    self.insert_element("p", Vec::new());
                }
                self.close_p();
            }
            "li" => {
                if self.in_scope("li", 2) {
                    self.generate_implied_end(Some("li"));
                    self.pop_until(|t| t == "li");
                }
            }
            "dd" | "dt" => {
                if self.in_scope(&name, 0) {
                    self.generate_implied_end(Some(&name));
                    self.pop_until(|t| t == name);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if self.in_scope_any(is_heading, 0) {
                    self.generate_implied_end(None);
                    self.pop_until(is_heading);
                }
            }
            n if is_formatting(n) => self.adoption(n),
            "applet" | "marquee" | "object" => {
                if self.in_scope(&name, 0) {
                    self.generate_implied_end(None);
                    self.pop_until(|t| t == name);
                    self.clear_active_to_marker();
                }
            }
            "br" => {
                self.ensure_body();
                self.reconstruct_active();
                self.insert_void("br", Vec::new());
            }
            "td" | "th" => {
                if self.in_scope(&name, 3) {
                    self.generate_implied_end(None);
                    self.pop_until(|t| t == name);
                    self.clear_active_to_marker();
                }
            }
            "tr" => {
                if self.in_scope("tr", 3) {
                    self.clear_to(&["tr"]);
                    self.stack.pop();
                }
            }
            "tbody" | "thead" | "tfoot" | "caption" | "colgroup" => {
                if self.in_scope(&name, 3) {
                    self.pop_until(|t| t == name);
                    if name == "caption" {
                        self.clear_active_to_marker();
                    }
                }
            }
            "table" => {
                if self.in_scope("table", 3) {
                    self.pop_until(|t| t == "table");
                }
            }
            "template" => {
                if self.stack.iter().any(|&i| self.tag_of(i) == "template") {
                    self.pop_until(|t| t == "template");
                }
            }
            "script" | "style" | "title" | "textarea" | "noscript" | "xmp" | "iframe" | "noembed" | "noframes" | "select" | "option"
            | "optgroup" | "plaintext" => {
                if self.stack.iter().any(|&i| self.tag_of(i) == name) {
                    self.pop_until(|t| t == name);
                }
            }
            _ => self.any_other_end(&name),
        }
    }

    fn doctype(&mut self, raw: &str) {
        let d = raw.to_ascii_lowercase();
        let body = d.trim_start_matches("<!doctype").trim().trim_end_matches('>').trim().to_string();
        let standards = body == "html"
            || body.contains("xhtml")
            || (body.contains("html 4.01") && body.contains("w3.org/tr/html4/strict.dtd"))
            || (body.contains("html 4.01") && body.contains("transitional") && body.contains("w3.org/tr/html4/loose.dtd"))
            || body.starts_with("html system");
        if !self.fragment {
            self.doc.quirks = !standards;
        }
    }

    fn run(&mut self, source: &str) {
        let normalized;
        let mut input = source;
        if source.contains('\r') {
            normalized = source.replace("\r\n", "\n").replace('\r', "\n");
            input = &normalized;
        }
        let input = input.strip_prefix('\u{feff}').unwrap_or(input);
        let bytes = input.as_bytes();
        let mut pos = 0;
        let mut text_start = 0;
        macro_rules! flush_text {
            ($end:expr) => {{
                if $end > text_start {
                    let t = decode_entities(&input[text_start..$end], false);
                    self.process_text(&t);
                }
            }};
        }
        while pos < bytes.len() {
            if bytes[pos] != b'<' {
                pos += input[pos..].find('<').unwrap_or(input.len() - pos);
                continue;
            }
            let rest = &input[pos..];
            if rest.starts_with("<!--") {
                flush_text!(pos);
                pos += rest[4..].find("-->").map_or(rest.len(), |e| 4 + e + 3);
                text_start = pos;
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                flush_text!(pos);
                let end = rest.find("]]>").unwrap_or(rest.len());
                if self.in_foreign() {
                    let data = rest[9.min(end)..end].to_string();
                    self.process_text(&data);
                }
                pos += (end + 3).min(rest.len());
                text_start = pos;
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") {
                flush_text!(pos);
                let end = rest.find('>').map_or(rest.len(), |e| e + 1);
                if rest.len() >= 9 && rest[2..9].eq_ignore_ascii_case("doctype") {
                    self.doctype(&rest[..end]);
                }
                pos += end;
                text_start = pos;
                continue;
            }
            if rest.starts_with("</") {
                let next = rest.as_bytes().get(2).copied().unwrap_or(0);
                flush_text!(pos);
                if next.is_ascii_alphabetic() {
                    let end = find_tag_end(rest);
                    let name_end = rest[2..].find(|c: char| c.is_whitespace() || c == '>' || c == '/').map_or(rest.len(), |e| e + 2);
                    let name = rest[2..name_end.min(end + 1)].to_ascii_lowercase();
                    pos += (end + 1).min(rest.len());
                    text_start = pos;
                    self.process_end(name);
                } else if next == b'>' {
                    pos += 3;
                    text_start = pos;
                } else {
                    pos += rest.find('>').map_or(rest.len(), |e| e + 1);
                    text_start = pos;
                }
                continue;
            }
            if let Some((name, attrs, tail, self_closing)) = start_tag(rest) {
                flush_text!(pos);
                pos += rest.len() - tail.len();
                text_start = pos;
                self.pending_raw = None;
                self.process_start(name, attrs, self_closing);
                if let Some((raw, decode)) = self.pending_raw.take() {
                    let rest = &input[pos..];
                    let close = find_end_tag(rest, &raw);
                    let content = &rest[..close.unwrap_or(rest.len())];
                    let mut text = if decode { decode_entities(content, false) } else { content.to_string() };
                    if self.skip_newline {
                        self.skip_newline = false;
                        if let Some(t) = text.strip_prefix('\n') {
                            text = t.to_string();
                        }
                    }
                    if !text.is_empty() {
                        let cur = self.current();
                        let id = self.doc.push(NodeKind::Text(text), None);
                        self.insert_at(cur, id);
                    }
                    pos += content.len();
                    if close.is_some() {
                        let rest = &input[pos..];
                        pos += (find_tag_end(rest) + 1).min(rest.len());
                    }
                    text_start = pos;
                    self.process_end(raw);
                }
                continue;
            }
            // a lone '<' is text
            pos += 1;
        }
        flush_text!(input.len());
    }
}

/// Index of the `>` that ends the tag starting at `s[0] == '<'`, respecting quoted values.
fn find_tag_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 1;
    let mut quote = 0u8;
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == quote {
                quote = 0;
            }
        } else if (b == b'"' || b == b'\'') && bytes[i - 1] == b'=' {
            quote = b;
        } else if b == b'>' {
            return i;
        }
        i += 1;
    }
    bytes.len().saturating_sub(1)
}

fn start_tag(input: &str) -> Option<StartTag<'_>> {
    let bytes = input.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'<' || !bytes[1].is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'>' | b'/') {
        i += 1;
    }
    let tag = input[1..i].to_ascii_lowercase();
    let mut attributes: Attrs = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/' && bytes.get(i + 1) != Some(&b'>')) {
            i += 1;
        }
        if i >= bytes.len() {
            return Some((tag, attributes, "", false));
        }
        if bytes[i] == b'>' {
            return Some((tag, attributes, &input[i + 1..], false));
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'>') {
            return Some((tag, attributes, &input[i + 2..], true));
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>') && !(bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'>')) {
            i += 1;
        }
        if i == start {
            i += 1;
            continue;
        }
        let name = input[start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if let Some(&quote @ (b'\'' | b'"')) = bytes.get(i) {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = decode_entities(&input[start..i], true);
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = decode_entities(&input[start..i], true);
            }
        }
        if !attributes.iter().any(|(key, _)| key == &name) {
            attributes.push((name, value));
        }
    }
    Some((tag, attributes, "", false))
}

/// Find `</name` (ASCII case-insensitive) followed by a tag-name terminator.
fn find_end_tag(haystack: &str, name: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = name.as_bytes();
    let mut i = 0;
    while i + n.len() + 2 <= h.len() {
        if h[i] == b'<' && h[i + 1] == b'/' && h[i + 2..i + 2 + n.len()].eq_ignore_ascii_case(n) {
            let after = h.get(i + 2 + n.len()).copied().unwrap_or(b'>');
            if after.is_ascii_whitespace() || after == b'>' || after == b'/' {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

const LEGACY: &[&str] = &[
    "AElig", "AMP", "Aacute", "Acirc", "Agrave", "Aring", "Atilde", "Auml", "COPY", "Ccedil", "ETH", "Eacute", "Ecirc", "Egrave", "Euml",
    "GT", "Iacute", "Icirc", "Igrave", "Iuml", "LT", "Ntilde", "Oacute", "Ocirc", "Ograve", "Oslash", "Otilde", "Ouml", "QUOT", "REG",
    "THORN", "Uacute", "Ucirc", "Ugrave", "Uuml", "Yacute", "aacute", "acirc", "acute", "aelig", "agrave", "amp", "aring", "atilde", "auml",
    "brvbar", "ccedil", "cedil", "cent", "copy", "curren", "deg", "divide", "eacute", "ecirc", "egrave", "eth", "euml", "frac12", "frac14",
    "frac34", "gt", "iacute", "icirc", "iexcl", "igrave", "iquest", "iuml", "laquo", "lt", "macr", "micro", "middot", "nbsp", "not", "ntilde",
    "oacute", "ocirc", "ograve", "ordf", "ordm", "oslash", "otilde", "ouml", "para", "plusmn", "pound", "quot", "raquo", "reg", "sect", "shy",
    "sup1", "sup2", "sup3", "szlig", "thorn", "times", "uacute", "ucirc", "ugrave", "uml", "uuml", "yacute", "yen", "yuml",
];

fn windows_1252(n: u32) -> Option<char> {
    const T: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018,
        0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..0xa0).contains(&n) { char::from_u32(T[(n - 0x80) as usize]) } else { None }
}

/// Decode character references. In attributes (`in_attr`), a legacy reference without a
/// semicolon is left alone when followed by `=` or an alphanumeric.
pub fn decode_entities(input: &str, in_attr: bool) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index..];
        let after = &rest[1..];
        if let Some(num) = after.strip_prefix('#') {
            let (digits, radix, skip) = if let Some(h) = num.strip_prefix(['x', 'X']) {
                (h.chars().take_while(|c| c.is_ascii_hexdigit()).collect::<String>(), 16, 3)
            } else {
                (num.chars().take_while(|c| c.is_ascii_digit()).collect::<String>(), 10, 2)
            };
            if !digits.is_empty() {
                let n = u32::from_str_radix(&digits, radix).unwrap_or(0x110000);
                let c = windows_1252(n).unwrap_or_else(|| char::from_u32(n).filter(|&c| c != '\0').unwrap_or(char::REPLACEMENT_CHARACTER));
                output.push(c);
                let mut consumed = skip + digits.len();
                if rest[consumed..].starts_with(';') {
                    consumed += 1;
                }
                rest = &rest[consumed..];
                continue;
            }
            output.push('&');
            rest = &rest[1..];
            continue;
        }
        let name_len = after.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(after.len());
        let name = &after[..name_len];
        if name_len <= 32
            && after[name_len..].starts_with(';')
            && let Ok(i) = crate::entities::NAMED.binary_search_by(|e| e.0.cmp(name))
        {
            output.push_str(crate::entities::NAMED[i].1);
            rest = &after[name_len + 1..];
            continue;
        }
        // legacy names without a semicolon: longest prefix match
        let matched = (2..=name.len().min(6)).rev().find(|&l| LEGACY.contains(&&name[..l]));
        if let Some(l) = matched {
            let next = after[l..].chars().next();
            let blocked = in_attr && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric());
            if !blocked && let Ok(i) = crate::entities::NAMED.binary_search_by(|e| e.0.cmp(&name[..l])) {
                output.push_str(crate::entities::NAMED[i].1);
                rest = &after[l..];
                continue;
            }
        }
        output.push('&');
        rest = &rest[1..];
    }
    output.push_str(rest);
    output
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

fn escape_text(input: &str, out: &mut String) {
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(input: &str, out: &mut String) {
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            c => out.push(c),
        }
    }
}

fn serialize_into(doc: &Document, id: NodeId, out: &mut String, depth: usize) {
    if depth > 400 {
        return;
    }
    match &doc.nodes[id].kind {
        NodeKind::Text(t) => {
            let raw = doc.nodes[id]
                .parent
                .and_then(|p| doc.nodes[p].tag())
                .is_some_and(|t| matches!(t, "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "plaintext" | "noscript"));
            if raw {
                out.push_str(t);
            } else {
                escape_text(t, out);
            }
        }
        NodeKind::Comment(t) => {
            out.push_str("<!--");
            out.push_str(t);
            out.push_str("-->");
        }
        NodeKind::Document | NodeKind::Fragment => {
            for &c in &doc.nodes[id].children {
                serialize_into(doc, c, out, depth + 1);
            }
        }
        NodeKind::Element { tag, attributes } => {
            out.push('<');
            out.push_str(tag);
            for (k, v) in attributes {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                escape_attr(v, out);
                out.push('"');
            }
            out.push('>');
            if is_void(tag) {
                return;
            }
            for (i, &c) in doc.nodes[id].children.iter().enumerate() {
                if i == 0
                    && matches!(tag.as_str(), "pre" | "textarea" | "listing")
                    && let NodeKind::Text(t) = &doc.nodes[c].kind
                    && t.starts_with('\n')
                {
                    out.push('\n');
                }
                serialize_into(doc, c, out, depth + 1);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
    }
}

/// `outerHTML` of a node.
pub fn serialize_outer(doc: &Document, id: NodeId) -> String {
    let mut out = String::new();
    serialize_into(doc, id, &mut out, 0);
    out
}

/// `innerHTML` of a node.
pub fn serialize_inner(doc: &Document, id: NodeId) -> String {
    let mut out = String::new();
    for &c in &doc.nodes[id].children {
        serialize_into(doc, c, &mut out, 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(doc: &Document, id: NodeId) -> Vec<String> {
        doc.nodes[id].children.iter().map(|&c| doc.nodes[c].tag().map_or_else(|| "#text".to_string(), |t| t.to_string())).collect()
    }

    #[test]
    fn repairs_missing_paragraph_end_tags() {
        let doc = parse("<p>one<p>two");
        assert_eq!(tags(&doc, doc.body), ["p", "p"]);
    }

    #[test]
    fn parses_attributes_and_entities() {
        let doc = parse("<a href='a&amp;b'>A &#x26; B &copy 2024 &nbsp;x</a>");
        let a = doc.nodes[doc.body].children[0];
        assert_eq!(doc.nodes[a].attribute("href"), Some("a&b"));
        assert_eq!(doc.text_content(a).as_deref(), Some("A & B © 2024 \u{a0}x"));
    }

    #[test]
    fn preserves_raw_inline_script_text() {
        let doc = parse("<head><script>document.body.textContent = '<ok>';</script></head>");
        let script = doc.nodes[doc.head].children[0];
        assert_eq!(doc.nodes[script].tag(), Some("script"));
        assert_eq!(doc.text_content(script).as_deref(), Some("document.body.textContent = '<ok>';"));
    }

    #[test]
    fn accepts_utf8_text() {
        let doc = parse("<p>Привет, світ!</p>");
        let p = doc.nodes[doc.body].children[0];
        assert_eq!(doc.text_content(p).as_deref(), Some("Привет, світ!"));
    }

    #[test]
    fn decodes_long_and_multicodepoint_named_entities() {
        let doc = parse("<p>&CounterClockwiseContourIntegral; &NotEqualTilde;</p>");
        let p = doc.nodes[doc.body].children[0];
        let text = doc.text_content(p).unwrap();
        assert!(text.starts_with('∳'));
        assert!(text.chars().count() > 3);
    }

    #[test]
    fn nested_lists_and_implied_tbody() {
        let doc = parse("<ul><li>a<ul><li>b</li><li>c</li></ul></li><li>d</li></ul><table><tr><td>x<td>y<tr><td>z</table>");
        let ul = doc.nodes[doc.body].children[0];
        assert_eq!(tags(&doc, ul), ["li", "li"]);
        let li = doc.nodes[ul].children[0];
        assert_eq!(tags(&doc, li), ["#text", "ul"]);
        let table = doc.nodes[doc.body].children[1];
        assert_eq!(tags(&doc, table), ["tbody"]);
        let tbody = doc.nodes[table].children[0];
        assert_eq!(tags(&doc, tbody), ["tr", "tr"]);
        assert_eq!(tags(&doc, doc.nodes[tbody].children[0]), ["td", "td"]);
    }

    #[test]
    fn formatting_elements_are_reconstructed_and_foster_parented() {
        let doc = parse("<p><b>bold<p>still bold</b> plain</p><table>text<tr><td>c</td></tr></table>");
        let ps = &doc.nodes[doc.body].children;
        assert_eq!(doc.nodes[doc.nodes[ps[1]].children[0]].tag(), Some("b"));
        let table = ps.iter().position(|&c| doc.nodes[c].tag() == Some("table")).unwrap();
        assert_eq!(doc.nodes[ps[table - 1]].kind, NodeKind::Text("text".to_string()));
    }

    #[test]
    fn svg_attributes_and_slash_handling() {
        let doc = parse("<div/><svg viewbox='0 0 1 1'><circle r='1'/><linearGradient id=g></linearGradient></svg><span>after</span>");
        let div = doc.nodes[doc.body].children[0];
        assert_eq!(tags(&doc, div), ["svg", "span"]);
        let svg = doc.nodes[div].children[0];
        assert_eq!(doc.nodes[svg].attribute("viewBox"), Some("0 0 1 1"));
        assert_eq!(tags(&doc, svg), ["circle", "linearGradient"]);
    }

    #[test]
    fn serialization_round_trips() {
        let doc = parse("<div id=a class='x y'><p>1 &amp; 2</p><br><img src=a.png></div>");
        let div = doc.nodes[doc.body].children[0];
        assert_eq!(serialize_outer(&doc, div), "<div id=\"a\" class=\"x y\"><p>1 &amp; 2</p><br><img src=\"a.png\"></div>");
    }

    #[test]
    fn doctype_controls_quirks_mode() {
        assert!(!parse("<!DOCTYPE html><p>x").quirks);
        assert!(parse("<p>x").quirks);
    }

    #[test]
    fn head_body_and_rawtext() {
        let doc = parse("<title>T &amp; U</title><meta charset=utf-8><style>p{}</style><noscript><img src=x></noscript><p>x");
        assert_eq!(tags(&doc, doc.head), ["title", "meta", "style", "noscript"]);
        assert_eq!(tags(&doc, doc.body), ["p"]);
        assert_eq!(doc.text_content(doc.nodes[doc.head].children[0]).as_deref(), Some("T & U"));
    }
}
