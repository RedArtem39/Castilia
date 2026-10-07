//! CSS selector parsing and matching (Selectors Level 4 subset), shared by the
//! style engine and by `querySelector` in the JS DOM bindings.
use crate::dom::{Document, NodeId, NodeKind};
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comb {
    Descendant,
    Child,
    Adjacent,
    Sibling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    FirstLine,
    FirstLetter,
    /// Parsed but never rendered (`::selection`, `::placeholder`, ...).
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrOp {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
    pub value: String,
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    /// `a`, `b`, counts from the end, counts within the same tag
    Nth(i32, i32, bool, bool),
    Not(Vec<Complex>),
    Is(Vec<Complex>),
    Where(Vec<Complex>),
    Has(Vec<(Comb, Complex)>),
    Empty,
    Root,
    Checked,
    Disabled,
    Enabled,
    Required,
    Optional,
    AnyLink,
    Lang(String),
    /// Dynamic state we never match (`:hover`, `:focus`, ...).
    Never,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Complex {
    pub compounds: Vec<Compound>,
    /// `combs[i]` relates `compounds[i]` to `compounds[i + 1]`.
    pub combs: Vec<Comb>,
    pub specificity: u32,
    pub pseudo_element: Option<PseudoElement>,
}

pub fn spec(a: u32, b: u32, c: u32) -> u32 {
    (a.min(255) << 16) | (b.min(255) << 8) | c.min(255)
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '-' || !c.is_ascii()
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn skip_ws(&mut self) -> bool {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_whitespace()) {
            self.bump();
        }
        self.pos > start
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn ident(&mut self) -> Option<String> {
        let mut out = String::new();
        loop {
            match self.peek() {
                Some('\\') => {
                    self.bump();
                    let mut hex = String::new();
                    while hex.len() < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        hex.push(self.bump()?);
                    }
                    if hex.is_empty() {
                        out.push(self.bump()?);
                    } else {
                        let n = u32::from_str_radix(&hex, 16).ok()?;
                        out.push(char::from_u32(n).filter(|&c| c != '\0').unwrap_or('\u{fffd}'));
                        if self.peek().is_some_and(|c| c.is_whitespace()) {
                            self.bump();
                        }
                    }
                }
                Some(c) if (out.is_empty() && is_ident_start(c)) || (!out.is_empty() && is_ident_char(c)) => {
                    out.push(c);
                    self.bump();
                }
                Some(c) if out.is_empty() && c.is_ascii_digit() => return None,
                _ => break,
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }
    fn string_or_ident(&mut self) -> Option<String> {
        match self.peek()? {
            q @ ('"' | '\'') => {
                self.bump();
                let mut out = String::new();
                loop {
                    match self.bump()? {
                        c if c == q => return Some(out),
                        '\\' => out.push(self.bump()?),
                        c => out.push(c),
                    }
                }
            }
            _ => {
                let start = self.pos;
                while self.peek().is_some_and(|c| c != ']' && !c.is_whitespace()) {
                    self.bump();
                }
                if self.pos == start { None } else { Some(self.s[start..self.pos].to_string()) }
            }
        }
    }
}

/// Parse a comma-separated selector list. Returns `None` if any selector is invalid.
pub fn parse_list(source: &str) -> Option<Vec<Complex>> {
    let mut p = Parser { s: source, pos: 0 };
    let list = parse_list_inner(&mut p, false)?;
    p.skip_ws();
    if p.pos < source.len() { None } else { Some(list) }
}

fn parse_list_inner(p: &mut Parser, forgiving: bool) -> Option<Vec<Complex>> {
    let mut list = Vec::new();
    loop {
        p.skip_ws();
        match parse_complex(p) {
            Some(c) => list.push(c),
            None => {
                if forgiving {
                    // skip to next top-level comma / closing paren
                    let mut depth = 0;
                    while let Some(c) = p.peek() {
                        match c {
                            '(' => depth += 1,
                            ')' if depth == 0 => break,
                            ')' => depth -= 1,
                            ',' if depth == 0 => break,
                            _ => {}
                        }
                        p.bump();
                    }
                } else {
                    return None;
                }
            }
        }
        p.skip_ws();
        if !p.eat(',') {
            break;
        }
    }
    Some(list)
}

fn parse_complex(p: &mut Parser) -> Option<Complex> {
    let mut compounds = Vec::new();
    let mut combs = Vec::new();
    let mut specificity = 0;
    let mut pseudo_element = None;
    p.skip_ws();
    loop {
        let (compound, pe, spec_add) = parse_compound(p)?;
        specificity += spec_add;
        compounds.push(compound);
        if pe.is_some() {
            pseudo_element = pe;
        }
        let had_ws = p.skip_ws();
        match p.peek() {
            None | Some(',') | Some(')') => break,
            Some('>') => {
                p.bump();
                combs.push(Comb::Child);
            }
            Some('+') => {
                p.bump();
                combs.push(Comb::Adjacent);
            }
            Some('~') => {
                p.bump();
                combs.push(Comb::Sibling);
            }
            Some(_) if had_ws => combs.push(Comb::Descendant),
            Some(_) => return None,
        }
        if pseudo_element.is_some() {
            return None;
        }
        p.skip_ws();
    }
    Some(Complex { compounds, combs, specificity, pseudo_element })
}

fn parse_compound(p: &mut Parser) -> Option<(Compound, Option<PseudoElement>, u32)> {
    let mut c = Compound::default();
    let mut pe = None;
    let mut specificity = 0;
    let mut any = false;
    // type selector or universal (namespaces ignored)
    if p.eat('*') {
        any = true;
    } else if p.peek().is_some_and(|ch| is_ident_start(ch) || ch == '\\') {
        let name = p.ident()?;
        c.tag = Some(name.to_ascii_lowercase());
        specificity += spec(0, 0, 1);
        any = true;
    }
    if p.peek() == Some('|') && p.s[p.pos..].chars().nth(1) != Some('=') {
        p.bump();
        c.tag = None;
        if !p.eat('*') {
            let name = p.ident()?;
            c.tag = Some(name.to_ascii_lowercase());
        }
    }
    loop {
        match p.peek() {
            Some('#') => {
                p.bump();
                let id = p.ident_or_digits()?;
                c.id = Some(id);
                specificity += spec(1, 0, 0);
            }
            Some('.') => {
                p.bump();
                c.classes.push(p.ident()?);
                specificity += spec(0, 1, 0);
            }
            Some('[') => {
                p.bump();
                p.skip_ws();
                let mut name = p.ident()?.to_ascii_lowercase();
                if p.peek() == Some('|') && p.s[p.pos..].chars().nth(1) != Some('=') {
                    p.bump();
                    name = p.ident()?.to_ascii_lowercase();
                }
                p.skip_ws();
                let op = match p.peek()? {
                    ']' => AttrOp::Exists,
                    '=' => {
                        p.bump();
                        AttrOp::Equals
                    }
                    ch @ ('~' | '|' | '^' | '$' | '*') => {
                        p.bump();
                        if !p.eat('=') {
                            return None;
                        }
                        match ch {
                            '~' => AttrOp::Includes,
                            '|' => AttrOp::DashMatch,
                            '^' => AttrOp::Prefix,
                            '$' => AttrOp::Suffix,
                            _ => AttrOp::Substring,
                        }
                    }
                    _ => return None,
                };
                let mut value = String::new();
                let mut ci = false;
                if op != AttrOp::Exists {
                    p.skip_ws();
                    value = p.string_or_ident()?;
                    p.skip_ws();
                    if let Some(f) = p.peek()
                        && (f == 'i' || f == 'I' || f == 's' || f == 'S')
                    {
                        ci = f == 'i' || f == 'I';
                        p.bump();
                        p.skip_ws();
                    }
                }
                if !p.eat(']') {
                    return None;
                }
                c.attrs.push(AttrSel { name, op, value, case_insensitive: ci });
                specificity += spec(0, 1, 0);
            }
            Some(':') => {
                p.bump();
                let double = p.eat(':');
                let name = p.ident()?.to_ascii_lowercase();
                if double || matches!(name.as_str(), "before" | "after" | "first-line" | "first-letter") {
                    if name.starts_with("-moz-") || name.starts_with("-ms-") {
                        return None;
                    }
                    pe = Some(match name.as_str() {
                        "before" => PseudoElement::Before,
                        "after" => PseudoElement::After,
                        "first-line" => PseudoElement::FirstLine,
                        "first-letter" => PseudoElement::FirstLetter,
                        _ => PseudoElement::Other,
                    });
                    if p.eat('(') {
                        // e.g. ::part(x), ::slotted(x)
                        let mut depth = 1;
                        while let Some(ch) = p.bump() {
                            match ch {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    specificity += spec(0, 0, 1);
                } else {
                    let (pseudo, add) = parse_pseudo_class(p, &name)?;
                    specificity += add;
                    c.pseudos.push(pseudo);
                }
            }
            _ => break,
        }
        any = true;
    }
    if any { Some((c, pe, specificity)) } else { None }
}

impl Parser<'_> {
    fn ident_or_digits(&mut self) -> Option<String> {
        let start = self.pos;
        let mut out = String::new();
        loop {
            match self.peek() {
                Some('\\') => {
                    // reuse ident's escape handling by parsing a single escaped char
                    let before = self.pos;
                    let part = {
                        self.bump();
                        let mut hex = String::new();
                        while hex.len() < 6 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                            hex.push(self.bump()?);
                        }
                        if hex.is_empty() {
                            self.bump()?.to_string()
                        } else {
                            let n = u32::from_str_radix(&hex, 16).ok()?;
                            if self.peek().is_some_and(|c| c.is_whitespace()) {
                                self.bump();
                            }
                            char::from_u32(n).unwrap_or('\u{fffd}').to_string()
                        }
                    };
                    let _ = before;
                    out.push_str(&part);
                }
                Some(c) if is_ident_char(c) => {
                    out.push(c);
                    self.bump();
                }
                _ => break,
            }
        }
        if self.pos == start { None } else { Some(out) }
    }
}

fn parse_nth(arg: &str) -> Option<(i32, i32)> {
    let s: String = arg.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    match s.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    if let Some(n) = s.find('n') {
        let a = match &s[..n] {
            "" | "+" => 1,
            "-" => -1,
            v => v.parse().ok()?,
        };
        let rest = &s[n + 1..];
        let b = if rest.is_empty() { 0 } else { rest.trim_start_matches('+').parse().ok()? };
        Some((a, b))
    } else {
        Some((0, s.parse().ok()?))
    }
}

fn parse_pseudo_class(p: &mut Parser, name: &str) -> Option<(Pseudo, u32)> {
    let class = spec(0, 1, 0);
    if p.eat('(') {
        let result = match name {
            "not" | "is" | "matches" | "where" | "-webkit-any" | "-moz-any" => {
                let list = parse_list_inner(p, name != "not")?;
                let max = list.iter().map(|c| c.specificity).max().unwrap_or(0);
                match name {
                    "not" => (Pseudo::Not(list), max),
                    "where" => (Pseudo::Where(list), 0),
                    _ => (Pseudo::Is(list), max),
                }
            }
            "has" => {
                let mut rels = Vec::new();
                loop {
                    p.skip_ws();
                    let comb = match p.peek()? {
                        '>' => {
                            p.bump();
                            Comb::Child
                        }
                        '+' => {
                            p.bump();
                            Comb::Adjacent
                        }
                        '~' => {
                            p.bump();
                            Comb::Sibling
                        }
                        _ => Comb::Descendant,
                    };
                    p.skip_ws();
                    rels.push((comb, parse_complex(p)?));
                    p.skip_ws();
                    if !p.eat(',') {
                        break;
                    }
                }
                let max = rels.iter().map(|(_, c)| c.specificity).max().unwrap_or(0);
                (Pseudo::Has(rels), max)
            }
            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                let start = p.pos;
                let mut depth = 0;
                while let Some(c) = p.peek() {
                    if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    p.bump();
                }
                let raw = &p.s[start..p.pos];
                let arg = raw.split(" of ").next().unwrap_or(raw);
                let (a, b) = parse_nth(arg)?;
                (
                    Pseudo::Nth(a, b, name.contains("last"), name.contains("of-type")),
                    class,
                )
            }
            "lang" => {
                let start = p.pos;
                while p.peek().is_some_and(|c| c != ')') {
                    p.bump();
                }
                let v = p.s[start..p.pos].trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase();
                (Pseudo::Lang(v), class)
            }
            "dir" | "host" | "host-context" | "state" | "current" | "past" | "future" => {
                let mut depth = 1;
                while let Some(c) = p.bump() {
                    if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        depth -= 1;
                        if depth == 0 {
                            p.pos -= 1;
                            break;
                        }
                    }
                }
                (Pseudo::Never, class)
            }
            _ => return None,
        };
        p.skip_ws();
        if !p.eat(')') {
            return None;
        }
        return Some(result);
    }
    let pseudo = match name {
        "first-child" => Pseudo::FirstChild,
        "last-child" => Pseudo::LastChild,
        "only-child" => Pseudo::OnlyChild,
        "first-of-type" => Pseudo::FirstOfType,
        "last-of-type" => Pseudo::LastOfType,
        "only-of-type" => Pseudo::OnlyOfType,
        "empty" => Pseudo::Empty,
        "root" | "scope" => Pseudo::Root,
        "checked" => Pseudo::Checked,
        "disabled" => Pseudo::Disabled,
        "enabled" => Pseudo::Enabled,
        "required" => Pseudo::Required,
        "optional" => Pseudo::Optional,
        "link" | "any-link" => Pseudo::AnyLink,
        "hover" | "focus" | "active" | "visited" | "focus-within" | "focus-visible" | "target"
        | "target-within" | "placeholder-shown" | "read-only" | "read-write" | "valid" | "invalid"
        | "in-range" | "out-of-range" | "indeterminate" | "default" | "fullscreen" | "modal"
        | "autofill" | "-webkit-autofill" | "user-invalid" | "user-valid" | "defined" | "popover-open"
        | "open" | "paused" | "playing" | "local-link" | "blank" | "host" | "first" | "left" | "right"
        | "future" | "past" | "current" | "picture-in-picture" | "buffering" | "muted" | "stalled"
        | "seeking" | "volume-locked" | "-webkit-full-screen" => Pseudo::Never,
        _ => return None,
    };
    Some((pseudo, class))
}

// ---------- matching ----------

fn attr_matches(doc: &Document, node: NodeId, a: &AttrSel) -> bool {
    let Some(value) = doc.nodes[node].attribute(&a.name) else {
        return false;
    };
    if a.op == AttrOp::Exists {
        return true;
    }
    let (v, w);
    let (value, want) = if a.case_insensitive {
        v = value.to_lowercase();
        w = a.value.to_lowercase();
        (v.as_str(), w.as_str())
    } else {
        (value, a.value.as_str())
    };
    match a.op {
        AttrOp::Exists => true,
        AttrOp::Equals => value == want,
        AttrOp::Includes => !want.is_empty() && value.split_whitespace().any(|t| t == want),
        AttrOp::DashMatch => value == want || value.strip_prefix(want).is_some_and(|r| r.starts_with('-')),
        AttrOp::Prefix => !want.is_empty() && value.starts_with(want),
        AttrOp::Suffix => !want.is_empty() && value.ends_with(want),
        AttrOp::Substring => !want.is_empty() && value.contains(want),
    }
}

fn element_siblings(doc: &Document, node: NodeId) -> Vec<NodeId> {
    match doc.nodes[node].parent {
        Some(p) => doc.nodes[p]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.nodes[c].tag().is_some())
            .collect(),
        None => vec![node],
    }
}

fn nth_matches(a: i32, b: i32, index: i32) -> bool {
    // index is 1-based
    if a == 0 {
        return index == b;
    }
    let diff = index - b;
    diff % a == 0 && diff / a >= 0
}

fn pseudo_matches(doc: &Document, node: NodeId, pseudo: &Pseudo) -> bool {
    let n = &doc.nodes[node];
    match pseudo {
        Pseudo::Never => false,
        Pseudo::FirstChild => element_siblings(doc, node).first() == Some(&node),
        Pseudo::LastChild => element_siblings(doc, node).last() == Some(&node),
        Pseudo::OnlyChild => element_siblings(doc, node).len() == 1,
        Pseudo::FirstOfType | Pseudo::LastOfType | Pseudo::OnlyOfType => {
            let same: Vec<_> = element_siblings(doc, node)
                .into_iter()
                .filter(|&s| doc.nodes[s].tag() == n.tag())
                .collect();
            match pseudo {
                Pseudo::FirstOfType => same.first() == Some(&node),
                Pseudo::LastOfType => same.last() == Some(&node),
                _ => same.len() == 1,
            }
        }
        Pseudo::Nth(a, b, from_end, of_type) => {
            let mut sibs = element_siblings(doc, node);
            if *of_type {
                sibs.retain(|&s| doc.nodes[s].tag() == n.tag());
            }
            if *from_end {
                sibs.reverse();
            }
            let Some(pos) = sibs.iter().position(|&s| s == node) else {
                return false;
            };
            nth_matches(*a, *b, pos as i32 + 1)
        }
        Pseudo::Not(list) => !list.iter().any(|c| matches_complex(doc, node, c)),
        Pseudo::Is(list) | Pseudo::Where(list) => list.iter().any(|c| matches_complex(doc, node, c)),
        Pseudo::Has(rels) => rels.iter().any(|(comb, complex)| has_match(doc, node, *comb, complex)),
        Pseudo::Empty => n.children.iter().all(|&c| match &doc.nodes[c].kind {
            NodeKind::Text(t) => t.is_empty(),
            _ => false,
        }),
        Pseudo::Root => n.tag() == Some("html"),
        Pseudo::Checked => {
            n.attribute("checked").is_some() && matches!(n.tag(), Some("input"))
                || n.tag() == Some("option") && n.attribute("selected").is_some()
        }
        Pseudo::Disabled => {
            matches!(n.tag(), Some("input" | "button" | "select" | "textarea" | "fieldset" | "option" | "optgroup"))
                && (n.attribute("disabled").is_some() || in_disabled_fieldset(doc, node))
        }
        Pseudo::Enabled => {
            matches!(n.tag(), Some("input" | "button" | "select" | "textarea"))
                && n.attribute("disabled").is_none()
                && !in_disabled_fieldset(doc, node)
        }
        Pseudo::Required => matches!(n.tag(), Some("input" | "select" | "textarea")) && n.attribute("required").is_some(),
        Pseudo::Optional => matches!(n.tag(), Some("input" | "select" | "textarea")) && n.attribute("required").is_none(),
        Pseudo::AnyLink => matches!(n.tag(), Some("a" | "area")) && n.attribute("href").is_some(),
        Pseudo::Lang(want) => {
            let mut cur = Some(node);
            while let Some(id) = cur {
                if let Some(l) = doc.nodes[id].attribute("lang") {
                    let l = l.to_ascii_lowercase();
                    return l == *want || l.starts_with(&alloc::format!("{want}-"));
                }
                cur = doc.nodes[id].parent;
            }
            false
        }
    }
}

fn in_disabled_fieldset(doc: &Document, node: NodeId) -> bool {
    let mut cur = doc.nodes[node].parent;
    while let Some(id) = cur {
        if doc.nodes[id].tag() == Some("fieldset") && doc.nodes[id].attribute("disabled").is_some() {
            return true;
        }
        cur = doc.nodes[id].parent;
    }
    false
}

fn has_match(doc: &Document, scope: NodeId, comb: Comb, complex: &Complex) -> bool {
    let anchored = |candidate: NodeId| match_chain(doc, candidate, complex, complex.compounds.len() - 1, Some((scope, comb)));
    match comb {
        Comb::Descendant | Comb::Child => {
            let mut stack: Vec<NodeId> = doc.nodes[scope].children.clone();
            while let Some(id) = stack.pop() {
                if doc.nodes[id].tag().is_some() {
                    if anchored(id) {
                        return true;
                    }
                    if comb == Comb::Descendant {
                        stack.extend(doc.nodes[id].children.iter().copied());
                    } else if false {
                        break;
                    }
                }
            }
            false
        }
        Comb::Adjacent | Comb::Sibling => {
            let Some(parent) = doc.nodes[scope].parent else {
                return false;
            };
            let mut after = false;
            for &sib in &doc.nodes[parent].children {
                if sib == scope {
                    after = true;
                    continue;
                }
                if after && doc.nodes[sib].tag().is_some() {
                    if anchored(sib) {
                        return true;
                    }
                    if comb == Comb::Adjacent {
                        return false;
                    }
                }
            }
            false
        }
    }
}

pub fn matches_compound(doc: &Document, node: NodeId, c: &Compound) -> bool {
    let n = &doc.nodes[node];
    let NodeKind::Element { tag, .. } = &n.kind else {
        return false;
    };
    if let Some(t) = &c.tag
        && t != tag
    {
        return false;
    }
    if let Some(id) = &c.id
        && n.attribute("id") != Some(id.as_str())
    {
        return false;
    }
    if !c.classes.is_empty() {
        let Some(classes) = n.attribute("class") else {
            return false;
        };
        if !c.classes.iter().all(|want| classes.split_ascii_whitespace().any(|have| have == want)) {
            return false;
        }
    }
    c.attrs.iter().all(|a| attr_matches(doc, node, a)) && c.pseudos.iter().all(|p| pseudo_matches(doc, node, p))
}

fn prev_element(doc: &Document, node: NodeId) -> Option<NodeId> {
    let parent = doc.nodes[node].parent?;
    let mut prev = None;
    for &c in &doc.nodes[parent].children {
        if c == node {
            return prev;
        }
        if doc.nodes[c].tag().is_some() {
            prev = Some(c);
        }
    }
    None
}

/// Match `complex.compounds[..=index]` ending at `node`. With a `scope`, the leftmost
/// compound must also stand in the given relation to the scope element (`:has`).
fn match_chain(doc: &Document, node: NodeId, complex: &Complex, index: usize, scope: Option<(NodeId, Comb)>) -> bool {
    if !matches_compound(doc, node, &complex.compounds[index]) {
        return false;
    }
    if index == 0 {
        return match scope {
            None => true,
            Some((s, Comb::Descendant)) => {
                let mut cur = doc.nodes[node].parent;
                while let Some(id) = cur {
                    if id == s {
                        return true;
                    }
                    cur = doc.nodes[id].parent;
                }
                false
            }
            Some((s, Comb::Child)) => doc.nodes[node].parent == Some(s),
            Some((s, Comb::Adjacent)) => prev_element(doc, node) == Some(s),
            Some((s, Comb::Sibling)) => {
                let mut cur = prev_element(doc, node);
                while let Some(id) = cur {
                    if id == s {
                        return true;
                    }
                    cur = prev_element(doc, id);
                }
                false
            }
        };
    }
    match complex.combs[index - 1] {
        Comb::Child => doc.nodes[node]
            .parent
            .is_some_and(|p| match_chain(doc, p, complex, index - 1, scope)),
        Comb::Descendant => {
            let mut cur = doc.nodes[node].parent;
            while let Some(id) = cur {
                if match_chain(doc, id, complex, index - 1, scope) {
                    return true;
                }
                cur = doc.nodes[id].parent;
            }
            false
        }
        Comb::Adjacent => prev_element(doc, node).is_some_and(|p| match_chain(doc, p, complex, index - 1, scope)),
        Comb::Sibling => {
            let mut cur = prev_element(doc, node);
            while let Some(id) = cur {
                if match_chain(doc, id, complex, index - 1, scope) {
                    return true;
                }
                cur = prev_element(doc, id);
            }
            false
        }
    }
}

pub fn matches_complex(doc: &Document, node: NodeId, complex: &Complex) -> bool {
    complex.pseudo_element.is_none() && match_chain(doc, node, complex, complex.compounds.len() - 1, None)
}

/// Like `matches_complex` but ignores a trailing pseudo-element (used when styling `::before`).
pub fn matches_complex_ignoring_pseudo_element(doc: &Document, node: NodeId, complex: &Complex) -> bool {
    match_chain(doc, node, complex, complex.compounds.len() - 1, None)
}

pub fn matches_list(doc: &Document, node: NodeId, list: &[Complex]) -> bool {
    list.iter().any(|c| matches_complex(doc, node, c))
}

/// Convenience used by the DOM bindings.
pub fn matches_str(doc: &Document, node: NodeId, selector: &str) -> bool {
    parse_list(selector).is_some_and(|l| matches_list(doc, node, &l))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn count(doc: &Document, sel: &str) -> usize {
        let list = parse_list(sel).unwrap_or_else(|| panic!("invalid {sel}"));
        (0..doc.nodes.len()).filter(|&i| matches_list(doc, i, &list)).count()
    }

    #[test]
    fn combinators_and_attributes() {
        let doc = html::parse(
            "<div id=a class='x y'><p>1</p><p class=z>2</p><span>3</span></div><a href='/u/1' data-k='v-1'>l</a>",
        );
        assert_eq!(count(&doc, "div > p"), 2);
        assert_eq!(count(&doc, "p + p"), 1);
        assert_eq!(count(&doc, "p ~ span"), 1);
        assert_eq!(count(&doc, "#a.x.y p:not(.z)"), 1);
        assert_eq!(count(&doc, "a[href^='/u/'][data-k|=v]"), 1);
        assert_eq!(count(&doc, "div:has(> span)"), 1);
        assert_eq!(count(&doc, "div:has(> a)"), 0);
        assert_eq!(count(&doc, "p:nth-child(2n)"), 1);
        assert_eq!(count(&doc, ":is(p, span):last-child"), 1);
    }

    #[test]
    fn escapes_and_invalid() {
        let doc = html::parse("<p class='md:flex w-1/2'>x</p>");
        assert_eq!(count(&doc, ".md\\:flex.w-1\\/2"), 1);
        assert!(parse_list("a::-moz-focus-inner").is_none());
        assert!(parse_list("a >").is_none());
        let l = parse_list("p::before").unwrap();
        assert_eq!(l[0].pseudo_element, Some(PseudoElement::Before));
    }

    #[test]
    fn specificity_orders() {
        let a = parse_list("#a .b p").unwrap()[0].specificity;
        let b = parse_list(".b .c .d p").unwrap()[0].specificity;
        assert!(a > b);
    }
}
