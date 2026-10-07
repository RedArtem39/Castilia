//! Reflected attributes, classList, dataset, style, form controls, focus/click and default actions.
use super::{builtins::*, dom::*, dom_setup::*, interp::*, url::Url, value::*};
use crate::{
    dom::{NodeId, NodeKind},
    html,
};
use alloc::{
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub fn setup(it: &mut Interp) {
    let he = it.protos().html_element.clone();
    let el = it.protos().element.clone();
    setup_reflection(it, &he);
    setup_class_list(it, &el);
    setup_dataset_and_style(it, &he);
    setup_controls(it, &he);
    setup_custom_elements(it);
}

// ---------------------------------------------------------------------------
// Reflection
// ---------------------------------------------------------------------------

fn reflect(it: &mut Interp, proto: &Obj, prop: &str, attr: &str, kind: char, default: &str) {
    let data = Value::string(format!("{kind}|{attr}|{default}"));
    let g = it.new_native_data(prop, 0, reflect_get, data.clone(), false);
    let s = it.new_native_data(prop, 1, reflect_set, data, false);
    it.define_accessor_part(proto, Key::str(prop), Some(g), true, CONFIGURABLE);
    it.define_accessor_part(proto, Key::str(prop), Some(s), false, CONFIGURABLE);
}

fn parts(it: &mut Interp, c: &Call) -> (char, String, String) {
    let d = it.native_data(c);
    let s = it.to_string_lossy(&d);
    let mut sp = s.splitn(3, '|');
    let k = sp.next().unwrap_or("s").chars().next().unwrap_or('s');
    (k, sp.next().unwrap_or("").to_string(), sp.next().unwrap_or("").to_string())
}

fn reflect_get(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let (kind, attr, default) = parts(it, c);
    let v = it.attr(id, &attr);
    Ok(match kind {
        'b' => Value::Bool(v.is_some()),
        'n' => Value::Num(v.and_then(|s| s.trim().parse::<f64>().ok().map(crate::math::trunc)).unwrap_or_else(|| default.parse().unwrap_or(0.0))),
        'u' => match v {
            Some(s) => Value::string(it.url_resolve(&s)),
            None => Value::str(""),
        },
        't' => {
            // enumerated, lowercased with default
            let s = v.map(|s| s.to_ascii_lowercase()).unwrap_or_default();
            Value::string(if s.is_empty() { default } else { s })
        }
        _ => Value::string(v.unwrap_or(default)),
    })
}

fn reflect_set(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let (kind, attr, _) = parts(it, c);
    match kind {
        'b' => {
            if c.arg(0).truthy() {
                it.dom_set_attr(id, &attr, "")?;
            } else {
                it.dom_remove_attr(id, &attr)?;
            }
        }
        _ => {
            let s = it.arg_str(c, 0)?;
            it.dom_set_attr(id, &attr, &s)?;
        }
    }
    it.attribute_changed(id, &attr)?;
    Ok(Value::Undefined)
}

fn setup_reflection(it: &mut Interp, p: &Obj) {
    for (prop, attr) in [
        ("id", "id"),
        ("className", "class"),
        ("title", "title"),
        ("lang", "lang"),
        ("name", "name"),
        ("placeholder", "placeholder"),
        ("alt", "alt"),
        ("rel", "rel"),
        ("target", "target"),
        ("content", "content"),
        ("charset", "charset"),
        ("media", "media"),
        ("htmlFor", "for"),
        ("accept", "accept"),
        ("autocomplete", "autocomplete"),
        ("enctype", "enctype"),
        ("label", "label"),
        ("pattern", "pattern"),
        ("min", "min"),
        ("max", "max"),
        ("step", "step"),
        ("crossOrigin", "crossorigin"),
        ("integrity", "integrity"),
        ("loading", "loading"),
        ("srcset", "srcset"),
        ("sizes", "sizes"),
        ("download", "download"),
        ("accessKey", "accesskey"),
        ("slot", "slot"),
        ("nonce", "nonce"),
        ("referrerPolicy", "referrerpolicy"),
        ("hreflang", "hreflang"),
        ("httpEquiv", "http-equiv"),
        ("wrap", "wrap"),
        ("scope", "scope"),
        ("headers", "headers"),
        ("abbr", "abbr"),
        ("coords", "coords"),
        ("shape", "shape"),
        ("useMap", "usemap"),
        ("inputMode", "inputmode"),
        ("enterKeyHint", "enterkeyhint"),
        ("ariaLabel", "aria-label"),
        ("ariaHidden", "aria-hidden"),
        ("ariaExpanded", "aria-expanded"),
        ("role", "role"),
        ("fetchPriority", "fetchpriority"),
        ("decoding", "decoding"),
        ("align", "align"),
        ("bgColor", "bgcolor"),
        ("color", "color"),
        ("face", "face"),
        ("vAlign", "valign"),
        ("htmlWidth", "width"),
    ] {
        reflect(it, p, prop, attr, 's', "");
    }
    reflect(it, p, "dir", "dir", 't', "");
    for (prop, attr) in [("href", "href"), ("src", "src"), ("poster", "poster"), ("action", "action"), ("formAction", "formaction"), ("cite", "cite"), ("longDesc", "longdesc")] {
        reflect(it, p, prop, attr, 'u', "");
    }
    for (prop, attr) in [
        ("hidden", "hidden"),
        ("disabled", "disabled"),
        ("readOnly", "readonly"),
        ("required", "required"),
        ("multiple", "multiple"),
        ("autofocus", "autofocus"),
        ("open", "open"),
        ("async", "async"),
        ("defer", "defer"),
        ("noModule", "nomodule"),
        ("controls", "controls"),
        ("loop", "loop"),
        ("muted", "muted"),
        ("autoplay", "autoplay"),
        ("noValidate", "novalidate"),
        ("formNoValidate", "formnovalidate"),
        ("reversed", "reversed"),
        ("default", "default"),
        ("playsInline", "playsinline"),
        ("inert", "inert"),
        ("isMap", "ismap"),
        ("defaultChecked", "checked"),
        ("defaultSelected", "selected"),
        ("spellcheck", "spellcheck"),
        ("draggable", "draggable"),
    ] {
        reflect(it, p, prop, attr, 'b', "");
    }
    for (prop, attr, d) in [("tabIndex", "tabindex", "-1"), ("maxLength", "maxlength", "-1"), ("minLength", "minlength", "-1"), ("size", "size", "0"), ("rows", "rows", "2"), ("cols", "cols", "20"), ("colSpan", "colspan", "1"), ("rowSpan", "rowspan", "1"), ("span", "span", "1"), ("start", "start", "1")] {
        reflect(it, p, prop, attr, 'n', d);
    }
    // type: input defaults to "text", button to "submit"
    it.def_accessor(
        p,
        "type",
        |it, c| {
            let id = it.this_node(c)?;
            let tag = it.tag_name(id);
            let v = it.attr(id, "type");
            Ok(Value::string(match (tag.as_str(), v) {
                ("input", Some(t)) => {
                    let t = t.to_ascii_lowercase();
                    if matches!(t.as_str(), "text" | "password" | "checkbox" | "radio" | "submit" | "reset" | "button" | "file" | "hidden" | "image" | "email" | "url" | "tel" | "search" | "number" | "range" | "color" | "date" | "datetime-local" | "month" | "week" | "time") { t } else { "text".to_string() }
                }
                ("input", None) => "text".to_string(),
                ("button", Some(t)) => {
                    let t = t.to_ascii_lowercase();
                    if matches!(t.as_str(), "button" | "reset") { t } else { "submit".to_string() }
                }
                ("button", None) => "submit".to_string(),
                ("select", _) => if it.attr(id, "multiple").is_some() { "select-multiple".into() } else { "select-one".into() },
                ("textarea", _) => "textarea".into(),
                (_, Some(t)) => t,
                _ => String::new(),
            }))
        },
        |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            it.dom_set_attr(id, "type", &s)?;
            Ok(Value::Undefined)
        },
    );
    // img/canvas/iframe numeric width/height
    for (prop, attr) in [("width", "width"), ("height", "height")] {
        let data = Value::string(format!("w|{attr}|"));
        let g = it.new_native_data(prop, 0, dim_get, data.clone(), false);
        let s = it.new_native_data(prop, 1, dim_set, data, false);
        it.define_accessor_part(p, Key::str(prop), Some(g), true, CONFIGURABLE);
        it.define_accessor_part(p, Key::str(prop), Some(s), false, CONFIGURABLE);
    }
    for (prop, idx) in [("naturalWidth", 2), ("naturalHeight", 3)] {
        let _ = idx;
        it.def_getter(p, prop, |_, _| Ok(Value::Num(0.0)));
    }
    it.def_getter(p, "complete", |_, _| Ok(Value::Bool(true)));
    it.def_getter(p, "currentSrc", |it, c| {
        let id = it.this_node(c)?;
        let s = it.attr(id, "src").unwrap_or_default();
        Ok(Value::string(if s.is_empty() { s } else { it.url_resolve(&s) }))
    });
    it.def_accessor(
        p,
        "contentEditable",
        |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::string(it.attr(id, "contenteditable").map(|v| if v.is_empty() { "true".to_string() } else { v }).unwrap_or_else(|| "inherit".to_string())))
        },
        |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            it.dom_set_attr(id, "contenteditable", &s)?;
            Ok(Value::Undefined)
        },
    );
    it.def_getter(p, "isContentEditable", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Bool(it.attr(id, "contenteditable").is_some_and(|v| v.is_empty() || v == "true")))
    });
    for (prop, kind) in [("innerText", 0), ("outerText", 1)] {
        let _ = kind;
        it.def_accessor(
            p,
            prop,
            |it, c| {
                let id = it.this_node(c)?;
                Ok(Value::string(inner_text(it, id)))
            },
            |it, c| {
                let id = it.this_node(c)?;
                let s = it.arg_str(c, 0)?;
                // newlines become <br>
                let parts: Vec<&str> = s.split('\n').collect();
                let d = it.dom()?;
                let old = d.nodes[id].children.clone();
                for o in old {
                    it.dom_remove(o)?;
                }
                for (i, part) in parts.iter().enumerate() {
                    if i > 0 {
                        let br = it.dom()?.create_element("br");
                        it.dom_insert(id, br, None)?;
                    }
                    if !part.is_empty() {
                        let t = it.dom()?.push(NodeKind::Text(part.to_string()), None);
                        it.dom_insert(id, t, None)?;
                    }
                }
                Ok(Value::Undefined)
            },
        );
    }
    // anchor/location-like URL parts for <a>, <area>
    for part in ["origin", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "username", "password"] {
        let data = Value::str(part);
        let g = it.new_native_data(part, 0, url_part_get, data.clone(), false);
        let s = it.new_native_data(part, 1, url_part_set, data, false);
        it.define_accessor_part(p, Key::str(part), Some(g), true, CONFIGURABLE);
        it.define_accessor_part(p, Key::str(part), Some(s), false, CONFIGURABLE);
    }
    it.def_method(p, "toString", 0, |it, c| {
        let id = it.this_node(c)?;
        let t = it.tag_name(id);
        if t == "a" || t == "area" {
            let h = it.attr(id, "href").unwrap_or_default();
            return Ok(Value::string(if h.is_empty() { h } else { it.url_resolve(&h) }));
        }
        let ctor = match t.as_str() {
            "div" => "HTMLDivElement",
            "span" => "HTMLSpanElement",
            "img" => "HTMLImageElement",
            "input" => "HTMLInputElement",
            "body" => "HTMLBodyElement",
            "button" => "HTMLButtonElement",
            "form" => "HTMLFormElement",
            "p" => "HTMLParagraphElement",
            "ul" => "HTMLUListElement",
            "li" => "HTMLLIElement",
            _ => "HTMLElement",
        };
        Ok(Value::string(format!("[object {ctor}]")))
    });
}

fn inner_text(it: &mut Interp, id: NodeId) -> String {
    let Some(d) = it.doc() else { return String::new() };
    fn walk(d: &crate::dom::Document, id: NodeId, out: &mut String) {
        match &d.nodes[id].kind {
            NodeKind::Text(t) => out.push_str(t),
            NodeKind::Element { tag, attributes } => {
                if matches!(tag.as_str(), "script" | "style" | "template" | "head") || attributes.iter().any(|(k, v)| k == "hidden" && v != "until-found") {
                    return;
                }
                if tag == "br" {
                    out.push('\n');
                    return;
                }
                let block = matches!(tag.as_str(), "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "ul" | "ol" | "table" | "tr" | "section" | "article" | "header" | "footer" | "nav" | "blockquote" | "pre" | "form" | "fieldset" | "dl" | "dt" | "dd" | "main" | "aside" | "figure" | "hr");
                if block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                for &c in &d.nodes[id].children {
                    walk(d, c, out);
                }
                if matches!(tag.as_str(), "td" | "th") {
                    out.push('\t');
                }
                if block && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {
                for &c in &d.nodes[id].children {
                    walk(d, c, out);
                }
            }
        }
    }
    let mut raw = String::new();
    walk(d, id, &mut raw);
    // collapse whitespace like normal flow (keep newlines produced by blocks)
    let mut out = String::new();
    let mut prev_space = true;
    for ch in raw.chars() {
        if ch == '\n' {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            prev_space = true;
        } else if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim_matches(|c| c == '\n' || c == ' ').to_string()
}

fn dim_get(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let (_, attr, _) = parts(it, c);
    let tag = it.tag_name(id);
    if matches!(tag.as_str(), "img" | "canvas" | "iframe" | "video" | "svg" | "embed" | "object") {
        if let Some(v) = it.attr(id, &attr).and_then(|s| s.trim().trim_end_matches("px").parse::<f64>().ok()) {
            return Ok(Value::Num(v));
        }
        if let Some(r) = it.dom.rects.get(&id) {
            return Ok(Value::Num(crate::math::round(r[if attr == "width" { 2 } else { 3 }] as f64)));
        }
        return Ok(Value::Num(if tag == "canvas" { if attr == "width" { 300.0 } else { 150.0 } } else { 0.0 }));
    }
    Ok(Value::string(it.attr(id, &attr).unwrap_or_default()))
}

fn dim_set(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let (_, attr, _) = parts(it, c);
    let s = it.arg_str(c, 0)?;
    it.dom_set_attr(id, &attr, &s)?;
    Ok(Value::Undefined)
}

fn element_url(it: &mut Interp, id: NodeId) -> Option<Url> {
    let h = it.attr(id, "href")?;
    let abs = it.url_resolve(&h);
    Url::parse(&abs, None)
}

fn url_part_get(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let part = it.to_string_lossy(&it.native_data(c));
    let tag = it.tag_name(id);
    if tag != "a" && tag != "area" {
        return Ok(if matches!(part.as_str(), "origin") { Value::Undefined } else { Value::Undefined });
    }
    let Some(u) = element_url(it, id) else { return Ok(Value::str("")) };
    Ok(Value::string(url_component(&u, &part)))
}

pub fn url_component(u: &Url, part: &str) -> String {
    match part {
        "origin" => u.origin(),
        "protocol" => format!("{}:", u.scheme),
        "host" => u.host_with_port(),
        "hostname" => u.host.clone(),
        "port" => u.port.clone(),
        "pathname" => u.path.clone(),
        "search" => u.search(),
        "hash" => u.hash(),
        "username" => u.username.clone(),
        "password" => u.password.clone(),
        _ => String::new(),
    }
}

fn url_part_set(it: &mut Interp, c: &Call) -> Res {
    let id = it.this_node(c)?;
    let part = it.to_string_lossy(&it.native_data(c));
    let v = it.arg_str(c, 0)?;
    let Some(mut u) = element_url(it, id) else { return Ok(Value::Undefined) };
    set_url_component(&mut u, &part, &v);
    it.dom_set_attr(id, "href", &u.href())?;
    Ok(Value::Undefined)
}

pub fn set_url_component(u: &mut Url, part: &str, v: &str) {
    match part {
        "protocol" => u.scheme = v.trim_end_matches(':').to_ascii_lowercase(),
        "host" => {
            let (h, p) = v.rsplit_once(':').unwrap_or((v, ""));
            u.host = h.to_ascii_lowercase();
            u.port = p.to_string();
        }
        "hostname" => u.host = v.to_ascii_lowercase(),
        "port" => u.port = v.to_string(),
        "pathname" => {
            let p = if v.starts_with('/') { v.to_string() } else { format!("/{v}") };
            if let Some(n) = Url::parse(&format!("{}://x{}", u.scheme, p), None) {
                u.path = n.path;
            }
        }
        "search" => u.query = if v.is_empty() { None } else { Some(v.trim_start_matches('?').to_string()) },
        "hash" => u.fragment = if v.is_empty() { None } else { Some(v.trim_start_matches('#').to_string()) },
        "username" => u.username = v.to_string(),
        "password" => u.password = v.to_string(),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// classList / relList
// ---------------------------------------------------------------------------

fn token_list(it: &mut Interp, id: NodeId, attr: &str) -> Value {
    let proto = it.protos().extra.get("domtokenlist").cloned().unwrap();
    let o = it.alloc(Object::new(Some(proto), Class::Plain));
    it.define_data(&o, Key::str("__id"), Value::Num(id as f64), 0);
    it.define_data(&o, Key::str("__attr"), Value::str(attr), 0);
    Value::Obj(o)
}

fn tl_parts(it: &mut Interp, c: &Call) -> Res<(NodeId, String, Vec<String>)> {
    let id = match it.get_str(&c.this, "__id")? {
        Value::Num(n) => n as usize,
        _ => return it.type_error("Illegal invocation"),
    };
    let attr = it.get_str(&c.this, "__attr")?;
    let attr = it.to_string_lossy(&attr);
    let toks: Vec<String> = it.attr(id, &attr).unwrap_or_default().split_whitespace().map(|s| s.to_string()).collect();
    Ok((id, attr, toks))
}

fn tl_write(it: &mut Interp, id: NodeId, attr: &str, toks: &[String], old_empty: bool) -> Res<()> {
    let s = toks.join(" ");
    if toks.is_empty() && old_empty {
        return Ok(());
    }
    it.dom_set_attr(id, attr, &s)
}

fn check_token(it: &mut Interp, t: &str) -> Res<()> {
    if t.is_empty() {
        return Err(it.error_ctl("SyntaxError", "The token provided must not be empty."));
    }
    if t.contains(char::is_whitespace) {
        return Err(it.error_ctl("InvalidCharacterError", "The token provided contains HTML space characters, which are not valid in tokens."));
    }
    Ok(())
}

fn setup_class_list(it: &mut Interp, el: &Obj) {
    let proto = it.new_object();
    it.protos.as_mut().unwrap().extra.insert("domtokenlist", proto.clone());
    it.def_method(&proto, "contains", 1, |it, c| {
        let (_, _, toks) = tl_parts(it, c)?;
        let t = it.arg_str(c, 0)?;
        Ok(Value::Bool(toks.iter().any(|x| **x == *t)))
    });
    it.def_method(&proto, "add", 1, |it, c| {
        let (id, attr, mut toks) = tl_parts(it, c)?;
        let old_empty = toks.is_empty();
        let mut changed = false;
        for a in c.args {
            let t = it.to_str(a)?;
            check_token(it, &t)?;
            if !toks.iter().any(|x| **x == *t) {
                toks.push(t.to_string());
                changed = true;
            }
        }
        if changed || (old_empty && false) {
            tl_write(it, id, &attr, &toks, old_empty)?;
        }
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "remove", 1, |it, c| {
        let (id, attr, mut toks) = tl_parts(it, c)?;
        let before = toks.len();
        for a in c.args {
            let t = it.to_str(a)?;
            check_token(it, &t)?;
            toks.retain(|x| **x != *t);
        }
        if toks.len() != before || it.attr(id, &attr).is_some_and(|s| s.trim().is_empty() && !s.is_empty()) {
            tl_write(it, id, &attr, &toks, false)?;
        }
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "toggle", 1, |it, c| {
        let (id, attr, mut toks) = tl_parts(it, c)?;
        let t = it.arg_str(c, 0)?;
        check_token(it, &t)?;
        let has = toks.iter().any(|x| **x == *t);
        let want = if c.args.len() > 1 && !c.arg(1).is_undefined() { c.arg(1).truthy() } else { !has };
        if want && !has {
            toks.push(t.to_string());
            tl_write(it, id, &attr, &toks, false)?;
        } else if !want && has {
            toks.retain(|x| **x != *t);
            tl_write(it, id, &attr, &toks, false)?;
        }
        Ok(Value::Bool(want))
    });
    it.def_method(&proto, "replace", 2, |it, c| {
        let (id, attr, mut toks) = tl_parts(it, c)?;
        let old = it.arg_str(c, 0)?;
        let new = it.arg_str(c, 1)?;
        match toks.iter().position(|x| **x == *old) {
            Some(i) => {
                if toks.iter().any(|x| **x == *new) {
                    toks.remove(i);
                } else {
                    toks[i] = new.to_string();
                }
                tl_write(it, id, &attr, &toks, false)?;
                Ok(Value::Bool(true))
            }
            None => Ok(Value::Bool(false)),
        }
    });
    it.def_method(&proto, "item", 1, |it, c| {
        let (_, _, toks) = tl_parts(it, c)?;
        let i = to_integer(it.arg_num(c, 0)?);
        Ok(if i >= 0.0 { toks.get(i as usize).map_or(Value::Null, |s| Value::str(s)) } else { Value::Null })
    });
    it.def_method(&proto, "supports", 1, |_, _| Ok(Value::Bool(true)));
    it.def_method(&proto, "toString", 0, |it, c| {
        let (id, attr, _) = tl_parts(it, c)?;
        Ok(Value::string(it.attr(id, &attr).unwrap_or_default()))
    });
    it.def_method(&proto, "forEach", 1, |it, c| {
        let (_, _, toks) = tl_parts(it, c)?;
        for (i, t) in toks.iter().enumerate() {
            it.call(&c.arg(0), c.arg(1), &[Value::str(t), Value::Num(i as f64), c.this.clone()])?;
        }
        Ok(Value::Undefined)
    });
    it.def_getter(&proto, "length", |it, c| {
        let (_, _, toks) = tl_parts(it, c)?;
        Ok(Value::Num(toks.len() as f64))
    });
    it.def_accessor(
        &proto,
        "value",
        |it, c| {
            let (id, attr, _) = tl_parts(it, c)?;
            Ok(Value::string(it.attr(id, &attr).unwrap_or_default()))
        },
        |it, c| {
            let (id, attr, _) = tl_parts(it, c)?;
            let s = it.arg_str(c, 0)?;
            it.dom_set_attr(id, &attr, &s)?;
            Ok(Value::Undefined)
        },
    );
    for (name, kind) in [("values", 1u8), ("keys", 0), ("entries", 2)] {
        let f: NativeFn = match kind {
            0 => |it, c| tl_iter(it, c, 0),
            1 => |it, c| tl_iter(it, c, 1),
            _ => |it, c| tl_iter(it, c, 2),
        };
        it.def_method(&proto, name, 0, f);
    }
    let iter_sym = it.wk().iterator.clone();
    it.def_sym_method(&proto, &iter_sym, "[Symbol.iterator]", 0, |it, c| tl_iter(it, c, 1));
    it.def_accessor(
        el,
        "classList",
        |it, c| {
            let id = it.this_node(c)?;
            Ok(token_list(it, id, "class"))
        },
        |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            it.dom_set_attr(id, "class", &s)
        .map(|_| Value::Undefined)
        },
    );
    it.def_getter(el, "relList", |it, c| {
        let id = it.this_node(c)?;
        Ok(token_list(it, id, "rel"))
    });
}

fn tl_iter(it: &mut Interp, c: &Call, kind: u8) -> Res {
    let (_, _, toks) = tl_parts(it, c)?;
    let items: Vec<Value> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| match kind {
            0 => Value::Num(i as f64),
            1 => Value::str(t),
            _ => it.new_array(vec![Value::Num(i as f64), Value::str(t)]),
        })
        .collect();
    Ok(it.make_list_iterator(items))
}

// ---------------------------------------------------------------------------
// style / dataset
// ---------------------------------------------------------------------------

pub fn parse_decls(s: &str) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut depth, mut quote) = (0i32, None::<char>);
    let mut flush = |cur: &mut String, out: &mut Vec<(String, String, bool)>| {
        if let Some((k, v)) = cur.split_once(':') {
            let k = k.trim();
            let mut v = v.trim().to_string();
            let mut important = false;
            let lower = v.to_ascii_lowercase();
            if let Some(i) = lower.rfind("!important") {
                v.truncate(i);
                v = v.trim().to_string();
                important = true;
            }
            if !k.is_empty() {
                let key = if k.starts_with("--") { k.to_string() } else { k.to_ascii_lowercase() };
                if let Some(e) = out.iter_mut().find(|e: &&mut (String, String, bool)| e.0 == key) {
                    e.1 = v;
                    e.2 = important;
                } else {
                    out.push((key, v, important));
                }
            }
        }
        cur.clear();
    };
    for ch in s.chars() {
        match quote {
            Some(q) => {
                cur.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                '(' => {
                    depth += 1;
                    cur.push(ch);
                }
                ')' => {
                    depth -= 1;
                    cur.push(ch);
                }
                ';' if depth <= 0 => flush(&mut cur, &mut out),
                _ => cur.push(ch),
            },
        }
    }
    flush(&mut cur, &mut out);
    out
}

pub fn serialize_decls(d: &[(String, String, bool)]) -> String {
    let mut s = String::new();
    for (k, v, imp) in d {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(k);
        s.push_str(": ");
        s.push_str(v);
        if *imp {
            s.push_str(" !important");
        }
        s.push(';');
    }
    s
}

fn css_name(prop: &str) -> String {
    if prop.starts_with("--") {
        return prop.to_string();
    }
    if prop == "cssFloat" {
        return "float".into();
    }
    let k = kebab(prop);
    if k.starts_with("webkit-") || k.starts_with("moz-") || k.starts_with("ms-") || k.starts_with("o-") {
        return format!("-{k}");
    }
    k
}

fn style_target_id(it: &mut Interp, target: &Value) -> Res<NodeId> {
    match it.get_str(target, "__id")? {
        Value::Num(n) => Ok(n as usize),
        _ => it.type_error("Illegal invocation"),
    }
}

fn style_decls(it: &mut Interp, id: NodeId) -> Vec<(String, String, bool)> {
    parse_decls(&it.attr(id, "style").unwrap_or_default())
}

fn style_store(it: &mut Interp, id: NodeId, d: &[(String, String, bool)]) -> Res<()> {
    if d.is_empty() {
        if it.attr(id, "style").is_some() {
            it.dom_set_attr(id, "style", "")?;
        }
        return Ok(());
    }
    it.dom_set_attr(id, "style", &serialize_decls(d))
}

pub fn set_style_prop(it: &mut Interp, id: NodeId, name: &str, value: &str, important: bool) -> Res<()> {
    let mut d = style_decls(it, id);
    let name = name.to_string();
    let value = value.trim();
    if value.is_empty() {
        d.retain(|e| e.0 != name);
    } else if let Some(e) = d.iter_mut().find(|e| e.0 == name) {
        e.1 = value.to_string();
        e.2 = important;
    } else {
        d.push((name, value.to_string(), important));
    }
    style_store(it, id, &d)
}

fn setup_dataset_and_style(it: &mut Interp, p: &Obj) {
    // dataset: Proxy over data-* attributes
    let dh = it.new_object();
    it.def_method(&dh, "get", 3, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Undefined) };
        Ok(it.attr(id, &format!("data-{}", kebab(&k))).map_or(Value::Undefined, Value::string))
    });
    it.def_method(&dh, "set", 4, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(true)) };
        let v = it.to_str(&c.arg(2))?;
        it.dom_set_attr(id, &format!("data-{}", kebab(&k)), &v)?;
        Ok(Value::Bool(true))
    });
    it.def_method(&dh, "has", 2, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(false)) };
        Ok(Value::Bool(it.attr(id, &format!("data-{}", kebab(&k))).is_some()))
    });
    it.def_method(&dh, "deleteProperty", 2, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(true)) };
        it.dom_remove_attr(id, &format!("data-{}", kebab(&k)))?;
        Ok(Value::Bool(true))
    });
    it.def_method(&dh, "ownKeys", 1, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let keys: Vec<Value> = attrs_data(it, id).into_iter().map(|(k, _)| Value::string(camel(&k))).collect();
        Ok(it.new_array(keys))
    });
    it.def_method(&dh, "getOwnPropertyDescriptor", 2, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Undefined) };
        match it.attr(id, &format!("data-{}", kebab(&k))) {
            Some(v) => {
                let o = it.new_object();
                it.define_data(&o, Key::str("value"), Value::string(v), ALL);
                it.define_data(&o, Key::str("writable"), Value::Bool(true), ALL);
                it.define_data(&o, Key::str("enumerable"), Value::Bool(true), ALL);
                it.define_data(&o, Key::str("configurable"), Value::Bool(true), ALL);
                Ok(Value::Obj(o))
            }
            None => Ok(Value::Undefined),
        }
    });
    it.protos.as_mut().unwrap().extra.insert("dataset_handler", dh);
    it.def_getter(p, "dataset", |it, c| {
        let id = it.this_node(c)?;
        let t = it.new_object();
        it.define_data(&t, Key::str("__id"), Value::Num(id as f64), 0);
        let h = it.protos().extra.get("dataset_handler").cloned().unwrap();
        Ok(Value::Obj(it.alloc(Object::new(None, Class::Proxy { target: t, handler: h }))))
    });

    // style
    let sh = it.new_object();
    it.def_method(&sh, "get", 3, |it, c| {
        let target = c.arg(0);
        let id = style_target_id(it, &target)?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Undefined) };
        match &*k {
            "cssText" => return Ok(Value::string(serialize_decls(&style_decls(it, id)))),
            "length" => return Ok(Value::Num(style_decls(it, id).len() as f64)),
            "getPropertyValue" | "setProperty" | "removeProperty" | "getPropertyPriority" | "item" | "toString" | "__id" | "constructor" | "hasOwnProperty" => {
                return it.get_v(&target, &Key::S(k));
            }
            "parentRule" => return Ok(Value::Null),
            _ => {}
        }
        if let Some(i) = str_index(&k) {
            return Ok(style_decls(it, id).get(i).map_or(Value::Undefined, |e| Value::str(&e.0)));
        }
        let name = css_name(&k);
        Ok(Value::string(style_decls(it, id).into_iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or_default()))
    });
    it.def_method(&sh, "set", 4, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(true)) };
        let v = c.arg(2);
        if &*k == "cssText" {
            let s = it.to_str(&v)?;
            let d = parse_decls(&s);
            style_store(it, id, &d)?;
            return Ok(Value::Bool(true));
        }
        let s = if v.is_nullish() { Rc::from("") } else { it.to_str(&v)? };
        set_style_prop(it, id, &css_name(&k), &s, false)?;
        Ok(Value::Bool(true))
    });
    it.def_method(&sh, "has", 2, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(false)) };
        let name = css_name(&k);
        Ok(Value::Bool(&*k == "cssText" || style_decls(it, id).iter().any(|e| e.0 == name) || is_known_css_prop(&name)))
    });
    it.def_method(&sh, "deleteProperty", 2, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(true)) };
        set_style_prop(it, id, &css_name(&k), "", false)?;
        Ok(Value::Bool(true))
    });
    it.def_method(&sh, "ownKeys", 1, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let keys: Vec<Value> = (0..style_decls(it, id).len()).map(|i| Value::string(i.to_string())).collect();
        Ok(it.new_array(keys))
    });
    it.def_method(&sh, "getOwnPropertyDescriptor", 2, |it, c| {
        let id = style_target_id(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Undefined) };
        let name = css_name(&k);
        match style_decls(it, id).into_iter().find(|e| e.0 == name) {
            Some(e) => {
                let o = it.new_object();
                it.define_data(&o, Key::str("value"), Value::string(e.1), ALL);
                it.define_data(&o, Key::str("writable"), Value::Bool(true), ALL);
                it.define_data(&o, Key::str("enumerable"), Value::Bool(true), ALL);
                it.define_data(&o, Key::str("configurable"), Value::Bool(true), ALL);
                Ok(Value::Obj(o))
            }
            None => Ok(Value::Undefined),
        }
    });
    it.protos.as_mut().unwrap().extra.insert("style_handler", sh);
    let sp = it.new_object();
    it.def_method(&sp, "getPropertyValue", 1, |it, c| {
        let id = style_target_id(it, &c.this)?;
        let n = it.arg_str(c, 0)?;
        let name = if n.starts_with("--") { n.to_string() } else { n.to_ascii_lowercase() };
        Ok(Value::string(style_decls(it, id).into_iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or_default()))
    });
    it.def_method(&sp, "getPropertyPriority", 1, |it, c| {
        let id = style_target_id(it, &c.this)?;
        let n = it.arg_str(c, 0)?.to_ascii_lowercase();
        Ok(Value::str(if style_decls(it, id).iter().any(|e| e.0 == n && e.2) { "important" } else { "" }))
    });
    it.def_method(&sp, "setProperty", 2, |it, c| {
        let id = style_target_id(it, &c.this)?;
        let n = it.arg_str(c, 0)?;
        let v = if c.arg(1).is_nullish() { Rc::from("") } else { it.arg_str(c, 1)? };
        let imp = matches!(c.arg(2), Value::Str(s) if s.eq_ignore_ascii_case("important"));
        let name = if n.starts_with("--") { n.to_string() } else { n.to_ascii_lowercase() };
        set_style_prop(it, id, &name, &v, imp)?;
        Ok(Value::Undefined)
    });
    it.def_method(&sp, "removeProperty", 1, |it, c| {
        let id = style_target_id(it, &c.this)?;
        let n = it.arg_str(c, 0)?;
        let name = if n.starts_with("--") { n.to_string() } else { n.to_ascii_lowercase() };
        let old = style_decls(it, id).into_iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or_default();
        set_style_prop(it, id, &name, "", false)?;
        Ok(Value::string(old))
    });
    it.def_method(&sp, "item", 1, |it, c| {
        let id = style_target_id(it, &c.this)?;
        let i = to_integer(it.arg_num(c, 0)?).max(0.0) as usize;
        Ok(Value::string(style_decls(it, id).get(i).map(|e| e.0.clone()).unwrap_or_default()))
    });
    it.def_method(&sp, "toString", 0, |it, c| {
        let id = style_target_id(it, &c.this)?;
        Ok(Value::string(serialize_decls(&style_decls(it, id))))
    });
    it.protos.as_mut().unwrap().extra.insert("style_proto", sp);
    it.def_accessor(
        p,
        "style",
        |it, c| {
            let id = it.this_node(c)?;
            let sp = it.protos().extra.get("style_proto").cloned().unwrap();
            let t = it.alloc(Object::new(Some(sp), Class::Plain));
            it.define_data(&t, Key::str("__id"), Value::Num(id as f64), 0);
            let h = it.protos().extra.get("style_handler").cloned().unwrap();
            Ok(Value::Obj(it.alloc(Object::new(None, Class::Proxy { target: t, handler: h }))))
        },
        |it, c| {
            let id = it.this_node(c)?;
            let s = it.arg_str(c, 0)?;
            it.dom_set_attr(id, "style", &s)?;
            Ok(Value::Undefined)
        },
    );
}

fn is_known_css_prop(name: &str) -> bool {
    matches!(
        name,
        "display" | "color" | "background" | "background-color" | "width" | "height" | "margin" | "padding" | "position" | "top" | "left" | "right" | "bottom" | "opacity" | "visibility" | "overflow" | "font-size" | "z-index" | "transform" | "transition" | "border" | "cursor" | "float" | "flex" | "grid" | "content" | "line-height" | "text-align" | "pointer-events" | "will-change" | "animation" | "box-sizing" | "-webkit-transform" | "-webkit-appearance" | "touch-action" | "user-select" | "max-width" | "min-width" | "max-height" | "min-height" | "white-space" | "font-weight" | "font-family" | "text-decoration" | "outline" | "box-shadow" | "filter" | "clip" | "object-fit" | "scroll-behavior" | "overscroll-behavior" | "mask" | "-webkit-mask"
    )
}

fn attrs_data(it: &mut Interp, id: NodeId) -> Vec<(String, String)> {
    match it.doc().map(|d| &d.nodes[id].kind) {
        Some(NodeKind::Element { attributes, .. }) => attributes.iter().filter(|(k, _)| k.starts_with("data-")).map(|(k, v)| (k[5..].to_string(), v.clone())).collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Form controls, focus, click, default actions
// ---------------------------------------------------------------------------

fn find_form(it: &mut Interp, id: NodeId) -> Option<NodeId> {
    let d = it.doc()?;
    if let Some(f) = d.nodes[id].attribute("form") {
        return d.get_element_by_id(f);
    }
    let mut cur = d.nodes[id].parent;
    while let Some(n) = cur {
        if d.nodes[n].tag() == Some("form") {
            return Some(n);
        }
        cur = d.nodes[n].parent;
    }
    None
}

fn form_controls(it: &mut Interp, form: NodeId) -> Vec<NodeId> {
    let Some(d) = it.doc() else { return Vec::new() };
    let mut out = Vec::new();
    let mut st = vec![d.root];
    while let Some(n) = st.pop() {
        if matches!(d.nodes[n].tag(), Some("input" | "button" | "select" | "textarea" | "fieldset" | "output" | "object")) {
            let owner = if let Some(f) = d.nodes[n].attribute("form") { d.get_element_by_id(f) } else { { let mut cur = d.nodes[n].parent; let mut r = None; while let Some(p) = cur { if d.nodes[p].tag() == Some("form") { r = Some(p); break; } cur = d.nodes[p].parent; } r } };
            if owner == Some(form) {
                out.push(n);
            }
        }
        st.extend(d.nodes[n].children.iter().rev().copied());
    }
    out
}

fn options_of(it: &mut Interp, select: NodeId) -> Vec<NodeId> {
    let Some(d) = it.doc() else { return Vec::new() };
    let mut out = Vec::new();
    let mut st: Vec<NodeId> = d.nodes[select].children.iter().rev().copied().collect();
    while let Some(n) = st.pop() {
        match d.nodes[n].tag() {
            Some("option") => out.push(n),
            Some("optgroup") => st.extend(d.nodes[n].children.iter().rev().copied()),
            _ => {}
        }
    }
    out
}

fn option_value(it: &mut Interp, opt: NodeId) -> String {
    match it.attr(opt, "value") {
        Some(v) => v,
        None => it.doc().and_then(|d| d.text_content(opt)).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn selected_index(it: &mut Interp, select: NodeId) -> i32 {
    let opts = options_of(it, select);
    let multiple = it.attr(select, "multiple").is_some();
    if let Some(i) = opts.iter().position(|&o| it.attr(o, "selected").is_some() && true) {
        // last selected wins for single selects
        if !multiple {
            let last = opts.iter().rposition(|&o| it.attr(o, "selected").is_some()).unwrap_or(i);
            return last as i32;
        }
        return i as i32;
    }
    if multiple || opts.is_empty() {
        return -1;
    }
    // first non-disabled option
    opts.iter().position(|&o| it.attr(o, "disabled").is_none()).map_or(0, |i| i as i32)
}

fn set_selected_index(it: &mut Interp, select: NodeId, idx: i32) -> Res<()> {
    let opts = options_of(it, select);
    for (i, &o) in opts.iter().enumerate() {
        if i as i32 == idx {
            it.dom_set_attr(o, "selected", "")?;
        } else {
            it.dom_remove_attr(o, "selected")?;
        }
    }
    Ok(())
}

pub fn control_value(it: &mut Interp, id: NodeId) -> String {
    match it.tag_name(id).as_str() {
        "input" => {
            let ty = it.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
            match it.attr(id, "value") {
                Some(v) => v,
                None if ty == "checkbox" || ty == "radio" => "on".into(),
                None => String::new(),
            }
        }
        "textarea" => it.doc().and_then(|d| d.text_content(id)).unwrap_or_default(),
        "select" => {
            let i = selected_index(it, id);
            let opts = options_of(it, id);
            if i >= 0 { option_value(it, opts[i as usize]) } else { String::new() }
        }
        "option" => option_value(it, id),
        "button" | "li" | "param" | "data" | "meter" | "progress" | "output" => it.attr(id, "value").unwrap_or_default(),
        _ => it.attr(id, "value").unwrap_or_default(),
    }
}

fn setup_controls(it: &mut Interp, p: &Obj) {
    it.def_accessor(
        p,
        "value",
        |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::string(control_value(it, id)))
        },
        |it, c| {
            let id = it.this_node(c)?;
            let v = if c.arg(0).is_nullish() { Rc::from("") } else { it.arg_str(c, 0)? };
            match it.tag_name(id).as_str() {
                "textarea" => it.dom_set_text(id, &v)?,
                "select" => {
                    let opts = options_of(it, id);
                    let mut found = -1;
                    for (i, &o) in opts.iter().enumerate() {
                        if option_value(it, o) == *v {
                            found = i as i32;
                            break;
                        }
                    }
                    set_selected_index(it, id, found)?;
                }
                _ => it.dom_set_attr(id, "value", &v)?,
            }
            Ok(Value::Undefined)
        },
    );
    it.def_accessor(p, "defaultValue", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::string(if it.tag_name(id) == "textarea" { it.doc().and_then(|d| d.text_content(id)).unwrap_or_default() } else { it.attr(id, "value").unwrap_or_default() }))
    }, |it, c| {
        let id = it.this_node(c)?;
        let v = it.arg_str(c, 0)?;
        it.dom_set_attr(id, "value", &v)?;
        Ok(Value::Undefined)
    });
    it.def_accessor(
        p,
        "checked",
        |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::Bool(it.attr(id, "checked").is_some()))
        },
        |it, c| {
            let id = it.this_node(c)?;
            set_checked(it, id, c.arg(0).truthy())?;
            Ok(Value::Undefined)
        },
    );
    it.def_getter(p, "indeterminate", |_, _| Ok(Value::Bool(false)));
    it.def_accessor(
        p,
        "selected",
        |it, c| {
            let id = it.this_node(c)?;
            let sel = parent_select(it, id);
            Ok(Value::Bool(match sel {
                Some(s) => {
                    let opts = options_of(it, s);
                    let i = selected_index(it, s);
                    i >= 0 && opts.get(i as usize) == Some(&id)
                }
                None => it.attr(id, "selected").is_some(),
            }))
        },
        |it, c| {
            let id = it.this_node(c)?;
            if c.arg(0).truthy() {
                if let Some(s) = parent_select(it, id) {
                    if it.attr(s, "multiple").is_none() {
                        for o in options_of(it, s) {
                            it.dom_remove_attr(o, "selected")?;
                        }
                    }
                }
                it.dom_set_attr(id, "selected", "")?;
            } else {
                it.dom_remove_attr(id, "selected")?;
            }
            Ok(Value::Undefined)
        },
    );
    it.def_getter(p, "text", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::string(it.doc().and_then(|d| d.text_content(id)).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")))
    });
    it.def_getter(p, "index", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Num(match parent_select(it, id) {
            Some(s) => options_of(it, s).iter().position(|&o| o == id).unwrap_or(0) as f64,
            None => 0.0,
        }))
    });
    it.def_accessor(
        p,
        "selectedIndex",
        |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::Num(selected_index(it, id) as f64))
        },
        |it, c| {
            let id = it.this_node(c)?;
            let n = it.arg_num(c, 0)?;
            set_selected_index(it, id, n as i32)?;
            Ok(Value::Undefined)
        },
    );
    it.def_getter(p, "options", |it, c| {
        let id = it.this_node(c)?;
        let o = options_of(it, id);
        Ok(it.node_list(&o))
    });
    it.def_getter(p, "selectedOptions", |it, c| {
        let id = it.this_node(c)?;
        let i = selected_index(it, id);
        let opts = options_of(it, id);
        let mut v = Vec::new();
        if it.attr(id, "multiple").is_some() {
            v = opts.iter().copied().filter(|&o| it.attr(o, "selected").is_some()).collect();
        } else if i >= 0 {
            v.push(opts[i as usize]);
        }
        Ok(it.node_list(&v))
    });
    it.def_getter(p, "form", |it, c| {
        let id = it.this_node(c)?;
        let f = find_form(it, id);
        Ok(match f {
            Some(f) => it.wrap(f),
            None => Value::Null,
        })
    });
    it.def_getter(p, "elements", |it, c| {
        let id = it.this_node(c)?;
        let ctl = form_controls(it, id);
        Ok(it.node_list(&ctl))
    });
    it.def_getter(p, "length", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Num(match it.tag_name(id).as_str() {
            "form" => form_controls(it, id).len() as f64,
            "select" => options_of(it, id).len() as f64,
            _ => 0.0,
        }))
    });
    it.def_getter(p, "files", |it, _| Ok(it.new_array(Vec::new())));
    it.def_getter(p, "labels", |it, _| Ok(it.new_array(Vec::new())));
    it.def_getter(p, "validity", |it, _| {
        let o = it.new_object();
        it.define_data(&o, Key::str("valid"), Value::Bool(true), ALL);
        for n in ["valueMissing", "typeMismatch", "patternMismatch", "tooLong", "tooShort", "rangeUnderflow", "rangeOverflow", "stepMismatch", "badInput", "customError"] {
            it.define_data(&o, Key::str(n), Value::Bool(false), ALL);
        }
        Ok(Value::Obj(o))
    });
    it.def_getter(p, "validationMessage", |_, _| Ok(Value::str("")));
    it.def_getter(p, "willValidate", |_, _| Ok(Value::Bool(true)));
    it.def_method(p, "checkValidity", 0, |_, _| Ok(Value::Bool(true)));
    it.def_method(p, "reportValidity", 0, |_, _| Ok(Value::Bool(true)));
    it.def_method(p, "setCustomValidity", 1, |_, _| Ok(Value::Undefined));
    it.def_method(p, "select", 0, |_, _| Ok(Value::Undefined));
    it.def_method(p, "setSelectionRange", 2, |_, _| Ok(Value::Undefined));
    for n in ["selectionStart", "selectionEnd"] {
        it.def_accessor(p, n, |it, c| {
            let id = it.this_node(c)?;
            Ok(Value::Num(u16_len(&control_value(it, id)) as f64))
        }, |_, _| Ok(Value::Undefined));
    }
    it.def_accessor(p, "valueAsNumber", |it, c| {
        let id = it.this_node(c)?;
        Ok(Value::Num(str_to_num(&control_value(it, id))))
    }, |it, c| {
        let id = it.this_node(c)?;
        let n = it.arg_num(c, 0)?;
        it.dom_set_attr(id, "value", &num_to_string(n))?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "stepUp", 0, |_, _| Ok(Value::Undefined));
    it.def_method(p, "stepDown", 0, |_, _| Ok(Value::Undefined));
    it.def_method(p, "focus", 0, |it, c| {
        let id = it.this_node(c)?;
        it.focus_node(id)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "blur", 0, |it, c| {
        let id = it.this_node(c)?;
        if it.dom.focused == Some(id) {
            it.blur_node(id)?;
        }
        Ok(Value::Undefined)
    });
    it.def_method(p, "click", 0, |it, c| {
        let id = it.this_node(c)?;
        it.element_click(id)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "submit", 0, |it, c| {
        let id = it.this_node(c)?;
        it.submit_form(id, None, false)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "requestSubmit", 0, |it, c| {
        let id = it.this_node(c)?;
        it.submit_form(id, None, true)?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "reset", 0, |it, c| {
        let id = it.this_node(c)?;
        it.reset_form(id)?;
        Ok(Value::Undefined)
    });
    it.def_getter(p, "content", |it, c| {
        let id = it.this_node(c)?;
        if it.tag_name(id) == "template" {
            let f = template_fragment(it, id)?;
            return Ok(it.wrap(f));
        }
        Ok(Value::string(it.attr(id, "content").unwrap_or_default()))
    });
    it.def_getter(p, "sheet", |_, _| Ok(Value::Null));
    it.def_getter(p, "contentWindow", |_, _| Ok(Value::Null));
    it.def_getter(p, "contentDocument", |_, _| Ok(Value::Null));
    it.def_method(p, "getContext", 1, |it, c| {
        let id = it.this_node(c)?;
        let ty = it.arg_str(c, 0)?;
        if &*ty != "2d" || it.tag_name(id) != "canvas" {
            return Ok(Value::Null);
        }
        Ok(canvas_context(it, id))
    });
    it.def_method(p, "toDataURL", 0, |_, _| Ok(Value::str("data:,")));
    it.def_method(p, "toBlob", 1, |_, _| Ok(Value::Undefined));
    it.def_method(p, "play", 0, |it, _| Ok(Value::Obj(it.promise_resolved(Value::Undefined)?)));
    for n in ["pause", "load"] {
        it.def_method(p, n, 0, |_, _| Ok(Value::Undefined));
    }
    it.def_method(p, "canPlayType", 1, |_, _| Ok(Value::str("")));
    it.def_method(p, "showModal", 0, |it, c| {
        let id = it.this_node(c)?;
        it.dom_set_attr(id, "open", "")?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "show", 0, |it, c| {
        let id = it.this_node(c)?;
        it.dom_set_attr(id, "open", "")?;
        Ok(Value::Undefined)
    });
    it.def_method(p, "close", 0, |it, c| {
        let id = it.this_node(c)?;
        if it.attr(id, "open").is_some() {
            it.dom_remove_attr(id, "open")?;
            it.fire_node_event(id, "close")?;
        }
        Ok(Value::Undefined)
    });
    it.def_method(p, "showPopover", 0, |_, _| Ok(Value::Undefined));
    it.def_method(p, "hidePopover", 0, |_, _| Ok(Value::Undefined));
    it.def_method(p, "togglePopover", 0, |_, _| Ok(Value::Bool(false)));
    // table helpers
    it.def_getter(p, "rows", |it, c| {
        let id = it.this_node(c)?;
        let r = tags_below(it, id, &["tr"]);
        Ok(it.node_list(&r))
    });
    it.def_getter(p, "cells", |it, c| {
        let id = it.this_node(c)?;
        let r = tags_below(it, id, &["td", "th"]);
        Ok(it.node_list(&r))
    });
    it.def_getter(p, "tBodies", |it, c| {
        let id = it.this_node(c)?;
        let r = tags_below(it, id, &["tbody"]);
        Ok(it.node_list(&r))
    });
    it.def_getter(p, "cellIndex", |it, c| {
        let id = it.this_node(c)?;
        let par = it.doc().and_then(|d| d.nodes[id].parent);
        Ok(Value::Num(par.map_or(0, |p| it.doc().map_or(0, |d| d.nodes[p].children.iter().filter(|&&c| matches!(d.nodes[c].tag(), Some("td" | "th"))).position(|&c| c == id).unwrap_or(0))) as f64))
    });
    it.def_method(p, "insertRow", 1, |it, c| {
        let id = it.this_node(c)?;
        let tr = it.dom()?.create_element("tr");
        let idx = if c.args.is_empty() { -1 } else { it.arg_num(c, 0)? as i32 };
        let rows = tags_below(it, id, &["tr"]);
        let (parent, before) = if idx < 0 || idx as usize >= rows.len() {
            let p = if it.tag_name(id) == "table" {
                let tb = tags_below(it, id, &["tbody"]);
                match tb.last() {
                    Some(&b) => b,
                    None => {
                        let b = it.dom()?.create_element("tbody");
                        it.dom_insert(id, b, None)?;
                        b
                    }
                }
            } else {
                id
            };
            (p, None)
        } else {
            let r = rows[idx as usize];
            (it.doc().and_then(|d| d.nodes[r].parent).unwrap_or(id), Some(r))
        };
        it.dom_insert(parent, tr, before)?;
        Ok(it.wrap(tr))
    });
    it.def_method(p, "insertCell", 1, |it, c| {
        let id = it.this_node(c)?;
        let td = it.dom()?.create_element("td");
        it.dom_insert(id, td, None)?;
        Ok(it.wrap(td))
    });
    it.def_method(p, "deleteRow", 1, |it, c| {
        let id = it.this_node(c)?;
        let rows = tags_below(it, id, &["tr"]);
        let i = it.arg_num(c, 0)? as i32;
        let target = if i < 0 { rows.last().copied() } else { rows.get(i as usize).copied() };
        if let Some(t) = target {
            it.dom_remove(t)?;
        }
        Ok(Value::Undefined)
    });
    it.def_getter(p, "sectionRowIndex", |_, _| Ok(Value::Num(0.0)));
    it.def_getter(p, "rowIndex", |_, _| Ok(Value::Num(0.0)));
}

fn tags_below(it: &mut Interp, id: NodeId, tags: &[&str]) -> Vec<NodeId> {
    let Some(d) = it.doc() else { return Vec::new() };
    let mut out = Vec::new();
    let mut st: Vec<NodeId> = d.nodes[id].children.iter().rev().copied().collect();
    while let Some(n) = st.pop() {
        if let Some(t) = d.nodes[n].tag() {
            if tags.contains(&t) {
                out.push(n);
                if t == "tr" || t == "tbody" {
                    // rows inside tbody must still be found for "tbody" queries only at depth; keep descending for nested sections
                }
            }
            if t != "table" || n == id {
                st.extend(d.nodes[n].children.iter().rev().copied());
            }
        }
    }
    out
}

fn parent_select(it: &mut Interp, opt: NodeId) -> Option<NodeId> {
    let d = it.doc()?;
    let mut cur = d.nodes[opt].parent;
    while let Some(n) = cur {
        if d.nodes[n].tag() == Some("select") {
            return Some(n);
        }
        cur = d.nodes[n].parent;
    }
    None
}

pub fn set_checked(it: &mut Interp, id: NodeId, on: bool) -> Res<()> {
    if on {
        if it.attr(id, "type").is_some_and(|t| t.eq_ignore_ascii_case("radio")) {
            // uncheck the rest of the group
            let name = it.attr(id, "name").unwrap_or_default();
            if !name.is_empty() {
                let form = find_form(it, id);
                let root = it.dom()?.root;
                let mut peers = Vec::new();
                let d = it.dom()?;
                let mut st = vec![root];
                while let Some(n) = st.pop() {
                    if d.nodes[n].tag() == Some("input") && d.nodes[n].attribute("type").is_some_and(|t| t.eq_ignore_ascii_case("radio")) && d.nodes[n].attribute("name") == Some(name.as_str()) && n != id {
                        peers.push(n);
                    }
                    st.extend(d.nodes[n].children.iter().rev().copied());
                }
                for n in peers {
                    if find_form(it, n) == form {
                        it.dom_remove_attr(n, "checked")?;
                    }
                }
            }
        }
        it.dom_set_attr(id, "checked", "")
    } else {
        it.dom_remove_attr(id, "checked")
    }
}

fn focusable(it: &mut Interp, id: NodeId) -> bool {
    let tag = it.tag_name(id);
    if it.attr(id, "disabled").is_some() {
        return false;
    }
    match tag.as_str() {
        "input" => !it.attr(id, "type").is_some_and(|t| t.eq_ignore_ascii_case("hidden")),
        "textarea" | "select" | "button" | "summary" | "iframe" => true,
        "a" | "area" => it.attr(id, "href").is_some() || it.attr(id, "tabindex").is_some(),
        _ => it.attr(id, "tabindex").is_some() || it.attr(id, "contenteditable").is_some_and(|v| v != "false"),
    }
}

impl Interp {
    pub fn focus_node(&mut self, id: NodeId) -> Res<()> {
        if self.dom.focused == Some(id) || !focusable(self, id) {
            return Ok(());
        }
        if let Some(old) = self.dom.focused {
            self.blur_node(old)?;
        }
        self.dom.focused = Some(id);
        self.dom.focus_value = if matches!(self.tag_name(id).as_str(), "input" | "textarea") { Some(control_value(self, id)) } else { None };
        let t = self.wrap(id);
        let ev = self.make_event("focus", "FocusEvent", false, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        let ev = self.make_event("focusin", "FocusEvent", true, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        Ok(())
    }

    pub fn blur_node(&mut self, id: NodeId) -> Res<()> {
        if self.dom.focused != Some(id) {
            return Ok(());
        }
        self.dom.focused = None;
        let t = self.wrap(id);
        if let Some(old) = self.dom.focus_value.take() {
            if control_value(self, id) != old {
                let ev = self.make_event("change", "Event", true, false, &[]);
                self.dispatch_event_obj(&t, &ev)?;
            }
        }
        let ev = self.make_event("blur", "FocusEvent", false, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        let ev = self.make_event("focusout", "FocusEvent", true, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        Ok(())
    }

    pub fn element_click(&mut self, id: NodeId) -> Res<()> {
        if self.attr(id, "disabled").is_some() && matches!(self.tag_name(id).as_str(), "button" | "input" | "select" | "textarea") {
            return Ok(());
        }
        let t = self.wrap(id);
        let ev = self.make_event("click", "MouseEvent", true, true, &[("detail", Value::Num(1.0))]);
        let ok = self.dispatch_event_obj(&t, &ev)?;
        if ok {
            self.run_default_action(id)?;
        }
        Ok(())
    }

    /// Activation behaviour of the nearest interactive ancestor (links, checkboxes, buttons, labels).
    pub fn run_default_action(&mut self, target: NodeId) -> Res<()> {
        let mut cur = Some(target);
        while let Some(id) = cur {
            let tag = self.tag_name(id);
            match tag.as_str() {
                "a" | "area" => {
                    if let Some(h) = self.attr(id, "href") {
                        let abs = self.url_resolve(&h);
                        if abs.starts_with("javascript:") {
                            let code = percent_decode(&abs["javascript:".len()..]);
                            if let Err(e) = self.eval_script(&code) {
                                self.logs.push(LogEntry { level: "error", text: format!("Uncaught {e}") });
                            }
                        } else {
                            self.navigate(&abs);
                        }
                        return Ok(());
                    }
                }
                "input" => {
                    let ty = self.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
                    match ty.as_str() {
                        "checkbox" => {
                            let on = self.attr(id, "checked").is_none();
                            set_checked(self, id, on)?;
                            self.fire_input_change(id)?;
                            return Ok(());
                        }
                        "radio" => {
                            if self.attr(id, "checked").is_none() {
                                set_checked(self, id, true)?;
                                self.fire_input_change(id)?;
                            }
                            return Ok(());
                        }
                        "submit" | "image" => {
                            if let Some(f) = find_form(self, id) {
                                self.submit_form(f, Some(id), true)?;
                            }
                            return Ok(());
                        }
                        "reset" => {
                            if let Some(f) = find_form(self, id) {
                                self.reset_form(f)?;
                            }
                            return Ok(());
                        }
                        _ => {
                            self.focus_node(id)?;
                            return Ok(());
                        }
                    }
                }
                "button" => {
                    let ty = self.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
                    if ty == "reset" {
                        if let Some(f) = find_form(self, id) {
                            self.reset_form(f)?;
                        }
                    } else if ty != "button" {
                        if let Some(f) = find_form(self, id) {
                            self.submit_form(f, Some(id), true)?;
                        }
                    }
                    return Ok(());
                }
                "label" => {
                    let ctl = match self.attr(id, "for") {
                        Some(f) => self.dom()?.get_element_by_id(&f),
                        None => {
                            let d = self.dom()?;
                            let mut st: Vec<NodeId> = d.nodes[id].children.iter().rev().copied().collect();
                            let mut found = None;
                            while let Some(n) = st.pop() {
                                if matches!(d.nodes[n].tag(), Some("input" | "select" | "textarea" | "button")) {
                                    found = Some(n);
                                    break;
                                }
                                st.extend(d.nodes[n].children.iter().rev().copied());
                            }
                            found
                        }
                    };
                    if let Some(c) = ctl {
                        if c != target && !self.dom()?.nodes[target].parent.is_some_and(|_| false) {
                            self.element_click(c)?;
                        }
                    }
                    return Ok(());
                }
                "summary" => {
                    if let Some(det) = self.dom()?.nodes[id].parent.filter(|&p| self.doc().is_some_and(|d| d.nodes[p].tag() == Some("details"))) {
                        if self.attr(det, "open").is_some() {
                            self.dom_remove_attr(det, "open")?;
                        } else {
                            self.dom_set_attr(det, "open", "")?;
                        }
                        self.fire_node_event(det, "toggle")?;
                    }
                    return Ok(());
                }
                "select" | "textarea" => {
                    self.focus_node(id)?;
                    return Ok(());
                }
                _ => {}
            }
            cur = self.dom()?.nodes[id].parent;
        }
        Ok(())
    }

    /// Typing: insert text into the focused control and fire `input`.
    pub fn input_insert(&mut self, id: NodeId, text: &str) -> Res<()> {
        let tag = self.tag_name(id);
        let editable = self.attr(id, "contenteditable").is_some_and(|v| v != "false");
        if self.attr(id, "readonly").is_some() || self.attr(id, "disabled").is_some() {
            return Ok(());
        }
        match tag.as_str() {
            "input" => {
                let ty = self.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
                if matches!(ty.as_str(), "checkbox" | "radio" | "submit" | "button" | "reset" | "file" | "image" | "hidden" | "color" | "range") {
                    return Ok(());
                }
                if ty == "number" && !text.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) {
                    return Ok(());
                }
                let mut v = control_value(self, id);
                if let Some(max) = self.attr(id, "maxlength").and_then(|m| m.trim().parse::<usize>().ok()) {
                    if v.chars().count() >= max {
                        return Ok(());
                    }
                }
                v.push_str(text);
                self.dom_set_attr(id, "value", &v)?;
            }
            "textarea" => {
                let v = format!("{}{}", control_value(self, id), text);
                self.dom_set_text(id, &v)?;
            }
            _ if editable => {
                let v = format!("{}{}", self.dom()?.text_content(id).unwrap_or_default(), text);
                self.dom_set_text(id, &v)?;
            }
            _ => return Ok(()),
        }
        let t = self.wrap(id);
        let ev = self.make_event("input", "InputEvent", true, false, &[("data", Value::str(text)), ("inputType", Value::str("insertText"))]);
        self.dispatch_event_obj(&t, &ev)?;
        Ok(())
    }

    pub fn key_default(&mut self, id: NodeId, key: &str) -> Res<()> {
        let tag = self.tag_name(id);
        match key {
            "Backspace" | "Delete" if matches!(tag.as_str(), "input" | "textarea") => {
                let mut v = control_value(self, id);
                if v.pop().is_some() {
                    if tag == "input" {
                        self.dom_set_attr(id, "value", &v)?;
                    } else {
                        self.dom_set_text(id, &v)?;
                    }
                    let t = self.wrap(id);
                    let ev = self.make_event("input", "InputEvent", true, false, &[("inputType", Value::str("deleteContentBackward"))]);
                    self.dispatch_event_obj(&t, &ev)?;
                }
            }
            "Enter" => match tag.as_str() {
                "textarea" => self.input_insert(id, "\n")?,
                "input" => {
                    let ty = self.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
                    if matches!(ty.as_str(), "submit" | "button" | "reset" | "checkbox" | "radio") {
                        self.element_click(id)?;
                    } else if let Some(f) = find_form(self, id) {
                        // implicit submission through the first submit button
                        let btn = form_controls(self, f).into_iter().find(|&c| {
                            let t = self.attr(c, "type").unwrap_or_default().to_ascii_lowercase();
                            (self.tag_name(c) == "button" && t != "button" && t != "reset") || (self.tag_name(c) == "input" && (t == "submit" || t == "image"))
                        });
                        match btn {
                            Some(b) => self.element_click(b)?,
                            None => self.submit_form(f, None, true)?,
                        }
                    }
                }
                "button" | "a" | "summary" => self.element_click(id)?,
                _ => {}
            },
            " " if matches!(tag.as_str(), "button" | "summary") => self.element_click(id)?,
            " " if tag == "input" && matches!(self.attr(id, "type").unwrap_or_default().to_ascii_lowercase().as_str(), "checkbox" | "radio" | "submit" | "button") => self.element_click(id)?,
            "Tab" => {
                let next = self.next_focusable(id);
                if let Some(n) = next {
                    self.focus_node(n)?;
                } else if let Some(f) = self.dom.focused {
                    self.blur_node(f)?;
                }
            }
            "Escape" => {
                if tag == "dialog" {
                    self.dom_remove_attr(id, "open")?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn next_focusable(&mut self, from: NodeId) -> Option<NodeId> {
        let d = self.dom().ok()?;
        let mut order = Vec::new();
        let mut st = vec![d.root];
        while let Some(n) = st.pop() {
            order.push(n);
            st.extend(d.nodes[n].children.iter().rev().copied());
        }
        let start = order.iter().position(|&n| n == from).unwrap_or(0);
        order[start + 1..].iter().copied().find(|&n| self.dom().is_ok_and(|d| matches!(d.nodes[n].kind, NodeKind::Element { .. })) && focusable(self, n))
    }

    pub fn fire_input_change(&mut self, id: NodeId) -> Res<()> {
        let t = self.wrap(id);
        let ev = self.make_event("input", "InputEvent", true, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        let ev = self.make_event("change", "Event", true, false, &[]);
        self.dispatch_event_obj(&t, &ev)?;
        Ok(())
    }

    pub fn navigate(&mut self, url: &str) {
        // same-document fragment navigation
        let cur = Url::parse(&self.dom.base_url, None);
        let nu = Url::parse(url, cur.as_ref());
        if let (Some(c), Some(n)) = (&cur, &nu) {
            let mut c2 = c.clone();
            c2.fragment = None;
            let mut n2 = n.clone();
            n2.fragment = None;
            if c2 == n2 && n.fragment.is_some() && c.fragment != n.fragment {
                let old = c.href();
                self.dom.base_url = n.href();
                self.dom.history.truncate(self.dom.history_pos + 1);
                self.dom.history.push(self.dom.base_url.clone());
                self.dom.history_pos = self.dom.history.len() - 1;
                let ev = self.make_event("hashchange", "HashChangeEvent", false, false, &[("oldURL", Value::string(old)), ("newURL", Value::string(n.href()))]);
                let g = Value::Obj(self.global.clone());
                let _ = self.dispatch_event_obj(&g, &ev);
                return;
            }
        }
        self.dom.navigate_to = Some(url.to_string());
        if let Some(h) = self.host.host.as_mut() {
            h.navigate(url);
        }
    }

    pub fn reset_form(&mut self, form: NodeId) -> Res<()> {
        let t = self.wrap(form);
        let ev = self.make_event("reset", "Event", true, true, &[]);
        if !self.dispatch_event_obj(&t, &ev)? {
            return Ok(());
        }
        for c in form_controls(self, form) {
            if self.tag_name(c) == "input" {
                let ty = self.attr(c, "type").unwrap_or_default();
                if ty != "checkbox" && ty != "radio" {
                    self.dom_remove_attr(c, "value")?;
                }
            } else if self.tag_name(c) == "textarea" {
                // keep default text
            }
        }
        Ok(())
    }

    pub fn form_data(&mut self, form: NodeId, submitter: Option<NodeId>) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for c in form_controls(self, form) {
            let Some(name) = self.attr(c, "name").filter(|n| !n.is_empty()) else { continue };
            if self.attr(c, "disabled").is_some() {
                continue;
            }
            match self.tag_name(c).as_str() {
                "input" => {
                    let ty = self.attr(c, "type").unwrap_or_default().to_ascii_lowercase();
                    match ty.as_str() {
                        "checkbox" | "radio" => {
                            if self.attr(c, "checked").is_some() {
                                out.push((name, control_value(self, c)));
                            }
                        }
                        "submit" | "image" | "button" | "reset" | "file" => {
                            if submitter == Some(c) && ty != "button" && ty != "reset" {
                                out.push((name, control_value(self, c)));
                            }
                        }
                        _ => out.push((name, control_value(self, c))),
                    }
                }
                "select" => {
                    if self.attr(c, "multiple").is_some() {
                        for o in options_of(self, c) {
                            if self.attr(o, "selected").is_some() {
                                let v = option_value(self, o);
                                out.push((name.clone(), v));
                            }
                        }
                    } else {
                        out.push((name, control_value(self, c)));
                    }
                }
                "textarea" => out.push((name, control_value(self, c))),
                "button" => {
                    if submitter == Some(c) {
                        out.push((name, control_value(self, c)));
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn submit_form(&mut self, form: NodeId, submitter: Option<NodeId>, fire_event: bool) -> Res<()> {
        if self.tag_name(form) != "form" {
            return Ok(());
        }
        if fire_event {
            let t = self.wrap(form);
            let sub = match submitter {
                Some(s) => self.wrap(s),
                None => Value::Null,
            };
            let ev = self.make_event("submit", "SubmitEvent", true, true, &[("submitter", sub)]);
            if !self.dispatch_event_obj(&t, &ev)? {
                return Ok(());
            }
        }
        let action = self.attr(form, "action").filter(|a| !a.is_empty()).unwrap_or_else(|| self.dom.base_url.clone());
        let method = self.attr(form, "method").unwrap_or_default().to_ascii_lowercase();
        let data = self.form_data(form, submitter);
        let encode = |s: &str| -> String {
            let mut o = String::new();
            for b in s.bytes() {
                match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => o.push(b as char),
                    b' ' => o.push('+'),
                    _ => o.push_str(&format!("%{b:02X}")),
                }
            }
            o
        };
        let qs: Vec<String> = data.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
        let abs = self.url_resolve(&action);
        if method == "post" {
            self.navigate(&abs);
            return Ok(());
        }
        let mut u = Url::parse(&abs, None);
        if let Some(u) = u.as_mut() {
            u.query = if qs.is_empty() { None } else { Some(qs.join("&")) };
            let href = u.href();
            self.navigate(&href);
        }
        Ok(())
    }
}

fn canvas_context(it: &mut Interp, id: NodeId) -> Value {
    let o = it.new_object();
    let canvas = it.wrap(id);
    it.define_data(&o, Key::str("canvas"), canvas, ALL);
    for n in ["fillStyle", "strokeStyle", "font", "textAlign", "textBaseline", "globalAlpha", "lineWidth", "lineCap", "lineJoin", "globalCompositeOperation", "shadowBlur", "shadowColor", "imageSmoothingEnabled"] {
        it.define_data(&o, Key::str(n), Value::str(""), ALL);
    }
    for n in ["fillRect", "strokeRect", "clearRect", "fillText", "strokeText", "beginPath", "closePath", "moveTo", "lineTo", "arc", "arcTo", "rect", "ellipse", "quadraticCurveTo", "bezierCurveTo", "fill", "stroke", "clip", "save", "restore", "scale", "rotate", "translate", "transform", "setTransform", "resetTransform", "drawImage", "putImageData", "setLineDash"] {
        it.def_method(&o, n, 0, |_, _| Ok(Value::Undefined));
    }
    it.def_method(&o, "measureText", 1, |it, c| {
        let s = it.arg_str(c, 0)?;
        let m = it.new_object();
        it.define_data(&m, Key::str("width"), Value::Num(u16_len(&s) as f64 * 6.0), ALL);
        Ok(Value::Obj(m))
    });
    for n in ["createLinearGradient", "createRadialGradient", "createPattern"] {
        it.def_method(&o, n, 0, |it, _| {
            let g = it.new_object();
            it.def_method(&g, "addColorStop", 2, |_, _| Ok(Value::Undefined));
            Ok(Value::Obj(g))
        });
    }
    it.def_method(&o, "getImageData", 4, |it, c| {
        let w = it.arg_num(c, 2)?.max(0.0) as usize;
        let h = it.arg_num(c, 3)?.max(0.0) as usize;
        let d = it.new_object();
        let arr = it.new_array(vec![Value::Num(0.0); (w * h * 4).min(4_000_000)]);
        it.define_data(&d, Key::str("data"), arr, ALL);
        it.define_data(&d, Key::str("width"), Value::Num(w as f64), ALL);
        it.define_data(&d, Key::str("height"), Value::Num(h as f64), ALL);
        Ok(Value::Obj(d))
    });
    Value::Obj(o)
}

// ---------------------------------------------------------------------------
// Custom elements
// ---------------------------------------------------------------------------

fn setup_custom_elements(it: &mut Interp) {
    let ce = it.new_object();
    let g = it.global.clone();
    it.define_data(&g, Key::str("customElements"), Value::Obj(ce.clone()), HIDDEN);
    it.def_method(&ce, "define", 2, |it, c| {
        let name = it.arg_str(c, 0)?.to_ascii_lowercase();
        let Value::Obj(ctor) = c.arg(1) else { return it.type_error("The constructor is not a constructor") };
        if !name.contains('-') {
            return Err(it.error_ctl("SyntaxError", &format!("'{name}' is not a valid custom element name")));
        }
        if it.custom_elements.iter().any(|(n, _)| *n == name) {
            return Err(it.error_ctl("NotSupportedError", &format!("the name \"{name}\" has already been used with this registry")));
        }
        it.custom_elements.push((name.clone(), ctor.clone()));
        // upgrade existing elements of the document, shadow trees included
        if let Ok(d) = it.dom() {
            let mut found = Vec::new();
            let mut st = vec![d.root];
            while let Some(n) = st.pop() {
                if d.nodes[n].tag() == Some(name.as_str()) {
                    found.push(n);
                }
                st.extend(d.nodes[n].children.iter().rev().copied());
                if let Some(&sr) = d.shadow_roots.get(&n) {
                    st.push(sr);
                }
            }
            for n in found {
                if it.upgraded.contains(&n) {
                    continue;
                }
                it.upgrade_element(n, &ctor);
                let w = it.wrap(n);
                if let Ok(cb) = it.get_str(&w, "connectedCallback") {
                    if cb.as_obj().is_some_and(|o| o.is_callable()) {
                        if let Err(Ctl::Throw(e)) = it.call(&cb, w, &[]) {
                            it.report_error(&e);
                        }
                    }
                }
            }
        }
        // settle whenDefined promises
        let waiting: Vec<Obj> = {
            let mut out = Vec::new();
            it.when_defined.retain(|(n, p)| {
                if *n == name {
                    out.push(p.clone());
                    false
                } else {
                    true
                }
            });
            out
        };
        for p in waiting {
            it.resolve_promise(&p, Value::Obj(ctor.clone()))?;
        }
        Ok(Value::Undefined)
    });
    it.def_method(&ce, "get", 1, |it, c| {
        let name = it.arg_str(c, 0)?.to_ascii_lowercase();
        Ok(it.custom_elements.iter().find(|(n, _)| *n == name).map_or(Value::Undefined, |(_, o)| Value::Obj(o.clone())))
    });
    it.def_method(&ce, "whenDefined", 1, |it, c| {
        let name = it.arg_str(c, 0)?.to_ascii_lowercase();
        if let Some(ctor) = it.custom_elements.iter().find(|(n, _)| *n == name).map(|(_, o)| o.clone()) {
            return Ok(Value::Obj(it.promise_resolved(Value::Obj(ctor))?));
        }
        let p = it.new_promise();
        it.when_defined.push((name, p.clone()));
        Ok(Value::Obj(p))
    });
    it.def_method(&ce, "upgrade", 1, |_, _| Ok(Value::Undefined));
}

#[allow(dead_code)]
fn unused(_: &str) -> Vec<String> {
    let _ = html::is_void("br");
    Vec::new()
}
