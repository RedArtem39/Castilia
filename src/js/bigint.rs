//! Arbitrary-precision integers for the `BigInt` type.
//!
//! Sign + magnitude with 32-bit limbs (little endian). Bitwise operators use two's complement
//! semantics on the fly, as the language requires.
use alloc::{string::String, vec, vec::Vec};
use core::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    pub neg: bool,
    /// Magnitude, least significant limb first, no trailing zero limbs (zero = empty).
    pub mag: Vec<u32>,
}

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u64;
    for i in 0..a.len().max(b.len()) {
        let s = *a.get(i).unwrap_or(&0) as u64 + *b.get(i).unwrap_or(&0) as u64 + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

/// a - b with |a| >= |b|
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let mut d = a[i] as i64 - *b.get(i).unwrap_or(&0) as i64 - borrow;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out.push(d as u32);
    }
    trim(&mut out);
    out
}

fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = out[i + j] as u64 + x as u64 * y as u64 + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        let mut k = i + b.len();
        while carry != 0 {
            let t = out[k] as u64 + carry;
            out[k] = t as u32;
            carry = t >> 32;
            k += 1;
        }
    }
    trim(&mut out);
    out
}

/// Divide by a small number; returns (quotient, remainder).
fn divrem_small(a: &[u32], d: u32) -> (Vec<u32>, u32) {
    let mut out = vec![0u32; a.len()];
    let mut rem = 0u64;
    for i in (0..a.len()).rev() {
        let cur = (rem << 32) | a[i] as u64;
        out[i] = (cur / d as u64) as u32;
        rem = cur % d as u64;
    }
    trim(&mut out);
    (out, rem as u32)
}

fn shl_mag(a: &[u32], n: usize) -> Vec<u32> {
    if a.is_empty() {
        return Vec::new();
    }
    let (limbs, bits) = (n / 32, n % 32);
    let mut out = vec![0u32; limbs];
    let mut carry = 0u32;
    for &x in a {
        if bits == 0 {
            out.push(x);
        } else {
            out.push((x << bits) | carry);
            carry = x >> (32 - bits);
        }
    }
    if carry != 0 {
        out.push(carry);
    }
    trim(&mut out);
    out
}

fn shr_mag(a: &[u32], n: usize) -> Vec<u32> {
    let (limbs, bits) = (n / 32, n % 32);
    if limbs >= a.len() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(a.len() - limbs);
    for i in limbs..a.len() {
        let lo = a[i] >> bits;
        let hi = if bits == 0 { 0 } else { a.get(i + 1).map_or(0, |x| x << (32 - bits)) };
        out.push(lo | hi);
    }
    trim(&mut out);
    out
}

fn bit_len(a: &[u32]) -> usize {
    match a.last() {
        None => 0,
        Some(&top) => (a.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
    }
}

/// Long division (binary shift-subtract per limb, adequate for the sizes pages use).
fn divrem_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if cmp_mag(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let (q, r) = divrem_small(a, b[0]);
        return (q, if r == 0 { Vec::new() } else { vec![r] });
    }
    let shift = bit_len(a) - bit_len(b);
    let mut rem = a.to_vec();
    let mut q = vec![0u32; shift / 32 + 1];
    let mut d = shl_mag(b, shift);
    for s in (0..=shift).rev() {
        if cmp_mag(&rem, &d) != Ordering::Less {
            rem = sub_mag(&rem, &d);
            q[s / 32] |= 1 << (s % 32);
        }
        d = shr_mag(&d, 1);
    }
    trim(&mut q);
    (q, rem)
}

impl BigInt {
    pub fn zero() -> BigInt {
        BigInt { neg: false, mag: Vec::new() }
    }

    fn new(neg: bool, mut mag: Vec<u32>) -> BigInt {
        trim(&mut mag);
        let neg = neg && !mag.is_empty();
        BigInt { neg, mag }
    }

    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub fn from_i64(v: i64) -> BigInt {
        let m = v.unsigned_abs();
        BigInt::new(v < 0, vec![m as u32, (m >> 32) as u32])
    }

    pub fn from_u64(v: u64) -> BigInt {
        BigInt::new(false, vec![v as u32, (v >> 32) as u32])
    }

    /// Exact conversion of an integral finite number.
    pub fn from_f64(x: f64) -> Option<BigInt> {
        if !x.is_finite() || crate::math::trunc(x) != x {
            return None;
        }
        let neg = x < 0.0;
        let bits = x.abs().to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i64;
        if exp == 0 {
            return Some(BigInt::zero());
        }
        let mant = (bits & ((1 << 52) - 1)) | (1 << 52);
        let e = exp - 1075;
        let m = BigInt::from_u64(mant);
        let mag = if e >= 0 { shl_mag(&m.mag, e as usize) } else { shr_mag(&m.mag, (-e) as usize) };
        Some(BigInt::new(neg, mag))
    }

    pub fn to_f64(&self) -> f64 {
        let mut x = 0.0f64;
        for &l in self.mag.iter().rev() {
            x = x * 4294967296.0 + l as f64;
        }
        if self.neg { -x } else { x }
    }

    /// Parse the digits of a literal or `BigInt("...")` (prefixes `0x`, `0o`, `0b` allowed).
    pub fn parse(s: &str) -> Option<BigInt> {
        let t = s.trim();
        let (neg, t) = match t.as_bytes().first() {
            Some(b'-') => (true, &t[1..]),
            Some(b'+') => (false, &t[1..]),
            _ => (false, t),
        };
        let (radix, digits) = if t.len() > 2 && t.as_bytes()[0] == b'0' {
            match t.as_bytes()[1] {
                b'x' | b'X' => (16, &t[2..]),
                b'o' | b'O' => (8, &t[2..]),
                b'b' | b'B' => (2, &t[2..]),
                _ => (10, t),
            }
        } else {
            (10, t)
        };
        if radix != 10 && neg {
            return None;
        }
        if digits.is_empty() {
            return if t.is_empty() { Some(BigInt::zero()) } else { None };
        }
        let mut mag: Vec<u32> = Vec::new();
        for c in digits.chars() {
            if c == '_' {
                continue;
            }
            let d = c.to_digit(radix)?;
            mag = mul_mag(&mag, &[radix]);
            mag = add_mag(&mag, &[d]);
            trim(&mut mag);
        }
        Some(BigInt::new(neg, mag))
    }

    pub fn to_string_radix(&self, radix: u32) -> String {
        if self.mag.is_empty() {
            return String::from("0");
        }
        let mut digits = Vec::new();
        let mut cur = self.mag.clone();
        while !cur.is_empty() {
            let (q, r) = divrem_small(&cur, radix);
            digits.push(core::char::from_digit(r, radix).unwrap_or('0'));
            cur = q;
        }
        let mut s = String::new();
        if self.neg {
            s.push('-');
        }
        s.extend(digits.iter().rev());
        s
    }

    pub fn neg(&self) -> BigInt {
        BigInt::new(!self.neg, self.mag.clone())
    }

    pub fn add(&self, o: &BigInt) -> BigInt {
        if self.neg == o.neg {
            return BigInt::new(self.neg, add_mag(&self.mag, &o.mag));
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Less => BigInt::new(o.neg, sub_mag(&o.mag, &self.mag)),
            _ => BigInt::new(self.neg, sub_mag(&self.mag, &o.mag)),
        }
    }

    pub fn sub(&self, o: &BigInt) -> BigInt {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &BigInt) -> BigInt {
        BigInt::new(self.neg != o.neg, mul_mag(&self.mag, &o.mag))
    }

    /// Truncating division and remainder (sign of the dividend); `None` on division by zero.
    pub fn divrem(&self, o: &BigInt) -> Option<(BigInt, BigInt)> {
        if o.is_zero() {
            return None;
        }
        let (q, r) = divrem_mag(&self.mag, &o.mag);
        Some((BigInt::new(self.neg != o.neg, q), BigInt::new(self.neg, r)))
    }

    pub fn pow(&self, e: &BigInt) -> Option<BigInt> {
        if e.neg {
            return None;
        }
        if e.mag.len() > 1 && !(self.mag.len() == 1 && self.mag[0] == 1) && !self.is_zero() {
            return None;
        }
        let mut n = e.mag.first().copied().unwrap_or(0);
        let mut base = self.clone();
        let mut acc = BigInt::from_i64(1);
        while n > 0 {
            if n & 1 == 1 {
                acc = acc.mul(&base);
            }
            n >>= 1;
            if n > 0 {
                base = base.mul(&base);
            }
            if bit_len(&acc.mag) > 1 << 24 {
                return None;
            }
        }
        Some(acc)
    }

    pub fn cmp(&self, o: &BigInt) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &o.mag),
            (true, true) => cmp_mag(&o.mag, &self.mag),
        }
    }

    /// Compare with a number (NaN → None).
    pub fn cmp_f64(&self, x: f64) -> Option<Ordering> {
        if x.is_nan() {
            return None;
        }
        if x == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if x == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let fl = crate::math::floor(x);
        let b = BigInt::from_f64(fl).unwrap_or_default();
        match self.cmp(&b) {
            Ordering::Equal if fl < x => Some(Ordering::Less),
            o => Some(o),
        }
    }

    pub fn shl(&self, n: i64) -> BigInt {
        if n < 0 {
            return self.shr(-n);
        }
        BigInt::new(self.neg, shl_mag(&self.mag, n as usize))
    }

    /// Arithmetic right shift (rounds toward negative infinity).
    pub fn shr(&self, n: i64) -> BigInt {
        if n < 0 {
            return self.shl(-n);
        }
        if !self.neg {
            return BigInt::new(false, shr_mag(&self.mag, n as usize));
        }
        // -((|x| - 1) >> n) - 1
        let m1 = sub_mag(&self.mag, &[1]);
        let s = shr_mag(&m1, n as usize);
        BigInt::new(true, add_mag(&s, &[1]))
    }

    /// Two's complement limbs of width `w` limbs.
    fn twos(&self, w: usize) -> Vec<u32> {
        let mut v = self.mag.clone();
        v.resize(w, 0);
        if self.neg {
            for x in v.iter_mut() {
                *x = !*x;
            }
            let mut carry = 1u64;
            for x in v.iter_mut() {
                let t = *x as u64 + carry;
                *x = t as u32;
                carry = t >> 32;
            }
        }
        v
    }

    fn from_twos(mut v: Vec<u32>) -> BigInt {
        let neg = v.last().is_some_and(|x| x & 0x8000_0000 != 0);
        if neg {
            for x in v.iter_mut() {
                *x = !*x;
            }
            let mut carry = 1u64;
            for x in v.iter_mut() {
                let t = *x as u64 + carry;
                *x = t as u32;
                carry = t >> 32;
            }
        }
        BigInt::new(neg, v)
    }

    pub fn bitop(&self, o: &BigInt, op: u8) -> BigInt {
        let w = self.mag.len().max(o.mag.len()) + 1;
        let (a, b) = (self.twos(w), o.twos(w));
        let v: Vec<u32> = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| match op {
                b'&' => x & y,
                b'|' => x | y,
                _ => x ^ y,
            })
            .collect();
        BigInt::from_twos(v)
    }

    pub fn not(&self) -> BigInt {
        // ~x = -x - 1
        self.neg().sub(&BigInt::from_i64(1))
    }

    /// `BigInt.asUintN(bits, x)`
    pub fn as_uint_n(&self, bits: usize) -> BigInt {
        let w = bits.div_ceil(32) + 1;
        let mut v = self.twos(w.max(self.mag.len() + 1));
        v.truncate(bits.div_ceil(32));
        if bits % 32 != 0 {
            if let Some(last) = v.last_mut() {
                *last &= (1u32 << (bits % 32)) - 1;
            }
        }
        BigInt::new(false, v)
    }

    /// `BigInt.asIntN(bits, x)`
    pub fn as_int_n(&self, bits: usize) -> BigInt {
        if bits == 0 {
            return BigInt::zero();
        }
        let u = self.as_uint_n(bits);
        if bit_len(&u.mag) == bits {
            // top bit set: subtract 2^bits
            u.sub(&BigInt::from_i64(1).shl(bits as i64))
        } else {
            u
        }
    }
}

impl Default for BigInt {
    fn default() -> Self {
        BigInt::zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = BigInt::parse("123456789012345678901234567890").unwrap();
        let b = BigInt::parse("987654321").unwrap();
        assert_eq!(a.mul(&b).to_string_radix(10), "121932631124828532112482853211126352690");
        let (q, r) = a.divrem(&b).unwrap();
        assert_eq!((q.to_string_radix(10).as_str(), r.to_string_radix(10).as_str()), ("124999998873437499901", "574845669"));
        assert_eq!(BigInt::from_i64(-7).divrem(&BigInt::from_i64(2)).map(|(q, r)| (q.to_string_radix(10), r.to_string_radix(10))), Some((String::from("-3"), String::from("-1"))));
        assert_eq!(BigInt::from_i64(2).pow(&BigInt::from_i64(64)).unwrap().to_string_radix(16), "10000000000000000");
        assert_eq!(BigInt::from_i64(-5).shr(1).to_string_radix(10), "-3");
        assert_eq!(BigInt::from_i64(-6).bitop(&BigInt::from_i64(3), b'&').to_string_radix(10), "2");
        assert_eq!(BigInt::from_i64(-1).as_uint_n(64).to_string_radix(10), "18446744073709551615");
        assert_eq!(BigInt::parse("18446744073709551615").unwrap().as_int_n(64).to_string_radix(10), "-1");
        assert_eq!(BigInt::from_f64(1e20).unwrap().to_string_radix(10), "100000000000000000000");
        assert_eq!(BigInt::parse("0xff").unwrap().to_string_radix(10), "255");
    }
}
