//! JS tokenizer.
use super::value::JsStr;
use alloc::{
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(JsStr),
    /// Template pieces: (cooked, raw) strings and the source ranges of the `${}` expressions.
    Template(Vec<(Option<JsStr>, JsStr)>, Vec<(usize, usize)>),
    Ident(JsStr),
    Private(JsStr),
    Punct(&'static str),
    Regex(JsStr, JsStr),
    /// BigInt literal digits (with radix prefix), without the `n`.
    BigInt(JsStr),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub nl: bool,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone)]
pub struct Lexer {
    pub src: Rc<str>,
    pub pos: usize,
    prev_regex_ok: bool,
    /// The previous token was `.`/`?.`: a following identifier is a property name.
    prev_dot: bool,
}

const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++",
    "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", "**", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*",
    "/", "%", "&", "|", "^", "!", "~", "?", ":", "=", ".", "@", "#",
];

fn is_id_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_alphabetic() || (!c.is_ascii() && c.is_alphanumeric())
}

fn is_id_part(c: char) -> bool {
    if c.is_ascii() {
        return c == '$' || c == '_' || c.is_ascii_alphanumeric();
    }
    // non-ASCII: letters, digits and combining marks (no category tables in core: exclude
    // whitespace and the punctuation/symbol blocks instead)
    c.is_alphanumeric() || c == '\u{200c}' || c == '\u{200d}' || (!super::value::is_js_ws(c) && !matches!(c, '\u{2000}'..='\u{206f}' | '\u{2190}'..='\u{2bff}' | '\u{3000}'..='\u{303f}' | '\u{fe30}'..='\u{fe4f}' | '\u{ff00}'..='\u{ff0f}' | '\u{ff1a}'..='\u{ff20}'))
}

const REGEX_KEYWORDS: &[&str] = &["return", "typeof", "case", "do", "else", "in", "of", "instanceof", "new", "delete", "void", "throw", "yield", "await"];

impl Lexer {
    pub fn new(src: Rc<str>) -> Lexer {
        let mut l = Lexer { src, pos: 0, prev_regex_ok: true, prev_dot: false };
        if l.src.starts_with("#!") {
            l.pos = l.src.find('\n').unwrap_or(l.src.len());
        }
        l
    }

    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn err<T>(&self, msg: &str) -> Result<T, String> {
        let line = self.src[..self.pos.min(self.src.len())].matches('\n').count() + 1;
        Err(format!("SyntaxError: {msg} (line {line})"))
    }

    pub fn line_of(&self, pos: usize) -> usize {
        self.src[..pos.min(self.src.len())].matches('\n').count() + 1
    }

    fn skip_trivia(&mut self) -> Result<bool, String> {
        let mut nl = false;
        loop {
            let r = self.rest();
            let Some(c) = r.chars().next() else { return Ok(nl) };
            if c == '\n' || c == '\r' || c == '\u{2028}' || c == '\u{2029}' {
                nl = true;
                self.pos += c.len_utf8();
            } else if c.is_whitespace() || c == '\u{feff}' {
                self.pos += c.len_utf8();
            } else if r.starts_with("//") {
                self.pos += r.find('\n').unwrap_or(r.len());
            } else if r.starts_with("/*") {
                match r[2..].find("*/") {
                    Some(e) => {
                        if r[..e + 2].contains('\n') {
                            nl = true;
                        }
                        self.pos += e + 4;
                    }
                    None => return self.err("unterminated comment"),
                }
            } else if r.starts_with("<!--") {
                self.pos += r.find('\n').unwrap_or(r.len());
            } else if r.starts_with("-->") && (nl || self.pos == 0) {
                self.pos += r.find('\n').unwrap_or(r.len());
            } else {
                return Ok(nl);
            }
        }
    }

    pub fn next(&mut self) -> Result<Token, String> {
        let nl = self.skip_trivia()?;
        let start = self.pos;
        let r = self.rest();
        let Some(c) = r.chars().next() else {
            return Ok(Token { tok: Tok::Eof, nl, start, end: start });
        };
        let tok = if c.is_ascii_digit() || (c == '.' && r[1..].starts_with(|d: char| d.is_ascii_digit())) {
            self.number()?
        } else if c == '"' || c == '\'' {
            self.string(c)?
        } else if c == '`' {
            self.template()?
        } else if is_id_start(c) || c == '\\' {
            Tok::Ident(Rc::from(self.ident()?))
        } else if c == '#' && r[1..].starts_with(|d: char| is_id_start(d)) {
            self.pos += 1;
            Tok::Private(Rc::from(self.ident()?))
        } else if c == '/' && self.prev_regex_ok {
            self.regex()?
        } else {
            let mut found = None;
            for p in PUNCTS {
                if r.starts_with(p) {
                    if *p == "?." && r[2..].starts_with(|d: char| d.is_ascii_digit()) {
                        continue;
                    }
                    found = Some(*p);
                    break;
                }
            }
            match found {
                Some(p) => {
                    self.pos += p.len();
                    Tok::Punct(p)
                }
                None => return self.err(&format!("unexpected character '{c}'")),
            }
        };
        self.prev_regex_ok = match &tok {
            Tok::Num(_) | Tok::BigInt(_) | Tok::Str(_) | Tok::Template(..) | Tok::Regex(..) | Tok::Private(_) => false,
            Tok::Ident(s) => !self.prev_dot && REGEX_KEYWORDS.contains(&&**s),
            // after `)`, `]`, `}` and postfix `++`/`--` a slash divides
            Tok::Punct(p) => !matches!(*p, ")" | "]" | "}" | "++" | "--"),
            Tok::Eof => true,
        };
        self.prev_dot = matches!(tok, Tok::Punct(".") | Tok::Punct("?."));
        Ok(Token { tok, nl, start, end: self.pos })
    }

    /// Re-read the token at `start` as a regular expression literal (the parser knows a `/` here
    /// starts an expression, which the lexer's one-token heuristic cannot always tell).
    pub fn rescan_regex(&mut self, start: usize, nl: bool) -> Result<Token, String> {
        self.pos = start;
        let tok = self.regex()?;
        self.prev_regex_ok = false;
        self.prev_dot = false;
        Ok(Token { tok, nl, start, end: self.pos })
    }

    fn ident(&mut self) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let r = self.rest();
            let Some(c) = r.chars().next() else { break };
            if c == '\\' {
                if !r[1..].starts_with('u') {
                    return self.err("invalid identifier escape");
                }
                self.pos += 2;
                let ch = self.unicode_escape()?;
                out.push(ch);
            } else if (out.is_empty() && is_id_start(c)) || (!out.is_empty() && is_id_part(c)) {
                out.push(c);
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
        Ok(out)
    }

    /// After `\u`: parses `XXXX` or `{X...}`.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let r = self.rest();
        if let Some(inner) = r.strip_prefix('{') {
            let end = inner.find('}').ok_or_else(|| "SyntaxError: bad unicode escape".to_string())?;
            let n = u32::from_str_radix(&inner[..end], 16).map_err(|_| "SyntaxError: bad unicode escape".to_string())?;
            self.pos += end + 2;
            return Ok(char::from_u32(n).unwrap_or('\u{fffd}'));
        }
        if r.len() < 4 || !r.is_char_boundary(4) {
            return self.err("bad unicode escape");
        }
        let n = u32::from_str_radix(&r[..4], 16).map_err(|_| "SyntaxError: bad unicode escape".to_string())?;
        self.pos += 4;
        if (0xd800..0xdc00).contains(&n) && self.rest().starts_with("\\u") && self.rest().len() >= 6 && self.rest().is_char_boundary(6) {
            if let Ok(lo) = u32::from_str_radix(&self.rest()[2..6], 16) {
                if (0xdc00..0xe000).contains(&lo) {
                    self.pos += 6;
                    return Ok(char::from_u32(0x10000 + ((n - 0xd800) << 10) + (lo - 0xdc00)).unwrap_or('\u{fffd}'));
                }
            }
        }
        Ok(char::from_u32(n).unwrap_or('\u{fffd}'))
    }

    fn number(&mut self) -> Result<Tok, String> {
        let r = self.rest();
        let b = r.as_bytes();
        if b[0] == b'0' && b.len() > 1 {
            let radix = match b[1] {
                b'x' | b'X' => 16,
                b'o' | b'O' => 8,
                b'b' | b'B' => 2,
                _ => 0,
            };
            if radix != 0 {
                let mut i = 2;
                let mut v = 0f64;
                while i < b.len() {
                    let c = b[i] as char;
                    if c == '_' {
                        i += 1;
                        continue;
                    }
                    match c.to_digit(radix) {
                        Some(d) => v = v * radix as f64 + d as f64,
                        None => break,
                    }
                    i += 1;
                }
                if i < b.len() && b[i] == b'n' {
                    let text: String = r[..i].chars().filter(|c| *c != '_').collect();
                    self.pos += i + 1;
                    return Ok(Tok::BigInt(Rc::from(text)));
                }
                self.pos += i;
                return Ok(Tok::Num(v));
            }
            // legacy octal literal
            if b[1].is_ascii_digit() && b.iter().take_while(|c| c.is_ascii_digit()).all(|c| *c < b'8') {
                let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let v = u64::from_str_radix(&r[..n], 8).unwrap_or(0);
                self.pos += n;
                return Ok(Tok::Num(v as f64));
            }
        }
        let mut i = 0;
        let mut text = String::new();
        let digits = |i: &mut usize, text: &mut String| {
            while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'_') {
                if b[*i] != b'_' {
                    text.push(b[*i] as char);
                }
                *i += 1;
            }
        };
        digits(&mut i, &mut text);
        if i < b.len() && b[i] == b'.' {
            text.push('.');
            i += 1;
            digits(&mut i, &mut text);
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                text.push_str(&r[i..j]);
                i = j;
                digits(&mut i, &mut text);
            }
        }
        if i < b.len() && b[i] == b'n' {
            self.pos += i + 1;
            return Ok(Tok::BigInt(Rc::from(text)));
        }
        self.pos += i;
        let t = if text.starts_with('.') { format!("0{text}") } else { text };
        let t = t.strip_suffix('.').unwrap_or(&t).to_string();
        t.parse::<f64>().map(Tok::Num).map_err(|_| "SyntaxError: bad number".to_string())
    }

    /// Parse an escape sequence after the backslash; pushes into `out`. Returns false for invalid escapes.
    fn escape(&mut self, out: &mut String, in_template: bool) -> Result<bool, String> {
        let r = self.rest();
        let Some(c) = r.chars().next() else { return self.err("unterminated string") };
        self.pos += c.len_utf8();
        match c {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '0' if !self.rest().starts_with(|d: char| d.is_ascii_digit()) => out.push('\0'),
            '0'..='7' if !in_template => {
                // legacy octal escape
                let mut v = c.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match self.rest().chars().next().and_then(|d| d.to_digit(8)) {
                        Some(d) if v * 8 + d < 256 => {
                            v = v * 8 + d;
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(v).unwrap_or('\0'));
            }
            'x' => {
                let r = self.rest();
                if r.len() >= 2 && r.is_char_boundary(2) && let Ok(n) = u32::from_str_radix(&r[..2], 16) {
                    self.pos += 2;
                    out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                } else {
                    return Ok(false);
                }
            }
            'u' => match self.unicode_escape() {
                Ok(ch) => out.push(ch),
                Err(_) => return Ok(false),
            },
            '\r' => {
                if self.rest().starts_with('\n') {
                    self.pos += 1;
                }
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            c => out.push(c),
        }
        Ok(true)
    }

    fn string(&mut self, q: char) -> Result<Tok, String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let r = self.rest();
            let Some(c) = r.chars().next() else { return self.err("unterminated string") };
            self.pos += c.len_utf8();
            if c == q {
                break;
            }
            if c == '\\' {
                self.escape(&mut out, false)?;
            } else if c == '\n' {
                return self.err("unterminated string");
            } else {
                out.push(c);
            }
        }
        Ok(Tok::Str(Rc::from(out)))
    }

    fn template(&mut self) -> Result<Tok, String> {
        self.pos += 1;
        let mut strs: Vec<(Option<JsStr>, JsStr)> = Vec::new();
        let mut exprs = Vec::new();
        let mut cooked = String::new();
        let mut valid = true;
        let mut raw_start = self.pos;
        loop {
            let r = self.rest();
            let Some(c) = r.chars().next() else { return self.err("unterminated template") };
            if c == '`' {
                let raw = self.src[raw_start..self.pos].replace("\r\n", "\n");
                strs.push((valid.then(|| Rc::from(core::mem::take(&mut cooked))), Rc::from(raw)));
                self.pos += 1;
                break;
            }
            if c == '$' && r[1..].starts_with('{') {
                let raw = self.src[raw_start..self.pos].replace("\r\n", "\n");
                strs.push((valid.then(|| Rc::from(core::mem::take(&mut cooked))), Rc::from(raw)));
                valid = true;
                cooked.clear();
                self.pos += 2;
                let start = self.pos;
                let end = self.skip_expr()?;
                exprs.push((start, end));
                self.pos = end + 1;
                raw_start = self.pos;
                continue;
            }
            self.pos += c.len_utf8();
            if c == '\\' {
                if !self.escape(&mut cooked, true)? {
                    valid = false;
                }
            } else if c == '\r' {
                if self.rest().starts_with('\n') {
                    self.pos += 1;
                }
                cooked.push('\n');
            } else {
                cooked.push(c);
            }
        }
        Ok(Tok::Template(strs, exprs))
    }

    /// Scan to the `}` closing a `${` expression; returns its index. Uses the real tokenizer so
    /// strings, regex literals, nested templates and comments are all handled consistently.
    fn skip_expr(&mut self) -> Result<usize, String> {
        let saved = self.prev_regex_ok;
        self.prev_regex_ok = true;
        let mut depth = 0usize;
        let result = loop {
            let t = self.next()?;
            match t.tok {
                Tok::Eof => break self.err("unterminated template expression"),
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => {
                    if depth == 0 {
                        break Ok(t.start);
                    }
                    depth -= 1;
                }
                _ => {}
            }
        };
        self.prev_regex_ok = saved;
        result
    }

    fn regex(&mut self) -> Result<Tok, String> {
        self.pos += 1;
        let start = self.pos;
        let mut in_class = false;
        loop {
            let r = self.rest();
            let Some(c) = r.chars().next() else { return self.err("unterminated regex") };
            self.pos += c.len_utf8();
            match c {
                '\\' => {
                    if let Some(n) = self.rest().chars().next() {
                        self.pos += n.len_utf8();
                    }
                }
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => break,
                '\n' => return self.err("unterminated regex"),
                _ => {}
            }
        }
        let pattern = self.src[start..self.pos - 1].to_string();
        let fstart = self.pos;
        while self.rest().starts_with(is_id_part) {
            self.pos += self.rest().chars().next().map_or(1, |c| c.len_utf8());
        }
        let flags = self.src[fstart..self.pos].to_string();
        Ok(Tok::Regex(Rc::from(pattern), Rc::from(flags)))
    }
}
