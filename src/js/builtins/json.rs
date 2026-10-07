//! JSON.parse / JSON.stringify.
use super::*;
use alloc::format;

pub fn setup(it: &mut Interp) {
    let json = it.new_object();
    let g = it.global.clone();
    it.define_data(&g, Key::str("JSON"), Value::Obj(json.clone()), HIDDEN);
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&json, Key::Y(tag.0.id), Value::str("JSON"), CONFIGURABLE);
    it.def_method(&json, "parse", 2, parse);
    it.def_method(&json, "stringify", 3, stringify);
}

struct P<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
    depth: usize,
}

type PR<T> = Result<T, String>;

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn err<T>(&self) -> PR<T> {
        if self.i >= self.b.len() {
            Err("Unexpected end of JSON input".into())
        } else {
            let ch = self.s[self.i..].chars().next().unwrap_or(' ');
            Err(format!("Unexpected token '{}', \"{}\" is not valid JSON", ch, self.s.chars().take(30).collect::<String>()))
        }
    }
    fn value(&mut self, it: &mut Interp) -> PR<Value> {
        self.ws();
        if self.i >= self.b.len() {
            return self.err();
        }
        self.depth += 1;
        if self.depth > 400 {
            return Err("JSON nested too deeply".into());
        }
        let r = match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let o = it.new_object();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        if self.b.get(self.i) != Some(&b'"') {
                            return self.err();
                        }
                        let k = self.string()?;
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') {
                            return self.err();
                        }
                        self.i += 1;
                        let v = self.value(it)?;
                        it.define_data(&o, Key::S(Rc::from(k)), v, ALL);
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return self.err(),
                        }
                    }
                }
                Value::Obj(o)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        items.push(self.value(it)?);
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return self.err(),
                        }
                    }
                }
                it.new_array(items)
            }
            b'"' => Value::string(self.string()?),
            b't' if self.s[self.i..].starts_with("true") => {
                self.i += 4;
                Value::Bool(true)
            }
            b'f' if self.s[self.i..].starts_with("false") => {
                self.i += 5;
                Value::Bool(false)
            }
            b'n' if self.s[self.i..].starts_with("null") => {
                self.i += 4;
                Value::Null
            }
            b'-' | b'0'..=b'9' => {
                let st = self.i;
                if self.b[self.i] == b'-' {
                    self.i += 1;
                }
                let ds = self.i;
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                if self.i == ds || (self.b[ds] == b'0' && self.i - ds > 1) {
                    return self.err();
                }
                if self.b.get(self.i) == Some(&b'.') {
                    self.i += 1;
                    let fs = self.i;
                    while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                        self.i += 1;
                    }
                    if fs == self.i {
                        return self.err();
                    }
                }
                if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
                    self.i += 1;
                    if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                        self.i += 1;
                    }
                    let es = self.i;
                    while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                        self.i += 1;
                    }
                    if es == self.i {
                        return self.err();
                    }
                }
                Value::Num(self.s[st..self.i].parse().unwrap_or(f64::NAN))
            }
            _ => return self.err(),
        };
        self.depth -= 1;
        Ok(r)
    }

    fn string(&mut self) -> PR<String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.b.get(self.i) else { return Err("Unterminated string in JSON".into()) };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let Some(&e) = self.b.get(self.i) else { return self.err() };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut unit = self.hex4()?;
                            if (0xD800..0xDC00).contains(&unit) && self.s[self.i..].starts_with("\\u") {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    let cp = 0x10000 + ((unit as u32 - 0xD800) << 10) + (lo as u32 - 0xDC00);
                                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                                    continue;
                                }
                                self.i = save;
                                unit = 0xFFFD;
                            }
                            out.push(char::from_u32(unit as u32).unwrap_or('\u{fffd}'));
                        }
                        _ => {
                            self.i -= 1;
                            return self.err();
                        }
                    }
                }
                0..=0x1f => return self.err(),
                _ => {
                    let ch = self.s[self.i..].chars().next().unwrap();
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    fn hex4(&mut self) -> PR<u16> {
        let h = self.s.get(self.i..self.i + 4).and_then(|h| u16::from_str_radix(h, 16).ok());
        match h {
            Some(v) => {
                self.i += 4;
                Ok(v)
            }
            None => self.err(),
        }
    }
}

fn parse(it: &mut Interp, c: &Call) -> Res {
    let s = it.arg_str(c, 0)?;
    let mut p = P { b: s.as_bytes(), s: &s, i: 0, depth: 0 };
    let v = match p.value(it) {
        Ok(v) => {
            p.ws();
            if p.i < p.b.len() {
                let e = p.err::<()>().err().unwrap_or_default();
                return Err(it.error_ctl("SyntaxError", &e));
            }
            v
        }
        Err(e) => return Err(it.error_ctl("SyntaxError", &e)),
    };
    let reviver = c.arg(1);
    if reviver.as_obj().is_some_and(|o| o.is_callable()) {
        let root = it.new_object();
        it.define_data(&root, Key::str(""), v, ALL);
        return revive(it, &Value::Obj(root), Key::str(""), &reviver);
    }
    Ok(v)
}

fn revive(it: &mut Interp, holder: &Value, key: Key, reviver: &Value) -> Res {
    let val = it.get_v(holder, &key)?;
    if let Value::Obj(o) = &val {
        let keys: Vec<Key> = if o.is_array() {
            let n = it.array_like_to_vec(&val)?.len();
            (0..n).map(|i| Key::S(Rc::from(i.to_string()))).collect()
        } else {
            it.own_keys(o)?.into_iter().filter(|k| matches!(k, Key::S(_)) && it.own_prop(o, k).is_some_and(|p| p.flags & ENUMERABLE != 0)).collect()
        };
        for k in keys {
            let nv = revive(it, &val, k.clone(), reviver)?;
            if nv.is_undefined() {
                it.delete_prop(o, &k)?;
            } else {
                it.define_data(o, k, nv, ALL);
            }
        }
    }
    let kv = it.key_to_value(&key);
    it.call(reviver, holder.clone(), &[kv, val])
}

// ---- stringify ----

struct S {
    replacer: Option<Value>,
    allow: Option<Vec<Key>>,
    gap: String,
    stack: Vec<usize>,
}

pub fn quote(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn stringify(it: &mut Interp, c: &Call) -> Res {
    let value = c.arg(0);
    let mut st = S { replacer: None, allow: None, gap: String::new(), stack: Vec::new() };
    if let Value::Obj(r) = c.arg(1) {
        if r.is_callable() {
            st.replacer = Some(Value::Obj(r));
        } else if r.is_array() {
            let mut keys = Vec::new();
            for v in it.array_like_to_vec(&Value::Obj(r))? {
                let k = match &v {
                    Value::Str(_) | Value::Num(_) => it.to_key(&v)?,
                    _ => continue,
                };
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            st.allow = Some(keys);
        }
    }
    match c.arg(2) {
        Value::Num(n) => st.gap = " ".repeat(n.clamp(0.0, 10.0) as usize),
        Value::Str(s) => st.gap = s.chars().take(10).collect(),
        _ => {}
    }
    let holder = it.new_object();
    it.define_data(&holder, Key::str(""), value, ALL);
    let mut out = String::new();
    let wrote = ser(it, &mut st, &Value::Obj(holder), Key::str(""), &mut out, "")?;
    Ok(if wrote { Value::string(out) } else { Value::Undefined })
}

fn ser(it: &mut Interp, st: &mut S, holder: &Value, key: Key, out: &mut String, indent: &str) -> Res<bool> {
    let mut v = it.get_v(holder, &key)?;
    if matches!(v, Value::Obj(_)) {
        let tj = it.get_v(&v, &it.names.to_json.clone())?;
        if tj.as_obj().is_some_and(|o| o.is_callable()) {
            let kv = it.key_to_value(&key);
            v = it.call(&tj, v.clone(), &[kv])?;
        }
    }
    if let Some(r) = st.replacer.clone() {
        let kv = it.key_to_value(&key);
        v = it.call(&r, holder.clone(), &[kv, v])?;
    }
    if let Value::Obj(o) = &v {
        let prim = match &o.borrow().class {
            Class::Num(n) => Some(Value::Num(*n)),
            Class::Str(s) => Some(Value::Str(s.clone())),
            Class::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        };
        if let Some(p) = prim {
            v = if let Value::Num(_) = p { Value::Num(it.to_number(&v)?) } else if let Value::Str(_) = p { Value::Str(it.to_str(&v)?) } else { p };
        }
    }
    match &v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => out.push_str(&if n.is_finite() { num_to_string(*n) } else { "null".to_string() }),
        Value::Str(s) => quote(s, out),
        Value::Undefined | Value::Sym(_) => return Ok(false),
        Value::Obj(o) => {
            if o.is_callable() {
                return Ok(false);
            }
            if st.stack.contains(&o.id()) {
                return it.type_error("Converting circular structure to JSON");
            }
            if st.stack.len() > 300 {
                return Err(it.error_ctl("RangeError", "Maximum call stack size exceeded"));
            }
            st.stack.push(o.id());
            let inner = format!("{indent}{}", st.gap);
            if o.is_array() {
                let items = it.array_like_to_vec(&v)?;
                if items.is_empty() {
                    out.push_str("[]");
                } else {
                    out.push('[');
                    for i in 0..items.len() {
                        if i > 0 {
                            out.push(',');
                        }
                        if !st.gap.is_empty() {
                            out.push('\n');
                            out.push_str(&inner);
                        }
                        if !ser(it, st, &v, Key::S(Rc::from(i.to_string())), out, &inner)? {
                            out.push_str("null");
                        }
                    }
                    if !st.gap.is_empty() {
                        out.push('\n');
                        out.push_str(indent);
                    }
                    out.push(']');
                }
            } else {
                let keys: Vec<Key> = match &st.allow {
                    Some(a) => a.clone(),
                    None => {
                        let all = it.own_keys(o)?;
                        let mut ks = Vec::new();
                        for k in all {
                            if matches!(k, Key::S(_)) && it.own_prop(o, &k).is_some_and(|p| p.flags & ENUMERABLE != 0) {
                                ks.push(k);
                            }
                        }
                        ks
                    }
                };
                out.push('{');
                let mut first = true;
                for k in keys {
                    let mark = out.len();
                    if !first {
                        out.push(',');
                    }
                    if !st.gap.is_empty() {
                        out.push('\n');
                        out.push_str(&inner);
                    }
                    if let Key::S(s) = &k {
                        quote(s, out);
                    }
                    out.push(':');
                    if !st.gap.is_empty() {
                        out.push(' ');
                    }
                    if ser(it, st, &v, k, out, &inner)? {
                        first = false;
                    } else {
                        out.truncate(mark);
                    }
                }
                if !first && !st.gap.is_empty() {
                    out.push('\n');
                    out.push_str(indent);
                }
                out.push('}');
            }
            st.stack.pop();
        }
    }
    Ok(true)
}
