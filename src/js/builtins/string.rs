//! String and RegExp.
use super::*;
use crate::js::regex;
use alloc::{format, vec};

pub fn setup(it: &mut Interp) {
    let proto = it.protos().string.clone();
    let ctor = it.make_ctor("String", 1, string_ctor, &proto);
    it.def_method(&ctor, "fromCharCode", 1, |it, c| {
        let mut units = Vec::new();
        for a in c.args {
            units.push(to_int32(it.to_number(a)?) as u16);
        }
        Ok(Value::string(from_u16(&units)))
    });
    it.def_method(&ctor, "fromCodePoint", 1, |it, c| {
        let mut s = String::new();
        for a in c.args {
            let n = it.to_number(a)?;
            match char::from_u32(n as u32).filter(|_| n >= 0.0 && n == crate::math::trunc(n)) {
                Some(ch) => s.push(ch),
                None => return Err(it.error_ctl("RangeError", &format!("Invalid code point {}", num_to_string(n)))),
            }
        }
        Ok(Value::string(s))
    });
    it.def_method(&ctor, "raw", 1, |it, c| {
        let strings = it.get_v(&c.arg(0), &Key::str("raw"))?;
        let parts = it.array_like_to_vec(&strings)?;
        let mut out = String::new();
        for (i, p) in parts.iter().enumerate() {
            out.push_str(&it.to_str(p)?);
            if i + 1 < parts.len() && i + 1 < c.args.len() {
                out.push_str(&it.to_str(&c.args[i + 1])?);
            }
        }
        Ok(Value::string(out))
    });

    let methods: &[(&str, u32, NativeFn)] = &[
        ("toString", 0, |it, c| this_prim_str(it, c)),
        ("valueOf", 0, |it, c| this_prim_str(it, c)),
        ("charAt", 1, char_at),
        ("charCodeAt", 1, char_code_at),
        ("codePointAt", 1, code_point_at),
        ("at", 1, at),
        ("indexOf", 1, index_of),
        ("lastIndexOf", 1, last_index_of),
        ("includes", 1, includes),
        ("startsWith", 1, starts_with),
        ("endsWith", 1, ends_with),
        ("slice", 2, slice),
        ("substring", 2, substring),
        ("substr", 2, substr),
        ("toUpperCase", 0, |it, c| Ok(Value::string(this_str(it, c)?.to_uppercase()))),
        ("toLowerCase", 0, |it, c| Ok(Value::string(this_str(it, c)?.to_lowercase()))),
        ("toLocaleUpperCase", 0, |it, c| Ok(Value::string(this_str(it, c)?.to_uppercase()))),
        ("toLocaleLowerCase", 0, |it, c| Ok(Value::string(this_str(it, c)?.to_lowercase()))),
        ("trim", 0, |it, c| Ok(Value::str(this_str(it, c)?.trim_matches(is_js_ws)))),
        ("trimStart", 0, |it, c| Ok(Value::str(this_str(it, c)?.trim_start_matches(is_js_ws)))),
        ("trimEnd", 0, |it, c| Ok(Value::str(this_str(it, c)?.trim_end_matches(is_js_ws)))),
        ("trimLeft", 0, |it, c| Ok(Value::str(this_str(it, c)?.trim_start_matches(is_js_ws)))),
        ("trimRight", 0, |it, c| Ok(Value::str(this_str(it, c)?.trim_end_matches(is_js_ws)))),
        ("padStart", 2, |it, c| pad(it, c, true)),
        ("padEnd", 2, |it, c| pad(it, c, false)),
        ("repeat", 1, repeat),
        ("concat", 1, concat),
        ("split", 2, split),
        ("replace", 2, |it, c| replace(it, c, false)),
        ("replaceAll", 2, |it, c| replace(it, c, true)),
        ("match", 1, match_),
        ("matchAll", 1, match_all),
        ("search", 1, search),
        ("localeCompare", 1, locale_compare),
        ("normalize", 0, |it, c| Ok(Value::Str(this_str(it, c)?))),
        ("isWellFormed", 0, |_, _| Ok(Value::Bool(true))),
        ("anchor", 1, |it, c| html_wrap(it, c, "a", "name")),
        ("link", 1, |it, c| html_wrap(it, c, "a", "href")),
    ];
    for (n, l, f) in methods {
        it.def_method(&proto, n, *l, *f);
    }
    let iter_sym = it.wk().iterator.clone();
    it.def_sym_method(&proto, &iter_sym, "[Symbol.iterator]", 0, |it, c| {
        let s = this_str(it, c)?;
        let items: Vec<Value> = s.chars().map(|ch| Value::string(ch.to_string())).collect();
        Ok(it.make_list_iterator(items))
    });
    setup_regexp(it);
}

fn html_wrap(it: &mut Interp, c: &Call, tag: &str, attr: &str) -> Res {
    let s = this_str(it, c)?;
    let v = it.arg_str(c, 0)?;
    Ok(Value::string(format!("<{tag} {attr}=\"{}\">{s}</{tag}>", v.replace('"', "&quot;"))))
}

fn string_ctor(it: &mut Interp, c: &Call) -> Res {
    let s: JsStr = if c.args.is_empty() {
        Rc::from("")
    } else if c.new_target.is_none() {
        if let Value::Sym(sym) = &c.args[0] {
            Rc::from(format!("Symbol({})", sym.0.desc.as_deref().unwrap_or("")))
        } else {
            it.to_str(&c.args[0])?
        }
    } else {
        it.to_str(&c.args[0])?
    };
    if c.new_target.is_none() {
        return Ok(Value::Str(s));
    }
    let proto = it.protos().string.clone();
    let p = it.proto_from_call(c, &proto)?;
    Ok(Value::Obj(it.alloc(Object::new(Some(p), Class::Str(s)))))
}

pub fn this_str(it: &mut Interp, c: &Call) -> Res<JsStr> {
    match &c.this {
        Value::Str(s) => Ok(s.clone()),
        Value::Undefined | Value::Null => it.type_error("String.prototype method called on null or undefined"),
        v => it.to_str(v),
    }
}

fn this_prim_str(it: &mut Interp, c: &Call) -> Res {
    match &c.this {
        Value::Str(_) => Ok(c.this.clone()),
        Value::Obj(o) => match &o.borrow().class {
            Class::Str(s) => Ok(Value::Str(s.clone())),
            _ => Err(it.error_ctl("TypeError", "String.prototype.toString requires that 'this' be a String")),
        },
        _ => it.type_error("String.prototype.toString requires that 'this' be a String"),
    }
}

fn clamp_pos(n: f64, len: usize) -> usize {
    let n = to_integer(n);
    if n < 0.0 { 0 } else { n.min(len as f64) as usize }
}

fn char_at(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let n = to_integer(it.arg_num(c, 0)?);
    if n < 0.0 {
        return Ok(Value::str(""));
    }
    Ok(match it.str_at16(&s, n as usize) {
        Some(u) => Value::string(from_u16(&[u])),
        None => Value::str(""),
    })
}

fn char_code_at(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let n = to_integer(it.arg_num(c, 0)?);
    if n < 0.0 {
        return Ok(Value::Num(f64::NAN));
    }
    Ok(Value::Num(it.str_at16(&s, n as usize).map_or(f64::NAN, |u| u as f64)))
}

fn code_point_at(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let n = to_integer(it.arg_num(c, 0)?);
    if n < 0.0 {
        return Ok(Value::Undefined);
    }
    let i = n as usize;
    let Some(u) = it.str_at16(&s, i) else { return Ok(Value::Undefined) };
    if (0xD800..0xDC00).contains(&u) {
        if let Some(l) = it.str_at16(&s, i + 1) {
            if (0xDC00..0xE000).contains(&l) {
                return Ok(Value::Num((0x10000 + ((u as u32 - 0xD800) << 10) + (l as u32 - 0xDC00)) as f64));
            }
        }
    }
    Ok(Value::Num(u as f64))
}

fn at(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let len = u16_len(&s) as f64;
    let n = to_integer(it.arg_num(c, 0)?);
    let i = if n < 0.0 { len + n } else { n };
    if i < 0.0 || i >= len {
        return Ok(Value::Undefined);
    }
    Ok(Value::string(u16_slice(&s, i as usize, i as usize + 1)))
}

fn no_regex(it: &mut Interp, v: &Value, what: &str) -> Res<()> {
    if let Value::Obj(o) = v {
        if matches!(o.borrow().class, Class::RegExp(_)) {
            return it.type_error(&format!("First argument to String.prototype.{what} must not be a regular expression"));
        }
    }
    Ok(())
}

fn index_of(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let needle = it.arg_str(c, 0)?;
    let from = clamp_pos(it.arg_num(c, 1)?, u16_len(&s));
    Ok(Value::Num(u16_index_of(&s, &needle, from).map_or(-1.0, |i| i as f64)))
}

fn last_index_of(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let needle = it.arg_str(c, 0)?;
    let n = it.arg_num(c, 1)?;
    let from = if n.is_nan() { usize::MAX } else { clamp_pos(n, u16_len(&s)) };
    Ok(Value::Num(u16_last_index_of(&s, &needle, from).map_or(-1.0, |i| i as f64)))
}

fn includes(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    no_regex(it, &c.arg(0), "includes")?;
    let needle = it.arg_str(c, 0)?;
    let from = clamp_pos(it.arg_num(c, 1)?, u16_len(&s));
    Ok(Value::Bool(u16_index_of(&s, &needle, from).is_some()))
}

fn starts_with(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    no_regex(it, &c.arg(0), "startsWith")?;
    let needle = it.arg_str(c, 0)?;
    let len = u16_len(&s);
    let from = clamp_pos(it.arg_num(c, 1)?, len);
    let tail = u16_slice(&s, from, len);
    Ok(Value::Bool(tail.starts_with(&*needle)))
}

fn ends_with(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    no_regex(it, &c.arg(0), "endsWith")?;
    let needle = it.arg_str(c, 0)?;
    let len = u16_len(&s);
    let end = match c.arg(1) {
        Value::Undefined => len,
        v => clamp_pos(it.to_number(&v)?, len),
    };
    let head = u16_slice(&s, 0, end);
    Ok(Value::Bool(head.ends_with(&*needle)))
}

fn slice(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let len = u16_len(&s);
    let rel = |n: f64| -> usize {
        let n = to_integer(n);
        if n < 0.0 { (len as f64 + n).max(0.0) as usize } else { n.min(len as f64) as usize }
    };
    let st = rel(it.arg_num(c, 0)?);
    let en = match c.arg(1) {
        Value::Undefined => len,
        v => rel(it.to_number(&v)?),
    };
    Ok(Value::string(if st < en { u16_slice(&s, st, en) } else { String::new() }))
}

fn substring(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let len = u16_len(&s);
    let a = clamp_pos(it.arg_num(c, 0)?, len);
    let b = match c.arg(1) {
        Value::Undefined => len,
        v => clamp_pos(it.to_number(&v)?, len),
    };
    Ok(Value::string(u16_slice(&s, a.min(b), a.max(b))))
}

fn substr(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let len = u16_len(&s);
    let n = to_integer(it.arg_num(c, 0)?);
    let st = if n < 0.0 { (len as f64 + n).max(0.0) as usize } else { n.min(len as f64) as usize };
    let l = match c.arg(1) {
        Value::Undefined => len - st,
        v => (to_integer(it.to_number(&v)?).max(0.0) as usize).min(len - st),
    };
    Ok(Value::string(u16_slice(&s, st, st + l)))
}

fn pad(it: &mut Interp, c: &Call, start: bool) -> Res {
    let s = this_str(it, c)?;
    let target = to_integer(it.arg_num(c, 0)?).max(0.0) as usize;
    let fill: JsStr = match c.arg(1) {
        Value::Undefined => Rc::from(" "),
        v => it.to_str(&v)?,
    };
    let len = u16_len(&s);
    if target <= len || fill.is_empty() {
        return Ok(Value::Str(s));
    }
    if target > 100_000_000 {
        return Err(it.error_ctl("RangeError", "Invalid string length"));
    }
    let need = target - len;
    let fill_u: Vec<u16> = to_u16(&fill);
    let mut padding: Vec<u16> = Vec::with_capacity(need);
    while padding.len() < need {
        let take = (need - padding.len()).min(fill_u.len());
        padding.extend_from_slice(&fill_u[..take]);
    }
    let p = from_u16(&padding);
    Ok(Value::string(if start { format!("{p}{s}") } else { format!("{s}{p}") }))
}

fn repeat(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let n = to_integer(it.arg_num(c, 0)?);
    if n < 0.0 || n.is_infinite() || (n * s.len() as f64) > 200_000_000.0 {
        return Err(it.error_ctl("RangeError", "Invalid count value"));
    }
    Ok(Value::string(s.repeat(n as usize)))
}

fn concat(it: &mut Interp, c: &Call) -> Res {
    let mut s = this_str(it, c)?.to_string();
    for a in c.args {
        s.push_str(&it.to_str(a)?);
    }
    Ok(Value::string(s))
}

fn locale_compare(it: &mut Interp, c: &Call) -> Res {
    let a = this_str(it, c)?;
    let b = it.arg_str(c, 0)?;
    let numeric = match c.arg(2) {
        Value::Obj(_) => it.get_str(&c.arg(2), "numeric")?.truthy(),
        _ => false,
    };
    Ok(Value::Num(locale_cmp(&a, &b, numeric) as f64))
}

pub fn locale_cmp(a: &str, b: &str, numeric: bool) -> i32 {
    use core::cmp::Ordering::*;
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => break,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(x), Some(y)) => {
                if numeric && x.is_ascii_digit() && y.is_ascii_digit() {
                    let mut xs = String::new();
                    let mut ys = String::new();
                    while let Some(&d) = ai.peek() {
                        if d.is_ascii_digit() {
                            xs.push(d);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    while let Some(&d) = bi.peek() {
                        if d.is_ascii_digit() {
                            ys.push(d);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    let xn = xs.trim_start_matches('0');
                    let yn = ys.trim_start_matches('0');
                    match xn.len().cmp(&yn.len()).then_with(|| xn.cmp(yn)) {
                        Less => return -1,
                        Greater => return 1,
                        Equal => continue,
                    }
                }
                // punctuation sorts before digits before letters
                let class = |c: char| if c.is_alphabetic() { 2 } else if c.is_numeric() { 1 } else { 0 };
                let (cx, cy) = (class(x), class(y));
                if cx != cy {
                    return if cx < cy { -1 } else { 1 };
                }
                let (fx, fy) = (fold(x), fold(y));
                if fx != fy {
                    return if fx < fy { -1 } else { 1 };
                }
                ai.next();
                bi.next();
            }
        }
    }
    // tie-break: lowercase before uppercase
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            return if x.is_lowercase() { -1 } else { 1 };
        }
    }
    0
}

// ---------------------------------------------------------------------------
// RegExp
// ---------------------------------------------------------------------------

pub fn new_regexp(it: &mut Interp, pattern: &str, flags: &str) -> Res {
    let proto = it.protos().regexp.clone();
    make_regexp(it, pattern, flags, proto)
}

fn make_regexp(it: &mut Interp, pattern: &str, flags: &str, proto: Obj) -> Res {
    let data = match regex::make_data(pattern, flags) {
        Ok(d) => d,
        Err(e) => return Err(it.error_ctl("SyntaxError", &format!("Invalid regular expression: /{pattern}/: {e}"))),
    };
    let o = it.alloc(Object::new(Some(proto), Class::RegExp(Box::new(data))));
    it.define_data(&o, Key::str("lastIndex"), Value::Num(0.0), WRITABLE);
    Ok(Value::Obj(o))
}

fn regex_parts(v: &Value) -> Option<(JsStr, JsStr)> {
    if let Value::Obj(o) = v {
        if let Class::RegExp(d) = &o.borrow().class {
            return Some((d.source.clone(), d.flags.clone()));
        }
    }
    None
}

fn regexp_ctor(it: &mut Interp, c: &Call) -> Res {
    let pat = c.arg(0);
    let fl = c.arg(1);
    let (src, flags): (JsStr, JsStr) = if let Some((s, f)) = regex_parts(&pat) {
        if c.new_target.is_none() && fl.is_undefined() {
            return Ok(pat);
        }
        (s, if fl.is_undefined() { f } else { it.to_str(&fl)? })
    } else {
        let s = if pat.is_undefined() { Rc::from("(?:)") } else { it.to_str(&pat)? };
        let f = if fl.is_undefined() { Rc::from("") } else { it.to_str(&fl)? };
        (s, f)
    };
    let proto = it.protos().regexp.clone();
    let p = it.proto_from_call(c, &proto)?;
    make_regexp(it, &src, &flags, p)
}

fn this_regex(it: &mut Interp, c: &Call) -> Res<Obj> {
    match &c.this {
        Value::Obj(o) if matches!(o.borrow().class, Class::RegExp(_)) => Ok(o.clone()),
        _ => it.type_error("Receiver is not a RegExp"),
    }
}

fn setup_regexp(it: &mut Interp) {
    let proto = it.protos().regexp.clone();
    it.make_ctor("RegExp", 2, regexp_ctor, &proto);
    it.def_method(&proto, "exec", 1, |it, c| {
        let rx = this_regex(it, c)?;
        let s = it.arg_str(c, 0)?;
        exec(it, &rx, &s)
    });
    it.def_method(&proto, "test", 1, |it, c| {
        let rx = this_regex(it, c)?;
        let s = it.arg_str(c, 0)?;
        Ok(Value::Bool(!matches!(exec(it, &rx, &s)?, Value::Null)))
    });
    // generic: "/" + ToString(this.source) + "/" + ToString(this.flags) for any object
    it.def_method(&proto, "toString", 0, |it, c| {
        if !matches!(c.this, Value::Obj(_)) {
            return it.type_error("RegExp.prototype.toString requires that 'this' be an Object");
        }
        let src = it.get_str(&c.this, "source")?;
        let src = it.to_str(&src)?;
        let flags = it.get_str(&c.this, "flags")?;
        let flags = it.to_str(&flags)?;
        Ok(Value::string(format!("/{src}/{flags}")))
    });
    it.def_method(&proto, "compile", 2, |_, c| Ok(c.this.clone()));
    it.def_getter(&proto, "source", |it, c| match regex_parts(&c.this) {
        Some((s, _)) => Ok(Value::Str(s)),
        None => {
            let _ = it;
            Ok(Value::str("(?:)"))
        }
    });
    it.def_getter(&proto, "flags", |_, c| Ok(regex_parts(&c.this).map_or(Value::Undefined, |(_, f)| Value::Str(f))));
    macro_rules! flag_getter {
        ($name:expr, $ch:expr) => {
            it.def_getter(&proto, $name, |_, c| Ok(regex_parts(&c.this).map_or(Value::Undefined, |(_, f)| Value::Bool(f.contains($ch)))));
        };
    }
    flag_getter!("global", 'g');
    flag_getter!("ignoreCase", 'i');
    flag_getter!("multiline", 'm');
    flag_getter!("sticky", 'y');
    flag_getter!("unicode", 'u');
    flag_getter!("dotAll", 's');
    flag_getter!("hasIndices", 'd');
}

struct Chars {
    chars: Vec<char>,
    /// char index -> UTF-16 index (only when the string has astral characters).
    map: Option<Vec<usize>>,
}

impl Chars {
    fn new(s: &str) -> Chars {
        let chars: Vec<char> = s.chars().collect();
        let map = if chars.iter().any(|c| (*c as u32) > 0xFFFF) {
            let mut m = Vec::with_capacity(chars.len() + 1);
            let mut p = 0;
            for c in &chars {
                m.push(p);
                p += c.len_utf16();
            }
            m.push(p);
            Some(m)
        } else {
            None
        };
        Chars { chars, map }
    }
    fn to_u16(&self, ci: usize) -> usize {
        match &self.map {
            Some(m) => m[ci.min(m.len() - 1)],
            None => ci,
        }
    }
    fn from_u16(&self, ui: usize) -> usize {
        match &self.map {
            Some(m) => match m.binary_search(&ui) {
                Ok(i) => i,
                Err(i) => i,
            },
            None => ui,
        }
    }
    fn sub(&self, a: usize, b: usize) -> String {
        self.chars[a..b].iter().collect()
    }
}

type Caps = Vec<Option<(usize, usize)>>;

fn get_prog(rx: &Obj) -> (Rc<regex::Program>, bool, bool) {
    if let Class::RegExp(d) = &rx.borrow().class {
        return (d.prog.clone(), d.global, d.sticky);
    }
    unreachable!()
}

fn get_last_index(it: &mut Interp, rx: &Obj) -> Res<f64> {
    let v = it.get_obj(rx, &Key::str("lastIndex"), &Value::Obj(rx.clone()))?;
    Ok(to_integer(it.to_number(&v)?))
}

fn set_last_index(it: &mut Interp, rx: &Obj, n: usize) -> Res<()> {
    it.set_v(&Value::Obj(rx.clone()), Key::str("lastIndex"), Value::Num(n as f64))
}

/// RegExp.prototype.exec semantics (lastIndex handling for g/y).
fn exec(it: &mut Interp, rx: &Obj, s: &JsStr) -> Res {
    let (prog, global, sticky) = get_prog(rx);
    let use_last = global || sticky;
    let li = if use_last { get_last_index(it, rx)? } else { 0.0 };
    let chars = Chars::new(s);
    let total = chars.map.as_ref().map_or(chars.chars.len(), |m| *m.last().unwrap());
    if li < 0.0 || li > total as f64 {
        if use_last {
            set_last_index(it, rx, 0)?;
        }
        return Ok(Value::Null);
    }
    let start = chars.from_u16(li as usize);
    match regex::search(&prog, &chars.chars, start, sticky) {
        None => {
            if use_last {
                set_last_index(it, rx, 0)?;
            }
            Ok(Value::Null)
        }
        Some(caps) => {
            if use_last {
                let e = chars.to_u16(caps[0].unwrap().1);
                set_last_index(it, rx, e)?;
            }
            Ok(build_match(it, &prog, &chars, &caps, s))
        }
    }
}

fn build_match(it: &mut Interp, prog: &regex::Program, chars: &Chars, caps: &Caps, input: &JsStr) -> Value {
    let items: Vec<Value> = caps.iter().map(|c| c.map_or(Value::Undefined, |(a, b)| Value::string(chars.sub(a, b)))).collect();
    let arr = it.new_array(items);
    let Value::Obj(ao) = &arr else { unreachable!() };
    let idx = chars.to_u16(caps[0].map_or(0, |c| c.0));
    it.define_data(ao, Key::str("index"), Value::Num(idx as f64), ALL);
    it.define_data(ao, Key::str("input"), Value::Str(input.clone()), ALL);
    let groups = if prog.names.is_empty() {
        Value::Undefined
    } else {
        let g = it.new_object();
        for (name, i) in &prog.names {
            let v = caps.get(*i).copied().flatten().map_or(Value::Undefined, |(a, b)| Value::string(chars.sub(a, b)));
            it.define_data(&g, Key::S(Rc::from(name.as_str())), v, ALL);
        }
        Value::Obj(g)
    };
    it.define_data(ao, Key::str("groups"), groups, ALL);
    arr
}

fn is_regex(v: &Value) -> Option<Obj> {
    match v {
        Value::Obj(o) if matches!(o.borrow().class, Class::RegExp(_)) => Some(o.clone()),
        _ => None,
    }
}

fn to_regex(it: &mut Interp, v: &Value, flags: &str) -> Res<Obj> {
    if let Some(r) = is_regex(v) {
        return Ok(r);
    }
    let src: JsStr = if v.is_undefined() { Rc::from("(?:)") } else { it.to_str(v)? };
    match new_regexp(it, &src, flags)? {
        Value::Obj(o) => Ok(o),
        _ => unreachable!(),
    }
}

fn match_(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let rx = to_regex(it, &c.arg(0), "")?;
    let (prog, global, _) = get_prog(&rx);
    if !global {
        return exec(it, &rx, &s);
    }
    let chars = Chars::new(&s);
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= chars.chars.len() {
        let Some(caps) = regex::search(&prog, &chars.chars, pos, false) else { break };
        let (a, b) = caps[0].unwrap();
        out.push(Value::string(chars.sub(a, b)));
        pos = if b == a { b + 1 } else { b };
    }
    set_last_index(it, &rx, 0)?;
    if out.is_empty() { Ok(Value::Null) } else { Ok(it.new_array(out)) }
}

fn match_all(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let arg = c.arg(0);
    if let Some(r) = is_regex(&arg) {
        let (_, f) = regex_parts(&Value::Obj(r)).unwrap();
        if !f.contains('g') {
            return it.type_error("String.prototype.matchAll called with a non-global RegExp argument");
        }
    }
    let rx = to_regex(it, &arg, "g")?;
    let (prog, _, sticky) = get_prog(&rx);
    let chars = Chars::new(&s);
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= chars.chars.len() {
        let Some(caps) = regex::search(&prog, &chars.chars, pos, sticky) else { break };
        let (a, b) = caps[0].unwrap();
        out.push(build_match(it, &prog, &chars, &caps, &s));
        pos = if b == a { b + 1 } else { b };
    }
    Ok(it.make_list_iterator(out))
}

fn search(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let rx = to_regex(it, &c.arg(0), "")?;
    let (prog, _, sticky) = get_prog(&rx);
    let chars = Chars::new(&s);
    Ok(Value::Num(match regex::search(&prog, &chars.chars, 0, sticky) {
        Some(caps) => chars.to_u16(caps[0].unwrap().0) as f64,
        None => -1.0,
    }))
}

fn expand_template(tpl: &str, matched: &str, before: &str, after: &str, caps: &[Value], named: Option<&Value>, it: &mut Interp) -> Res<String> {
    if !tpl.contains('$') {
        return Ok(tpl.to_string());
    }
    let t: Vec<char> = tpl.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < t.len() {
        if t[i] != '$' || i + 1 >= t.len() {
            out.push(t[i]);
            i += 1;
            continue;
        }
        match t[i + 1] {
            '$' => {
                out.push('$');
                i += 2;
            }
            '&' => {
                out.push_str(matched);
                i += 2;
            }
            '`' => {
                out.push_str(before);
                i += 2;
            }
            '\'' => {
                out.push_str(after);
                i += 2;
            }
            d if d.is_ascii_digit() => {
                let d1 = d.to_digit(10).unwrap() as usize;
                let two = t.get(i + 2).and_then(|c| c.to_digit(10)).map(|d2| d1 * 10 + d2 as usize);
                let (n, used) = match two {
                    Some(n) if n >= 1 && n <= caps.len() => (n, 3),
                    _ => (d1, 2),
                };
                if n >= 1 && n <= caps.len() {
                    if !caps[n - 1].is_undefined() {
                        out.push_str(&it.to_str(&caps[n - 1])?);
                    }
                    i += used;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            '<' if named.is_some() => {
                if let Some(end) = t[i + 2..].iter().position(|c| *c == '>') {
                    let name: String = t[i + 2..i + 2 + end].iter().collect();
                    let v = it.get_v(named.unwrap(), &Key::S(Rc::from(name)))?;
                    if !v.is_undefined() {
                        out.push_str(&it.to_str(&v)?);
                    }
                    i += end + 3;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    Ok(out)
}

fn replace(it: &mut Interp, c: &Call, all: bool) -> Res {
    let s = this_str(it, c)?;
    let pat = c.arg(0);
    let rep = c.arg(1);
    let rep_fn = rep.as_obj().is_some_and(|o| o.is_callable());
    let rep_str: JsStr = if rep_fn { Rc::from("") } else { it.to_str(&rep)? };
    if let Some(rx) = is_regex(&pat) {
        let (prog, global, sticky) = get_prog(&rx);
        if all && !global {
            return it.type_error("replaceAll must be called with a global RegExp");
        }
        let chars = Chars::new(&s);
        let mut out = String::new();
        let mut last = 0usize;
        let mut pos = if global { 0 } else if sticky { chars.from_u16(get_last_index(it, &rx)?.max(0.0) as usize) } else { 0 };
        loop {
            if pos > chars.chars.len() {
                break;
            }
            let Some(caps) = regex::search(&prog, &chars.chars, pos, sticky) else {
                if sticky {
                    set_last_index(it, &rx, 0)?;
                }
                break;
            };
            let (a, b) = caps[0].unwrap();
            out.push_str(&chars.sub(last, a));
            let matched = chars.sub(a, b);
            let capv: Vec<Value> = caps.iter().skip(1).map(|c| c.map_or(Value::Undefined, |(x, y)| Value::string(chars.sub(x, y)))).collect();
            let groups = if prog.names.is_empty() {
                None
            } else {
                let g = it.new_object();
                for (name, i) in &prog.names {
                    let v = caps.get(*i).copied().flatten().map_or(Value::Undefined, |(x, y)| Value::string(chars.sub(x, y)));
                    it.define_data(&g, Key::S(Rc::from(name.as_str())), v, ALL);
                }
                Some(Value::Obj(g))
            };
            if rep_fn {
                let mut args = vec![Value::string(matched)];
                args.extend(capv);
                args.push(Value::Num(chars.to_u16(a) as f64));
                args.push(Value::Str(s.clone()));
                if let Some(g) = groups {
                    args.push(g);
                }
                let r = it.call(&rep, Value::Undefined, &args)?;
                out.push_str(&it.to_str(&r)?);
            } else {
                let before = chars.sub(0, a);
                let after = chars.sub(b, chars.chars.len());
                out.push_str(&expand_template(&rep_str, &matched, &before, &after, &capv, groups.as_ref(), it)?);
            }
            last = b;
            if !global {
                if sticky {
                    set_last_index(it, &rx, chars.to_u16(b))?;
                }
                break;
            }
            pos = if b == a { b + 1 } else { b };
        }
        if global {
            set_last_index(it, &rx, 0)?;
        }
        out.push_str(&chars.sub(last.min(chars.chars.len()), chars.chars.len()));
        return Ok(Value::string(out));
    }
    // string pattern
    let needle = it.to_str(&pat)?;
    let mut out = String::new();
    let mut from = 0usize; // byte offset
    let mut replaced_any = false;
    loop {
        let found = if needle.is_empty() {
            if from <= s.len() { Some(from) } else { None }
        } else {
            s[from..].find(&*needle).map(|i| i + from)
        };
        let Some(at) = found else { break };
        out.push_str(&s[from.min(at)..at]);
        let replacement = if rep_fn {
            let r = it.call(&rep, Value::Undefined, &[Value::str(&needle), Value::Num(u16_len(&s[..at]) as f64), Value::Str(s.clone())])?;
            it.to_str(&r)?.to_string()
        } else {
            expand_template(&rep_str, &needle, &s[..at], &s[at + needle.len()..], &[], None, it)?
        };
        out.push_str(&replacement);
        replaced_any = true;
        let next = at + needle.len();
        if needle.is_empty() {
            // advance one char, copying it
            if at < s.len() {
                let ch = s[at..].chars().next().unwrap();
                out.push(ch);
                from = at + ch.len_utf8();
            } else {
                from = s.len() + 1;
            }
        } else {
            from = next;
        }
        if !all {
            break;
        }
    }
    if from <= s.len() {
        out.push_str(&s[from..]);
    }
    let _ = replaced_any;
    Ok(Value::string(out))
}

fn split(it: &mut Interp, c: &Call) -> Res {
    let s = this_str(it, c)?;
    let sep = c.arg(0);
    let limit = match c.arg(1) {
        Value::Undefined => u32::MAX as usize,
        v => to_uint32(it.to_number(&v)?) as usize,
    };
    if limit == 0 {
        return Ok(it.new_array(Vec::new()));
    }
    if sep.is_undefined() {
        return Ok(it.new_array(vec![Value::Str(s)]));
    }
    let mut out: Vec<Value> = Vec::new();
    if let Some(rx) = is_regex(&sep) {
        let (prog, _, _) = get_prog(&rx);
        let chars = Chars::new(&s);
        let size = chars.chars.len();
        if size == 0 {
            return Ok(if regex::match_at(&prog, &chars.chars, 0).is_some() { it.new_array(Vec::new()) } else { it.new_array(vec![Value::Str(s)]) });
        }
        let (mut p, mut q) = (0usize, 0usize);
        while q < size {
            match regex::match_at(&prog, &chars.chars, q) {
                None => q += 1,
                Some(caps) => {
                    let e = caps[0].unwrap().1.min(size);
                    if e == p {
                        q += 1;
                    } else {
                        out.push(Value::string(chars.sub(p, q)));
                        if out.len() >= limit {
                            return Ok(it.new_array(out));
                        }
                        for cp in caps.iter().skip(1) {
                            out.push(cp.map_or(Value::Undefined, |(a, b)| Value::string(chars.sub(a, b))));
                            if out.len() >= limit {
                                return Ok(it.new_array(out));
                            }
                        }
                        p = e;
                        q = p;
                    }
                }
            }
        }
        out.push(Value::string(chars.sub(p, size)));
        return Ok(it.new_array(out));
    }
    let sep = it.to_str(&sep)?;
    if sep.is_empty() {
        for u in s.chars() {
            if s.is_ascii() || (u as u32) <= 0xFFFF {
                out.push(Value::string(u.to_string()));
            } else {
                let mut b = [0u16; 2];
                for unit in u.encode_utf16(&mut b) {
                    out.push(Value::string(from_u16(&[*unit])));
                }
            }
            if out.len() >= limit {
                out.truncate(limit);
                break;
            }
        }
        return Ok(it.new_array(out));
    }
    for part in s.split(&*sep) {
        out.push(Value::str(part));
        if out.len() >= limit {
            break;
        }
    }
    Ok(it.new_array(out))
}
