//! Core value model of the JS engine: values, objects, property maps, conversions.
use super::ast::FuncDef;
use super::interp::{Env, Interp};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::{Ref, RefCell, RefMut};

pub type JsStr = Rc<str>;

/// A JS value. The layout is fixed (`tag: u8` at offset 0, payload at offset 8) because the
/// native tier reads and writes numbers and booleans in place; see [`Value::TAG_NUM`].
#[derive(Clone)]
#[repr(C, u8)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(JsStr),
    Obj(Obj),
    Sym(Sym),
    BigInt(Rc<super::bigint::BigInt>),
}

#[derive(Clone)]
pub struct Obj(pub Rc<RefCell<Object>>);

impl Obj {
    pub fn new(o: Object) -> Obj {
        Obj(Rc::new(RefCell::new(o)))
    }
    pub fn borrow(&self) -> Ref<'_, Object> {
        self.0.borrow()
    }
    pub fn borrow_mut(&self) -> RefMut<'_, Object> {
        self.0.borrow_mut()
    }
    pub fn ptr_eq(&self, other: &Obj) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    pub fn id(&self) -> usize {
        Rc::as_ptr(&self.0) as *const u8 as usize
    }
    pub fn is_callable(&self) -> bool {
        match &self.borrow().class {
            Class::Function(_) => true,
            Class::Proxy { target, .. } => target.is_callable(),
            _ => false,
        }
    }
    pub fn is_array(&self) -> bool {
        match &self.borrow().class {
            Class::Array(_) => true,
            Class::Proxy { target, .. } => target.is_array(),
            _ => false,
        }
    }
}

#[derive(Clone)]
pub struct Sym(pub Rc<SymData>);

pub struct SymData {
    pub id: u32,
    pub desc: Option<JsStr>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Key {
    S(JsStr),
    Y(u32),
}

impl Key {
    pub fn str(s: &str) -> Key {
        Key::S(Rc::from(s))
    }
    /// Array index if the key is a canonical non-negative integer.
    pub fn as_index(&self) -> Option<usize> {
        match self {
            Key::S(s) => str_index(s),
            _ => None,
        }
    }
}

pub fn str_index(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 10 || (b[0] == b'0' && b.len() > 1) || !b.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u64 = s.parse().ok()?;
    if n < 4294967295 { Some(n as usize) } else { None }
}

pub const WRITABLE: u8 = 1;
pub const ENUMERABLE: u8 = 2;
pub const CONFIGURABLE: u8 = 4;
pub const ALL: u8 = 7;
/// Writable + configurable but not enumerable (builtin methods).
pub const HIDDEN: u8 = 5;

#[derive(Clone)]
pub enum Slot {
    Data(Value),
    Accessor(Option<Obj>, Option<Obj>),
}

#[derive(Clone)]
pub struct Prop {
    pub slot: Slot,
    pub flags: u8,
}

/// Own properties in insertion order. Small maps (most objects) are searched linearly, comparing
/// key pointers first; a sorted index is built only once a map grows past `INDEX_AT` entries.
#[derive(Default)]
pub struct PropMap {
    list: Vec<Option<(Key, Prop)>>,
    index: Option<HashIndex>,
    live: usize,
}

const INDEX_AT: usize = 12;

#[inline]
fn key_eq(a: &Key, b: &Key) -> bool {
    match (a, b) {
        (Key::S(x), Key::S(y)) => Rc::ptr_eq(x, y) || x.len() == y.len() && **x == **y,
        (Key::Y(x), Key::Y(y)) => x == y,
        _ => false,
    }
}

#[inline]
fn key_hash(k: &Key) -> u64 {
    match k {
        Key::S(s) => {
            // FNV-1a
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for &b in s.as_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
            h
        }
        Key::Y(id) => (*id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5555,
    }
}

/// Open-addressing table of positions into `PropMap::list` (`0` = empty slot, else index + 1).
struct HashIndex {
    slots: Vec<u32>,
}

impl HashIndex {
    fn build(list: &[Option<(Key, Prop)>]) -> HashIndex {
        let mut cap = 16;
        while cap < list.len() * 2 + 2 {
            cap *= 2;
        }
        let mut ix = HashIndex { slots: alloc::vec![0; cap] };
        for (i, e) in list.iter().enumerate() {
            if let Some((k, _)) = e {
                ix.put(k, i);
            }
        }
        ix
    }
    fn put(&mut self, k: &Key, i: usize) {
        let mask = self.slots.len() - 1;
        let mut at = key_hash(k) as usize & mask;
        while self.slots[at] != 0 {
            at = (at + 1) & mask;
        }
        self.slots[at] = i as u32 + 1;
    }
    #[inline]
    fn find(&self, k: &Key, list: &[Option<(Key, Prop)>]) -> Option<usize> {
        let mask = self.slots.len() - 1;
        let mut at = key_hash(k) as usize & mask;
        loop {
            let v = self.slots[at];
            if v == 0 {
                return None;
            }
            let i = v as usize - 1;
            if matches!(&list[i], Some((key, _)) if key_eq(key, k)) {
                return Some(i);
            }
            at = (at + 1) & mask;
        }
    }
}

impl PropMap {
    #[inline]
    fn find(&self, k: &Key) -> Option<usize> {
        match &self.index {
            Some(ix) => ix.find(k, &self.list),
            None => self.list.iter().position(|e| matches!(e, Some((key, _)) if key_eq(key, k))),
        }
    }
    pub fn get(&self, k: &Key) -> Option<&Prop> {
        self.find(k).and_then(|i| self.list[i].as_ref().map(|e| &e.1))
    }
    pub fn get_mut(&mut self, k: &Key) -> Option<&mut Prop> {
        match self.find(k) {
            Some(i) => self.list[i].as_mut().map(|e| &mut e.1),
            None => None,
        }
    }
    pub fn insert(&mut self, k: Key, p: Prop) {
        if let Some(i) = self.find(&k) {
            if let Some(e) = self.list[i].as_mut() {
                e.1 = p;
            }
            return;
        }
        let i = self.list.len();
        self.list.push(Some((k, p)));
        self.live += 1;
        match self.index.as_mut() {
            Some(ix) if (self.list.len() + 1) * 2 <= ix.slots.len() => {
                let k = &self.list[i].as_ref().expect("just pushed").0;
                ix.put(k, i);
            }
            Some(_) => self.rebuild_index(),
            None if self.list.len() > INDEX_AT => self.rebuild_index(),
            None => {}
        }
    }
    fn rebuild_index(&mut self) {
        self.index = Some(HashIndex::build(&self.list));
    }
    pub fn remove(&mut self, k: &Key) -> bool {
        let Some(i) = self.find(k) else { return false };
        self.list[i] = None;
        self.live -= 1;
        if self.list.len() > 32 && self.live * 2 < self.list.len() {
            self.compact();
        } else if self.index.is_none() && self.list.len() > 2 * self.live + 4 {
            self.list.retain(|e| e.is_some());
        } else if self.index.is_some() {
            // tombstones are skipped by probing, but removed entries must not be found
            self.rebuild_index();
        }
        true
    }
    fn compact(&mut self) {
        self.list.retain(|e| e.is_some());
        if self.list.len() > INDEX_AT {
            self.rebuild_index();
        } else {
            self.index = None;
        }
    }
    pub fn len(&self) -> usize {
        self.live
    }
    /// Own properties in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&Key, &Prop)> {
        self.list.iter().flatten().map(|e| (&e.0, &e.1))
    }
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }
    /// Own keys in JS order: integer keys ascending, then strings, then symbols (insertion order).
    pub fn keys(&self) -> Vec<Key> {
        let mut ints: Vec<(usize, Key)> = Vec::new();
        let mut strs: Vec<Key> = Vec::new();
        let mut syms: Vec<Key> = Vec::new();
        for (k, _) in self.list.iter().flatten() {
            match k {
                Key::S(_) => match k.as_index() {
                    Some(i) => ints.push((i, k.clone())),
                    None => strs.push(k.clone()),
                },
                Key::Y(_) => syms.push(k.clone()),
            }
        }
        ints.sort_by_key(|x| x.0);
        let mut out: Vec<Key> = ints.into_iter().map(|x| x.1).collect();
        out.extend(strs);
        out.extend(syms);
        out
    }
}

pub struct Object {
    pub proto: Option<Obj>,
    pub props: PropMap,
    pub class: Class,
    pub extensible: bool,
}

impl Object {
    pub fn new(proto: Option<Obj>, class: Class) -> Object {
        Object { proto, props: PropMap::default(), class, extensible: true }
    }
}

pub struct Call<'a> {
    pub this: Value,
    pub args: &'a [Value],
    pub callee: &'a Obj,
    pub new_target: Option<Obj>,
}

impl Call<'_> {
    pub fn arg(&self, i: usize) -> Value {
        self.args.get(i).cloned().unwrap_or(Value::Undefined)
    }
}

pub type Res<T = Value> = Result<T, Ctl>;
pub type NativeFn = fn(&mut Interp, &Call) -> Res;

pub enum Ctl {
    Throw(Value),
    Break(Option<u32>),
    Continue(Option<u32>),
    Return(Value),
    /// Short-circuit of an optional chain (`a?.b`).
    OptShort,
    /// Internal: abort a generator that produced too many values.
    GenStop,
}

pub struct Closure {
    pub def: Rc<FuncDef>,
    pub env: Env,
    pub home: Option<Obj>,
    pub is_class_ctor: bool,
    pub derived: bool,
    pub fields: Rc<Vec<super::interp::FieldInit>>,
}

pub enum Func {
    Closure(Box<Closure>),
    Native { f: NativeFn, data: Value, ctor: bool },
    Bound { target: Obj, this: Value, args: Vec<Value> },
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MapKey {
    Undef,
    Null,
    Bool(bool),
    Num(u64),
    Str(JsStr),
    Obj(usize),
    Sym(u32),
    Big(String),
}

pub fn map_key(v: &Value) -> MapKey {
    match v {
        Value::Undefined => MapKey::Undef,
        Value::Null => MapKey::Null,
        Value::Bool(b) => MapKey::Bool(*b),
        Value::Num(n) => {
            let n = if *n == 0.0 { 0.0 } else { *n };
            MapKey::Num(if n.is_nan() { 0x7ff8_0000_0000_0000 } else { n.to_bits() })
        }
        Value::Str(s) => MapKey::Str(s.clone()),
        Value::Obj(o) => MapKey::Obj(o.id()),
        Value::Sym(s) => MapKey::Sym(s.0.id),
        Value::BigInt(b) => MapKey::Big(b.to_string_radix(16)),
    }
}

#[derive(Default)]
pub struct MapData {
    pub entries: Vec<Option<(Value, Value)>>,
    pub index: BTreeMap<MapKey, usize>,
    pub live: usize,
}

impl MapData {
    pub fn get(&self, k: &Value) -> Option<&Value> {
        self.index.get(&map_key(k)).and_then(|&i| self.entries[i].as_ref().map(|e| &e.1))
    }
    pub fn has(&self, k: &Value) -> bool {
        self.index.contains_key(&map_key(k))
    }
    pub fn set(&mut self, k: Value, v: Value) {
        let mk = map_key(&k);
        if let Some(&i) = self.index.get(&mk) {
            if let Some(e) = self.entries[i].as_mut() {
                e.1 = v;
            }
            return;
        }
        // normalise -0 to +0
        let k = match k {
            Value::Num(n) if n == 0.0 => Value::Num(0.0),
            k => k,
        };
        self.index.insert(mk, self.entries.len());
        self.entries.push(Some((k, v)));
        self.live += 1;
    }
    pub fn delete(&mut self, k: &Value) -> bool {
        if let Some(i) = self.index.remove(&map_key(k)) {
            self.entries[i] = None;
            self.live -= 1;
            true
        } else {
            false
        }
    }
    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            *e = None;
        }
        self.index.clear();
        self.live = 0;
    }
}

pub enum PromiseState {
    Pending,
    Fulfilled(Value),
    Rejected(Value),
}

pub struct Reaction {
    pub on_fulfilled: Option<Value>,
    pub on_rejected: Option<Value>,
    /// Derived promise to settle with the handler result (None for internal awaiters).
    pub result: Option<Obj>,
}

pub struct PromiseData {
    pub state: PromiseState,
    pub reactions: Vec<Reaction>,
    pub handled: bool,
}

pub struct GenData {
    pub values: Vec<Value>,
    pub pos: usize,
    pub started: bool,
    pub ret: Value,
    pub pending: Option<(Rc<FuncDef>, Env)>,
    /// Suspended bytecode frame (generators and async functions run on the VM).
    pub co: Option<Box<super::vm::CoState>>,
}

pub enum Class {
    Plain,
    Array(Vec<Value>),
    Function(Func),
    Error,
    Bool(bool),
    Num(f64),
    Str(JsStr),
    Symbol(Sym),
    Date(f64),
    RegExp(Box<super::regex::RegexData>),
    Map(Box<MapData>),
    Set(Box<MapData>),
    WeakMap(Box<MapData>),
    WeakSet(Box<MapData>),
    Promise(Box<PromiseData>),
    Generator(Box<GenData>),
    Proxy { target: Obj, handler: Obj },
    /// `Object(1n)` wrapper.
    BigInt(Rc<super::bigint::BigInt>),
    /// A DOM node wrapper.
    Node(usize),
    Window,
    Document,
    /// Arguments object (array-like).
    Arguments,
    /// Holder for a module's top-level environment (used by namespace getters).
    ModuleEnv(Env),
    /// Raw bytes of an `ArrayBuffer`.
    ArrayBuffer(super::builtins::binary::Bytes),
    /// Typed array view.
    Typed(Box<super::builtins::binary::View>),
    DataView(Box<super::builtins::binary::View>),
}

impl Value {
    pub const TAG_UNDEFINED: u8 = 0;
    pub const TAG_NULL: u8 = 1;
    pub const TAG_BOOL: u8 = 2;
    pub const TAG_NUM: u8 = 3;
    /// Tags above this own heap references (cloning/dropping needs Rust code).
    pub const TAG_LAST_PLAIN: u8 = 3;
    pub const PAYLOAD_OFFSET: usize = 8;
    pub const SIZE: usize = core::mem::size_of::<Value>();

    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }
    pub fn string(s: String) -> Value {
        Value::Str(Rc::from(s))
    }
    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }
    pub fn as_obj(&self) -> Option<&Obj> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::BigInt(b) => !b.is_zero(),
            _ => true,
        }
    }
    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Sym(_) => "symbol",
            Value::BigInt(_) => "bigint",
            Value::Obj(o) => {
                if o.is_callable() {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }
}

/// Marker stored in array storage for a missing element (`new Array(3)`, `[,,]`, `delete a[i]`).
/// It never escapes to scripts: every read path turns it into `undefined` or skips it.
pub fn hole() -> Value {
    Value::Sym(Sym(Rc::new(SymData { id: 0, desc: None })))
}

#[inline]
pub fn is_hole(v: &Value) -> bool {
    matches!(v, Value::Sym(s) if s.0.id == 0)
}

#[inline]
pub fn unhole(v: Value) -> Value {
    if is_hole(&v) { Value::Undefined } else { v }
}

pub fn unhole_vec(mut v: Vec<Value>) -> Vec<Value> {
    for x in v.iter_mut() {
        if is_hole(x) {
            *x = Value::Undefined;
        }
    }
    v
}

pub fn strict_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Obj(x), Value::Obj(y)) => x.ptr_eq(y),
        (Value::Sym(x), Value::Sym(y)) => x.0.id == y.0.id,
        (Value::BigInt(x), Value::BigInt(y)) => x == y,
        _ => false,
    }
}

pub fn same_value_zero(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => strict_equals(a, b),
    }
}

pub fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative()),
        _ => strict_equals(a, b),
    }
}

// ---------------------------------------------------------------------------
// Number <-> string
// ---------------------------------------------------------------------------

pub fn num_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() };
    }
    let neg = n < 0.0;
    let a = n.abs();
    // shortest round-trip digits via scientific formatting
    let sci = alloc::format!("{:e}", a);
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let nexp = exp + 1; // position of decimal point relative to digits start
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if k <= nexp && nexp <= 21 {
        s.push_str(&digits);
        for _ in 0..(nexp - k) {
            s.push('0');
        }
    } else if 0 < nexp && nexp <= 21 {
        s.push_str(&digits[..nexp as usize]);
        s.push('.');
        s.push_str(&digits[nexp as usize..]);
    } else if -6 < nexp && nexp <= 0 {
        s.push_str("0.");
        for _ in 0..(-nexp) {
            s.push('0');
        }
        s.push_str(&digits);
    } else {
        s.push_str(&digits[..1]);
        if k > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(if nexp - 1 >= 0 { '+' } else { '-' });
        s.push_str(&(nexp - 1).abs().to_string());
    }
    s
}

pub fn is_js_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

pub fn str_to_num(s: &str) -> f64 {
    let t = s.trim_matches(is_js_ws);
    if t.is_empty() {
        return 0.0;
    }
    let lower = t.as_bytes();
    if lower.len() > 2 && lower[0] == b'0' {
        let radix = match lower[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return match u64::from_str_radix(&t[2..], radix) {
                Ok(v) => v as f64,
                Err(_) => f64::NAN,
            };
        }
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if !t.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

pub fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    let t = crate::math::trunc(n);
    let m = t % 4294967296.0;
    let m = if m < 0.0 { m + 4294967296.0 } else { m };
    (m as u32) as i32
}

pub fn to_uint32(n: f64) -> u32 {
    to_int32(n) as u32
}

pub fn to_integer(n: f64) -> f64 {
    if n.is_nan() {
        0.0
    } else if n.is_infinite() {
        n
    } else {
        crate::math::trunc(n)
    }
}

// ---------------------------------------------------------------------------
// UTF-16 string helpers (JS strings are indexed by UTF-16 code units)
// ---------------------------------------------------------------------------

pub fn u16_len(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.encode_utf16().count() }
}

pub fn to_u16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn from_u16(v: &[u16]) -> String {
    char::decode_utf16(v.iter().copied()).map(|r| r.unwrap_or('\u{fffd}')).collect()
}

pub fn u16_slice(s: &str, start: usize, end: usize) -> String {
    if s.is_ascii() {
        let e = end.min(s.len());
        let st = start.min(e);
        return s[st..e].to_string();
    }
    let v = to_u16(s);
    let e = end.min(v.len());
    let st = start.min(e);
    from_u16(&v[st..e])
}

pub fn u16_at(s: &str, i: usize) -> Option<u16> {
    if s.is_ascii() {
        return s.as_bytes().get(i).map(|b| *b as u16);
    }
    s.encode_utf16().nth(i)
}

pub fn u16_index_of(hay: &str, needle: &str, from: usize) -> Option<usize> {
    if hay.is_ascii() && needle.is_ascii() {
        if from > hay.len() {
            return if needle.is_empty() { Some(hay.len()) } else { None };
        }
        return hay[from..].find(needle).map(|i| i + from);
    }
    let h = to_u16(hay);
    let n = to_u16(needle);
    if n.is_empty() {
        return Some(from.min(h.len()));
    }
    if n.len() > h.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()] == n[..])
}

pub fn u16_last_index_of(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = to_u16(hay);
    let n = to_u16(needle);
    if n.len() > h.len() {
        return None;
    }
    let start = from.min(h.len() - n.len());
    (0..=start).rev().find(|&i| h[i..i + n.len()] == n[..])
}
