//! Binary data: `ArrayBuffer`, typed arrays and `DataView` over shared byte storage.
//!
//! Element access (`ta[i]`) is handled by the object model (`own_prop`, `set_obj`); this module
//! provides construction, the accessors and the methods that need the raw bytes. The remaining
//! `%TypedArray%.prototype` methods are generic and live in the JS prelude.
use crate::js::{interp::Interp, value::*};
use alloc::{rc::Rc, string::String, vec, vec::Vec};
use core::cell::RefCell;

pub type Bytes = Rc<RefCell<Vec<u8>>>;

const MAX_BYTES: usize = 1 << 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaKind {
    I8,
    U8,
    U8C,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
    BI64,
    BU64,
}

pub const KINDS: [(&str, TaKind); 11] = [
    ("Int8Array", TaKind::I8),
    ("Uint8Array", TaKind::U8),
    ("Uint8ClampedArray", TaKind::U8C),
    ("Int16Array", TaKind::I16),
    ("Uint16Array", TaKind::U16),
    ("Int32Array", TaKind::I32),
    ("Uint32Array", TaKind::U32),
    ("Float32Array", TaKind::F32),
    ("Float64Array", TaKind::F64),
    ("BigInt64Array", TaKind::BI64),
    ("BigUint64Array", TaKind::BU64),
];

impl TaKind {
    pub fn size(self) -> usize {
        match self {
            TaKind::I8 | TaKind::U8 | TaKind::U8C => 1,
            TaKind::I16 | TaKind::U16 => 2,
            TaKind::I32 | TaKind::U32 | TaKind::F32 => 4,
            TaKind::F64 | TaKind::BI64 | TaKind::BU64 => 8,
        }
    }

    pub fn name(self) -> &'static str {
        KINDS.iter().find(|k| k.1 == self).map_or("Uint8Array", |k| k.0)
    }

    /// Read one element (little endian).
    pub fn read(self, b: &[u8]) -> f64 {
        match self {
            TaKind::I8 => b[0] as i8 as f64,
            TaKind::U8 | TaKind::U8C => b[0] as f64,
            TaKind::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            TaKind::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            TaKind::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            TaKind::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            TaKind::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            TaKind::F64 => f64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])),
            TaKind::BI64 => i64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])) as f64,
            TaKind::BU64 => u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])) as f64,
        }
    }

    /// Write one element (little endian) with the kind's conversion.
    pub fn write(self, b: &mut [u8], n: f64) {
        match self {
            TaKind::I8 | TaKind::U8 => b[0] = to_int32(n) as u8,
            TaKind::U8C => b[0] = if n.is_nan() { 0 } else { round_half_even(n.clamp(0.0, 255.0)) as u8 },
            TaKind::I16 | TaKind::U16 => b[..2].copy_from_slice(&(to_int32(n) as u16).to_le_bytes()),
            TaKind::I32 | TaKind::U32 => b[..4].copy_from_slice(&to_uint32(n).to_le_bytes()),
            TaKind::F32 => b[..4].copy_from_slice(&(n as f32).to_le_bytes()),
            TaKind::F64 => b[..8].copy_from_slice(&n.to_le_bytes()),
            TaKind::BI64 => b[..8].copy_from_slice(&(if n.is_finite() { n as i64 } else { 0 }).to_le_bytes()),
            TaKind::BU64 => b[..8].copy_from_slice(&(if n.is_finite() { n as i64 as u64 } else { 0 }).to_le_bytes()),
        }
    }
}

fn round_half_even(x: f64) -> f64 {
    let f = crate::math::floor(x);
    let d = x - f;
    if d < 0.5 {
        f
    } else if d > 0.5 {
        f + 1.0
    } else if f % 2.0 == 0.0 {
        f
    } else {
        f + 1.0
    }
}

/// A typed array or `DataView` over (part of) an `ArrayBuffer`.
pub struct View {
    pub bytes: Bytes,
    pub buffer: Obj,
    pub kind: TaKind,
    /// Byte offset into the buffer.
    pub offset: usize,
    /// Length in elements (bytes for a `DataView`).
    pub len: usize,
}

impl TaKind {
    pub fn is_big(self) -> bool {
        matches!(self, TaKind::BI64 | TaKind::BU64)
    }
}

/// Element value of a BigInt64/BigUint64 slot.
fn big_value(kind: TaKind, b: &[u8]) -> Value {
    let raw = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
    let big = if kind == TaKind::BI64 { crate::js::bigint::BigInt::from_i64(raw as i64) } else { crate::js::bigint::BigInt::from_u64(raw) };
    Value::BigInt(Rc::new(big))
}

impl View {
    /// Element as a JS value (BigInt for the 64-bit integer kinds).
    pub fn get_value(&self, i: usize) -> Option<Value> {
        if !self.kind.is_big() {
            return self.get(i).map(Value::Num);
        }
        if i >= self.len {
            return None;
        }
        let b = self.bytes.borrow();
        let at = self.offset + i * 8;
        b.get(at..at + 8).map(|x| big_value(self.kind, x))
    }

    /// Store raw 64 bits (BigInt kinds).
    pub fn set_bits(&self, i: usize, bits: u64) {
        if i >= self.len {
            return;
        }
        let mut b = self.bytes.borrow_mut();
        let at = self.offset + i * 8;
        if let Some(x) = b.get_mut(at..at + 8) {
            x.copy_from_slice(&bits.to_le_bytes());
        }
    }

    pub fn get(&self, i: usize) -> Option<f64> {
        if i >= self.len {
            return None;
        }
        let s = self.kind.size();
        let b = self.bytes.borrow();
        let at = self.offset + i * s;
        b.get(at..at + s).map(|x| self.kind.read(x))
    }

    pub fn set(&self, i: usize, n: f64) {
        if i >= self.len {
            return;
        }
        let s = self.kind.size();
        let mut b = self.bytes.borrow_mut();
        let at = self.offset + i * s;
        if let Some(x) = b.get_mut(at..at + s) {
            self.kind.write(x, n);
        }
    }
}

fn buffer_bytes(o: &Obj) -> Option<Bytes> {
    match &o.borrow().class {
        Class::ArrayBuffer(b) => Some(b.clone()),
        _ => None,
    }
}

impl Interp {
    pub fn new_array_buffer(&mut self, bytes: Vec<u8>) -> Obj {
        let proto = self.protos().extra.get("ArrayBuffer").cloned();
        self.alloc(Object::new(proto, Class::ArrayBuffer(Rc::new(RefCell::new(bytes)))))
    }

    /// A fresh `Uint8Array` (used by encoders, crypto, fetch).
    pub fn new_uint8_array(&mut self, bytes: Vec<u8>) -> Value {
        let len = bytes.len();
        let buf = self.new_array_buffer(bytes);
        Value::Obj(self.new_view(TaKind::U8, buf, 0, len, None))
    }

    fn new_view(&mut self, kind: TaKind, buffer: Obj, offset: usize, len: usize, proto: Option<Obj>) -> Obj {
        let bytes = buffer_bytes(&buffer).unwrap_or_default();
        let proto = proto.or_else(|| self.protos().extra.get(kind.name()).cloned());
        self.alloc(Object::new(proto, Class::Typed(alloc::boxed::Box::new(View { bytes, buffer, kind, offset, len }))))
    }

    /// Bytes of an ArrayBuffer or a view (copied), for APIs that accept `BufferSource`.
    pub fn buffer_source_bytes(&self, v: &Value) -> Option<Vec<u8>> {
        let Value::Obj(o) = v else { return None };
        match &o.borrow().class {
            Class::ArrayBuffer(b) => Some(b.borrow().clone()),
            Class::Typed(view) | Class::DataView(view) => {
                let s = view.kind.size();
                let n = if matches!(o.borrow().class, Class::DataView(_)) { view.len } else { view.len * s };
                let b = view.bytes.borrow();
                b.get(view.offset..view.offset + n).map(|x| x.to_vec())
            }
            _ => None,
        }
    }
}

fn this_view(it: &mut Interp, c: &Call) -> Res<(Bytes, Obj, TaKind, usize, usize)> {
    if let Value::Obj(o) = &c.this {
        if let Class::Typed(v) | Class::DataView(v) = &o.borrow().class {
            return Ok((v.bytes.clone(), v.buffer.clone(), v.kind, v.offset, v.len));
        }
    }
    Err(it.error_ctl("TypeError", "this is not a typed array"))
}

fn rel_index(it: &mut Interp, v: &Value, len: usize, default: usize) -> Res<usize> {
    if v.is_undefined() {
        return Ok(default);
    }
    let n = to_integer(it.to_number(v)?);
    Ok(if n < 0.0 { (len as f64 + n).max(0.0) as usize } else { (n as usize).min(len) })
}

fn kind_of_ctor(it: &mut Interp, c: &Call) -> TaKind {
    let name = match it.get_obj(c.callee, &Key::str("name"), &Value::Obj(c.callee.clone())) {
        Ok(Value::Str(s)) => s,
        _ => Rc::from("Uint8Array"),
    };
    KINDS.iter().find(|k| *k.0 == *name).map_or(TaKind::U8, |k| k.1)
}

fn typed_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_none() {
        return it.type_error("Constructor requires 'new'");
    }
    let kind = kind_of_ctor(it, c);
    let default_proto = it.protos().extra.get(kind.name()).cloned().expect("typed proto");
    let proto = it.proto_from_call(c, &default_proto)?;
    let size = kind.size();
    let a0 = c.arg(0);
    // (buffer, byteOffset, length)
    if let Value::Obj(o) = &a0 {
        if let Some(bytes) = buffer_bytes(o) {
            let total = bytes.borrow().len();
            let off = to_integer(it.to_number(&c.arg(1))?).max(0.0) as usize;
            if off % size != 0 {
                return Err(it.error_ctl("RangeError", "start offset of typed array should be a multiple of the element size"));
            }
            let len = if c.arg(2).is_undefined() {
                if (total.saturating_sub(off)) % size != 0 {
                    return Err(it.error_ctl("RangeError", "byte length of typed array should be a multiple of the element size"));
                }
                total.saturating_sub(off) / size
            } else {
                to_integer(it.to_number(&c.arg(2))?).max(0.0) as usize
            };
            if off + len * size > total {
                return Err(it.error_ctl("RangeError", "invalid typed array length"));
            }
            return Ok(Value::Obj(it.new_view(kind, o.clone(), off, len, Some(proto))));
        }
    }
    // (length) | (typed array / iterable / array-like)
    let values: Vec<f64> = match &a0 {
        Value::Obj(o) => {
            let src_view = match &o.borrow().class {
                Class::Typed(v) => Some((0..v.len).filter_map(|i| v.get(i)).collect::<Vec<f64>>()),
                _ => None,
            };
            match src_view {
                Some(v) => v,
                None => {
                    let has_iter = !it.get_v(&a0, &Key::Y(it.wk().iterator.0.id))?.is_nullish();
                    let items = if has_iter { it.iter_to_vec(&a0)? } else { it.array_like_to_vec(&a0)? };
                    let mut out = Vec::with_capacity(items.len());
                    for v in items {
                        out.push(it.to_number(&v)?);
                    }
                    out
                }
            }
        }
        Value::Undefined => Vec::new(),
        v => {
            let n = to_integer(it.to_number(v)?);
            if n < 0.0 || n * size as f64 > MAX_BYTES as f64 {
                return Err(it.error_ctl("RangeError", "Invalid typed array length"));
            }
            let buf = it.new_array_buffer(vec![0; n as usize * size]);
            return Ok(Value::Obj(it.new_view(kind, buf, 0, n as usize, Some(proto))));
        }
    };
    if values.len() * size > MAX_BYTES {
        return Err(it.error_ctl("RangeError", "Invalid typed array length"));
    }
    let mut bytes = vec![0u8; values.len() * size];
    for (i, n) in values.iter().enumerate() {
        kind.write(&mut bytes[i * size..], *n);
    }
    let len = values.len();
    let buf = it.new_array_buffer(bytes);
    Ok(Value::Obj(it.new_view(kind, buf, 0, len, Some(proto))))
}

fn data_view_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_none() {
        return it.type_error("Constructor DataView requires 'new'");
    }
    let Value::Obj(o) = c.arg(0) else { return it.type_error("First argument to DataView constructor must be an ArrayBuffer") };
    let Some(bytes) = buffer_bytes(&o) else { return it.type_error("First argument to DataView constructor must be an ArrayBuffer") };
    let total = bytes.borrow().len();
    let off = to_integer(it.to_number(&c.arg(1))?).max(0.0) as usize;
    let len = if c.arg(2).is_undefined() { total.saturating_sub(off) } else { to_integer(it.to_number(&c.arg(2))?).max(0.0) as usize };
    if off + len > total {
        return Err(it.error_ctl("RangeError", "Invalid DataView length"));
    }
    let default_proto = it.protos().extra.get("DataView").cloned().expect("dataview proto");
    let proto = it.proto_from_call(c, &default_proto)?;
    let v = View { bytes, buffer: o, kind: TaKind::U8, offset: off, len };
    Ok(Value::Obj(it.alloc(Object::new(Some(proto), Class::DataView(alloc::boxed::Box::new(v))))))
}

/// DataView get/set: `kind` element at byte `pos`, honouring `littleEndian`.
fn dv_access(it: &mut Interp, c: &Call, kind: TaKind, set: bool) -> Res {
    let (bytes, _, _, off, len) = this_view(it, c)?;
    let pos = to_integer(it.to_number(&c.arg(0))?);
    let size = kind.size();
    if pos < 0.0 || pos as usize + size > len {
        return Err(it.error_ctl("RangeError", "Offset is outside the bounds of the DataView"));
    }
    let at = off + pos as usize;
    let little = if set { c.arg(2).truthy() } else { c.arg(1).truthy() };
    if set {
        let mut tmp = [0u8; 8];
        if kind.is_big() {
            let b = crate::js::builtins::number::to_bigint_strict(it, &c.arg(1))?;
            tmp = b.to_u64_wrapping().to_le_bytes();
        } else {
            let n = it.to_number(&c.arg(1))?;
            kind.write(&mut tmp, n);
        }
        if !little {
            tmp[..size].reverse();
        }
        bytes.borrow_mut()[at..at + size].copy_from_slice(&tmp[..size]);
        Ok(Value::Undefined)
    } else {
        let mut tmp = [0u8; 8];
        tmp[..size].copy_from_slice(&bytes.borrow()[at..at + size]);
        if !little {
            tmp[..size].reverse();
        }
        if kind.is_big() {
            return Ok(big_value(kind, &tmp));
        }
        Ok(Value::Num(kind.read(&tmp)))
    }
}

pub fn setup(it: &mut Interp) {
    // ArrayBuffer
    let op = it.protos().object.clone();
    let abp = it.alloc(Object::new(Some(op.clone()), Class::Plain));
    let ab = it.make_ctor(
        "ArrayBuffer",
        1,
        |it, c| {
            if c.new_target.is_none() {
                return it.type_error("Constructor ArrayBuffer requires 'new'");
            }
            let n = to_integer(it.to_number(&c.arg(0))?);
            if !(0.0..=MAX_BYTES as f64).contains(&n) {
                return Err(it.error_ctl("RangeError", "Array buffer allocation failed"));
            }
            let default_proto = it.protos().extra.get("ArrayBuffer").cloned().expect("ab proto");
            let proto = it.proto_from_call(c, &default_proto)?;
            Ok(Value::Obj(it.alloc(Object::new(Some(proto), Class::ArrayBuffer(Rc::new(RefCell::new(vec![0; n as usize])))))))
        },
        &abp,
    );
    it.def_method(&ab, "isView", 1, |_, c| Ok(Value::Bool(matches!(c.arg(0), Value::Obj(o) if matches!(o.borrow().class, Class::Typed(_) | Class::DataView(_))))));
    it.def_getter(&abp, "byteLength", |_, c| {
        Ok(match &c.this {
            Value::Obj(o) => match &o.borrow().class {
                Class::ArrayBuffer(b) => Value::Num(b.borrow().len() as f64),
                _ => Value::Undefined,
            },
            _ => Value::Undefined,
        })
    });
    it.def_method(&abp, "slice", 2, |it, c| {
        let Some(bytes) = c.this.as_obj().and_then(buffer_bytes) else { return it.type_error("not an ArrayBuffer") };
        let len = bytes.borrow().len();
        let b = rel_index(it, &c.arg(0), len, 0)?;
        let e = rel_index(it, &c.arg(1), len, len)?;
        let out = if b < e { bytes.borrow()[b..e].to_vec() } else { Vec::new() };
        Ok(Value::Obj(it.new_array_buffer(out)))
    });
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&abp, Key::Y(tag.0.id), Value::str("ArrayBuffer"), CONFIGURABLE);
    it.protos.as_mut().expect("realm").extra.insert("ArrayBuffer", abp);

    // %TypedArray%
    let tap = it.alloc(Object::new(Some(op.clone()), Class::Plain));
    it.def_getter(&tap, "buffer", |it, c| Ok(Value::Obj(this_view(it, c)?.1)));
    it.def_getter(&tap, "byteOffset", |it, c| Ok(Value::Num(this_view(it, c)?.3 as f64)));
    it.def_getter(&tap, "length", |it, c| Ok(Value::Num(this_view(it, c)?.4 as f64)));
    it.def_getter(&tap, "byteLength", |it, c| {
        let (_, _, k, _, l) = this_view(it, c)?;
        Ok(Value::Num((l * k.size()) as f64))
    });
    it.def_method(&tap, "set", 1, |it, c| {
        let (bytes, _, kind, off, len) = this_view(it, c)?;
        let start = to_integer(it.to_number(&c.arg(1))?).max(0.0) as usize;
        let src = c.arg(0);
        let values: Vec<f64> = match &src {
            Value::Obj(o) if matches!(o.borrow().class, Class::Typed(_)) => match &o.borrow().class {
                Class::Typed(v) => (0..v.len).filter_map(|i| v.get(i)).collect(),
                _ => Vec::new(),
            },
            _ => {
                let items = it.array_like_to_vec(&src)?;
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    out.push(it.to_number(&v)?);
                }
                out
            }
        };
        if start + values.len() > len {
            return Err(it.error_ctl("RangeError", "offset is out of bounds"));
        }
        let size = kind.size();
        let mut b = bytes.borrow_mut();
        for (i, n) in values.into_iter().enumerate() {
            let at = off + (start + i) * size;
            kind.write(&mut b[at..at + size], n);
        }
        Ok(Value::Undefined)
    });
    it.def_method(&tap, "subarray", 2, |it, c| {
        let (_, buffer, kind, off, len) = this_view(it, c)?;
        let b = rel_index(it, &c.arg(0), len, 0)?;
        let e = rel_index(it, &c.arg(1), len, len)?.max(b);
        let proto = c.this.as_obj().and_then(|o| o.borrow().proto.clone());
        Ok(Value::Obj(it.new_view(kind, buffer, off + b * kind.size(), e - b, proto)))
    });
    it.def_method(&tap, "slice", 2, |it, c| {
        let (bytes, _, kind, off, len) = this_view(it, c)?;
        let b = rel_index(it, &c.arg(0), len, 0)?;
        let e = rel_index(it, &c.arg(1), len, len)?.max(b);
        let s = kind.size();
        let out = bytes.borrow()[off + b * s..off + e * s].to_vec();
        let buf = it.new_array_buffer(out);
        let proto = c.this.as_obj().and_then(|o| o.borrow().proto.clone());
        Ok(Value::Obj(it.new_view(kind, buf, 0, e - b, proto)))
    });
    it.def_method(&tap, "fill", 1, |it, c| {
        let (bytes, _, kind, off, len) = this_view(it, c)?;
        let n = it.to_number(&c.arg(0))?;
        let b = rel_index(it, &c.arg(1), len, 0)?;
        let e = rel_index(it, &c.arg(2), len, len)?;
        let s = kind.size();
        let mut buf = bytes.borrow_mut();
        for i in b..e {
            let at = off + i * s;
            kind.write(&mut buf[at..at + s], n);
        }
        Ok(c.this.clone())
    });
    // iteration and the generic algorithms reuse Array.prototype (they only need index + length)
    let ap = it.protos().array.clone();
    for name in ["values", "keys", "entries"] {
        let f = it.get_obj(&ap, &Key::str(name), &Value::Obj(ap.clone())).unwrap_or(Value::Undefined);
        it.define_data(&tap, Key::str(name), f.clone(), HIDDEN);
        if name == "values" {
            let sym = it.wk().iterator.clone();
            it.define_data(&tap, Key::Y(sym.0.id), f, HIDDEN);
        }
    }
    for name in [
        "forEach", "join", "indexOf", "lastIndexOf", "includes", "find", "findIndex", "findLast", "findLastIndex", "every", "some", "reduce", "reduceRight", "at", "toLocaleString",
    ] {
        let f = it.get_obj(&ap, &Key::str(name), &Value::Obj(ap.clone())).unwrap_or(Value::Undefined);
        it.define_data(&tap, Key::str(name), f, HIDDEN);
    }
    it.def_method(&tap, "toString", 0, |it, c| {
        let j = it.get_str(&c.this, "join")?;
        it.call(&j, c.this.clone(), &[])
    });
    let ta_tag = it.new_native("get [Symbol.toStringTag]", 0, |_, c| {
        Ok(match &c.this {
            Value::Obj(o) => match &o.borrow().class {
                Class::Typed(v) => Value::str(v.kind.name()),
                _ => Value::Undefined,
            },
            _ => Value::Undefined,
        })
    });
    it.define_accessor_part(&tap, Key::Y(tag.0.id), Some(ta_tag), true, CONFIGURABLE);
    it.protos.as_mut().expect("realm").extra.insert("TypedArray", tap.clone());

    for (name, kind) in KINDS {
        let proto = it.alloc(Object::new(Some(tap.clone()), Class::Plain));
        let size = Value::Num(kind.size() as f64);
        it.define_data(&proto, Key::str("BYTES_PER_ELEMENT"), size.clone(), 0);
        let ctor = it.make_ctor(name, 3, typed_ctor, &proto);
        it.define_data(&ctor, Key::str("BYTES_PER_ELEMENT"), size, 0);
        it.def_method(&ctor, "from", 1, |it, c| {
            let src = c.arg(0);
            let has_iter = !it.get_v(&src, &Key::Y(it.wk().iterator.0.id))?.is_nullish();
            let mut items = if has_iter { it.iter_to_vec(&src)? } else { it.array_like_to_vec(&src)? };
            if let Value::Obj(f) = c.arg(1) {
                for (i, v) in items.iter_mut().enumerate() {
                    *v = it.call(&Value::Obj(f.clone()), c.arg(2), &[v.clone(), Value::Num(i as f64)])?;
                }
            }
            let arr = it.new_array(items);
            it.construct(&c.this, &[arr], None)
        });
        it.def_method(&ctor, "of", 0, |it, c| {
            let arr = it.new_array(c.args.to_vec());
            it.construct(&c.this, &[arr], None)
        });
        it.protos.as_mut().expect("realm").extra.insert(name, proto);
    }

    // native UTF-8 bridges used by TextEncoder/TextDecoder in the prelude
    let g = it.global.clone();
    let enc = it.new_native("__utf8Encode", 1, |it, c| {
        let s = it.to_str(&c.arg(0))?;
        // JS strings may hold lone surrogates; Rust strings here are already valid UTF-8
        Ok(it.new_uint8_array(s.as_bytes().to_vec()))
    });
    it.define_data(&g, Key::str("__utf8Encode"), Value::Obj(enc), HIDDEN);
    let dec = it.new_native("__utf8Decode", 2, |it, c| {
        let Some(bytes) = it.buffer_source_bytes(&c.arg(0)) else { return Ok(Value::str("")) };
        let bytes = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) { &bytes[3..] } else { &bytes[..] };
        if c.arg(1).truthy() && core::str::from_utf8(bytes).is_err() {
            return it.type_error("The encoded data was not valid for encoding utf-8");
        }
        Ok(Value::string(String::from_utf8_lossy(bytes).into_owned()))
    });
    it.define_data(&g, Key::str("__utf8Decode"), Value::Obj(dec), HIDDEN);

    // DataView
    let dvp = it.alloc(Object::new(Some(op), Class::Plain));
    it.make_ctor("DataView", 1, data_view_ctor, &dvp);
    it.def_getter(&dvp, "buffer", |it, c| Ok(Value::Obj(this_view(it, c)?.1)));
    it.def_getter(&dvp, "byteOffset", |it, c| Ok(Value::Num(this_view(it, c)?.3 as f64)));
    it.def_getter(&dvp, "byteLength", |it, c| Ok(Value::Num(this_view(it, c)?.4 as f64)));
    macro_rules! dv {
        ($get:literal, $set:literal, $k:expr) => {
            it.def_method(&dvp, $get, 1, |it, c| dv_access(it, c, $k, false));
            it.def_method(&dvp, $set, 2, |it, c| dv_access(it, c, $k, true));
        };
    }
    dv!("getInt8", "setInt8", TaKind::I8);
    dv!("getUint8", "setUint8", TaKind::U8);
    dv!("getInt16", "setInt16", TaKind::I16);
    dv!("getUint16", "setUint16", TaKind::U16);
    dv!("getInt32", "setInt32", TaKind::I32);
    dv!("getUint32", "setUint32", TaKind::U32);
    dv!("getFloat32", "setFloat32", TaKind::F32);
    dv!("getFloat64", "setFloat64", TaKind::F64);
    dv!("getBigInt64", "setBigInt64", TaKind::BI64);
    dv!("getBigUint64", "setBigUint64", TaKind::BU64);
    it.define_data(&dvp, Key::Y(tag.0.id), Value::str("DataView"), CONFIGURABLE);
    it.protos.as_mut().expect("realm").extra.insert("DataView", dvp);
}

/// Text of a typed array element key for diagnostics.
pub fn describe_kind(k: TaKind) -> String {
    String::from(k.name())
}
