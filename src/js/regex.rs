//! Backtracking regular expression engine with JS semantics (captures, named groups,
//! lookaround, backreferences, lazy quantifiers, case folding, unicode escapes).
use alloc::{
    boxed::Box,
    format,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[derive(Clone, Debug)]
enum ClassItem {
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
    Prop(String, bool),
}

#[derive(Clone, Debug)]
struct ClassSet {
    neg: bool,
    items: Vec<ClassItem>,
}

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class(Box<ClassSet>),
    Start,
    End,
    WordB(bool),
    Group(Box<Node>, Option<usize>),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: u32, greedy: bool },
    BackRef(usize),
    NamedRef(String),
    Look { node: Box<Node>, ahead: bool, neg: bool },
}

#[derive(Clone, Debug)]
enum Inst {
    Char(char),
    Any,
    Class(usize),
    Start,
    End,
    WordB(bool),
    Save(usize),
    Split(usize, usize),
    Jmp(usize),
    BackRef(usize),
    /// pos register, loop head, exit
    LoopBack(usize, usize, usize),
    MarkPos(usize),
    Look { ahead: bool, neg: bool, end: usize },
    Match,
}

pub struct Program {
    insts: Vec<Inst>,
    classes: Vec<ClassSet>,
    pub ncap: usize,
    nreg: usize,
    pub names: Vec<(String, usize)>,
    icase: bool,
    multiline: bool,
    dot_all: bool,
    unicode: bool,
}

pub struct RegexData {
    pub source: Rc<str>,
    pub flags: Rc<str>,
    pub prog: Rc<Program>,
    pub global: bool,
    pub sticky: bool,
}

struct Parser<'a> {
    p: &'a [char],
    pos: usize,
    ncap: usize,
    names: Vec<(String, usize)>,
    unicode: bool,
    total_caps: usize,
}

type PR<T> = Result<T, String>;

fn count_groups(p: &[char]) -> usize {
    let mut n = 0;
    let mut i = 0;
    let mut in_class = false;
    while i < p.len() {
        match p[i] {
            '\\' => i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '(' if !in_class => {
                if p.get(i + 1) != Some(&'?') || (p.get(i + 2) == Some(&'<') && !matches!(p.get(i + 3), Some('=') | Some('!'))) {
                    n += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.p.get(self.pos).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn alt(&mut self) -> PR<Node> {
        let mut alts = vec![self.seq()?];
        while self.eat('|') {
            alts.push(self.seq()?);
        }
        Ok(if alts.len() == 1 { alts.pop().expect("one") } else { Node::Alt(alts) })
    }

    fn seq(&mut self) -> PR<Node> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let atom = self.quantifier(atom)?;
            items.push(atom);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().expect("one"),
            _ => Node::Seq(items),
        })
    }

    fn quantifier(&mut self, atom: Node) -> PR<Node> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, u32::MAX)
            }
            Some('+') => {
                self.pos += 1;
                (1, u32::MAX)
            }
            Some('?') => {
                self.pos += 1;
                (0, 1)
            }
            Some('{') => {
                let save = self.pos;
                self.pos += 1;
                let mut a = String::new();
                while let Some(c) = self.peek().filter(|c| c.is_ascii_digit()) {
                    a.push(c);
                    self.pos += 1;
                }
                let mut b = None;
                if self.eat(',') {
                    let mut s = String::new();
                    while let Some(c) = self.peek().filter(|c| c.is_ascii_digit()) {
                        s.push(c);
                        self.pos += 1;
                    }
                    b = Some(s);
                }
                if a.is_empty() || !self.eat('}') {
                    self.pos = save;
                    return Ok(atom);
                }
                let min: u32 = a.parse().unwrap_or(u32::MAX);
                let max = match b {
                    None => min,
                    Some(s) if s.is_empty() => u32::MAX,
                    Some(s) => s.parse().unwrap_or(u32::MAX),
                };
                (min, max)
            }
            _ => return Ok(atom),
        };
        if matches!(atom, Node::Start | Node::End | Node::WordB(_)) {
            return Err("Nothing to repeat".to_string());
        }
        let greedy = !self.eat('?');
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy })
    }

    fn atom(&mut self) -> PR<Node> {
        let c = self.peek().ok_or("unexpected end")?;
        self.pos += 1;
        match c {
            '.' => Ok(Node::Any),
            '^' => Ok(Node::Start),
            '$' => Ok(Node::End),
            '(' => {
                if self.eat('?') {
                    if self.eat(':') {
                        let n = self.alt()?;
                        self.expect_close()?;
                        return Ok(Node::Group(Box::new(n), None));
                    }
                    if self.eat('=') || self.peek() == Some('!') {
                        let neg = if self.peek() == Some('!') {
                            self.pos += 1;
                            true
                        } else {
                            false
                        };
                        let n = self.alt()?;
                        self.expect_close()?;
                        return Ok(Node::Look { node: Box::new(n), ahead: true, neg });
                    }
                    if self.eat('<') {
                        if self.peek() == Some('=') || self.peek() == Some('!') {
                            let neg = self.peek() == Some('!');
                            self.pos += 1;
                            let n = self.alt()?;
                            self.expect_close()?;
                            return Ok(Node::Look { node: Box::new(n), ahead: false, neg });
                        }
                        let mut name = String::new();
                        while let Some(ch) = self.peek() {
                            self.pos += 1;
                            if ch == '>' {
                                break;
                            }
                            name.push(ch);
                        }
                        self.ncap += 1;
                        let idx = self.ncap;
                        self.names.push((name, idx));
                        let n = self.alt()?;
                        self.expect_close()?;
                        return Ok(Node::Group(Box::new(n), Some(idx)));
                    }
                    return Err("Invalid group".to_string());
                }
                self.ncap += 1;
                let idx = self.ncap;
                let n = self.alt()?;
                self.expect_close()?;
                Ok(Node::Group(Box::new(n), Some(idx)))
            }
            '[' => self.class(),
            '\\' => self.escape(false).map(|e| match e {
                Esc::Char(c) => Node::Char(c),
                Esc::Item(i) => Node::Class(Box::new(ClassSet { neg: false, items: vec![i] })),
                Esc::Node(n) => n,
            }),
            '*' | '+' | '?' => Err("Nothing to repeat".to_string()),
            c => Ok(Node::Char(c)),
        }
    }

    fn expect_close(&mut self) -> PR<()> {
        if self.eat(')') { Ok(()) } else { Err("Unterminated group".to_string()) }
    }

    fn hex(&mut self, n: usize) -> Option<u32> {
        let s: String = self.p.get(self.pos..self.pos + n)?.iter().collect();
        let v = u32::from_str_radix(&s, 16).ok()?;
        self.pos += n;
        Some(v)
    }

    fn escape(&mut self, in_class: bool) -> PR<Esc> {
        let c = self.peek().ok_or("\\ at end of pattern")?;
        self.pos += 1;
        Ok(match c {
            'd' => Esc::Item(ClassItem::Digit(true)),
            'D' => Esc::Item(ClassItem::Digit(false)),
            'w' => Esc::Item(ClassItem::Word(true)),
            'W' => Esc::Item(ClassItem::Word(false)),
            's' => Esc::Item(ClassItem::Space(true)),
            'S' => Esc::Item(ClassItem::Space(false)),
            'b' if in_class => Esc::Char('\u{8}'),
            'b' => Esc::Node(Node::WordB(true)),
            'B' if !in_class => Esc::Node(Node::WordB(false)),
            'n' => Esc::Char('\n'),
            'r' => Esc::Char('\r'),
            't' => Esc::Char('\t'),
            'v' => Esc::Char('\u{b}'),
            'f' => Esc::Char('\u{c}'),
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => Esc::Char('\0'),
            'c' => match self.peek() {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.pos += 1;
                    Esc::Char(char::from_u32((l as u32) % 32).unwrap_or('\0'))
                }
                _ => Esc::Char('\\'),
            },
            'x' => match self.hex(2) {
                Some(v) => Esc::Char(char::from_u32(v).unwrap_or('\0')),
                None => Esc::Char('x'),
            },
            'u' => {
                if self.peek() == Some('{') && self.unicode {
                    self.pos += 1;
                    let mut s = String::new();
                    while let Some(ch) = self.peek() {
                        self.pos += 1;
                        if ch == '}' {
                            break;
                        }
                        s.push(ch);
                    }
                    Esc::Char(u32::from_str_radix(&s, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'))
                } else {
                    match self.hex(4) {
                        Some(v) => {
                            if (0xd800..0xdc00).contains(&v) && self.p.get(self.pos) == Some(&'\\') && self.p.get(self.pos + 1) == Some(&'u') {
                                let save = self.pos;
                                self.pos += 2;
                                if let Some(lo) = self.hex(4) {
                                    if (0xdc00..0xe000).contains(&lo) {
                                        return Ok(Esc::Char(char::from_u32(0x10000 + ((v - 0xd800) << 10) + (lo - 0xdc00)).unwrap_or('\u{fffd}')));
                                    }
                                }
                                self.pos = save;
                            }
                            Esc::Char(char::from_u32(v).unwrap_or('\u{fffd}'))
                        }
                        None => Esc::Char('u'),
                    }
                }
            }
            'p' | 'P' if self.unicode => {
                let neg = c == 'P';
                if self.eat('{') {
                    let mut name = String::new();
                    while let Some(ch) = self.peek() {
                        self.pos += 1;
                        if ch == '}' {
                            break;
                        }
                        name.push(ch);
                    }
                    Esc::Item(ClassItem::Prop(name, !neg))
                } else {
                    Esc::Char(c)
                }
            }
            'k' if !in_class && (self.peek() == Some('<')) => {
                self.pos += 1;
                let mut name = String::new();
                while let Some(ch) = self.peek() {
                    self.pos += 1;
                    if ch == '>' {
                        break;
                    }
                    name.push(ch);
                }
                Esc::Node(Node::NamedRef(name))
            }
            '1'..='9' if !in_class => {
                let mut n = c.to_digit(10).unwrap_or(0) as usize;
                let save = self.pos;
                while let Some(d) = self.peek().and_then(|d| d.to_digit(10)) {
                    n = n * 10 + d as usize;
                    self.pos += 1;
                }
                if n <= self.total_caps {
                    Esc::Node(Node::BackRef(n))
                } else {
                    self.pos = save;
                    Esc::Char(c)
                }
            }
            other => Esc::Char(other),
        })
    }

    fn class(&mut self) -> PR<Node> {
        let neg = self.eat('^');
        let mut items = Vec::new();
        loop {
            let c = self.peek().ok_or("Unterminated character class")?;
            self.pos += 1;
            if c == ']' {
                break;
            }
            let first = if c == '\\' {
                match self.escape(true)? {
                    Esc::Char(ch) => Item1::Char(ch),
                    Esc::Item(i) => Item1::Set(i),
                    Esc::Node(_) => Item1::Char('b'),
                }
            } else {
                Item1::Char(c)
            };
            match first {
                Item1::Set(i) => items.push(i),
                Item1::Char(lo) => {
                    if self.peek() == Some('-') && self.p.get(self.pos + 1).is_some_and(|n| *n != ']') {
                        self.pos += 1;
                        let hc = self.peek().ok_or("bad range")?;
                        self.pos += 1;
                        let hi = if hc == '\\' {
                            match self.escape(true)? {
                                Esc::Char(ch) => Some(ch),
                                Esc::Item(i) => {
                                    // `[a-\d]`: treat as literal '-' and the set
                                    items.push(ClassItem::Range(lo, lo));
                                    items.push(ClassItem::Range('-', '-'));
                                    items.push(i);
                                    None
                                }
                                Esc::Node(_) => Some('b'),
                            }
                        } else {
                            Some(hc)
                        };
                        if let Some(hi) = hi {
                            if hi < lo {
                                return Err("Range out of order in character class".to_string());
                            }
                            items.push(ClassItem::Range(lo, hi));
                        }
                    } else {
                        items.push(ClassItem::Range(lo, lo));
                    }
                }
            }
        }
        Ok(Node::Class(Box::new(ClassSet { neg, items })))
    }
}

enum Esc {
    Char(char),
    Item(ClassItem),
    Node(Node),
}

enum Item1 {
    Char(char),
    Set(ClassItem),
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

struct Compiler {
    insts: Vec<Inst>,
    classes: Vec<ClassSet>,
    nreg: usize,
    names: Vec<(String, usize)>,
    budget: usize,
}

impl Compiler {
    fn emit(&mut self, i: Inst) -> usize {
        self.insts.push(i);
        self.insts.len() - 1
    }

    fn node(&mut self, n: &Node) -> PR<()> {
        if self.insts.len() > self.budget {
            return Err("Regular expression too large".to_string());
        }
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                self.emit(Inst::Char(*c));
            }
            Node::Any => {
                self.emit(Inst::Any);
            }
            Node::Class(c) => {
                self.classes.push((**c).clone());
                let idx = self.classes.len() - 1;
                self.emit(Inst::Class(idx));
            }
            Node::Start => {
                self.emit(Inst::Start);
            }
            Node::End => {
                self.emit(Inst::End);
            }
            Node::WordB(b) => {
                self.emit(Inst::WordB(*b));
            }
            Node::Group(inner, cap) => match cap {
                Some(i) => {
                    self.emit(Inst::Save(i * 2));
                    self.node(inner)?;
                    self.emit(Inst::Save(i * 2 + 1));
                }
                None => self.node(inner)?,
            },
            Node::Seq(items) => {
                for i in items {
                    self.node(i)?;
                }
            }
            Node::Alt(alts) => {
                let mut jumps = Vec::new();
                for (k, a) in alts.iter().enumerate() {
                    if k + 1 < alts.len() {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(a)?;
                        jumps.push(self.emit(Inst::Jmp(0)));
                        let next = self.insts.len();
                        self.insts[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(a)?;
                    }
                }
                let end = self.insts.len();
                for j in jumps {
                    self.insts[j] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node, min, max, greedy } => {
                if *min > 1000 || (*max != u32::MAX && *max > 1000) {
                    return Err("Regular expression repetition too large".to_string());
                }
                for _ in 0..*min {
                    self.node(node)?;
                }
                if *max == u32::MAX {
                    // star loop with an empty-iteration guard
                    let reg = self.nreg;
                    self.nreg += 1;
                    let head = self.emit(Inst::Split(0, 0));
                    self.emit(Inst::MarkPos(reg));
                    self.node(node)?;
                    let back = self.emit(Inst::LoopBack(reg, head, 0));
                    let exit = self.insts.len();
                    self.insts[head] = if *greedy { Inst::Split(head + 1, exit) } else { Inst::Split(exit, head + 1) };
                    self.insts[back] = Inst::LoopBack(reg, head, exit);
                } else {
                    let extra = max.saturating_sub(*min);
                    let mut splits = Vec::new();
                    for _ in 0..extra {
                        let s = self.emit(Inst::Split(0, 0));
                        splits.push(s);
                        self.node(node)?;
                    }
                    let end = self.insts.len();
                    for s in splits {
                        self.insts[s] = if *greedy { Inst::Split(s + 1, end) } else { Inst::Split(end, s + 1) };
                    }
                }
            }
            Node::BackRef(i) => {
                self.emit(Inst::BackRef(*i));
            }
            Node::NamedRef(name) => {
                let idx = self.names.iter().find(|(n, _)| n == name).map(|(_, i)| *i).ok_or("Invalid named reference")?;
                self.emit(Inst::BackRef(idx));
            }
            Node::Look { node, ahead, neg } => {
                let l = self.emit(Inst::Look { ahead: *ahead, neg: *neg, end: 0 });
                self.node(node)?;
                self.emit(Inst::Match);
                let end = self.insts.len();
                self.insts[l] = Inst::Look { ahead: *ahead, neg: *neg, end };
            }
        }
        Ok(())
    }
}

pub fn compile(pattern: &str, flags: &str) -> Result<Rc<Program>, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let unicode = flags.contains('u') || flags.contains('v');
    let total = count_groups(&chars);
    let mut p = Parser { p: &chars, pos: 0, ncap: 0, names: Vec::new(), unicode, total_caps: total };
    let root = p.alt()?;
    if p.pos < chars.len() {
        return Err("Unmatched ')'".to_string());
    }
    let mut c = Compiler { insts: Vec::new(), classes: Vec::new(), nreg: 0, names: p.names.clone(), budget: 200_000 };
    c.emit(Inst::Save(0));
    c.node(&root)?;
    c.emit(Inst::Save(1));
    c.emit(Inst::Match);
    Ok(Rc::new(Program {
        insts: c.insts,
        classes: c.classes,
        ncap: p.ncap + 1,
        nreg: c.nreg,
        names: p.names,
        icase: flags.contains('i'),
        multiline: flags.contains('m'),
        dot_all: flags.contains('s'),
        unicode,
    }))
}

// ---------------------------------------------------------------------------
// Matcher
// ---------------------------------------------------------------------------

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_space(c: char) -> bool {
    super::value::is_js_ws(c)
}

fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn prop_matches(name: &str, c: char) -> bool {
    let n = name.strip_prefix("General_Category=").or_else(|| name.strip_prefix("gc=")).unwrap_or(name);
    match n {
        "L" | "Letter" | "Alphabetic" | "Alpha" => c.is_alphabetic(),
        "Lu" | "Uppercase_Letter" | "Uppercase" => c.is_uppercase(),
        "Ll" | "Lowercase_Letter" | "Lowercase" => c.is_lowercase(),
        "N" | "Number" => c.is_numeric(),
        "Nd" | "Decimal_Number" | "digit" => c.is_ascii_digit() || (c.is_numeric() && c.to_digit(10).is_some()),
        "P" | "Punctuation" | "punct" => c.is_ascii_punctuation() && !matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~') || matches!(c as u32, 0x2010..=0x2027 | 0x3001..=0x3003),
        "S" | "Symbol" => matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~') || matches!(c as u32, 0x20a0..=0x20cf | 0x2100..=0x214f | 0x2190..=0x23ff | 0x2600..=0x27bf | 0x1f300..=0x1faff),
        "Z" | "Separator" | "Zs" | "White_Space" | "space" => c.is_whitespace(),
        "Emoji" | "Emoji_Presentation" | "Extended_Pictographic" => matches!(c as u32, 0x1f300..=0x1faff | 0x2600..=0x27bf | 0x1f000..=0x1f2ff | 0x2b00..=0x2bff | 0x2190..=0x21ff),
        "ASCII" => c.is_ascii(),
        "Any" => true,
        s if s.starts_with("Script=") || s.starts_with("sc=") || s.starts_with("Script_Extensions=") => {
            let script = s.split('=').nth(1).unwrap_or("");
            match script {
                "Latin" | "Latn" => c.is_alphabetic() && (c as u32) < 0x250 || matches!(c as u32, 0x1e00..=0x1eff),
                "Cyrillic" | "Cyrl" => matches!(c as u32, 0x400..=0x52f | 0x1c80..=0x1c8f | 0x2de0..=0x2dff | 0xa640..=0xa69f),
                "Greek" | "Grek" => matches!(c as u32, 0x370..=0x3ff | 0x1f00..=0x1fff),
                "Han" | "Hani" => matches!(c as u32, 0x4e00..=0x9fff | 0x3400..=0x4dbf | 0x20000..=0x2a6df | 0xf900..=0xfaff),
                "Hiragana" => matches!(c as u32, 0x3040..=0x309f),
                "Katakana" => matches!(c as u32, 0x30a0..=0x30ff),
                "Arabic" | "Arab" => matches!(c as u32, 0x600..=0x6ff | 0x750..=0x77f),
                "Hebrew" | "Hebr" => matches!(c as u32, 0x590..=0x5ff),
                "Hangul" | "Hang" => matches!(c as u32, 0xac00..=0xd7af | 0x1100..=0x11ff),
                _ => c.is_alphabetic(),
            }
        }
        _ => false,
    }
}

impl ClassSet {
    fn matches(&self, c: char, icase: bool) -> bool {
        let mut hit = false;
        for it in &self.items {
            let m = match it {
                ClassItem::Range(lo, hi) => {
                    (*lo <= c && c <= *hi)
                        || (icase && {
                            let f = fold(c);
                            let u = c.to_uppercase().next().unwrap_or(c);
                            (*lo <= f && f <= *hi) || (*lo <= u && u <= *hi)
                        })
                }
                ClassItem::Digit(p) => c.is_ascii_digit() == *p,
                ClassItem::Word(p) => is_word(c) == *p,
                ClassItem::Space(p) => is_space(c) == *p,
                ClassItem::Prop(n, p) => prop_matches(n, c) == *p,
            };
            if m {
                hit = true;
                break;
            }
        }
        hit != self.neg
    }
}

enum Bt {
    Branch(usize, usize),
    Cap(usize, usize),
    Reg(usize, usize),
}

const NONE: usize = usize::MAX;

pub struct Matcher<'a> {
    prog: &'a Program,
    input: &'a [char],
    caps: Vec<usize>,
    regs: Vec<usize>,
    steps: u64,
}

impl Matcher<'_> {
    fn run(&mut self, pc0: usize, pos0: usize, end_req: Option<usize>) -> Option<usize> {
        let mut stack: Vec<Bt> = Vec::new();
        let mut pc = pc0;
        let mut pos = pos0;
        let icase = self.prog.icase;
        loop {
            self.steps += 1;
            if self.steps > 20_000_000 {
                return None;
            }
            let ok = match &self.prog.insts[pc] {
                Inst::Char(c) => {
                    if pos < self.input.len() && (self.input[pos] == *c || (icase && fold(self.input[pos]) == fold(*c))) {
                        pos += 1;
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Any => {
                    if pos < self.input.len() && (self.prog.dot_all || !matches!(self.input[pos], '\n' | '\r' | '\u{2028}' | '\u{2029}')) {
                        pos += 1;
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Class(i) => {
                    if pos < self.input.len() && self.prog.classes[*i].matches(self.input[pos], icase) {
                        pos += 1;
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Start => {
                    if pos == 0 || (self.prog.multiline && matches!(self.input[pos - 1], '\n' | '\r' | '\u{2028}' | '\u{2029}')) {
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::End => {
                    if pos == self.input.len() || (self.prog.multiline && matches!(self.input[pos], '\n' | '\r' | '\u{2028}' | '\u{2029}')) {
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::WordB(want) => {
                    let a = pos > 0 && is_word(self.input[pos - 1]);
                    let b = pos < self.input.len() && is_word(self.input[pos]);
                    if (a != b) == *want {
                        pc += 1;
                        true
                    } else {
                        false
                    }
                }
                Inst::Save(n) => {
                    stack.push(Bt::Cap(*n, self.caps[*n]));
                    self.caps[*n] = pos;
                    pc += 1;
                    true
                }
                Inst::Split(a, b) => {
                    stack.push(Bt::Branch(*b, pos));
                    pc = *a;
                    true
                }
                Inst::Jmp(t) => {
                    pc = *t;
                    true
                }
                Inst::MarkPos(r) => {
                    stack.push(Bt::Reg(*r, self.regs[*r]));
                    self.regs[*r] = pos;
                    pc += 1;
                    true
                }
                Inst::LoopBack(r, head, exit) => {
                    if self.regs[*r] == pos {
                        pc = *exit;
                    } else {
                        pc = *head;
                    }
                    true
                }
                Inst::BackRef(i) => {
                    let (s, e) = (self.caps[i * 2], self.caps[i * 2 + 1]);
                    if s == NONE || e == NONE {
                        pc += 1;
                        true
                    } else {
                        let len = e - s;
                        if pos + len <= self.input.len() && (0..len).all(|k| self.input[s + k] == self.input[pos + k] || (icase && fold(self.input[s + k]) == fold(self.input[pos + k]))) {
                            pos += len;
                            pc += 1;
                            true
                        } else {
                            false
                        }
                    }
                }
                Inst::Look { ahead, neg, end } => {
                    let (ahead, neg, end) = (*ahead, *neg, *end);
                    let saved_caps = self.caps.clone();
                    let matched = if ahead {
                        self.run(pc + 1, pos, None).is_some()
                    } else {
                        let mut found = false;
                        for j in (0..=pos).rev() {
                            if self.run(pc + 1, j, Some(pos)).is_some() {
                                found = true;
                                break;
                            }
                        }
                        found
                    };
                    if matched == neg {
                        self.caps = saved_caps;
                        false
                    } else {
                        if neg {
                            self.caps = saved_caps;
                        } else {
                            // keep captures set inside the lookaround; make them restorable
                            for (i, old) in saved_caps.iter().enumerate() {
                                if self.caps[i] != *old {
                                    stack.push(Bt::Cap(i, *old));
                                }
                            }
                        }
                        pc = end;
                        true
                    }
                }
                Inst::Match => {
                    if let Some(e) = end_req {
                        if pos != e {
                            false
                        } else {
                            return Some(pos);
                        }
                    } else {
                        return Some(pos);
                    }
                }
            };
            if !ok {
                loop {
                    match stack.pop() {
                        None => return None,
                        Some(Bt::Branch(p, ps)) => {
                            pc = p;
                            pos = ps;
                            break;
                        }
                        Some(Bt::Cap(n, old)) => self.caps[n] = old,
                        Some(Bt::Reg(r, old)) => self.regs[r] = old,
                    }
                }
            }
        }
    }
}

/// Match at exactly `start`. Returns capture positions (char indices) on success.
pub fn match_at(prog: &Program, input: &[char], start: usize) -> Option<Vec<Option<(usize, usize)>>> {
    let mut m = Matcher { prog, input, caps: vec![NONE; prog.ncap * 2], regs: vec![NONE; prog.nreg], steps: 0 };
    m.run(0, start, None)?;
    let mut out = Vec::with_capacity(prog.ncap);
    for i in 0..prog.ncap {
        let (s, e) = (m.caps[i * 2], m.caps[i * 2 + 1]);
        out.push(if s != NONE && e != NONE && s <= e { Some((s, e)) } else { None });
    }
    Some(out)
}

/// Search forward from `start` (char index); `sticky` restricts to a match exactly at `start`.
pub fn search(prog: &Program, input: &[char], start: usize, sticky: bool) -> Option<Vec<Option<(usize, usize)>>> {
    let mut i = start;
    // quick literal-prefix scan
    let first = match prog.insts.get(1) {
        Some(Inst::Char(c)) if !prog.icase => Some(*c),
        _ => None,
    };
    while i <= input.len() {
        if let Some(f) = first {
            if !sticky {
                match input[i.min(input.len())..].iter().position(|c| *c == f) {
                    Some(off) => i += off,
                    None => return None,
                }
            }
        }
        if let Some(c) = match_at(prog, input, i) {
            return Some(c);
        }
        if sticky {
            return None;
        }
        i += 1;
    }
    None
}

impl Program {
    pub fn unicode(&self) -> bool {
        self.unicode
    }
}

pub fn make_data(source: &str, flags: &str) -> Result<RegexData, String> {
    for c in flags.chars() {
        if !"dgimsuvy".contains(c) || flags.matches(c).count() > 1 {
            return Err(format!("Invalid flags supplied to RegExp constructor '{flags}'"));
        }
    }
    let prog = compile(source, flags)?;
    Ok(RegexData { source: Rc::from(source), flags: Rc::from(flags), prog, global: flags.contains('g'), sticky: flags.contains('y') })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(p: &str, f: &str, s: &str) -> Option<(usize, usize)> {
        let prog = compile(p, f).unwrap();
        let chars: Vec<char> = s.chars().collect();
        search(&prog, &chars, 0, false).and_then(|c| c[0])
    }

    #[test]
    fn basics() {
        assert_eq!(find("a+b", "", "xaaab"), Some((1, 5)));
        assert_eq!(find("^\\d{3}-\\d{4}$", "", "555-1234"), Some((0, 8)));
        assert_eq!(find("(a|ab)(c|bcd)(d*)", "", "abcd"), Some((0, 4)));
        assert_eq!(find("a.*?b", "", "a1b2b"), Some((0, 3)));
        assert_eq!(find("HELLO", "i", "say hello"), Some((4, 9)));
        assert_eq!(find("(\\w)\\1", "", "abccd"), Some((2, 4)));
        assert_eq!(find("foo(?=bar)", "", "foobaz foobar"), Some((7, 10)));
        assert_eq!(find("(?<!x)y", "", "xy zy"), Some((4, 5)));
        assert_eq!(find("[^a-c]+", "", "abcxyz"), Some((3, 6)));
        assert_eq!(find("\\bfoo\\b", "", "a foo b"), Some((2, 5)));
        assert_eq!(find("(a*)*b", "", "aaab"), Some((0, 4)));
        assert_eq!(find("x{2,3}", "", "xxxx"), Some((0, 3)));
    }

    #[test]
    fn named_groups() {
        let prog = compile("(?<y>\\d{4})-(?<m>\\d\\d)", "").unwrap();
        assert_eq!(prog.names.len(), 2);
        let chars: Vec<char> = "on 2024-05".chars().collect();
        let c = search(&prog, &chars, 0, false).unwrap();
        assert_eq!(c[1], Some((3, 7)));
        assert_eq!(c[2], Some((8, 10)));
    }
}
