//! console, timers, base64, structuredClone, performance, typed arrays.
use super::*;
use alloc::{format, vec};

pub fn setup(it: &mut Interp) {
    setup_console(it);
    setup_timers(it);
    super::binary::setup(it);
    it.global_fn("atob", 1, atob);
    it.global_fn("btoa", 1, btoa);
    it.global_fn("structuredClone", 1, |it, c| {
        let mut seen = Vec::new();
        clone_value(it, &c.arg(0), &mut seen)
    });
    let perf = it.new_object();
    let g = it.global.clone();
    it.define_data(&g, Key::str("performance"), Value::Obj(perf.clone()), HIDDEN);
    it.def_method(&perf, "now", 0, |it, _| Ok(Value::Num(it.now + 12.5)));
    it.def_method(&perf, "mark", 1, |_, _| Ok(Value::Undefined));
    it.def_method(&perf, "measure", 1, |_, _| Ok(Value::Undefined));
    it.def_method(&perf, "getEntriesByName", 1, |it, _| Ok(it.new_array(Vec::new())));
    it.def_method(&perf, "getEntriesByType", 1, |it, _| Ok(it.new_array(Vec::new())));
    it.def_method(&perf, "getEntries", 0, |it, _| Ok(it.new_array(Vec::new())));
    it.define_data(&perf, Key::str("timeOrigin"), Value::Num(it.dom.epoch_ms), ALL);
}

// ---------------------------------------------------------------------------
// console
// ---------------------------------------------------------------------------

pub fn inspect(it: &mut Interp, v: &Value, depth: usize, top: bool, seen: &mut Vec<usize>) -> String {
    match v {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => if *n == 0.0 && n.is_sign_negative() { "-0".into() } else { num_to_string(*n) },
        Value::Str(s) => {
            if top {
                s.to_string()
            } else {
                format!("'{}'", s.replace('\'', "\\'").replace('\n', "\\n"))
            }
        }
        Value::Sym(s) => format!("Symbol({})", s.0.desc.as_deref().unwrap_or("")),
        Value::Obj(o) => {
            if seen.contains(&o.id()) {
                return "[Circular *1]".into();
            }
            if o.is_callable() {
                let name = it.get_obj(o, &it.names.name.clone(), v).ok().map(|n| it.to_string_lossy(&n)).unwrap_or_default();
                let is_class = matches!(&o.borrow().class, Class::Function(Func::Closure(c)) if c.is_class_ctor);
                return if is_class { format!("[class {}]", if name.is_empty() { "(anonymous)" } else { &name }) } else if name.is_empty() { "[Function (anonymous)]".into() } else { format!("[Function: {name}]") };
            }
            enum K {
                Arr(Vec<Value>),
                Err,
                Date(f64),
                Re,
                Map(Vec<(Value, Value)>),
                Set(Vec<Value>),
                Prom(Option<(bool, Value)>),
                Plain,
                Prim(Value),
            }
            let kind = match &o.borrow().class {
                Class::Array(a) => K::Arr(a.clone()),
                Class::Error => K::Err,
                Class::Date(t) => K::Date(*t),
                Class::RegExp(_) => K::Re,
                Class::Map(d) => K::Map(d.entries.iter().flatten().cloned().collect()),
                Class::Set(d) => K::Set(d.entries.iter().flatten().map(|e| e.0.clone()).collect()),
                Class::Promise(d) => K::Prom(match &d.state {
                    PromiseState::Pending => None,
                    PromiseState::Fulfilled(v) => Some((true, v.clone())),
                    PromiseState::Rejected(v) => Some((false, v.clone())),
                }),
                Class::Num(n) => K::Prim(Value::Num(*n)),
                Class::Str(s) => K::Prim(Value::Str(s.clone())),
                Class::Bool(b) => K::Prim(Value::Bool(*b)),
                _ => K::Plain,
            };
            match kind {
                K::Err => it.error_text(v),
                K::Date(t) => if t.is_nan() { "Invalid Date".into() } else { super::date::iso(t) },
                K::Re => it.to_string_lossy(v),
                K::Prim(p) => format!("[{}: {}]", p.type_of().chars().next().map(|c| c.to_ascii_uppercase()).into_iter().chain(p.type_of().chars().skip(1)).collect::<String>(), inspect(it, &p, depth, false, seen)),
                K::Prom(st) => match st {
                    None => "Promise { <pending> }".into(),
                    Some((true, v)) => format!("Promise {{ {} }}", inspect(it, &v, depth + 1, false, seen)),
                    Some((false, v)) => format!("Promise {{ <rejected> {} }}", inspect(it, &v, depth + 1, false, seen)),
                },
                K::Arr(items) => {
                    if depth > 2 {
                        return "[Array]".into();
                    }
                    seen.push(o.id());
                    let mut parts: Vec<String> = items.iter().take(100).map(|x| inspect(it, x, depth + 1, false, seen)).collect();
                    if items.len() > 100 {
                        parts.push(format!("... {} more items", items.len() - 100));
                    }
                    seen.pop();
                    wrap("[", "]", parts)
                }
                K::Map(es) => {
                    if depth > 2 {
                        return "[Map]".into();
                    }
                    seen.push(o.id());
                    let parts: Vec<String> = es.iter().map(|(k, x)| format!("{} => {}", inspect(it, k, depth + 1, false, seen), inspect(it, x, depth + 1, false, seen))).collect();
                    seen.pop();
                    format!("Map({}) {}", es.len(), wrap("{", "}", parts))
                }
                K::Set(es) => {
                    if depth > 2 {
                        return "[Set]".into();
                    }
                    seen.push(o.id());
                    let parts: Vec<String> = es.iter().map(|k| inspect(it, k, depth + 1, false, seen)).collect();
                    seen.pop();
                    format!("Set({}) {}", es.len(), wrap("{", "}", parts))
                }
                K::Plain => {
                    let class_name = it
                        .get_obj(o, &it.names.constructor.clone(), v)
                        .ok()
                        .and_then(|c| if let Value::Obj(co) = &c { it.get_obj(co, &it.names.name.clone(), &c).ok() } else { None })
                        .map(|n| it.to_string_lossy(&n))
                        .unwrap_or_default();
                    if depth > 2 {
                        return if class_name.is_empty() || class_name == "Object" { "[Object]".into() } else { format!("[{class_name}]") };
                    }
                    seen.push(o.id());
                    let keys = it.own_keys(o).unwrap_or_default();
                    let mut parts = Vec::new();
                    for k in keys {
                        let Some(p) = it.own_prop(o, &k) else { continue };
                        if p.flags & ENUMERABLE == 0 {
                            continue;
                        }
                        let ks = match &k {
                            Key::S(s) => {
                                if s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') && !s.is_empty() && !s.starts_with(|c: char| c.is_ascii_digit()) { s.to_string() } else { format!("'{s}'") }
                            }
                            Key::Y(id) => format!("[Symbol({})]", it.symbol_by_id(*id).and_then(|s| s.0.desc.clone()).as_deref().unwrap_or("")),
                        };
                        let vs = match p.slot {
                            Slot::Data(x) => inspect(it, &x, depth + 1, false, seen),
                            Slot::Accessor(g, s) => match (g.is_some(), s.is_some()) {
                                (true, true) => "[Getter/Setter]".into(),
                                (true, false) => "[Getter]".into(),
                                _ => "[Setter]".into(),
                            },
                        };
                        parts.push(format!("{ks}: {vs}"));
                    }
                    seen.pop();
                    let body = wrap("{", "}", parts);
                    if class_name.is_empty() || class_name == "Object" { body } else { format!("{class_name} {body}") }
                }
            }
        }
    }
}

fn wrap(open: &str, close: &str, parts: Vec<String>) -> String {
    if parts.is_empty() {
        return format!("{open}{close}");
    }
    let total: usize = parts.iter().map(|p| p.len() + 2).sum();
    if total <= 72 && !parts.iter().any(|p| p.contains('\n')) {
        format!("{open} {} {close}", parts.join(", "))
    } else {
        let inner: Vec<String> = parts.iter().map(|p| p.replace('\n', "\n  ")).collect();
        format!("{open}\n  {}\n{close}", inner.join(",\n  "))
    }
}

pub fn format_log(it: &mut Interp, args: &[Value]) -> String {
    let mut out = String::new();
    let mut rest = 0;
    if let Some(Value::Str(f)) = args.first() {
        if f.contains('%') && args.len() > 1 {
            let mut idx = 1;
            let chars: Vec<char> = f.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '%' && i + 1 < chars.len() {
                    let spec = chars[i + 1];
                    match spec {
                        's' | 'd' | 'i' | 'f' | 'o' | 'O' | 'j' if idx < args.len() => {
                            let a = args[idx].clone();
                            idx += 1;
                            let s = match spec {
                                's' => inspect(it, &a, 0, true, &mut Vec::new()),
                                'd' | 'i' => {
                                    let n = it.to_number(&a).unwrap_or(f64::NAN);
                                    num_to_string(if spec == 'i' { crate::math::trunc(n) } else { n })
                                }
                                'f' => num_to_string(it.to_number(&a).unwrap_or(f64::NAN)),
                                _ => inspect(it, &a, 0, false, &mut Vec::new()),
                            };
                            out.push_str(&s);
                            i += 2;
                            continue;
                        }
                        '%' => {
                            out.push('%');
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
                out.push(chars[i]);
                i += 1;
            }
            rest = idx;
            for a in &args[rest..] {
                out.push(' ');
                out.push_str(&inspect(it, a, 0, true, &mut Vec::new()));
            }
            return out;
        }
    }
    for (i, a) in args.iter().enumerate().skip(rest) {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&inspect(it, a, 0, true, &mut Vec::new()));
    }
    out
}

fn log_level(it: &mut Interp, c: &Call, level: &'static str) -> Res {
    let text = format_log(it, c.args);
    let indent = it.dom_indent();
    let text = if indent > 0 { text.lines().map(|l| format!("{}{l}", "  ".repeat(indent))).collect::<Vec<_>>().join("\n") } else { text };
    it.logs.push(LogEntry { level, text });
    Ok(Value::Undefined)
}

impl Interp {
    fn dom_indent(&self) -> usize {
        self.console_indent
    }
}

fn setup_console(it: &mut Interp) {
    let console = it.new_object();
    let g = it.global.clone();
    it.define_data(&g, Key::str("console"), Value::Obj(console.clone()), HIDDEN);
    it.def_method(&console, "log", 0, |it, c| log_level(it, c, "log"));
    it.def_method(&console, "info", 0, |it, c| log_level(it, c, "info"));
    it.def_method(&console, "debug", 0, |it, c| log_level(it, c, "debug"));
    it.def_method(&console, "warn", 0, |it, c| log_level(it, c, "warn"));
    it.def_method(&console, "error", 0, |it, c| log_level(it, c, "error"));
    it.def_method(&console, "trace", 0, |it, c| log_level(it, c, "trace"));
    it.def_method(&console, "dir", 0, |it, c| log_level(it, c, "log"));
    it.def_method(&console, "dirxml", 0, |it, c| log_level(it, c, "log"));
    it.def_method(&console, "table", 1, |it, c| log_level(it, c, "log"));
    it.def_method(&console, "group", 0, |it, c| {
        if !c.args.is_empty() {
            log_level(it, c, "log")?;
        }
        it.console_indent += 1;
        Ok(Value::Undefined)
    });
    it.def_method(&console, "groupCollapsed", 0, |it, c| {
        if !c.args.is_empty() {
            log_level(it, c, "log")?;
        }
        it.console_indent += 1;
        Ok(Value::Undefined)
    });
    it.def_method(&console, "groupEnd", 0, |it, _| {
        it.console_indent = it.console_indent.saturating_sub(1);
        Ok(Value::Undefined)
    });
    it.def_method(&console, "assert", 0, |it, c| {
        if !c.arg(0).truthy() {
            let rest: Vec<Value> = c.args.iter().skip(1).cloned().collect();
            let text = format_log(it, &rest);
            it.logs.push(LogEntry { level: "error", text: format!("Assertion failed{}{text}", if text.is_empty() { "" } else { ": " }) });
        }
        Ok(Value::Undefined)
    });
    it.def_method(&console, "count", 0, |it, c| {
        let label = match c.arg(0) {
            Value::Undefined => "default".to_string(),
            v => it.to_str(&v)?.to_string(),
        };
        let n = it.counters.entry(label.clone()).or_insert(0);
        *n += 1;
        let n = *n;
        it.logs.push(LogEntry { level: "log", text: format!("{label}: {n}") });
        Ok(Value::Undefined)
    });
    for n in ["time", "timeEnd", "timeLog", "clear", "countReset", "profile", "profileEnd", "timeStamp"] {
        it.def_method(&console, n, 0, |_, _| Ok(Value::Undefined));
    }
}

// ---------------------------------------------------------------------------
// timers
// ---------------------------------------------------------------------------

fn add_timer(it: &mut Interp, c: &Call, repeat: bool) -> Res {
    let f = c.arg(0);
    let mut delay = it.arg_num(c, 1)?;
    if !delay.is_finite() || delay < 0.0 {
        delay = 0.0;
    }
    if !f.as_obj().is_some_and(|o| o.is_callable()) {
        return Ok(Value::Num(0.0));
    }
    it.timer_seq += 1;
    let id = it.timer_seq;
    let args: Vec<Value> = c.args.iter().skip(2).cloned().collect();
    let due = it.now + delay;
    it.timers.push(Timer { id, due, interval: if repeat { Some(delay.max(1.0)) } else { None }, callback: f, args, raf: false });
    Ok(Value::Num(id as f64))
}

fn clear_timer(it: &mut Interp, c: &Call) -> Res {
    if let Value::Num(n) = c.arg(0) {
        let id = n as u32;
        it.timers.retain(|t| t.id != id);
    }
    Ok(Value::Undefined)
}

fn setup_timers(it: &mut Interp) {
    it.global_fn("setTimeout", 2, |it, c| add_timer(it, c, false));
    it.global_fn("setInterval", 2, |it, c| add_timer(it, c, true));
    it.global_fn("setImmediate", 1, |it, c| {
        let f = c.arg(0);
        it.timer_seq += 1;
        let id = it.timer_seq;
        let args: Vec<Value> = c.args.iter().skip(1).cloned().collect();
        it.timers.push(Timer { id, due: it.now, interval: None, callback: f, args, raf: false });
        Ok(Value::Num(id as f64))
    });
    it.global_fn("clearTimeout", 1, clear_timer);
    it.global_fn("clearInterval", 1, clear_timer);
    it.global_fn("clearImmediate", 1, clear_timer);
    it.global_fn("cancelAnimationFrame", 1, clear_timer);
    it.global_fn("cancelIdleCallback", 1, clear_timer);
    it.global_fn("requestAnimationFrame", 1, |it, c| {
        let f = c.arg(0);
        it.timer_seq += 1;
        let id = it.timer_seq;
        let due = m_next16(it.now);
        it.timers.push(Timer { id, due, interval: None, callback: f, args: Vec::new(), raf: true });
        Ok(Value::Num(id as f64))
    });
    it.global_fn("requestIdleCallback", 1, |it, c| {
        let f = c.arg(0);
        it.timer_seq += 1;
        let id = it.timer_seq;
        let deadline = it.new_object();
        it.def_method(&deadline, "timeRemaining", 0, |_, _| Ok(Value::Num(50.0)));
        it.define_data(&deadline, Key::str("didTimeout"), Value::Bool(false), ALL);
        it.timers.push(Timer { id, due: it.now + 1.0, interval: None, callback: f, args: vec![Value::Obj(deadline)], raf: false });
        Ok(Value::Num(id as f64))
    });
}

fn m_next16(t: f64) -> f64 {
    (crate::math::floor(t / 16.0) + 1.0) * 16.0
}

// ---------------------------------------------------------------------------
// base64
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn btoa(it: &mut Interp, c: &Call) -> Res {
    let s = it.arg_str(c, 0)?;
    let mut bytes = Vec::new();
    for ch in s.chars() {
        if (ch as u32) > 255 {
            return Err(it.error_ctl("InvalidCharacterError", "The string to be encoded contains characters outside of the Latin1 range."));
        }
        bytes.push(ch as u8);
    }
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    Ok(Value::string(out))
}

fn atob(it: &mut Interp, c: &Call) -> Res {
    let s = it.arg_str(c, 0)?;
    let mut vals = Vec::new();
    for ch in s.chars().filter(|c| !c.is_ascii_whitespace()) {
        if ch == '=' {
            break;
        }
        match B64.iter().position(|b| *b as char == ch) {
            Some(v) => vals.push(v as u32),
            None => return Err(it.error_ctl("InvalidCharacterError", "The string to be decoded is not correctly encoded.")),
        }
    }
    let mut out = String::new();
    for chunk in vals.chunks(4) {
        let mut n = 0u32;
        for (i, v) in chunk.iter().enumerate() {
            n |= v << (18 - 6 * i);
        }
        out.push(((n >> 16) & 255) as u8 as char);
        if chunk.len() > 2 {
            out.push(((n >> 8) & 255) as u8 as char);
        }
        if chunk.len() > 3 {
            out.push((n & 255) as u8 as char);
        }
    }
    Ok(Value::string(out))
}

// ---------------------------------------------------------------------------
// structuredClone
// ---------------------------------------------------------------------------

fn clone_value(it: &mut Interp, v: &Value, seen: &mut Vec<(Obj, Obj)>) -> Res {
    let Value::Obj(o) = v else {
        if let Value::Sym(_) = v {
            return Err(it.error_ctl("DataCloneError", "Symbol could not be cloned."));
        }
        return Ok(v.clone());
    };
    if let Some((_, n)) = seen.iter().find(|(a, _)| a.ptr_eq(o)) {
        return Ok(Value::Obj(n.clone()));
    }
    if o.is_callable() {
        return Err(it.error_ctl("DataCloneError", "function could not be cloned."));
    }
    enum K {
        Arr(Vec<Value>),
        Date(f64),
        Map(Vec<(Value, Value)>),
        Set(Vec<Value>),
        Prim(Class),
        Plain,
    }
    let k = match &o.borrow().class {
        Class::Array(a) => K::Arr(a.clone()),
        Class::Date(t) => K::Date(*t),
        Class::Map(d) => K::Map(d.entries.iter().flatten().cloned().collect()),
        Class::Set(d) => K::Set(d.entries.iter().flatten().map(|e| e.0.clone()).collect()),
        Class::Num(n) => K::Prim(Class::Num(*n)),
        Class::Str(s) => K::Prim(Class::Str(s.clone())),
        Class::Bool(b) => K::Prim(Class::Bool(*b)),
        Class::Node(_) => return Err(it.error_ctl("DataCloneError", "Node could not be cloned.")),
        _ => K::Plain,
    };
    match k {
        K::Arr(items) => {
            let arr = it.new_array(Vec::new());
            let ao = arr.as_obj().unwrap().clone();
            seen.push((o.clone(), ao.clone()));
            let mut out = Vec::new();
            for x in items {
                out.push(clone_value(it, &x, seen)?);
            }
            if let Class::Array(a) = &mut ao.borrow_mut().class {
                *a = out;
            }
            Ok(arr)
        }
        K::Date(t) => Ok(Value::Obj(it.alloc(Object::new(Some(it.protos().date.clone()), Class::Date(t))))),
        K::Prim(c) => {
            let proto = match &c {
                Class::Num(_) => it.protos().number.clone(),
                Class::Str(_) => it.protos().string.clone(),
                _ => it.protos().boolean.clone(),
            };
            Ok(Value::Obj(it.alloc(Object::new(Some(proto), c))))
        }
        K::Map(es) => {
            let n = it.alloc(Object::new(Some(it.protos().map.clone()), Class::Map(Box::default())));
            seen.push((o.clone(), n.clone()));
            for (a, b) in es {
                let (a, b) = (clone_value(it, &a, seen)?, clone_value(it, &b, seen)?);
                if let Class::Map(d) = &mut n.borrow_mut().class {
                    d.set(a, b);
                }
            }
            Ok(Value::Obj(n))
        }
        K::Set(es) => {
            let n = it.alloc(Object::new(Some(it.protos().set.clone()), Class::Set(Box::default())));
            seen.push((o.clone(), n.clone()));
            for a in es {
                let a = clone_value(it, &a, seen)?;
                if let Class::Set(d) = &mut n.borrow_mut().class {
                    d.set(a.clone(), a);
                }
            }
            Ok(Value::Obj(n))
        }
        K::Plain => {
            let n = it.new_object();
            seen.push((o.clone(), n.clone()));
            for key in it.own_keys(o)? {
                if let Key::S(_) = key {
                    if it.own_prop(o, &key).is_some_and(|p| p.flags & ENUMERABLE != 0) {
                        let x = it.get_v(v, &key)?;
                        let x = clone_value(it, &x, seen)?;
                        it.define_data(&n, key, x, ALL);
                    }
                }
            }
            Ok(Value::Obj(n))
        }
    }
}

