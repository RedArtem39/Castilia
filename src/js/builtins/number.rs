//! Number, Math, parseInt/parseFloat and URI coding.
use super::*;
use crate::math as m;
use alloc::format;

pub fn setup(it: &mut Interp) {
    let proto = it.protos().number.clone();
    let ctor = it.make_ctor("Number", 1, number_ctor, &proto);
    for (n, v) in [
        ("EPSILON", f64::EPSILON),
        ("MAX_SAFE_INTEGER", 9007199254740991.0),
        ("MIN_SAFE_INTEGER", -9007199254740991.0),
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", 5e-324),
        ("POSITIVE_INFINITY", f64::INFINITY),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("NaN", f64::NAN),
    ] {
        it.define_data(&ctor, Key::str(n), Value::Num(v), 0);
    }
    it.def_method(&ctor, "isInteger", 1, |_, c| Ok(Value::Bool(matches!(c.arg(0), Value::Num(n) if n.is_finite() && n == m::trunc(n)))));
    it.def_method(&ctor, "isSafeInteger", 1, |_, c| Ok(Value::Bool(matches!(c.arg(0), Value::Num(n) if n.is_finite() && n == m::trunc(n) && n.abs() <= 9007199254740991.0))));
    it.def_method(&ctor, "isFinite", 1, |_, c| Ok(Value::Bool(matches!(c.arg(0), Value::Num(n) if n.is_finite()))));
    it.def_method(&ctor, "isNaN", 1, |_, c| Ok(Value::Bool(matches!(c.arg(0), Value::Num(n) if n.is_nan()))));
    let pf = it.new_native("parseFloat", 1, parse_float);
    let pi = it.new_native("parseInt", 2, parse_int);
    let g = it.global.clone();
    for (n, f) in [("parseFloat", &pf), ("parseInt", &pi)] {
        it.define_data(&ctor, Key::str(n), Value::Obj(f.clone()), HIDDEN);
        it.define_data(&g, Key::str(n), Value::Obj(f.clone()), HIDDEN);
    }
    it.global_fn("isNaN", 1, |it, c| Ok(Value::Bool(it.arg_num(c, 0)?.is_nan())));
    it.global_fn("isFinite", 1, |it, c| Ok(Value::Bool(it.arg_num(c, 0)?.is_finite())));

    it.def_method(&proto, "toString", 1, to_string);
    it.def_method(&proto, "toLocaleString", 0, to_locale_string);
    it.def_method(&proto, "valueOf", 0, |it, c| Ok(Value::Num(this_num(it, c)?)));
    it.def_method(&proto, "toFixed", 1, to_fixed);
    it.def_method(&proto, "toPrecision", 1, to_precision);
    it.def_method(&proto, "toExponential", 1, to_exponential);

    setup_math(it);
    it.global_fn("encodeURIComponent", 1, |it, c| uri_encode(it, c, URI_UNRESERVED));
    it.global_fn("encodeURI", 1, |it, c| uri_encode(it, c, URI_RESERVED));
    it.global_fn("decodeURIComponent", 1, |it, c| uri_decode(it, c, ""));
    it.global_fn("decodeURI", 1, |it, c| uri_decode(it, c, ";/?:@&=+$,#"));
    it.global_fn("escape", 1, |it, c| uri_encode(it, c, "@*_+-./"));
    it.global_fn("unescape", 1, |it, c| uri_decode(it, c, ""));
}

fn number_ctor(it: &mut Interp, c: &Call) -> Res {
    let n = if c.args.is_empty() {
        0.0
    } else {
        // Number(1n) converts (ToNumeric), unlike arithmetic on BigInts
        match it.to_primitive(&c.args[0], "number")? {
            Value::BigInt(b) => b.to_f64(),
            p => it.to_number(&p)?,
        }
    };
    if c.new_target.is_none() {
        return Ok(Value::Num(n));
    }
    let proto = it.protos().number.clone();
    let p = it.proto_from_call(c, &proto)?;
    Ok(Value::Obj(it.alloc(Object::new(Some(p), Class::Num(n)))))
}

fn this_num(it: &mut Interp, c: &Call) -> Res<f64> {
    match &c.this {
        Value::Num(n) => Ok(*n),
        Value::Obj(o) => match &o.borrow().class {
            Class::Num(n) => Ok(*n),
            _ => Err(it.error_ctl("TypeError", "Number.prototype method requires that 'this' be a Number")),
        },
        _ => it.type_error("Number.prototype method requires that 'this' be a Number"),
    }
}

pub fn parse_float(it: &mut Interp, c: &Call) -> Res {
    let s = it.arg_str(c, 0)?;
    let t = s.trim_start_matches(is_js_ws);
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if t[i..].starts_with("Infinity") {
        return Ok(Value::Num(if b.first() == Some(&b'-') { f64::NEG_INFINITY } else { f64::INFINITY }));
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == ds || (i == ds + 1 && b[ds] == b'.') {
        return Ok(Value::Num(f64::NAN));
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let es = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > es {
            i = j;
        }
    }
    Ok(Value::Num(t[..i].parse::<f64>().unwrap_or(f64::NAN)))
}

pub fn parse_int(it: &mut Interp, c: &Call) -> Res {
    let s = it.arg_str(c, 0)?;
    let mut radix = to_int32(it.arg_num(c, 1)?);
    let mut t = s.trim_matches(is_js_ws);
    let mut neg = false;
    if let Some(r) = t.strip_prefix('-') {
        neg = true;
        t = r;
    } else if let Some(r) = t.strip_prefix('+') {
        t = r;
    }
    if radix == 0 {
        radix = 10;
        if t.starts_with("0x") || t.starts_with("0X") {
            radix = 16;
            t = &t[2..];
        }
    } else if radix == 16 && (t.starts_with("0x") || t.starts_with("0X")) {
        t = &t[2..];
    }
    if !(2..=36).contains(&radix) {
        return Ok(Value::Num(f64::NAN));
    }
    let mut v = 0f64;
    let mut any = false;
    for ch in t.chars() {
        match ch.to_digit(36) {
            Some(d) if (d as i32) < radix => {
                v = v * radix as f64 + d as f64;
                any = true;
            }
            _ => break,
        }
    }
    if !any {
        return Ok(Value::Num(f64::NAN));
    }
    Ok(Value::Num(if neg { -v } else { v }))
}

fn to_string(it: &mut Interp, c: &Call) -> Res {
    let n = this_num(it, c)?;
    let radix = match c.arg(0) {
        Value::Undefined => 10,
        v => to_integer(it.to_number(&v)?) as i32,
    };
    if !(2..=36).contains(&radix) {
        return Err(it.error_ctl("RangeError", "toString() radix must be between 2 and 36"));
    }
    if radix == 10 || !n.is_finite() {
        return Ok(Value::string(num_to_string(n)));
    }
    Ok(Value::string(radix_string(n, radix as u32)))
}

pub fn radix_string(n: f64, radix: u32) -> String {
    let neg = n < 0.0;
    let a = n.abs();
    let mut int = m::floor(a);
    let mut frac = a - int;
    let digit = |d: u32| core::char::from_digit(d, radix).unwrap();
    let mut ds = Vec::new();
    if int == 0.0 {
        ds.push('0');
    }
    while int > 0.0 {
        let d = (int % radix as f64) as u32;
        ds.push(digit(d));
        int = m::floor(int / radix as f64);
    }
    ds.reverse();
    let mut s: String = ds.into_iter().collect();
    if frac > 0.0 {
        s.push('.');
        let mut count = 0;
        while frac > 0.0 && count < 52 {
            frac *= radix as f64;
            let d = m::floor(frac);
            s.push(digit(d as u32));
            frac -= d;
            count += 1;
        }
    }
    if neg { format!("-{s}") } else { s }
}

fn round_decimal_string(x: f64, digits: usize) -> String {
    // exact decimal expansion rounded half up (as JS toFixed does)
    let full = format!("{:.*}", digits + 30, x);
    let (int, frac) = full.split_once('.').unwrap_or((&full, ""));
    let keep = &frac[..digits];
    let next = frac.as_bytes()[digits];
    let mut digs: Vec<u8> = int.bytes().chain(keep.bytes()).collect();
    if next >= b'5' {
        let mut i = digs.len();
        loop {
            if i == 0 {
                digs.insert(0, b'1');
                break;
            }
            i -= 1;
            if digs[i] == b'9' {
                digs[i] = b'0';
            } else {
                digs[i] += 1;
                break;
            }
        }
    }
    let int_len = digs.len() - digits;
    let mut s = String::from_utf8(digs[..int_len].to_vec()).unwrap();
    if digits > 0 {
        s.push('.');
        s.push_str(core::str::from_utf8(&digs[int_len..]).unwrap());
    }
    s
}

fn to_fixed(it: &mut Interp, c: &Call) -> Res {
    let n = this_num(it, c)?;
    let d = to_integer(it.arg_num(c, 0)?);
    if !(0.0..=100.0).contains(&d) {
        return Err(it.error_ctl("RangeError", "toFixed() digits argument must be between 0 and 100"));
    }
    if !n.is_finite() || n.abs() >= 1e21 {
        return Ok(Value::string(num_to_string(n)));
    }
    let s = round_decimal_string(n.abs(), d as usize);
    let neg = n < 0.0 && s.bytes().any(|b| b != b'0' && b != b'.');
    Ok(Value::string(if neg { format!("-{s}") } else { s }))
}

fn to_exponential(it: &mut Interp, c: &Call) -> Res {
    let n = this_num(it, c)?;
    if !n.is_finite() {
        return Ok(Value::string(num_to_string(n)));
    }
    let s = match c.arg(0) {
        Value::Undefined => format!("{:e}", n),
        v => {
            let d = to_integer(it.to_number(&v)?);
            if !(0.0..=100.0).contains(&d) {
                return Err(it.error_ctl("RangeError", "toExponential() argument must be between 0 and 100"));
            }
            format!("{:.*e}", d as usize, n)
        }
    };
    Ok(Value::string(fix_exp(&s)))
}

fn fix_exp(s: &str) -> String {
    match s.split_once('e') {
        Some((m, e)) => {
            let e: i32 = e.parse().unwrap_or(0);
            format!("{m}e{}{}", if e < 0 { '-' } else { '+' }, e.abs())
        }
        None => s.to_string(),
    }
}

fn to_precision(it: &mut Interp, c: &Call) -> Res {
    let n = this_num(it, c)?;
    if c.arg(0).is_undefined() || !n.is_finite() {
        return Ok(Value::string(num_to_string(n)));
    }
    let p = to_integer(it.arg_num(c, 0)?);
    if !(1.0..=100.0).contains(&p) {
        return Err(it.error_ctl("RangeError", "toPrecision() argument must be between 1 and 100"));
    }
    let p = p as usize;
    if n == 0.0 {
        return Ok(Value::string(if p > 1 { format!("0.{}", "0".repeat(p - 1)) } else { "0".to_string() }));
    }
    let sci = format!("{:.*e}", p - 1, n);
    let (mant, exp) = sci.split_once('e').unwrap();
    let e: i32 = exp.parse().unwrap();
    if e < -6 || e >= p as i32 {
        return Ok(Value::string(fix_exp(&sci)));
    }
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if e >= 0 {
        let ip = (e + 1) as usize;
        out.push_str(&digits[..ip]);
        if ip < digits.len() {
            out.push('.');
            out.push_str(&digits[ip..]);
        }
    } else {
        out.push_str("0.");
        for _ in 0..(-e - 1) {
            out.push('0');
        }
        out.push_str(&digits);
    }
    Ok(Value::string(out))
}

fn group_int(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn format_locale_number(n: f64, min_frac: usize, max_frac: usize, grouping: bool) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let s = round_decimal_string(n.abs(), max_frac);
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut frac = frac.to_string();
    while frac.len() > min_frac && frac.ends_with('0') {
        frac.pop();
    }
    let mut out = String::new();
    if n < 0.0 && s.bytes().any(|b| b != b'0' && b != b'.') {
        out.push('-');
    }
    out.push_str(&if grouping { group_int(int) } else { int.to_string() });
    if !frac.is_empty() {
        out.push('.');
        out.push_str(&frac);
    }
    out
}

fn to_locale_string(it: &mut Interp, c: &Call) -> Res {
    let n = this_num(it, c)?;
    let opts = c.arg(1);
    let (mut minf, mut maxf, mut grouping) = (0usize, 3usize, true);
    let mut style = String::new();
    let mut currency = String::new();
    if let Value::Obj(_) = &opts {
        let g = |it: &mut Interp, k: &str| it.get_str(&opts, k);
        let v = g(it, "minimumFractionDigits")?;
        if !v.is_undefined() {
            minf = it.to_number(&v)? as usize;
            maxf = maxf.max(minf);
        }
        let v = g(it, "maximumFractionDigits")?;
        if !v.is_undefined() {
            maxf = it.to_number(&v)? as usize;
            minf = minf.min(maxf);
        }
        let v = g(it, "useGrouping")?;
        if !v.is_undefined() {
            grouping = v.truthy();
        }
        let v = g(it, "style")?;
        if !v.is_undefined() {
            style = it.to_str(&v)?.to_string();
        }
        let v = g(it, "currency")?;
        if !v.is_undefined() {
            currency = it.to_str(&v)?.to_string();
        }
        if style == "currency" && g(it, "minimumFractionDigits")?.is_undefined() {
            minf = 2;
            maxf = maxf.max(2);
        }
        if style == "percent" {
            let s = format_locale_number(n * 100.0, minf, if g(it, "maximumFractionDigits")?.is_undefined() { 0 } else { maxf }, grouping);
            return Ok(Value::string(format!("{s}%")));
        }
    }
    let s = format_locale_number(n, minf, maxf, grouping);
    if style == "currency" {
        let sym = match currency.as_str() {
            "USD" => "$",
            "EUR" => "€",
            "GBP" => "£",
            "JPY" => "¥",
            _ => "",
        };
        let (neg, body) = s.strip_prefix('-').map_or(("", &s[..]), |b| ("-", b));
        return Ok(Value::string(if sym.is_empty() { format!("{neg}{currency} {body}") } else { format!("{neg}{sym}{body}") }));
    }
    Ok(Value::string(s))
}

// ---------------------------------------------------------------------------
// Math
// ---------------------------------------------------------------------------

fn setup_math(it: &mut Interp) {
    let math = it.new_object();
    let g = it.global.clone();
    it.define_data(&g, Key::str("Math"), Value::Obj(math.clone()), HIDDEN);
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&math, Key::Y(tag.0.id), Value::str("Math"), CONFIGURABLE);
    for (n, v) in [
        ("PI", core::f64::consts::PI),
        ("E", core::f64::consts::E),
        ("LN2", core::f64::consts::LN_2),
        ("LN10", core::f64::consts::LN_10),
        ("LOG2E", core::f64::consts::LOG2_E),
        ("LOG10E", core::f64::consts::LOG10_E),
        ("SQRT2", core::f64::consts::SQRT_2),
        ("SQRT1_2", core::f64::consts::FRAC_1_SQRT_2),
    ] {
        it.define_data(&math, Key::str(n), Value::Num(v), 0);
    }
    macro_rules! f1 {
        ($name:expr, $f:expr) => {
            it.def_method(&math, $name, 1, |it, c| {
                let x = it.arg_num(c, 0)?;
                let f: fn(f64) -> f64 = $f;
                Ok(Value::Num(f(x)))
            });
        };
    }
    f1!("abs", |x| x.abs());
    f1!("floor", m::floor);
    f1!("ceil", m::ceil);
    f1!("trunc", m::trunc);
    f1!("sqrt", |x| if x < 0.0 { f64::NAN } else { m::sqrt(x) });
    f1!("cbrt", m::cbrt);
    f1!("sin", m::sin);
    f1!("cos", m::cos);
    f1!("tan", m::tan);
    f1!("asin", |x| if x.abs() > 1.0 { f64::NAN } else { m::asin(x) });
    f1!("acos", |x| if x.abs() > 1.0 { f64::NAN } else { m::acos(x) });
    f1!("atan", m::atan);
    f1!("sinh", m::sinh);
    f1!("cosh", m::cosh);
    f1!("tanh", m::tanh);
    f1!("asinh", |x| if x.is_infinite() || x == 0.0 { x } else { m::ln(x + m::sqrt(x * x + 1.0)) });
    f1!("acosh", |x| if x < 1.0 { f64::NAN } else { m::ln(x + m::sqrt(x * x - 1.0)) });
    f1!("atanh", |x| if x.abs() > 1.0 { f64::NAN } else { 0.5 * m::ln((1.0 + x) / (1.0 - x)) });
    f1!("exp", m::exp);
    f1!("expm1", |x| if x.abs() < 1e-5 { x + x * x / 2.0 } else { m::exp(x) - 1.0 });
    f1!("log", |x| if x < 0.0 { f64::NAN } else { m::ln(x) });
    f1!("log1p", |x| if x < -1.0 { f64::NAN } else if x.abs() < 1e-5 { x - x * x / 2.0 } else { m::ln(1.0 + x) });
    f1!("log10", |x| if x < 0.0 { f64::NAN } else { m::log10(x) });
    f1!("log2", |x| if x < 0.0 { f64::NAN } else { m::log2(x) });
    f1!("sign", |x| if x.is_nan() || x == 0.0 { x } else if x > 0.0 { 1.0 } else { -1.0 });
    f1!("fround", |x| x as f32 as f64);
    f1!("round", |x| {
        if !x.is_finite() || x == 0.0 {
            x
        } else if x.abs() >= 4503599627370496.0 {
            x
        } else {
            let r = m::floor(x + 0.5);
            if r == 0.0 && x < 0.0 { -0.0 } else { r }
        }
    });
    it.def_method(&math, "clz32", 1, |it, c| Ok(Value::Num(to_uint32(it.arg_num(c, 0)?).leading_zeros() as f64)));
    it.def_method(&math, "imul", 2, |it, c| {
        let a = to_int32(it.arg_num(c, 0)?);
        let b = to_int32(it.arg_num(c, 1)?);
        Ok(Value::Num(a.wrapping_mul(b) as f64))
    });
    it.def_method(&math, "atan2", 2, |it, c| {
        let y = it.arg_num(c, 0)?;
        let x = it.arg_num(c, 1)?;
        Ok(Value::Num(m::atan2(y, x)))
    });
    it.def_method(&math, "pow", 2, |it, c| {
        let x = it.arg_num(c, 0)?;
        let y = it.arg_num(c, 1)?;
        Ok(Value::Num(js_pow(x, y)))
    });
    it.def_method(&math, "random", 0, |it, _| Ok(Value::Num(it.next_random())));
    it.def_method(&math, "hypot", 2, |it, c| {
        let mut s = 0.0;
        let mut inf = false;
        let mut nan = false;
        for a in c.args {
            let n = it.to_number(a)?;
            if n.is_infinite() {
                inf = true;
            }
            if n.is_nan() {
                nan = true;
            }
            s += n * n;
        }
        Ok(Value::Num(if inf { f64::INFINITY } else if nan { f64::NAN } else { m::sqrt(s) }))
    });
    it.def_method(&math, "max", 2, |it, c| {
        let mut r = f64::NEG_INFINITY;
        let mut nan = false;
        for a in c.args {
            let n = it.to_number(a)?;
            if n.is_nan() {
                nan = true;
            } else if n > r || (n == 0.0 && r == 0.0 && !n.is_sign_negative()) {
                r = n;
            }
        }
        Ok(Value::Num(if nan { f64::NAN } else { r }))
    });
    it.def_method(&math, "min", 2, |it, c| {
        let mut r = f64::INFINITY;
        let mut nan = false;
        for a in c.args {
            let n = it.to_number(a)?;
            if n.is_nan() {
                nan = true;
            } else if n < r || (n == 0.0 && r == 0.0 && n.is_sign_negative()) {
                r = n;
            }
        }
        Ok(Value::Num(if nan { f64::NAN } else { r }))
    });
}

pub fn js_pow(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x.abs() == 1.0 && y.is_infinite() {
        return f64::NAN;
    }
    if y == m::trunc(y) && y.abs() < 1e9 {
        return m::powi(x, y as i64);
    }
    if x < 0.0 && x.is_finite() {
        return f64::NAN;
    }
    m::powf(x, y)
}

// ---------------------------------------------------------------------------
// URI coding
// ---------------------------------------------------------------------------

const URI_UNRESERVED: &str = "-_.!~*'()";
const URI_RESERVED: &str = "-_.!~*'();/?:@&=+$,#";

fn uri_encode(it: &mut Interp, c: &Call, keep: &str) -> Res {
    let s = it.arg_str(c, 0)?;
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || keep.contains(ch) {
            out.push(ch);
        } else {
            let mut b = [0u8; 4];
            for byte in ch.encode_utf8(&mut b).bytes() {
                out.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    Ok(Value::string(out))
}

fn uri_decode(it: &mut Interp, c: &Call, reserved: &str) -> Res {
    let s = it.arg_str(c, 0)?;
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = core::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            match h {
                Some(v) => {
                    if v < 0x80 && reserved.contains(v as char) {
                        out.extend_from_slice(&b[i..i + 3]);
                    } else {
                        out.push(v);
                    }
                    i += 3;
                }
                None => return Err(it.error_ctl("URIError", "URI malformed")),
            }
        } else if b[i] == b'%' {
            return Err(it.error_ctl("URIError", "URI malformed"));
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(s) => Ok(Value::string(s)),
        Err(_) => Err(it.error_ctl("URIError", "URI malformed")),
    }
}

// ---------------------------------------------------------------------------
// BigInt
// ---------------------------------------------------------------------------

use crate::js::bigint::BigInt;

/// ToBigInt (typed arrays, DataView): unlike `BigInt()`, Numbers are rejected.
pub(crate) fn to_bigint_strict(it: &mut Interp, v: &Value) -> Res<BigInt> {
    let p = it.to_primitive(v, "number")?;
    if let Value::Num(n) = p {
        return Err(it.error_ctl("TypeError", &alloc::format!("Cannot convert {} to a BigInt", num_to_string(n))));
    }
    to_bigint(it, &p)
}

pub(crate) fn to_bigint(it: &mut Interp, v: &Value) -> Res<BigInt> {
    let p = it.to_primitive(v, "number")?;
    match &p {
        Value::BigInt(b) => Ok((**b).clone()),
        Value::Bool(b) => Ok(BigInt::from_i64(*b as i64)),
        Value::Str(s) => BigInt::parse(s).ok_or_else(|| it.error_ctl("SyntaxError", &alloc::format!("Cannot convert {s} to a BigInt"))),
        Value::Num(n) => BigInt::from_f64(*n).ok_or_else(|| it.error_ctl("RangeError", &alloc::format!("The number {} cannot be converted to a BigInt because it is not an integer", num_to_string(*n)))),
        other => {
            let d = it.describe(other);
            Err(it.error_ctl("TypeError", &alloc::format!("Cannot convert {d} to a BigInt")))
        }
    }
}

fn this_bigint(it: &mut Interp, c: &Call) -> Res<BigInt> {
    match &c.this {
        Value::BigInt(b) => Ok((**b).clone()),
        Value::Obj(o) => match &o.borrow().class {
            Class::BigInt(b) => Ok((**b).clone()),
            _ => Err(it.error_ctl("TypeError", "BigInt.prototype method called on incompatible receiver")),
        },
        _ => Err(it.error_ctl("TypeError", "BigInt.prototype method called on incompatible receiver")),
    }
}

pub fn setup_bigint(it: &mut Interp) {
    let op = it.protos().object.clone();
    let proto = it.alloc(Object::new(Some(op), Class::Plain));
    let ctor = it.make_ctor(
        "BigInt",
        1,
        |it, c| {
            if c.new_target.is_some() {
                return it.type_error("BigInt is not a constructor");
            }
            Ok(Value::BigInt(Rc::new(to_bigint(it, &c.arg(0))?)))
        },
        &proto,
    );
    it.def_method(&ctor, "asUintN", 2, |it, c| {
        let bits = it.to_number(&c.arg(0))?.clamp(0.0, 1e6) as usize;
        let b = to_bigint(it, &c.arg(1))?;
        Ok(Value::BigInt(Rc::new(b.as_uint_n(bits))))
    });
    it.def_method(&ctor, "asIntN", 2, |it, c| {
        let bits = it.to_number(&c.arg(0))?.clamp(0.0, 1e6) as usize;
        let b = to_bigint(it, &c.arg(1))?;
        Ok(Value::BigInt(Rc::new(b.as_int_n(bits))))
    });
    it.def_method(&proto, "toString", 0, |it, c| {
        let b = this_bigint(it, c)?;
        let radix = match c.arg(0) {
            Value::Undefined => 10,
            v => it.to_number(&v)? as u32,
        };
        if !(2..=36).contains(&radix) {
            return Err(it.error_ctl("RangeError", "toString() radix must be between 2 and 36"));
        }
        Ok(Value::string(b.to_string_radix(radix)))
    });
    it.def_method(&proto, "toLocaleString", 0, |it, c| Ok(Value::string(this_bigint(it, c)?.to_string_radix(10))));
    it.def_method(&proto, "valueOf", 0, |it, c| Ok(Value::BigInt(Rc::new(this_bigint(it, c)?))));
    let tag = it.wk().to_string_tag.clone();
    it.define_data(&proto, Key::Y(tag.0.id), Value::str("BigInt"), CONFIGURABLE);
    it.protos.as_mut().expect("realm").extra.insert("BigInt", proto);
}
