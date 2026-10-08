//! CSS value parsing helpers: tokenizing, lengths, `calc()`, gradients, `var()`.
use crate::{
    color::{self, Color},
    math,
};
use alloc::{
    collections::BTreeMap,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};

/// A CSS size. Lengths are kept as `px + percent * base` so `calc(100% - 20px)` works.
#[derive(Debug, Clone, PartialEq)]
pub enum Dim {
    Auto,
    Len(f32, f32),
    MinContent,
    MaxContent,
    FitContent,
    /// `min()` (false) or `max()` (true) of the arguments.
    Func(bool, Rc<Vec<Dim>>),
}

impl Dim {
    pub const ZERO: Dim = Dim::Len(0.0, 0.0);
    pub fn px(v: f32) -> Dim {
        Dim::Len(v, 0.0)
    }
    pub fn pct(v: f32) -> Dim {
        Dim::Len(0.0, v)
    }
    pub fn is_auto(&self) -> bool {
        matches!(self, Dim::Auto)
    }
    /// True when the value cannot be resolved without a percentage base.
    pub fn has_percent(&self) -> bool {
        match self {
            Dim::Len(_, p) => *p != 0.0,
            Dim::Func(_, args) => args.iter().any(Dim::has_percent),
            _ => false,
        }
    }
    /// Position along an axis where percentages refer to the free space (`object-position`, `background-position`).
    pub fn resolve_offset(&self, free: f32) -> f32 {
        match self {
            Dim::Len(px, pct) => px + free * pct / 100.0,
            _ => free / 2.0,
        }
    }
    /// Resolve against `base`. Intrinsic keywords and `auto` give `None`.
    pub fn resolve(&self, base: f32) -> Option<f32> {
        match self {
            Dim::Len(px, pct) => Some(px + base * pct / 100.0),
            Dim::Func(is_max, args) => {
                let mut result: Option<f32> = None;
                for a in args.iter() {
                    let v = a.resolve(base)?;
                    result = Some(match result {
                        None => v,
                        Some(r) if *is_max => r.max(v),
                        Some(r) => r.min(v),
                    });
                }
                result
            }
            _ => None,
        }
    }
}

/// Split on top-level whitespace, keeping function arguments and quotes together.
pub fn split_ws(value: &str) -> Vec<&str> {
    split_top(value, |c| c.is_whitespace(), true)
}

/// Split on top-level commas.
pub fn split_commas(value: &str) -> Vec<&str> {
    split_top(value, |c| c == ',', false)
}

fn split_top(value: &str, is_sep: impl Fn(char) -> bool, skip_empty: bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut chars = value.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' {
                chars.next();
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if depth == 0 && is_sep(c) => {
                let part = value[start..i].trim();
                if !(skip_empty && part.is_empty()) {
                    out.push(part);
                }
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let part = value[start..].trim();
    if !(skip_empty && part.is_empty()) {
        out.push(part);
    }
    out
}

/// Strip a leading function name: `strip_function("calc(1px)", "calc")` → `Some("1px")`.
pub fn strip_function<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let value = value.trim();
    if value.len() <= name.len() + 1 || !value[..name.len()].eq_ignore_ascii_case(name) {
        return None;
    }
    let rest = value[name.len()..].trim_start();
    rest.strip_prefix('(')?.strip_suffix(')')
}

pub struct LenCtx {
    pub font_size: f32,
    pub root_font_size: f32,
    pub vw: f32,
    pub vh: f32,
}

#[derive(Clone, Copy, Debug)]
struct Val {
    px: f32,
    pct: f32,
    /// A pure number (no unit).
    number: bool,
}

struct CalcParser<'a> {
    s: &'a str,
    pos: usize,
    ctx: &'a LenCtx,
}

impl CalcParser<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }
    fn expr(&mut self) -> Option<Val> {
        let mut left = self.term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    let r = self.term()?;
                    if left.number != r.number {
                        return None;
                    }
                    left = Val { px: left.px + r.px, pct: left.pct + r.pct, number: left.number };
                }
                Some('-') => {
                    self.pos += 1;
                    let r = self.term()?;
                    if left.number != r.number {
                        return None;
                    }
                    left = Val { px: left.px - r.px, pct: left.pct - r.pct, number: left.number };
                }
                _ => return Some(left),
            }
        }
    }
    fn term(&mut self) -> Option<Val> {
        let mut left = self.factor()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    let r = self.factor()?;
                    left = if r.number {
                        Val { px: left.px * r.px, pct: left.pct * r.px, number: left.number }
                    } else if left.number {
                        Val { px: r.px * left.px, pct: r.pct * left.px, number: false }
                    } else {
                        return None;
                    };
                }
                Some('/') => {
                    self.pos += 1;
                    let r = self.factor()?;
                    if !r.number || r.px == 0.0 {
                        return None;
                    }
                    left = Val { px: left.px / r.px, pct: left.pct / r.px, number: left.number };
                }
                _ => return Some(left),
            }
        }
    }
    fn factor(&mut self) -> Option<Val> {
        self.skip_ws();
        if self.peek()? == '(' {
            self.pos += 1;
            let v = self.expr()?;
            self.skip_ws();
            if self.peek()? != ')' {
                return None;
            }
            self.pos += 1;
            return Some(v);
        }
        // nested function or number/dimension
        let start = self.pos;
        let rest = &self.s[start..];
        let name_len = rest.find(|c: char| !c.is_ascii_alphabetic() && c != '-').unwrap_or(rest.len());
        if name_len > 0 && rest[name_len..].starts_with('(') {
            let name = rest[..name_len].to_ascii_lowercase();
            let mut depth = 0;
            let mut end = None;
            for (i, c) in rest.char_indices().skip(name_len) {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = end?;
            let inner = &rest[name_len + 1..end];
            self.pos = start + end + 1;
            return match name.as_str() {
                "calc" | "-webkit-calc" | "-moz-calc" => eval_calc(inner, self.ctx),
                "min" | "max" | "clamp" => {
                    let args: Vec<Val> = split_commas(inner)
                        .into_iter()
                        .map(|a| eval_calc(a, self.ctx))
                        .collect::<Option<_>>()?;
                    if args.iter().any(|a| a.pct != 0.0) {
                        return None;
                    }
                    let px = match name.as_str() {
                        "min" => args.iter().map(|a| a.px).fold(f32::INFINITY, f32::min),
                        "max" => args.iter().map(|a| a.px).fold(f32::NEG_INFINITY, f32::max),
                        _ => {
                            if args.len() != 3 {
                                return None;
                            }
                            args[1].px.min(args[2].px).max(args[0].px)
                        }
                    };
                    Some(Val { px, pct: 0.0, number: args.iter().all(|a| a.number) })
                }
                "round" | "mod" | "rem" | "abs" | "sign" | "sqrt" | "pow" | "hypot" | "sin" | "cos" | "tan" | "env" => None,
                _ => None,
            };
        }
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '+' || c == '-' || c == 'e' || c == 'E') || false)
            .unwrap_or(rest.len());
        // numbers may contain 'e' (1e3) but also start units like 'em'; resolve ambiguity.
        let mut num_end = 0;
        let bytes = rest.as_bytes();
        let mut i = 0;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            let mut j = i + 1;
            if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                j += 1;
            }
            if j < bytes.len() && bytes[j].is_ascii_digit() {
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        num_end = num_end.max(i);
        let _ = end;
        if num_end == 0 {
            return None;
        }
        let number: f32 = rest[..num_end].parse().ok()?;
        let unit_end = rest[num_end..]
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '%'))
            .map_or(rest.len(), |p| p + num_end);
        let unit = rest[num_end..unit_end].to_ascii_lowercase();
        self.pos = start + unit_end;
        let c = self.ctx;
        let px = |v: f32| Some(Val { px: v, pct: 0.0, number: false });
        match unit.as_str() {
            "" => Some(Val { px: number, pct: 0.0, number: true }),
            "%" => Some(Val { px: 0.0, pct: number, number: false }),
            "px" => px(number),
            "em" => px(number * c.font_size),
            "rem" => px(number * c.root_font_size),
            "ex" | "ch" | "cap" | "ic" => px(number * c.font_size * 0.5),
            "lh" | "rlh" => px(number * c.font_size * 1.2),
            "vw" | "dvw" | "svw" | "lvw" => px(number * c.vw / 100.0),
            "vh" | "dvh" | "svh" | "lvh" => px(number * c.vh / 100.0),
            "vmin" | "dvmin" | "svmin" | "lvmin" => px(number * c.vw.min(c.vh) / 100.0),
            "vmax" | "dvmax" | "svmax" | "lvmax" => px(number * c.vw.max(c.vh) / 100.0),
            "pt" => px(number * 4.0 / 3.0),
            "pc" => px(number * 16.0),
            "in" => px(number * 96.0),
            "cm" => px(number * 96.0 / 2.54),
            "mm" => px(number * 96.0 / 25.4),
            "q" => px(number * 96.0 / 101.6),
            _ => None,
        }
    }
}

fn eval_calc(expr: &str, ctx: &LenCtx) -> Option<Val> {
    let mut p = CalcParser { s: expr, pos: 0, ctx };
    let v = p.expr()?;
    p.skip_ws();
    if p.pos == expr.len() { Some(v) } else { None }
}

/// Parse a length / percentage / keyword. Plain numbers other than 0 are rejected.
pub fn parse_dim(value: &str, ctx: &LenCtx) -> Option<Dim> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let lower = v.to_ascii_lowercase();
    match lower.as_str() {
        "auto" | "none" | "normal" => return Some(Dim::Auto),
        "min-content" | "-webkit-min-content" | "-moz-min-content" => return Some(Dim::MinContent),
        "max-content" | "-webkit-max-content" | "-moz-max-content" => return Some(Dim::MaxContent),
        "fit-content" | "-webkit-fit-content" | "-moz-fit-content" | "stretch" | "-webkit-fill-available" | "fill-available" => {
            return Some(if lower.contains("fit") { Dim::FitContent } else { Dim::pct(100.0) });
        }
        "thin" => return Some(Dim::px(1.0)),
        "medium" => return Some(Dim::px(3.0)),
        "thick" => return Some(Dim::px(5.0)),
        _ => {}
    }
    for (name, is_max) in [("min", false), ("max", true)] {
        if let Some(inner) = strip_function(v, name) {
            let args: Vec<Dim> = split_commas(inner)
                .into_iter()
                .map(|a| parse_dim(a, ctx))
                .collect::<Option<_>>()?;
            if args.is_empty() || args.iter().any(|a| !matches!(a, Dim::Len(..) | Dim::Func(..))) {
                return None;
            }
            return Some(Dim::Func(is_max, Rc::new(args)));
        }
    }
    if let Some(inner) = strip_function(v, "clamp") {
        let args: Vec<Dim> = split_commas(inner)
            .into_iter()
            .map(|a| parse_dim(a, ctx))
            .collect::<Option<_>>()?;
        if args.len() != 3 {
            return None;
        }
        let inner_min = Dim::Func(false, Rc::new(Vec::from([args[1].clone(), args[2].clone()])));
        return Some(Dim::Func(true, Rc::new(Vec::from([args[0].clone(), inner_min]))));
    }
    let val = eval_calc(v, ctx)?;
    if val.number && val.px != 0.0 {
        return None;
    }
    Some(Dim::Len(val.px, val.pct))
}

/// Parse a length that must resolve without a percentage base.
pub fn parse_px(value: &str, ctx: &LenCtx) -> Option<f32> {
    match parse_dim(value, ctx)? {
        Dim::Len(px, pct) if pct == 0.0 => Some(px),
        _ => None,
    }
}

pub fn parse_number(value: &str) -> Option<f32> {
    value.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

/// `var(--x, fallback)` substitution. Returns `None` when a variable is missing and has no fallback.
/// Custom properties of an element: its own declarations over the parent's scope. Elements that
/// declare nothing share the parent's scope, so design-token sheets with thousands of variables
/// are not copied per element.
#[derive(Debug, Default, PartialEq)]
pub struct CssVars {
    parent: Option<Rc<CssVars>>,
    /// `None`: the variable was reset (`initial`) at this level.
    own: BTreeMap<String, Option<String>>,
    depth: u32,
}

impl CssVars {
    pub fn get(&self, name: &str) -> Option<&String> {
        let mut cur = Some(self);
        while let Some(c) = cur {
            if let Some(v) = c.own.get(name) {
                return v.as_ref();
            }
            cur = c.parent.as_deref();
        }
        None
    }

    /// A scope with `own` declarations over `parent`; long chains are flattened to keep lookups short.
    pub fn child(parent: &Rc<CssVars>, own: BTreeMap<String, Option<String>>) -> CssVars {
        if parent.depth < 24 {
            return CssVars { parent: Some(parent.clone()), own, depth: parent.depth + 1 };
        }
        let mut all = parent.flatten();
        for (k, v) in own {
            all.insert(k, v);
        }
        CssVars { parent: None, own: all, depth: 0 }
    }

    fn flatten(&self) -> BTreeMap<String, Option<String>> {
        let mut out = match &self.parent {
            Some(p) => p.flatten(),
            None => BTreeMap::new(),
        };
        for (k, v) in &self.own {
            out.insert(k.clone(), v.clone());
        }
        out
    }
}

/// Variable lookup for `var()` substitution.
pub trait VarLookup {
    fn lookup(&self, name: &str) -> Option<&String>;
}

impl VarLookup for BTreeMap<String, String> {
    fn lookup(&self, name: &str) -> Option<&String> {
        self.get(name)
    }
}

impl VarLookup for CssVars {
    fn lookup(&self, name: &str) -> Option<&String> {
        self.get(name)
    }
}

pub fn substitute_vars<V: VarLookup + ?Sized>(value: &str, vars: &V, depth: usize) -> Option<String> {
    if !value.contains("var(") {
        return Some(value.to_string());
    }
    if depth > 16 {
        return None;
    }
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("var(") {
        // make sure it's not part of a longer identifier like "somevar("
        let prev_ok = rest[..start].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'));
        if !prev_ok {
            out.push_str(&rest[..start + 4]);
            rest = &rest[start + 4..];
            continue;
        }
        out.push_str(&rest[..start]);
        let args_start = start + 4;
        let mut depth_p = 1;
        let mut end = None;
        for (i, c) in rest[args_start..].char_indices() {
            match c {
                '(' => depth_p += 1,
                ')' => {
                    depth_p -= 1;
                    if depth_p == 0 {
                        end = Some(args_start + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end?;
        let inner = &rest[args_start..end];
        let (name, fallback) = match inner.find(',') {
            Some(i) => (inner[..i].trim(), Some(inner[i + 1..].trim())),
            None => (inner.trim(), None),
        };
        let replacement = match vars.lookup(name) {
            Some(v) => substitute_vars(v, vars, depth + 1)?,
            None => substitute_vars(fallback?, vars, depth + 1)?,
        };
        out.push_str(&replacement);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    /// CSS angle in degrees (180 = top to bottom).
    pub angle: f32,
    pub radial: bool,
    /// ARGB color and optional position as a fraction 0..=1.
    pub stops: Vec<(Color, Option<f32>)>,
}

pub fn parse_gradient(value: &str, ctx: &LenCtx) -> Option<Gradient> {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    let (radial, inner) = if let Some(i) = ["linear-gradient", "repeating-linear-gradient", "-webkit-linear-gradient"]
        .iter()
        .find_map(|n| strip_function(v, n))
    {
        (false, i)
    } else if let Some(i) = ["radial-gradient", "repeating-radial-gradient", "conic-gradient", "-webkit-radial-gradient"]
        .iter()
        .find_map(|n| strip_function(v, n))
    {
        (true, i)
    } else {
        let _ = lower;
        return None;
    };
    let mut parts = split_commas(inner);
    let mut angle = 180.0;
    if let Some(first) = parts.first() {
        let f = first.to_ascii_lowercase();
        if let Some(rest) = f.strip_prefix("to ") {
            let mut dx = 0.0;
            let mut dy = 0.0;
            for w in rest.split_whitespace() {
                match w {
                    "left" => dx = -1.0,
                    "right" => dx = 1.0,
                    "top" => dy = -1.0,
                    "bottom" => dy = 1.0,
                    _ => {}
                }
            }
            angle = match (dx, dy) {
                (0.0, -1.0) => 0.0,
                (1.0, 0.0) => 90.0,
                (0.0, 1.0) => 180.0,
                (-1.0, 0.0) => 270.0,
                (1.0, -1.0) => 45.0,
                (1.0, 1.0) => 135.0,
                (-1.0, 1.0) => 225.0,
                (-1.0, -1.0) => 315.0,
                _ => 180.0,
            };
            parts.remove(0);
        } else if f.ends_with("deg") || f.ends_with("turn") || f.ends_with("rad") || f.ends_with("grad") {
            if let Some(a) = parse_angle(&f) {
                angle = a;
                parts.remove(0);
            }
        } else if radial && (f.contains("circle") || f.contains("ellipse") || f.starts_with("closest") || f.starts_with("farthest") || f.contains(" at ") || f.starts_with("at ") || f.starts_with("from ")) {
            parts.remove(0);
        }
    }
    let _ = ctx;
    let mut stops = Vec::new();
    for part in parts {
        let tokens = split_ws(part);
        let Some(first) = tokens.first() else { continue };
        let Some(c) = color::parse_color(first) else {
            // color hints / unsupported stop syntax
            continue;
        };
        let pos = tokens.get(1).and_then(|t| t.strip_suffix('%')).and_then(|p| p.parse::<f32>().ok()).map(|p| p / 100.0);
        stops.push((c, pos));
    }
    if stops.len() < 2 {
        return None;
    }
    Some(Gradient { angle, radial, stops })
}

pub fn parse_angle(v: &str) -> Option<f32> {
    let v = v.trim();
    if let Some(n) = v.strip_suffix("deg") {
        n.trim().parse().ok()
    } else if let Some(n) = v.strip_suffix("grad") {
        n.trim().parse::<f32>().ok().map(|g| g * 0.9)
    } else if let Some(n) = v.strip_suffix("rad") {
        n.trim().parse::<f32>().ok().map(|r| r * 180.0 / core::f32::consts::PI)
    } else if let Some(n) = v.strip_suffix("turn") {
        n.trim().parse::<f32>().ok().map(|t| t * 360.0)
    } else {
        None
    }
}

/// Extract the URL inside `url(...)`.
pub fn parse_url(value: &str) -> Option<String> {
    let inner = strip_function(value, "url")?.trim();
    let inner = inner
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(inner);
    Some(inner.trim().to_string())
}

/// Decode a CSS string token body (`"abc\201C x"`).
pub fn parse_css_string(value: &str) -> Option<String> {
    let v = value.trim();
    let q = v.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let inner = v[1..].strip_suffix(q)?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.push(chars.next()?);
        }
        if hex.is_empty() {
            if let Some(n) = chars.next()
                && n != '\n'
            {
                out.push(n);
            }
        } else {
            if chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            let n = u32::from_str_radix(&hex, 16).ok()?;
            out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
        }
    }
    Some(out)
}

pub fn round_px(v: f32) -> f32 {
    math::round32(v * 64.0) / 64.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LenCtx {
        LenCtx { font_size: 16.0, root_font_size: 16.0, vw: 1000.0, vh: 800.0 }
    }

    #[test]
    fn lengths_and_calc() {
        let c = ctx();
        assert_eq!(parse_dim("2em", &c), Some(Dim::px(32.0)));
        assert_eq!(parse_dim("10vw", &c), Some(Dim::px(100.0)));
        assert_eq!(parse_dim("calc(100% - 20px)", &c), Some(Dim::Len(-20.0, 100.0)));
        assert_eq!(parse_dim("calc(2 * (10px + 5px))", &c), Some(Dim::px(30.0)));
        assert_eq!(parse_dim("calc(1rem + 50%/2)", &c), Some(Dim::Len(16.0, 25.0)));
        assert_eq!(parse_dim("12", &c), None);
        let d = parse_dim("min(100%, 600px)", &c).unwrap();
        assert_eq!(d.resolve(400.0), Some(400.0));
        assert_eq!(d.resolve(900.0), Some(600.0));
        let d = parse_dim("clamp(1rem, 2.5vw, 2rem)", &c).unwrap();
        assert_eq!(d.resolve(0.0), Some(25.0));
    }

    #[test]
    fn variables() {
        let mut vars = BTreeMap::new();
        vars.insert("--a".to_string(), "10px".to_string());
        vars.insert("--b".to_string(), "var(--a)".to_string());
        assert_eq!(substitute_vars("calc(var(--b) * 2)", &vars, 0).as_deref(), Some("calc(10px * 2)"));
        assert_eq!(substitute_vars("var(--zzz, 3px)", &vars, 0).as_deref(), Some("3px"));
        assert_eq!(substitute_vars("var(--zzz)", &vars, 0), None);
    }

    #[test]
    fn gradients_urls_strings() {
        let c = ctx();
        let g = parse_gradient("linear-gradient(to right, #fff, rgba(0,0,0,.5) 80%)", &c).unwrap();
        assert_eq!(g.angle, 90.0);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(parse_url("url('a b.png')").as_deref(), Some("a b.png"));
        assert_eq!(parse_css_string("\"a\\201C b\"").as_deref(), Some("a\u{201c}b"));
    }
}
