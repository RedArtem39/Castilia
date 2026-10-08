//! Window / BOM: location, history, navigator, storage, matchMedia, getComputedStyle, host bridges.
use super::{builtins::*, dom::*, dom_extra::*, interp::*, url::Url, value::*};
use crate::dom::NodeKind;
use alloc::{
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub(crate) const PRELUDE: &str = include_str!("prelude.js");

pub fn setup(it: &mut Interp) {
    let g = it.global.clone();
    it.def_getter(&g, "document", |it, _| {
        let r = it.dom()?.root;
        Ok(it.wrap(r))
    });
    setup_location(it, &g);
    setup_history(it, &g);
    setup_navigator(it, &g);
    setup_storage(it);
    for (name, f) in [
        ("innerWidth", (|it: &mut Interp, _: &Call| Ok(Value::Num(it.dom.viewport.0 as f64))) as NativeFn),
        ("innerHeight", |it, _| Ok(Value::Num(it.dom.viewport.1 as f64))),
        ("outerWidth", |it, _| Ok(Value::Num(it.dom.viewport.0 as f64))),
        ("outerHeight", |it, _| Ok(Value::Num(it.dom.viewport.1 as f64 + 80.0))),
        ("scrollX", |it, _| Ok(Value::Num(it.dom.scroll.0 as f64))),
        ("scrollY", |it, _| Ok(Value::Num(it.dom.scroll.1 as f64))),
        ("pageXOffset", |it, _| Ok(Value::Num(it.dom.scroll.0 as f64))),
        ("pageYOffset", |it, _| Ok(Value::Num(it.dom.scroll.1 as f64))),
    ] {
        it.def_getter(&g, name, f);
    }
    for (n, v) in [("devicePixelRatio", Value::Num(1.0)), ("isSecureContext", Value::Bool(true)), ("name", Value::str("")), ("closed", Value::Bool(false)), ("length", Value::Num(0.0)), ("frameElement", Value::Null), ("opener", Value::Null), ("status", Value::str("")), ("origin", Value::str("null"))] {
        it.define_data(&g, Key::str(n), v, ALL);
    }
    it.def_getter(&g, "origin", |it, _| Ok(Value::string(Url::parse(&it.dom.base_url, None).map(|u| u.origin()).unwrap_or_else(|| "null".into()))));
    for n in ["scrollTo", "scroll", "scrollBy", "focus", "blur", "stop", "print", "moveTo", "resizeTo", "close"] {
        it.def_method(&g, n, 0, |_, _| Ok(Value::Undefined));
    }
    it.def_method(&g, "alert", 1, |it, c| {
        let m = it.arg_str(c, 0)?;
        if let Some(h) = it.host.host.as_mut() {
            h.alert(&m);
        }
        Ok(Value::Undefined)
    });
    it.def_method(&g, "confirm", 1, |it, c| {
        let m = it.arg_str(c, 0)?;
        Ok(Value::Bool(it.host.host.as_mut().is_some_and(|h| h.confirm(&m))))
    });
    it.def_method(&g, "prompt", 1, |it, c| {
        let m = it.arg_str(c, 0)?;
        let d = if c.args.len() > 1 { it.arg_str(c, 1)?.to_string() } else { String::new() };
        Ok(match it.host.host.as_mut().and_then(|h| h.prompt(&m, &d)) {
            Some(s) => Value::string(s),
            None => Value::Null,
        })
    });
    it.def_method(&g, "open", 1, |it, c| {
        if !c.arg(0).is_nullish() {
            let u = it.arg_str(c, 0)?;
            if !u.is_empty() && &*u != "about:blank" {
                let abs = it.url_resolve(&u);
                it.navigate(&abs);
            }
        }
        Ok(Value::Null)
    });
    it.def_method(&g, "postMessage", 1, |it, c| {
        let data = c.arg(0);
        let f = it.new_native_data("", 0, |it, c| {
            let d = it.native_data(c);
            let origin = Url::parse(&it.dom.base_url, None).map(|u| u.origin()).unwrap_or_default();
            let ev = it.make_event("message", "MessageEvent", false, false, &[("data", d), ("origin", Value::string(origin))]);
            let g = Value::Obj(it.global.clone());
            it.dispatch_event_obj(&g, &ev)?;
            Ok(Value::Undefined)
        }, data, false);
        it.timer_seq += 1;
        let id = it.timer_seq;
        it.timers.push(Timer { id, due: it.now, interval: None, callback: Value::Obj(f), args: Vec::new(), raf: false });
        Ok(Value::Undefined)
    });
    it.def_method(&g, "getSelection", 0, |it, _| {
        let doc = it.get_str(&Value::Obj(it.global.clone()), "document")?;
        let f = it.get_str(&doc, "getSelection")?;
        it.call(&f, doc, &[])
    });
    it.def_method(&g, "reportError", 1, |it, c| {
        it.report_error(&c.arg(0));
        Ok(Value::Undefined)
    });
    it.def_method(&g, "getComputedStyle", 1, get_computed_style);
    it.def_method(&g, "matchMedia", 1, match_media);
    // low level bridges used by the prelude
    it.global_fn("__parseURL", 2, |it, c| {
        let href = it.arg_str(c, 0)?;
        let base = if c.arg(1).is_nullish() { None } else { Url::parse(&it.arg_str(c, 1)?, None) };
        Ok(match Url::parse(&href, base.as_ref()) {
            Some(u) => url_object(it, &u),
            None => Value::Null,
        })
    });
    it.global_fn("__setURLPart", 3, |it, c| {
        let href = it.arg_str(c, 0)?;
        let part = it.arg_str(c, 1)?;
        let val = it.arg_str(c, 2)?;
        Ok(match Url::parse(&href, None) {
            Some(mut u) => {
                set_url_component(&mut u, &part, &val);
                url_object(it, &u)
            }
            None => Value::Null,
        })
    });
    it.global_fn("__hostFetch", 4, |it, c| {
        let method = it.arg_str(c, 0)?;
        let url = it.arg_str(c, 1)?;
        let mut headers = Vec::new();
        for h in it.array_like_to_vec(&c.arg(2))? {
            let k = it.get_str(&h, "0")?;
            let v = it.get_str(&h, "1")?;
            headers.push((it.to_str(&k)?.to_string(), it.to_str(&v)?.to_string()));
        }
        let body = if c.arg(3).is_nullish() { None } else { Some(it.arg_str(c, 3)?.to_string()) };
        let abs = it.url_resolve(&url);
        match it.host_fetch(&method, &abs, &headers, body.as_deref()) {
            Some(r) => {
                let o = it.new_object();
                it.define_data(&o, Key::str("status"), Value::Num(r.status as f64), ALL);
                it.define_data(&o, Key::str("statusText"), Value::string(r.status_text), ALL);
                it.define_data(&o, Key::str("url"), Value::string(r.url), ALL);
                it.define_data(&o, Key::str("body"), Value::string(r.body), ALL);
                let hs: Vec<Value> = r.headers.into_iter().map(|(k, v)| it.new_array(vec![Value::string(k), Value::string(v)])).collect();
                let hs = it.new_array(hs);
                it.define_data(&o, Key::str("headers"), hs, ALL);
                Ok(Value::Obj(o))
            }
            None => Ok(Value::Null),
        }
    });
    it.global_fn("__formData", 1, |it, c| {
        let form = it.node_id(&c.arg(0))?;
        let data = it.form_data(form, None);
        let items: Vec<Value> = data.into_iter().map(|(k, v)| it.new_array(vec![Value::string(k), Value::string(v)])).collect();
        Ok(it.new_array(items))
    });
    it.global_fn("__setImportMap", 1, super::modules::set_import_map);
    it.global_fn("__navigate", 1, |it, c| {
        let u = it.arg_str(c, 0)?;
        let abs = it.url_resolve(&u);
        it.navigate(&abs);
        Ok(Value::Undefined)
    });
    it.global_fn("__randomBytes", 1, |it, c| {
        let n = it.arg_num(c, 0)?.max(0.0) as usize;
        let v: Vec<Value> = (0..n.min(1 << 20)).map(|_| Value::Num((it.next_random() * 256.0) as u8 as f64)).collect();
        Ok(it.new_array(v))
    });
    // hide the bridges from enumeration
    for n in ["__parseURL", "__setURLPart", "__hostFetch", "__formData", "__navigate", "__randomBytes", "__setImportMap"] {
        if let Some(p) = it.own_prop(&g, &Key::str(n)) {
            if let Slot::Data(v) = p.slot {
                it.define_data(&g, Key::str(n), v, HIDDEN);
            }
        }
    }
    match super::parser::parse_program(PRELUDE, &mut it.interner, false) {
        Ok(prog) => {
            if let Err(Ctl::Throw(e)) = it.run_program(&prog) {
                let t = it.error_text(&e);
                it.logs.push(LogEntry { level: "error", text: format!("prelude failed: {t}") });
            }
        }
        Err(e) => it.logs.push(LogEntry { level: "error", text: format!("prelude parse error: {e}") }),
    }
}

fn url_object(it: &mut Interp, u: &Url) -> Value {
    let o = it.new_object();
    for (k, v) in [
        ("href", u.href()),
        ("protocol", format!("{}:", u.scheme)),
        ("username", u.username.clone()),
        ("password", u.password.clone()),
        ("host", u.host_with_port()),
        ("hostname", u.host.clone()),
        ("port", u.port.clone()),
        ("pathname", u.path.clone()),
        ("search", u.search()),
        ("hash", u.hash()),
        ("origin", u.origin()),
    ] {
        it.define_data(&o, Key::str(k), Value::string(v), ALL);
    }
    Value::Obj(o)
}

// ---------------------------------------------------------------------------
// location / history
// ---------------------------------------------------------------------------

fn cur_url(it: &Interp) -> Url {
    Url::parse(&it.dom.base_url, None).unwrap_or_else(|| Url::parse("about:blank", None).unwrap())
}

fn setup_location(it: &mut Interp, g: &Obj) {
    let loc = it.new_object();
    for part in ["origin", "protocol", "host", "hostname", "port", "pathname", "search", "hash"] {
        let data = Value::str(part);
        let gt = it.new_native_data(part, 0, |it, c| {
            let part = it.to_string_lossy(&it.native_data(c));
            Ok(Value::string(url_component(&cur_url(it), &part)))
        }, data.clone(), false);
        let st = it.new_native_data(part, 1, |it, c| {
            let part = it.to_string_lossy(&it.native_data(c));
            let v = it.arg_str(c, 0)?;
            let mut u = cur_url(it);
            set_url_component(&mut u, &part, &v);
            let href = u.href();
            it.navigate(&href);
            Ok(Value::Undefined)
        }, data, false);
        it.define_accessor_part(&loc, Key::str(part), Some(gt), true, ENUMERABLE | CONFIGURABLE);
        it.define_accessor_part(&loc, Key::str(part), Some(st), false, ENUMERABLE | CONFIGURABLE);
    }
    it.def_accessor(
        &loc,
        "href",
        |it, _| Ok(Value::string(it.dom.base_url.clone())),
        |it, c| {
            let v = it.arg_str(c, 0)?;
            let abs = it.url_resolve(&v);
            it.navigate(&abs);
            Ok(Value::Undefined)
        },
    );
    for n in ["assign", "replace"] {
        it.def_method(&loc, n, 1, |it, c| {
            let v = it.arg_str(c, 0)?;
            let abs = it.url_resolve(&v);
            it.navigate(&abs);
            Ok(Value::Undefined)
        });
    }
    it.def_method(&loc, "reload", 0, |it, _| {
        let u = it.dom.base_url.clone();
        it.dom.navigate_to = Some(u);
        Ok(Value::Undefined)
    });
    it.def_method(&loc, "toString", 0, |it, _| Ok(Value::string(it.dom.base_url.clone())));
    it.define_data(g, Key::str("location"), Value::Obj(loc), ALL);
}

fn setup_history(it: &mut Interp, g: &Obj) {
    let h = it.new_object();
    it.def_getter(&h, "length", |it, _| Ok(Value::Num(it.dom.history.len().max(1) as f64)));
    it.def_getter(&h, "state", |it, _| {
        let s = it.history_state.get(&it.dom.history_pos).cloned().unwrap_or(Value::Null);
        Ok(s)
    });
    it.define_data(&h, Key::str("scrollRestoration"), Value::str("auto"), ALL);
    for (name, replace) in [("pushState", false), ("replaceState", true)] {
        let f: NativeFn = if replace { |it, c| push_state(it, c, true) } else { |it, c| push_state(it, c, false) };
        it.def_method(&h, name, 3, f);
    }
    it.def_method(&h, "back", 0, |it, _| history_go(it, -1));
    it.def_method(&h, "forward", 0, |it, _| history_go(it, 1));
    it.def_method(&h, "go", 1, |it, c| {
        let d = it.arg_num(c, 0)?;
        history_go(it, d as i32)
    });
    it.define_data(g, Key::str("history"), Value::Obj(h), ALL);
}

fn push_state(it: &mut Interp, c: &Call, replace: bool) -> Res {
    if it.dom.history.is_empty() {
        it.dom.history.push(it.dom.base_url.clone());
    }
    let url = c.arg(2);
    if !url.is_nullish() {
        let s = it.to_str(&url)?;
        let abs = it.url_resolve(&s);
        let (cur, new) = (cur_url(it), Url::parse(&abs, None));
        match new {
            Some(n) if n.origin() == cur.origin() || n.scheme == "file" => {
                it.dom.base_url = n.href();
            }
            _ => return Err(it.error_ctl("SecurityError", "Failed to execute 'pushState' on 'History': cross-origin URL")),
        }
    }
    if replace {
        let pos = it.dom.history_pos;
        it.dom.history[pos] = it.dom.base_url.clone();
        it.history_state.insert(pos, c.arg(0));
    } else {
        let pos = it.dom.history_pos;
        it.dom.history.truncate(pos + 1);
        it.dom.history.push(it.dom.base_url.clone());
        it.dom.history_pos += 1;
        let p = it.dom.history_pos;
        it.history_state.retain(|k, _| *k < p);
        it.history_state.insert(p, c.arg(0));
    }
    Ok(Value::Undefined)
}

fn history_go(it: &mut Interp, delta: i32) -> Res {
    let n = it.dom.history.len() as i32;
    let target = it.dom.history_pos as i32 + delta;
    if n == 0 || target < 0 || target >= n {
        return Ok(Value::Undefined);
    }
    it.dom.history_pos = target as usize;
    it.dom.base_url = it.dom.history[target as usize].clone();
    let state = it.history_state.get(&(target as usize)).cloned().unwrap_or(Value::Null);
    // popstate fires asynchronously
    let f = it.new_native_data("", 0, |it, c| {
        let st = it.native_data(c);
        let ev = it.make_event("popstate", "PopStateEvent", false, false, &[("state", st)]);
        let g = Value::Obj(it.global.clone());
        it.dispatch_event_obj(&g, &ev)?;
        Ok(Value::Undefined)
    }, state, false);
    it.timer_seq += 1;
    let id = it.timer_seq;
    it.timers.push(Timer { id, due: it.now, interval: None, callback: Value::Obj(f), args: Vec::new(), raf: false });
    Ok(Value::Undefined)
}

// ---------------------------------------------------------------------------
// navigator / screen
// ---------------------------------------------------------------------------

fn setup_navigator(it: &mut Interp, g: &Obj) {
    let n = it.new_object();
    it.def_getter(&n, "userAgent", |it, _| Ok(Value::string(it.dom.user_agent.clone())));
    it.def_getter(&n, "appVersion", |it, _| Ok(Value::string(it.dom.user_agent.trim_start_matches("Mozilla/").to_string())));
    for (k, v) in [
        ("platform", Value::str("Linux x86_64")),
        ("language", Value::str("en-US")),
        ("onLine", Value::Bool(true)),
        ("cookieEnabled", Value::Bool(true)),
        ("hardwareConcurrency", Value::Num(4.0)),
        ("deviceMemory", Value::Num(8.0)),
        ("maxTouchPoints", Value::Num(0.0)),
        ("vendor", Value::str("Google Inc.")),
        ("appName", Value::str("Netscape")),
        ("appCodeName", Value::str("Mozilla")),
        ("product", Value::str("Gecko")),
        ("webdriver", Value::Bool(false)),
        ("doNotTrack", Value::Null),
        ("pdfViewerEnabled", Value::Bool(false)),
    ] {
        it.define_data(&n, Key::str(k), v, ENUMERABLE | CONFIGURABLE);
    }
    let langs = it.new_array(vec![Value::str("en-US"), Value::str("en")]);
    it.define_data(&n, Key::str("languages"), langs, ENUMERABLE | CONFIGURABLE);
    it.def_method(&n, "sendBeacon", 1, |_, _| Ok(Value::Bool(true)));
    it.def_method(&n, "javaEnabled", 0, |_, _| Ok(Value::Bool(false)));
    it.def_method(&n, "vibrate", 1, |_, _| Ok(Value::Bool(false)));
    let clip = it.new_object();
    it.def_method(&clip, "writeText", 1, |it, _| Ok(Value::Obj(it.promise_resolved(Value::Undefined)?)));
    it.def_method(&clip, "readText", 0, |it, _| Ok(Value::Obj(it.promise_resolved(Value::str(""))?)));
    it.define_data(&n, Key::str("clipboard"), Value::Obj(clip), ENUMERABLE | CONFIGURABLE);
    let perms = it.new_object();
    it.def_method(&perms, "query", 1, |it, _| {
        let o = it.new_object();
        it.define_data(&o, Key::str("state"), Value::str("denied"), ALL);
        it.def_method(&o, "addEventListener", 2, |_, _| Ok(Value::Undefined));
        Ok(Value::Obj(it.promise_resolved(Value::Obj(o))?))
    });
    it.define_data(&n, Key::str("permissions"), Value::Obj(perms), ENUMERABLE | CONFIGURABLE);
    let geo = it.new_object();
    for name in ["getCurrentPosition", "watchPosition", "clearWatch"] {
        it.def_method(&geo, name, 1, |_, _| Ok(Value::Undefined));
    }
    it.define_data(&n, Key::str("geolocation"), Value::Obj(geo), ENUMERABLE | CONFIGURABLE);
    it.define_data(g, Key::str("navigator"), Value::Obj(n), ALL);

    let s = it.new_object();
    it.def_getter(&s, "width", |it, _| Ok(Value::Num(it.dom.viewport.0 as f64)));
    it.def_getter(&s, "availWidth", |it, _| Ok(Value::Num(it.dom.viewport.0 as f64)));
    it.def_getter(&s, "height", |it, _| Ok(Value::Num(it.dom.viewport.1 as f64)));
    it.def_getter(&s, "availHeight", |it, _| Ok(Value::Num(it.dom.viewport.1 as f64)));
    it.define_data(&s, Key::str("colorDepth"), Value::Num(24.0), ALL);
    it.define_data(&s, Key::str("pixelDepth"), Value::Num(24.0), ALL);
    let orient = it.new_object();
    it.define_data(&orient, Key::str("type"), Value::str("landscape-primary"), ALL);
    it.define_data(&orient, Key::str("angle"), Value::Num(0.0), ALL);
    it.define_data(&s, Key::str("orientation"), Value::Obj(orient), ALL);
    it.define_data(g, Key::str("screen"), Value::Obj(s), ALL);
}

// ---------------------------------------------------------------------------
// localStorage / sessionStorage
// ---------------------------------------------------------------------------

fn store<'a>(it: &'a mut Interp, session: bool) -> &'a mut alloc::collections::BTreeMap<String, String> {
    if session { &mut it.dom.session_storage } else { &mut it.dom.local_storage }
}

fn storage_kind(it: &mut Interp, v: &Value) -> Res<bool> {
    Ok(it.get_str(v, "__session")?.truthy())
}

fn setup_storage(it: &mut Interp) {
    let proto = it.new_object();
    let g = it.global.clone();
    it.make_ctor("Storage", 0, |it, _| it.type_error("Illegal constructor"), &proto);
    it.def_method(&proto, "getItem", 1, |it, c| {
        let k = it.arg_str(c, 0)?;
        let s = storage_kind(it, &c.this)?;
        Ok(store(it, s).get(&*k).cloned().map_or(Value::Null, Value::string))
    });
    it.def_method(&proto, "setItem", 2, |it, c| {
        let k = it.arg_str(c, 0)?;
        let v = it.arg_str(c, 1)?;
        let s = storage_kind(it, &c.this)?;
        store(it, s).insert(k.to_string(), v.to_string());
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "removeItem", 1, |it, c| {
        let k = it.arg_str(c, 0)?;
        let s = storage_kind(it, &c.this)?;
        store(it, s).remove(&*k);
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "clear", 0, |it, c| {
        let s = storage_kind(it, &c.this)?;
        store(it, s).clear();
        Ok(Value::Undefined)
    });
    it.def_method(&proto, "key", 1, |it, c| {
        let i = to_integer(it.arg_num(c, 0)?).max(0.0) as usize;
        let s = storage_kind(it, &c.this)?;
        Ok(store(it, s).keys().nth(i).cloned().map_or(Value::Null, Value::string))
    });
    it.def_getter(&proto, "length", |it, c| {
        let s = storage_kind(it, &c.this)?;
        Ok(Value::Num(store(it, s).len() as f64))
    });
    let h = it.new_object();
    it.def_method(&h, "get", 3, |it, c| {
        let target = c.arg(0);
        let Value::Str(k) = c.arg(1) else {
            let key = it.key_of(&c.arg(1))?;
            return it.get_v(&target, &key);
        };
        match &*k {
            "getItem" | "setItem" | "removeItem" | "clear" | "key" | "length" | "constructor" | "__session" | "toString" | "hasOwnProperty" => it.get_v(&target, &Key::S(k)),
            _ => {
                let s = storage_kind(it, &target)?;
                Ok(store(it, s).get(&*k).cloned().map_or(Value::Undefined, Value::string))
            }
        }
    });
    it.def_method(&h, "set", 4, |it, c| {
        let target = c.arg(0);
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(true)) };
        let v = it.to_str(&c.arg(2))?;
        let s = storage_kind(it, &target)?;
        store(it, s).insert(k.to_string(), v.to_string());
        Ok(Value::Bool(true))
    });
    it.def_method(&h, "has", 2, |it, c| {
        let target = c.arg(0);
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(false)) };
        let s = storage_kind(it, &target)?;
        Ok(Value::Bool(store(it, s).contains_key(&*k)))
    });
    it.def_method(&h, "deleteProperty", 2, |it, c| {
        let target = c.arg(0);
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(true)) };
        let s = storage_kind(it, &target)?;
        store(it, s).remove(&*k);
        Ok(Value::Bool(true))
    });
    it.def_method(&h, "ownKeys", 1, |it, c| {
        let s = storage_kind(it, &c.arg(0))?;
        let keys: Vec<Value> = store(it, s).keys().cloned().map(Value::string).collect();
        Ok(it.new_array(keys))
    });
    it.def_method(&h, "getOwnPropertyDescriptor", 2, |it, c| {
        let s = storage_kind(it, &c.arg(0))?;
        let Value::Str(k) = c.arg(1) else { return Ok(Value::Undefined) };
        match store(it, s).get(&*k).cloned() {
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
    for (name, session) in [("localStorage", false), ("sessionStorage", true)] {
        let t = it.alloc(Object::new(Some(proto.clone()), Class::Plain));
        it.define_data(&t, Key::str("__session"), Value::Bool(session), 0);
        let p = it.alloc(Object::new(None, Class::Proxy { target: t, handler: h.clone() }));
        it.define_data(&g, Key::str(name), Value::Obj(p), HIDDEN);
    }
}

impl Interp {
    pub fn key_of(&mut self, v: &Value) -> Res<Key> {
        self.to_key(v)
    }
}

// ---------------------------------------------------------------------------
// getComputedStyle / matchMedia
// ---------------------------------------------------------------------------

fn default_display(tag: &str) -> &'static str {
    match tag {
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "section" | "article" | "header" | "footer" | "nav" | "main" | "aside" | "form" | "fieldset" | "blockquote" | "pre" | "hr" | "dl" | "dt" | "dd" | "figure" | "figcaption" | "address" | "details" | "summary" | "body" | "html" | "table" | "center" | "menu" | "dialog" => "block",
        "head" | "script" | "style" | "meta" | "link" | "title" | "template" | "noscript" | "base" | "datalist" => "none",
        "img" | "input" | "button" | "select" | "textarea" | "video" | "canvas" | "iframe" | "audio" => "inline-block",
        "tr" => "table-row",
        "td" | "th" => "table-cell",
        _ => "inline",
    }
}

/// Longhands a computed style enumerates (`length`, `cs[i]`), in Chromium's alphabetical order.
const COMPUTED_PROPS: &[&str] = &[
    "accent-color", "align-content", "align-items", "align-self", "animation-delay", "animation-direction", "animation-duration", "animation-fill-mode",
    "animation-iteration-count", "animation-name", "animation-play-state", "animation-timing-function", "appearance", "aspect-ratio", "backdrop-filter",
    "backface-visibility", "background-attachment", "background-blend-mode", "background-clip", "background-color", "background-image", "background-origin",
    "background-position", "background-repeat", "background-size", "block-size", "border-block-end-color", "border-block-end-style", "border-block-end-width",
    "border-block-start-color", "border-block-start-style", "border-block-start-width", "border-bottom-color", "border-bottom-left-radius",
    "border-bottom-right-radius", "border-bottom-style", "border-bottom-width", "border-collapse", "border-image-outset", "border-image-repeat",
    "border-image-slice", "border-image-source", "border-image-width", "border-inline-end-color", "border-inline-end-style", "border-inline-end-width",
    "border-inline-start-color", "border-inline-start-style", "border-inline-start-width", "border-left-color", "border-left-style", "border-left-width",
    "border-right-color", "border-right-style", "border-right-width", "border-spacing", "border-top-color", "border-top-left-radius", "border-top-right-radius",
    "border-top-style", "border-top-width", "bottom", "box-shadow", "box-sizing", "break-after", "break-before", "break-inside", "caption-side", "caret-color",
    "clear", "clip", "clip-path", "color", "column-count", "column-gap", "column-rule-color", "column-rule-style", "column-rule-width", "column-span",
    "column-width", "contain", "container-name", "container-type", "content", "counter-increment", "counter-reset", "cursor", "direction", "display",
    "empty-cells", "filter", "flex-basis", "flex-direction", "flex-grow", "flex-shrink", "flex-wrap", "float", "font-family", "font-feature-settings",
    "font-kerning", "font-optical-sizing", "font-size", "font-stretch", "font-style", "font-variant", "font-variation-settings", "font-weight", "gap",
    "grid-auto-columns", "grid-auto-flow", "grid-auto-rows", "grid-column-end", "grid-column-start", "grid-row-end", "grid-row-start",
    "grid-template-areas", "grid-template-columns", "grid-template-rows", "height", "hyphens", "image-rendering", "inline-size", "inset-block-end",
    "inset-block-start", "inset-inline-end", "inset-inline-start", "isolation", "justify-content", "justify-items", "justify-self", "left",
    "letter-spacing", "line-break", "line-height", "list-style-image", "list-style-position", "list-style-type", "margin-block-end", "margin-block-start",
    "margin-bottom", "margin-inline-end", "margin-inline-start", "margin-left", "margin-right", "margin-top", "mask-image", "max-block-size", "max-height",
    "max-inline-size", "max-width", "min-block-size", "min-height", "min-inline-size", "min-width", "mix-blend-mode", "object-fit", "object-position",
    "opacity", "order", "orphans", "outline-color", "outline-offset", "outline-style", "outline-width", "overflow-anchor", "overflow-wrap", "overflow-x",
    "overflow-y", "overscroll-behavior-block", "overscroll-behavior-inline", "padding-block-end", "padding-block-start", "padding-bottom",
    "padding-inline-end", "padding-inline-start", "padding-left", "padding-right", "padding-top", "perspective", "perspective-origin", "pointer-events",
    "position", "quotes", "resize", "right", "rotate", "row-gap", "scale", "scroll-behavior", "scroll-margin-block-end", "scroll-padding-block-end",
    "scroll-snap-align", "scroll-snap-type", "scrollbar-gutter", "tab-size", "table-layout", "text-align", "text-align-last", "text-decoration-color",
    "text-decoration-line", "text-decoration-style", "text-indent", "text-overflow", "text-rendering", "text-shadow", "text-transform",
    "text-underline-offset", "text-wrap", "top", "touch-action", "transform", "transform-origin", "transform-style", "transition-delay",
    "transition-duration", "transition-property", "transition-timing-function", "translate", "unicode-bidi", "user-select", "vertical-align",
    "visibility", "white-space", "widows", "width", "will-change", "word-break", "word-spacing", "writing-mode", "z-index", "zoom",
];

fn get_computed_style(it: &mut Interp, c: &Call) -> Res {
    let id = it.node_id(&c.arg(0))?;
    // the target carries CSSStyleDeclaration.prototype so instanceof / toStringTag look right
    let proto = match it.get_str(&Value::Obj(it.global.clone()), "CSSStyleDeclaration")? {
        Value::Obj(ctor) => match it.get_str(&Value::Obj(ctor), "prototype")? {
            Value::Obj(p) => Some(p),
            _ => None,
        },
        _ => None,
    };
    let t = it.alloc(Object::new(proto, Class::Plain));
    it.define_data(&t, Key::str("__id"), Value::Num(id as f64), 0);
    let h = it.protos().extra.get("computed_handler").cloned();
    let h = match h {
        Some(h) => h,
        None => {
            let h = it.new_object();
            it.def_method(&h, "get", 3, |it, c| {
                let target = c.arg(0);
                let id = match it.get_str(&target, "__id")? {
                    Value::Num(n) => n as usize,
                    _ => 0,
                };
                let Value::Str(k) = c.arg(1) else {
                    // symbols (toStringTag, iterator) come from the prototype
                    let key = c.arg(1);
                    let key = it.to_key(&key)?;
                    return it.get_v(&target, &key);
                };
                if let Some(i) = str_index(&k) {
                    return Ok(COMPUTED_PROPS.get(i as usize).map_or(Value::Undefined, |p| Value::str(p)));
                }
                match &*k {
                    "getPropertyValue" => {
                        let f = it.new_native_data("getPropertyValue", 1, |it, c| {
                            let id = it.native_data(c);
                            let id = if let Value::Num(n) = id { n as usize } else { 0 };
                            let n = it.arg_str(c, 0)?;
                            Ok(Value::string(computed_value(it, id, &n)))
                        }, Value::Num(id as f64), false);
                        return Ok(Value::Obj(f));
                    }
                    "getPropertyPriority" => {
                        let f = it.new_native("getPropertyPriority", 1, |_, _| Ok(Value::str("")));
                        return Ok(Value::Obj(f));
                    }
                    "length" => return Ok(Value::Num(COMPUTED_PROPS.len() as f64)),
                    "item" => {
                        let f = it.new_native("item", 1, |it, c| {
                            let i = it.arg_num(c, 0)?;
                            Ok(COMPUTED_PROPS.get(i as usize).map_or(Value::str(""), |p| Value::str(p)))
                        });
                        return Ok(Value::Obj(f));
                    }
                    "cssText" => return Ok(Value::str("")),
                    "toString" => {
                        let f = it.new_native("toString", 0, |_, _| Ok(Value::str("[object CSSStyleDeclaration]")));
                        return Ok(Value::Obj(f));
                    }
                    "__id" => return Ok(Value::Num(id as f64)),
                    _ => {}
                }
                if str_index(&k).is_some() || k.starts_with("__") {
                    return Ok(Value::Undefined);
                }
                let name = if k.starts_with("--") { k.to_string() } else { kebab(&k) };
                let name = if name.starts_with("webkit-") || name.starts_with("moz-") { format!("-{name}") } else { name };
                Ok(Value::string(computed_value(it, id, &name)))
            });
            it.def_method(&h, "set", 4, |_, _| Ok(Value::Bool(true)));
            it.def_method(&h, "has", 2, |_, c| {
                let Value::Str(k) = c.arg(1) else { return Ok(Value::Bool(false)) };
                if let Some(i) = str_index(&k) {
                    return Ok(Value::Bool((i as usize) < COMPUTED_PROPS.len()));
                }
                let name = if k.starts_with("--") { k.to_string() } else { kebab(&k) };
                Ok(Value::Bool(COMPUTED_PROPS.contains(&name.as_str()) || matches!(&*k, "length" | "cssText" | "getPropertyValue" | "getPropertyPriority" | "item" | "setProperty" | "removeProperty" | "parentRule" | "cssFloat")))
            });
            it.def_method(&h, "ownKeys", 1, |it, _| {
                let keys: Vec<Value> = (0..COMPUTED_PROPS.len()).map(|i| Value::string(i.to_string())).collect();
                Ok(it.new_array(keys))
            });
            it.def_method(&h, "getOwnPropertyDescriptor", 2, |it, c| {
                let Value::Str(k) = c.arg(1) else { return Ok(Value::Undefined) };
                let Some(i) = str_index(&k) else { return Ok(Value::Undefined) };
                let Some(p) = COMPUTED_PROPS.get(i as usize) else { return Ok(Value::Undefined) };
                let o = it.new_object();
                it.define_data(&o, Key::str("value"), Value::str(p), ALL);
                it.define_data(&o, Key::str("writable"), Value::Bool(false), ALL);
                it.define_data(&o, Key::str("enumerable"), Value::Bool(true), ALL);
                it.define_data(&o, Key::str("configurable"), Value::Bool(true), ALL);
                Ok(Value::Obj(o))
            });
            it.protos.as_mut().unwrap().extra.insert("computed_handler", h.clone());
            h
        }
    };
    Ok(Value::Obj(it.alloc(Object::new(None, Class::Proxy { target: t, handler: h }))))
}

fn computed_value(it: &mut Interp, id: usize, name: &str) -> String {
    if let Some(list) = it.dom.computed.get(&id) {
        if let Some((_, v)) = list.iter().find(|(k, _)| k == name) {
            return v.clone();
        }
    }
    if let Some(v) = cascaded_value(it, id, name) {
        return v;
    }
    // inline style (as declared) is the best approximation without a computed snapshot
    let inline = it.attr(id, "style").map(|s| parse_decls(&s)).unwrap_or_default();
    if let Some((_, v, _)) = inline.into_iter().find(|(k, _, _)| k == name) {
        return v;
    }
    let tag = it.tag_name(id);
    match name {
        "display" => default_display(&tag).to_string(),
        "visibility" => "visible".into(),
        "position" => "static".into(),
        "opacity" => "1".into(),
        "color" => "rgb(0, 0, 0)".into(),
        "background-color" => "rgba(0, 0, 0, 0)".into(),
        "font-size" => "16px".into(),
        "font-weight" => "400".into(),
        "overflow" | "overflow-x" | "overflow-y" => "visible".into(),
        "float" => "none".into(),
        "width" | "height" => match it.dom.rects.get(&id) {
            Some(r) => format!("{}px", num_to_string(r[if name == "width" { 2 } else { 3 }] as f64)),
            None => "auto".into(),
        },
        "z-index" => "auto".into(),
        "pointer-events" => "auto".into(),
        "cursor" => "auto".into(),
        "transform" => "none".into(),
        "box-sizing" => "content-box".into(),
        _ => String::new(),
    }
}

/// Per-task and per-page limits on full cascade runs for `getComputedStyle`.
const STYLE_RECOMPUTES_PER_TASK: u32 = 6;
const STYLE_RECOMPUTES_TOTAL: u32 = 80;

/// Styles of the whole document, recomputed after DOM mutations (within the limits above; past
/// them a stale snapshot is used as long as it covers every node).
fn document_styles(it: &mut Interp) -> Option<Rc<crate::css::Styles>> {
    let version = it.dom.version;
    let doc = it.dom().ok()?;
    let stale = match &it.dom.style_cache {
        Some((v, s)) if *v == version => return Some(s.clone()),
        Some((_, s)) => Some(s.clone()),
        None => None,
    };
    let (task, total) = it.dom.style_recomputes;
    if stale.as_ref().is_some_and(|s| s.nodes.len() == doc.nodes.len()) && (task >= STYLE_RECOMPUTES_PER_TASK || total >= STYLE_RECOMPUTES_TOTAL) {
        return stale;
    }
    let sources = it.dom.style_sources.clone()?;
    it.dom.style_recomputes = (task + 1, total + 1);
    let mut css = crate::embedded_css(doc, &sources.0, &sources.1);
    if let Some(text) = doc.adopted_css.get(&doc.root) {
        css.push_str(text);
        css.push('\n');
    }
    let sheet = match &it.dom.style_sheet {
        Some((text, sheet)) if *text == css => sheet.clone(),
        _ => {
            let sheet = Rc::new(crate::css::parse(&css));
            it.dom.style_sheet = Some((css, sheet.clone()));
            sheet
        }
    };
    let (w, h) = it.dom.viewport;
    let styles = Rc::new(crate::css::compute_styles(doc, &sheet, crate::css::Viewport::new(w, h)));
    it.dom.style_cache = Some((version, styles.clone()));
    Some(styles)
}

fn px(v: f32) -> String {
    format!("{}px", num_to_string(v as f64))
}

fn dim_str(d: &crate::css_values::Dim) -> String {
    use crate::css_values::Dim;
    match d {
        Dim::Len(p, pct) if *pct == 0.0 => px(*p),
        Dim::Len(p, pct) if *p == 0.0 => format!("{}%", num_to_string(*pct as f64)),
        Dim::Len(p, pct) => format!("calc({}% + {})", num_to_string(*pct as f64), px(*p)),
        Dim::MinContent => "min-content".into(),
        Dim::MaxContent => "max-content".into(),
        Dim::FitContent => "fit-content".into(),
        _ => "auto".into(),
    }
}

fn color_str(c: u32) -> String {
    let (a, r, g, b) = (c >> 24, (c >> 16) & 255, (c >> 8) & 255, c & 255);
    if a == 255 {
        format!("rgb({r}, {g}, {b})")
    } else {
        format!("rgba({r}, {g}, {b}, {})", num_to_string(crate::math::round(a as f64 / 255.0 * 1000.0) / 1000.0))
    }
}

fn side_index(n: &str) -> usize {
    match n {
        "top" => 0,
        "right" => 1,
        "bottom" => 2,
        _ => 3,
    }
}

/// `getComputedStyle` value from the real cascade (None: property not modelled here).
fn cascaded_value(it: &mut Interp, id: usize, name: &str) -> Option<String> {
    use crate::css::*;
    use crate::css_values::Dim;
    let styles = document_styles(it)?;
    let s = styles.nodes.get(id)?;
    let rect = it.dom.rects.get(&id).copied();
    Some(match name {
        n if n.starts_with("--") => s.vars.get(n).cloned().unwrap_or_default(),
        "display" => match s.display {
            Display::None => "none",
            Display::Contents => "contents",
            Display::Block => "block",
            Display::FlowRoot => "flow-root",
            Display::Inline => "inline",
            Display::InlineBlock => "inline-block",
            Display::Flex => "flex",
            Display::InlineFlex => "inline-flex",
            Display::Grid => "grid",
            Display::InlineGrid => "inline-grid",
            Display::ListItem => "list-item",
            Display::Table => "table",
            Display::InlineTable => "inline-table",
            Display::TableRowGroup => "table-row-group",
            Display::TableRow => "table-row",
            Display::TableCell => "table-cell",
            Display::TableCaption => "table-caption",
            Display::TableColumn => "table-column",
        }
        .into(),
        "position" => match s.position {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Fixed => "fixed",
            Position::Sticky => "sticky",
        }
        .into(),
        "float" => match s.float {
            Float::None => "none",
            Float::Left => "left",
            Float::Right => "right",
        }
        .into(),
        "visibility" => if s.visibility_visible { "visible" } else { "hidden" }.into(),
        "opacity" => num_to_string(s.opacity as f64),
        "pointer-events" => if s.pointer_events { "auto" } else { "none" }.into(),
        "z-index" => s.z_index.map_or("auto".into(), |z| z.to_string()),
        "overflow" | "overflow-x" | "overflow-y" => {
            let o = if name == "overflow-y" { s.overflow_y } else { s.overflow_x };
            match o {
                Overflow::Visible => "visible",
                Overflow::Hidden => "hidden",
                Overflow::Scroll => "scroll",
                Overflow::Auto => "auto",
            }
            .into()
        }
        "color" => color_str(s.color),
        "background-color" => color_str(s.background_color),
        "font-size" => px(s.font_size),
        "font-weight" => s.font_weight.to_string(),
        "font-style" => if s.font_italic { "italic" } else { "normal" }.into(),
        "font-family" => s.font_names.as_deref().map_or("serif".into(), |n| n.replace(',', ", ")),
        "line-height" => match (s.line_height_px, s.line_height_number) {
            (Some(p), _) => px(p),
            (_, Some(n)) => px(n * s.font_size),
            _ => "normal".into(),
        },
        "letter-spacing" => if s.letter_spacing == 0.0 { "normal".into() } else { px(s.letter_spacing) },
        "text-align" => match s.text_align {
            TextAlign::Left => "start",
            TextAlign::Right => "right",
            TextAlign::Center => "center",
            TextAlign::Justify => "justify",
        }
        .into(),
        "white-space" => match s.white_space {
            WhiteSpace::Normal => "normal",
            WhiteSpace::Nowrap => "nowrap",
            WhiteSpace::Pre => "pre",
            WhiteSpace::PreWrap => "pre-wrap",
            WhiteSpace::PreLine => "pre-line",
        }
        .into(),
        "box-sizing" => if s.border_box { "border-box" } else { "content-box" }.into(),
        "flex-direction" => match s.flex_direction {
            FlexDirection::Row => "row",
            FlexDirection::RowReverse => "row-reverse",
            FlexDirection::Column => "column",
            FlexDirection::ColumnReverse => "column-reverse",
        }
        .into(),
        "flex-grow" => num_to_string(s.flex_grow as f64),
        "flex-shrink" => num_to_string(s.flex_shrink as f64),
        "top" => dim_str(&s.top),
        "right" => dim_str(&s.right),
        "bottom" => dim_str(&s.bottom),
        "left" => dim_str(&s.left),
        n if n.starts_with("margin-") => dim_str(&s.margin[side_index(&n[7..])]),
        n if n.starts_with("padding-") => dim_str(&s.padding[side_index(&n[8..])]),
        n if n.starts_with("border-") && n.ends_with("-width") && n.len() > 13 => px(s.border_width[side_index(&n[7..n.len() - 6])]),
        "width" | "height" => match rect {
            Some(r) => px(r[if name == "width" { 2 } else { 3 }]),
            None => dim_str(if name == "width" { &s.width } else { &s.height }),
        },
        "max-width" => if matches!(s.max_width, Dim::Auto) { "none".into() } else { dim_str(&s.max_width) },
        "max-height" => if matches!(s.max_height, Dim::Auto) { "none".into() } else { dim_str(&s.max_height) },
        "min-width" => dim_str(&s.min_width),
        "min-height" => dim_str(&s.min_height),
        "transform" if s.transform.is_empty() && s.transform_scale == 1.0 => "none".into(),
        _ => return None,
    })
}

fn match_media(it: &mut Interp, c: &Call) -> Res {
    let q = it.arg_str(c, 0)?;
    let (w, h) = it.dom.viewport;
    let matches = crate::css::media_matches(&q, crate::css::Viewport::new(w, h));
    let proto = match it.get_str(&Value::Obj(it.global.clone()), "__MediaQueryListProto")? {
        Value::Obj(o) => Some(o),
        _ => None,
    };
    let o = match proto {
        Some(p) => it.alloc(Object::new(Some(p), Class::Plain)),
        None => it.new_object(),
    };
    it.define_data(&o, Key::str("matches"), Value::Bool(matches), ALL);
    it.define_data(&o, Key::str("media"), Value::Str(q), ALL);
    it.define_data(&o, Key::str("onchange"), Value::Null, ALL);
    for n in ["addListener", "removeListener", "addEventListener", "removeEventListener"] {
        it.def_method(&o, n, 2, |_, _| Ok(Value::Undefined));
    }
    it.def_method(&o, "dispatchEvent", 1, |_, _| Ok(Value::Bool(true)));
    Ok(Value::Obj(o))
}

#[allow(dead_code)]
fn unused(_: NodeKind) {}
