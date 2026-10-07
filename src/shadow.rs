//! Shadow DOM: flattening of shadow trees (with slot distribution) and style scoping.
//!
//! Shadow trees live outside the light tree (`Document::shadow_roots` maps host → root fragment).
//! For rendering they are composed into a clone of the document in which each host's children are
//! the shadow root's children, `<slot>` elements contain their assigned light nodes, and every
//! element carries `data-cs-scope`/`data-cs-host` markers so scoped rules can target it.
use crate::dom::{Document, NodeId, NodeKind};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

pub struct Composed {
    pub doc: Document,
    /// Scoped CSS collected from `<style>` elements inside shadow roots.
    pub css: String,
}

fn is_slot(doc: &Document, id: NodeId) -> bool {
    doc.nodes[id].tag() == Some("slot")
}

/// Light-tree children of `host` assigned to the slot named `name` ("" = default slot).
pub fn assigned_nodes(doc: &Document, host: NodeId, name: &str) -> Vec<NodeId> {
    doc.nodes[host]
        .children
        .iter()
        .copied()
        .filter(|&c| match &doc.nodes[c].kind {
            NodeKind::Element { .. } => doc.nodes[c].attribute("slot").unwrap_or("") == name,
            NodeKind::Text(t) => name.is_empty() && !t.trim().is_empty(),
            _ => false,
        })
        .collect()
}

/// The shadow host whose root contains `node` (walking up through fragments), if any.
pub fn host_of(doc: &Document, node: NodeId) -> Option<NodeId> {
    let mut cur = node;
    loop {
        match doc.nodes[cur].parent {
            Some(p) => cur = p,
            None => break,
        }
    }
    doc.shadow_roots.iter().find(|e| *e.1 == cur).map(|e| *e.0)
}

pub fn compose(doc: &Document) -> Option<Composed> {
    if doc.shadow_roots.is_empty() {
        return None;
    }
    let mut d = doc.clone();
    let mut css = String::new();
    let mut counter = 0usize;
    // hosts that are reachable from the document root (directly or through other shadow trees)
    let hosts: Vec<(NodeId, NodeId)> = doc.shadow_roots.iter().map(|(&h, &r)| (h, r)).collect();
    for (host, root) in hosts {
        counter += 1;
        let n = counter;
        d.set_attribute(host, "data-cs-host", &n.to_string());
        // adopted style sheets come first, then <style> elements (document order)
        if let Some(text) = doc.adopted_css.get(&root) {
            css.push_str(&scope_css(text, n));
            css.push('\n');
        }
        // mark the shadow tree and collect its styles
        let mut stack: Vec<NodeId> = doc.nodes[root].children.clone();
        while let Some(id) = stack.pop() {
            if matches!(doc.nodes[id].kind, NodeKind::Element { .. }) {
                d.set_attribute(id, "data-cs-scope", &n.to_string());
                if doc.nodes[id].tag() == Some("style") {
                    let text = doc.text_content(id).unwrap_or_default();
                    css.push_str(&scope_css(&text, n));
                    css.push('\n');
                }
            }
            if doc.nodes[id].tag() != Some("template") {
                stack.extend(doc.nodes[id].children.iter().copied());
            }
        }
        // slot distribution inside this shadow tree
        let light: Vec<NodeId> = doc.nodes[host].children.clone();
        let top: Vec<NodeId> = doc.nodes[root].children.clone();
        let _ = light;
        d.nodes[host].children = top.clone();
        for &c in &top {
            d.nodes[c].parent = Some(host);
        }
        let mut stack = top;
        while let Some(id) = stack.pop() {
            if is_slot(doc, id) {
                let name = doc.nodes[id].attribute("name").unwrap_or("").to_string();
                let assigned = assigned_nodes(doc, host, &name);
                if !assigned.is_empty() {
                    for &a in &assigned {
                        d.nodes[a].parent = Some(id);
                    }
                    d.nodes[id].children = assigned;
                }
                // fallback content stays when nothing is assigned
            }
            if doc.nodes[id].tag() != Some("template") {
                stack.extend(d.nodes[id].children.clone());
            }
        }
    }
    Some(Composed { doc: d, css })
}

/// Rewrite the selectors of `css` so that they only match inside shadow tree `n`.
pub fn scope_css(css: &str, n: usize) -> String {
    let mut out = String::new();
    let mut rest = css;
    while !rest.trim().is_empty() {
        let Some(open) = find_top(rest, '{') else {
            out.push_str(rest);
            break;
        };
        let prelude = rest[..open].trim();
        let Some(close) = matching_brace(rest, open) else {
            out.push_str(rest);
            break;
        };
        let body = &rest[open + 1..close];
        if let Some(at) = prelude.strip_prefix('@') {
            let name: String = at.chars().take_while(|c| c.is_ascii_alphabetic() || *c == '-').collect();
            if matches!(name.as_str(), "media" | "supports" | "layer" | "container" | "document") {
                out.push_str(&format!("{prelude} {{ {} }}\n", scope_css(body, n)));
            } else {
                out.push_str(&format!("{prelude} {{ {body} }}\n"));
            }
        } else {
            let list: Vec<String> = split_selectors(prelude).iter().map(|s| scope_selector(s, n)).collect();
            out.push_str(&format!("{} {{ {} }}\n", list.join(", "), body));
        }
        rest = &rest[close + 1..];
    }
    out
}

fn find_top(s: &str, ch: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => depth += 1,
            ')' => depth -= 1,
            c2 if c2 == ch && depth <= 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in s[open..].char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_selectors(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim().to_string());
    out
}

fn replace_function(s: &str, name: &str, with: impl Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(name) {
        let after = &rest[i + name.len()..];
        out.push_str(&rest[..i]);
        if let Some(inner) = after.strip_prefix('(') {
            // find matching paren
            let mut depth = 1;
            let mut end = None;
            for (j, c) in inner.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match end {
                Some(e) => {
                    out.push_str(&with(&inner[..e]));
                    rest = &inner[e + 1..];
                    continue;
                }
                None => {
                    out.push_str(&rest[i..]);
                    return out;
                }
            }
        }
        out.push_str(&with(""));
        rest = after;
    }
    out.push_str(rest);
    out
}

fn scope_selector(sel: &str, n: usize) -> String {
    let host_attr = format!("[data-cs-host=\"{n}\"]");
    let scope_attr = format!("[data-cs-scope=\"{n}\"]");
    if sel.contains(":host") {
        let a = replace_function(sel, ":host-context", |_| host_attr.clone());
        let a = replace_function(&a, ":host", |inner| format!("{host_attr}{inner}"));
        return a;
    }
    if sel.contains("::slotted") {
        return replace_function(sel, "::slotted", |inner| inner.to_string());
    }
    // insert the scope attribute before a trailing pseudo-element
    match sel.rfind("::") {
        Some(i) => format!("{}{}{}", &sel[..i], scope_attr, &sel[i..]),
        None => format!("{sel}{scope_attr}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    #[test]
    fn selectors_are_scoped() {
        let css = scope_css(":host { display:block } :host(.x) b, p::before { color:red } @media (min-width:1px) { i { top:0 } } ::slotted(span) { margin:0 }", 3);
        assert!(css.contains("[data-cs-host=\"3\"] {"), "{css}");
        assert!(css.contains("[data-cs-host=\"3\"].x b"), "{css}");
        assert!(css.contains("p[data-cs-scope=\"3\"]::before"), "{css}");
        assert!(css.contains("i[data-cs-scope=\"3\"]"), "{css}");
        assert!(css.contains("span {"), "{css}");
    }

    #[test]
    fn slots_receive_light_children() {
        let mut doc = html::parse("<div id=h><span slot=a>A</span><b>def</b></div>");
        let host = doc.get_element_by_id("h").unwrap();
        let root = doc.push(NodeKind::Fragment, None);
        html::parse_fragment(&mut doc, root, "<p><slot name=a></slot></p><slot></slot>");
        doc.shadow_roots.insert(host, root);
        let c = compose(&doc).unwrap();
        let kids: Vec<_> = c.doc.nodes[c.doc.nodes[host].children[0]].children.clone();
        let slot_a = kids[0];
        assert_eq!(c.doc.nodes[c.doc.nodes[slot_a].children[0]].tag(), Some("span"));
    }
}
