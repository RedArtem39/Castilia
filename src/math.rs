//! Float helpers for `no_std` builds (the standard library's versions are not in `core`).

pub const PI: f64 = core::f64::consts::PI;

pub fn trunc(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4503599627370496.0 {
        return x;
    }
    let t = (x as i64) as f64;
    if t == 0.0 && x.is_sign_negative() { -0.0 } else { t }
}

pub fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if t > x { t - 1.0 } else { t }
}

pub fn ceil(x: f64) -> f64 {
    let t = trunc(x);
    if t < x { t + 1.0 } else { t }
}

/// JS `Math.round`: ties go toward +infinity.
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    floor(x + 0.5)
}

pub fn fract(x: f64) -> f64 {
    x - trunc(x)
}

pub fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    // Starting guess from the exponent bits, then Newton iterations.
    let mut g = f64::from_bits((x.to_bits() >> 1) + 0x1ff8_0000_0000_0000);
    for _ in 0..60 {
        let next = 0.5 * (g + x / g);
        if next == g {
            break;
        }
        g = next;
    }
    g
}

pub fn sqrt32(x: f32) -> f32 {
    sqrt(x as f64) as f32
}

pub fn floor32(x: f32) -> f32 {
    floor(x as f64) as f32
}

pub fn ceil32(x: f32) -> f32 {
    ceil(x as f64) as f32
}

pub fn round32(x: f32) -> f32 {
    if !x.is_finite() {
        return x;
    }
    floor((x + 0.5) as f64) as f32
}

pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    // x = m * 2^e with m in [0.5, 1)
    let bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7ff) as i64;
    let mut m;
    if e == 0 {
        // subnormal
        let y = x * 18014398509481984.0; // 2^54
        let b = y.to_bits();
        e = ((b >> 52) & 0x7ff) as i64 - 54;
        m = f64::from_bits((b & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000);
    } else {
        m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3fe0_0000_0000_0000);
    }
    e -= 1022;
    if m < core::f64::consts::FRAC_1_SQRT_2 {
        m *= 2.0;
        e -= 1;
    }
    let f = (m - 1.0) / (m + 1.0);
    let f2 = f * f;
    let mut term = f;
    let mut sum = 0.0;
    let mut k = 1.0;
    for _ in 0..40 {
        sum += term / k;
        term *= f2;
        k += 2.0;
    }
    2.0 * sum + (e as f64) * core::f64::consts::LN_2
}

pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.78 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    let k = round(x / core::f64::consts::LN_2);
    let r = x - k * core::f64::consts::LN_2;
    let mut term = 1.0;
    let mut sum = 1.0;
    for i in 1..30 {
        term *= r / i as f64;
        sum += term;
    }
    let mut result = sum;
    let mut n = k as i32;
    while n > 0 {
        let step = n.min(60);
        result *= (1u64 << step) as f64;
        n -= step;
    }
    while n < 0 {
        let step = (-n).min(60);
        result /= (1u64 << step) as f64;
        n += step;
    }
    result
}

pub fn powi(x: f64, mut n: i64) -> f64 {
    let neg = n < 0;
    if neg {
        n = -n;
    }
    let mut base = x;
    let mut result = 1.0;
    while n > 0 {
        if n & 1 == 1 {
            result *= base;
        }
        base *= base;
        n >>= 1;
    }
    if neg { 1.0 / result } else { result }
}

pub fn powf(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if y.is_finite() && y == trunc(y) && y.abs() < 1e9 {
        return powi(x, y as i64);
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    exp(y * ln(x))
}

fn reduce(x: f64) -> f64 {
    // reduce to [-pi, pi]
    let two_pi = 2.0 * PI;
    let mut r = x - two_pi * round(x / two_pi);
    if r > PI {
        r -= two_pi;
    } else if r < -PI {
        r += two_pi;
    }
    r
}

pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let r = reduce(x);
    let r2 = r * r;
    let mut term = r;
    let mut sum = r;
    for i in 1..14 {
        term *= -r2 / ((2 * i) as f64 * (2 * i + 1) as f64);
        sum += term;
    }
    sum
}

pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let r = reduce(x);
    let r2 = r * r;
    let mut term = 1.0;
    let mut sum = 1.0;
    for i in 1..14 {
        term *= -r2 / ((2 * i - 1) as f64 * (2 * i) as f64);
        sum += term;
    }
    sum
}

pub fn tan(x: f64) -> f64 {
    sin(x) / cos(x)
}

pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.is_infinite() {
        return if x > 0.0 { PI / 2.0 } else { -PI / 2.0 };
    }
    let neg = x < 0.0;
    let mut v = x.abs();
    let mut add = 0.0;
    let mut invert = false;
    if v > 1.0 {
        v = 1.0 / v;
        invert = true;
    }
    // further reduce: atan(v) = 2*atan(v / (1 + sqrt(1+v^2)))
    let mut scale = 1.0;
    for _ in 0..3 {
        v /= 1.0 + sqrt(1.0 + v * v);
        scale *= 2.0;
    }
    let v2 = v * v;
    let mut term = v;
    let mut sum = v;
    for i in 1..20 {
        term *= -v2;
        sum += term / (2 * i + 1) as f64;
    }
    let mut result = sum * scale;
    if invert {
        result = PI / 2.0 - result;
        add = 0.0;
    }
    let result = result + add;
    if neg { -result } else { result }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x > 0.0 {
        atan(y / x)
    } else if x < 0.0 {
        if y >= 0.0 { atan(y / x) + PI } else { atan(y / x) - PI }
    } else if y > 0.0 {
        PI / 2.0
    } else if y < 0.0 {
        -PI / 2.0
    } else {
        0.0
    }
}

pub fn asin(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    atan2(x, sqrt(1.0 - x * x))
}

pub fn acos(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    PI / 2.0 - asin(x)
}

pub fn sinh(x: f64) -> f64 {
    (exp(x) - exp(-x)) / 2.0
}
pub fn cosh(x: f64) -> f64 {
    (exp(x) + exp(-x)) / 2.0
}
pub fn tanh(x: f64) -> f64 {
    if x > 20.0 {
        return 1.0;
    }
    if x < -20.0 {
        return -1.0;
    }
    let a = exp(2.0 * x);
    (a - 1.0) / (a + 1.0)
}

pub fn log10(x: f64) -> f64 {
    ln(x) / core::f64::consts::LN_10
}
pub fn log2(x: f64) -> f64 {
    ln(x) / core::f64::consts::LN_2
}
pub fn cbrt(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let r = powf(x.abs(), 1.0 / 3.0);
    // one Newton step to polish
    let r = r - (r * r * r - x.abs()) / (3.0 * r * r);
    if x < 0.0 { -r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }
    #[test]
    fn basics() {
        assert_eq!(floor(-1.5), -2.0);
        assert_eq!(ceil(-1.5), -1.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert!(close(sqrt(2.0), 1.4142135623730951));
        assert!(close(exp(1.0), core::f64::consts::E));
        assert!(close(ln(10.0), core::f64::consts::LN_10));
        assert!(close(powf(2.0, 0.5), 1.4142135623730951));
        assert!(close(sin(1.0), 0.8414709848078965));
        assert!(close(cos(1.0), 0.5403023058681398));
        assert!(close(atan(1.0), PI / 4.0));
        assert!(close(atan2(1.0, -1.0), 3.0 * PI / 4.0));
        assert!(close(cbrt(27.0), 3.0));
    }
}
