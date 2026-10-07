//! HTML presentational attributes (`bgcolor`, `align`, `width`, ...) mapped to declarations.
use crate::{
    css::Declaration,
    dom::{Document, NodeId},
};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

fn decl(name: &str, value: String) -> Declaration {
    Declaration { name: name.to_string(), value, important: false }
}

fn dimension(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if let Some(p) = v.strip_suffix('%') {
        let n: f32 = p.trim().parse().ok()?;
        return Some(format!("{n}%"));
    }
    let digits: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let n: f32 = digits.parse().ok()?;
    Some(format!("{n}px"))
}

fn legacy_color(value: &str) -> String {
    let v = value.trim();
    // Bare hex without '#', as old pages write it.
    if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{v}")
    } else {
        v.to_string()
    }
}

pub fn hints(doc: &Document, id: NodeId) -> Vec<Declaration> {
    let node = &doc.nodes[id];
    let mut out = Vec::new();
    let Some(tag) = node.tag() else { return out };
    let attr = |n: &str| node.attribute(n);
    match tag {
        "body" => {
            if let Some(c) = attr("bgcolor") {
                out.push(decl("background-color", legacy_color(c)));
            }
            if let Some(c) = attr("text") {
                out.push(decl("color", legacy_color(c)));
            }
            if let Some(m) = attr("marginwidth").or(attr("leftmargin")) {
                if let Some(d) = dimension(m) {
                    out.push(decl("margin-left", d.clone()));
                    out.push(decl("margin-right", d));
                }
            }
            if let Some(m) = attr("marginheight").or(attr("topmargin")) {
                if let Some(d) = dimension(m) {
                    out.push(decl("margin-top", d.clone()));
                    out.push(decl("margin-bottom", d));
                }
            }
        }
        "table" => {
            if let Some(c) = attr("bgcolor") {
                out.push(decl("background-color", legacy_color(c)));
            }
            if let Some(w) = attr("width").and_then(dimension) {
                out.push(decl("width", w));
            }
            if let Some(h) = attr("height").and_then(dimension) {
                out.push(decl("height", h));
            }
            if let Some(b) = attr("border") {
                let n: f32 = b.trim().parse().unwrap_or(if b.trim().is_empty() { 1.0 } else { 0.0 });
                if n > 0.0 {
                    out.push(decl("border", format!("{n}px outset #808080")));
                }
            }
            if let Some(s) = attr("cellspacing").and_then(dimension) {
                out.push(decl("border-spacing", s));
            }
            if let Some(a) = attr("align") {
                match a.to_ascii_lowercase().as_str() {
                    "center" => {
                        out.push(decl("margin-left", "auto".into()));
                        out.push(decl("margin-right", "auto".into()));
                    }
                    "left" => out.push(decl("float", "left".into())),
                    "right" => out.push(decl("float", "right".into())),
                    _ => {}
                }
            }
        }
        "td" | "th" => {
            if let Some(c) = attr("bgcolor") {
                out.push(decl("background-color", legacy_color(c)));
            }
            if let Some(w) = attr("width").and_then(dimension) {
                out.push(decl("width", w));
            }
            if let Some(h) = attr("height").and_then(dimension) {
                out.push(decl("height", h));
            }
            if let Some(a) = attr("align") {
                out.push(decl("text-align", a.to_ascii_lowercase()));
            }
            if let Some(a) = attr("valign") {
                out.push(decl("vertical-align", a.to_ascii_lowercase()));
            }
            if attr("nowrap").is_some() {
                out.push(decl("white-space", "nowrap".into()));
            }
            // cellpadding / border from the owning table
            let mut cur = node.parent;
            while let Some(p) = cur {
                if doc.nodes[p].tag() == Some("table") {
                    let t = &doc.nodes[p];
                    if let Some(pad) = t.attribute("cellpadding").and_then(dimension) {
                        out.push(decl("padding", pad));
                    }
                    if let Some(b) = t.attribute("border") {
                        let n: f32 = b.trim().parse().unwrap_or(if b.trim().is_empty() { 1.0 } else { 0.0 });
                        if n > 0.0 {
                            out.push(decl("border", "1px inset #808080".into()));
                        }
                    }
                    break;
                }
                cur = doc.nodes[p].parent;
            }
        }
        "tr" | "thead" | "tbody" | "tfoot" => {
            if let Some(c) = attr("bgcolor") {
                out.push(decl("background-color", legacy_color(c)));
            }
            if let Some(a) = attr("align") {
                out.push(decl("text-align", a.to_ascii_lowercase()));
            }
            if let Some(a) = attr("valign") {
                out.push(decl("vertical-align", a.to_ascii_lowercase()));
            }
        }
        "img" | "iframe" | "canvas" | "video" | "embed" | "object" | "input" | "svg" => {
            if let Some(w) = attr("width").and_then(dimension) {
                if !(tag == "input" && attr("type").is_some_and(|t| t != "image")) {
                    out.push(decl("width", w));
                }
            }
            if let Some(h) = attr("height").and_then(dimension) {
                if !(tag == "input" && attr("type").is_some_and(|t| t != "image")) {
                    out.push(decl("height", h));
                }
            }
            if let Some(a) = attr("align") {
                match a.to_ascii_lowercase().as_str() {
                    "left" => out.push(decl("float", "left".into())),
                    "right" => out.push(decl("float", "right".into())),
                    "middle" | "absmiddle" => out.push(decl("vertical-align", "middle".into())),
                    "top" => out.push(decl("vertical-align", "top".into())),
                    "bottom" => out.push(decl("vertical-align", "baseline".into())),
                    _ => {}
                }
            }
            if let Some(b) = attr("border").and_then(dimension) {
                out.push(decl("border", format!("{b} solid")));
            }
            if let Some(h) = attr("hspace").and_then(dimension) {
                out.push(decl("margin-left", h.clone()));
                out.push(decl("margin-right", h));
            }
            if let Some(v) = attr("vspace").and_then(dimension) {
                out.push(decl("margin-top", v.clone()));
                out.push(decl("margin-bottom", v));
            }
        }
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "header" | "footer" | "nav"
        | "ul" | "ol" | "li" | "dl" | "dd" | "dt" | "blockquote" | "pre" | "form" | "fieldset" | "legend"
        | "caption" => {
            if let Some(a) = attr("align") {
                match a.to_ascii_lowercase().as_str() {
                    "center" | "middle" => out.push(decl("text-align", "center".into())),
                    "right" => out.push(decl("text-align", "right".into())),
                    "left" => out.push(decl("text-align", "left".into())),
                    "justify" => out.push(decl("text-align", "justify".into())),
                    _ => {}
                }
            }
            if tag == "ol" || tag == "ul" || tag == "li" {
                if let Some(t) = attr("type") {
                    let v = match t {
                        "1" => "decimal",
                        "a" => "lower-alpha",
                        "A" => "upper-alpha",
                        "i" => "lower-roman",
                        "I" => "upper-roman",
                        "disc" | "circle" | "square" => t,
                        _ => "",
                    };
                    if !v.is_empty() {
                        out.push(decl("list-style-type", v.into()));
                    }
                }
            }
        }
        "font" => {
            if let Some(c) = attr("color") {
                out.push(decl("color", legacy_color(c)));
            }
            if let Some(f) = attr("face") {
                out.push(decl("font-family", f.to_string()));
            }
            if let Some(s) = attr("size") {
                let s = s.trim();
                let n = if let Some(rel) = s.strip_prefix('+') {
                    3 + rel.parse::<i32>().unwrap_or(0)
                } else if let Some(rel) = s.strip_prefix('-') {
                    3 - rel.parse::<i32>().unwrap_or(0)
                } else {
                    s.parse::<i32>().unwrap_or(3)
                };
                let kw = match n.clamp(1, 7) {
                    1 => "x-small",
                    2 => "small",
                    3 => "medium",
                    4 => "large",
                    5 => "x-large",
                    6 => "xx-large",
                    _ => "xxx-large",
                };
                out.push(decl("font-size", kw.into()));
            }
        }
        "a" => {
            // link/vlink/alink on body handled by authors via CSS in modern pages.
        }
        "hr" => {
            if let Some(w) = attr("width").and_then(dimension) {
                out.push(decl("width", w));
            }
            if let Some(s) = attr("size").and_then(dimension) {
                out.push(decl("height", s));
            }
            if let Some(a) = attr("align") {
                if a.eq_ignore_ascii_case("left") {
                    out.push(decl("margin-left", "0".into()));
                } else if a.eq_ignore_ascii_case("right") {
                    out.push(decl("margin-right", "0".into()));
                }
            }
        }
        "col" | "colgroup" => {
            if let Some(w) = attr("width").and_then(dimension) {
                out.push(decl("width", w));
            }
        }
        _ => {}
    }
    out
}
