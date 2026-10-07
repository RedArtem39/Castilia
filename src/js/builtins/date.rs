//! Date (the engine's local time zone is UTC).
use super::*;
use crate::math as m;
use alloc::format;

const MS_DAY: f64 = 86_400_000.0;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const LONG_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Broken-down time: (year, month0, day, hours, minutes, seconds, ms, weekday)
pub fn decompose(t: f64) -> (i64, i64, i64, i64, i64, i64, i64, i64) {
    let days = m::floor(t / MS_DAY) as i64;
    let ms = (t - days as f64 * MS_DAY) as i64;
    let (y, mo, d) = civil_from_days(days);
    let wd = (days + 4).rem_euclid(7);
    (y, mo - 1, d, ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000, wd)
}

pub fn make_time(y: f64, mo: f64, d: f64, h: f64, mi: f64, s: f64, ms: f64) -> f64 {
    if ![y, mo, d, h, mi, s, ms].iter().all(|v| v.is_finite()) {
        return f64::NAN;
    }
    let ym = y + m::floor(mo / 12.0);
    let mn = mo - m::floor(mo / 12.0) * 12.0;
    let days = days_from_civil(ym as i64, mn as i64 + 1, 1) as f64 + m::trunc(d) - 1.0;
    let t = days * MS_DAY + m::trunc(h) * 3_600_000.0 + m::trunc(mi) * 60_000.0 + m::trunc(s) * 1000.0 + m::trunc(ms);
    time_clip(t)
}

fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > 8.64e15 { f64::NAN } else { m::trunc(t) + 0.0 }
}

pub fn parse_date(s: &str) -> f64 {
    let s = s.trim();
    if let Some(t) = parse_iso(s) {
        return t;
    }
    parse_loose(s).unwrap_or(f64::NAN)
}

fn parse_iso(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let num = |i: &mut usize, n: usize| -> Option<f64> {
        if *i + n > b.len() || !b[*i..*i + n].iter().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: f64 = s[*i..*i + n].parse().ok()?;
        *i += n;
        Some(v)
    };
    let sign = if b.first() == Some(&b'+') || b.first() == Some(&b'-') {
        i = 1;
        if b[0] == b'-' { -1.0 } else { 1.0 }
    } else {
        1.0
    };
    let y = if sign != 1.0 || b.first() == Some(&b'+') { num(&mut i, 6)? * sign } else { num(&mut i, 4)? };
    let (mut mo, mut d) = (1.0, 1.0);
    if b.get(i) == Some(&b'-') {
        i += 1;
        mo = num(&mut i, 2)?;
        if b.get(i) == Some(&b'-') {
            i += 1;
            d = num(&mut i, 2)?;
        }
    }
    let (mut h, mut mi, mut sec, mut ms) = (0.0, 0.0, 0.0, 0.0);
    let mut offset = 0.0;
    let mut has_time = false;
    if i < b.len() && (b[i] == b'T' || b[i] == b't' || b[i] == b' ') {
        i += 1;
        has_time = true;
        h = num(&mut i, 2)?;
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        mi = num(&mut i, 2)?;
        if b.get(i) == Some(&b':') {
            i += 1;
            sec = num(&mut i, 2)?;
            if b.get(i) == Some(&b'.') || b.get(i) == Some(&b',') {
                i += 1;
                let st = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if st == i {
                    return None;
                }
                let frac: f64 = format!("0.{}", &s[st..i]).parse().ok()?;
                ms = m::floor(frac * 1000.0);
            }
        }
        if i < b.len() {
            match b[i] {
                b'Z' | b'z' => i += 1,
                b'+' | b'-' => {
                    let sg = if b[i] == b'-' { -1.0 } else { 1.0 };
                    i += 1;
                    let oh = num(&mut i, 2)?;
                    if b.get(i) == Some(&b':') {
                        i += 1;
                    }
                    let om = num(&mut i, 2).unwrap_or(0.0);
                    offset = sg * (oh * 60.0 + om) * 60_000.0;
                }
                _ => return None,
            }
        }
    }
    let _ = has_time;
    if i != b.len() || !(1.0..=12.0).contains(&mo) || !(1.0..=31.0).contains(&d) || h > 24.0 || mi > 59.0 || sec > 59.0 {
        return None;
    }
    Some(make_time(y, mo - 1.0, d, h, mi, sec, ms) - offset)
}

fn parse_loose(s: &str) -> Option<f64> {
    // e.g. "Tue, 03 Mar 2020 10:00:00 GMT", "March 7, 2021 10:00", "2020/03/04", "3/4/2020"
    let cleaned: String = s.replace(',', " ");
    let toks: Vec<&str> = cleaned.split_whitespace().collect();
    let (mut y, mut mo, mut d) = (None, None, None);
    let (mut h, mut mi, mut sec) = (0.0, 0.0, 0.0);
    let mut offset = 0.0;
    let mut pm = None;
    for t in toks {
        let tl = t.to_ascii_lowercase();
        if let Some(i) = MONTHS.iter().position(|mn| tl.starts_with(&mn.to_ascii_lowercase())) {
            mo = Some(i as f64);
        } else if DAYS.iter().any(|dn| tl.starts_with(&dn.to_ascii_lowercase())) {
        } else if tl == "am" || tl == "pm" {
            pm = Some(tl == "pm");
        } else if tl == "gmt" || tl == "utc" || tl == "z" {
        } else if (tl.starts_with("gmt+") || tl.starts_with("gmt-") || tl.starts_with('+') || tl.starts_with('-')) && tl.len() >= 5 {
            let t2 = tl.trim_start_matches("gmt");
            let sg = if t2.starts_with('-') { -1.0 } else { 1.0 };
            let digits: String = t2.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() == 4 {
                offset = sg * (digits[..2].parse::<f64>().ok()? * 60.0 + digits[2..].parse::<f64>().ok()?) * 60_000.0;
            }
        } else if tl.contains(':') {
            let parts: Vec<&str> = tl.split(':').collect();
            h = parts.first()?.parse().ok()?;
            mi = parts.get(1)?.parse().ok()?;
            if let Some(sp) = parts.get(2) {
                sec = sp.split('.').next()?.parse().ok()?;
            }
        } else if tl.contains('/') {
            let parts: Vec<f64> = tl.split('/').filter_map(|p| p.parse().ok()).collect();
            if parts.len() == 3 {
                if parts[0] > 31.0 {
                    y = Some(parts[0]);
                    mo = Some(parts[1] - 1.0);
                    d = Some(parts[2]);
                } else {
                    mo = Some(parts[0] - 1.0);
                    d = Some(parts[1]);
                    y = Some(parts[2]);
                }
            } else {
                return None;
            }
        } else if let Ok(n) = tl.trim_end_matches(|c: char| c.is_alphabetic()).parse::<f64>() {
            if tl.len() >= 3 && n > 31.0 || y.is_none() && d.is_some() {
                y = Some(n);
            } else if d.is_none() {
                d = Some(n);
            } else {
                y = Some(n);
            }
        } else {
            return None;
        }
    }
    if let Some(p) = pm {
        if p && h < 12.0 {
            h += 12.0;
        } else if !p && h == 12.0 {
            h = 0.0;
        }
    }
    let y = y?;
    let y = if y < 50.0 { y + 2000.0 } else if y < 100.0 { y + 1900.0 } else { y };
    Some(make_time(y, mo?, d.unwrap_or(1.0), h, mi, sec, 0.0) - offset)
}

pub fn setup(it: &mut Interp) {
    let proto = it.protos().date.clone();
    let ctor = it.make_ctor("Date", 7, date_ctor, &proto);
    it.def_method(&ctor, "now", 0, |it, _| Ok(Value::Num(now(it))));
    it.def_method(&ctor, "parse", 1, |it, c| Ok(Value::Num(parse_date(&it.arg_str(c, 0)?))));
    it.def_method(&ctor, "UTC", 7, |it, c| {
        let mut a = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        for (i, slot) in a.iter_mut().enumerate() {
            if i < c.args.len() {
                *slot = it.to_number(&c.args[i])?;
            }
        }
        if (0.0..=99.0).contains(&a[0]) {
            a[0] += 1900.0;
        }
        Ok(Value::Num(make_time(a[0], a[1], a[2], a[3], a[4], a[5], a[6])))
    });
    it.def_method(&proto, "getTime", 0, |it, c| Ok(Value::Num(this_time(it, c)?)));
    it.def_method(&proto, "valueOf", 0, |it, c| Ok(Value::Num(this_time(it, c)?)));
    it.def_method(&proto, "getTimezoneOffset", 0, |it, c| {
        let t = this_time(it, c)?;
        Ok(Value::Num(if t.is_nan() { f64::NAN } else { 0.0 }))
    });
    let getters: &[(&str, u8)] = &[("FullYear", 0), ("Month", 1), ("Date", 2), ("Hours", 3), ("Minutes", 4), ("Seconds", 5), ("Milliseconds", 6), ("Day", 7)];
    for (name, idx) in getters {
        let f: NativeFn = match idx {
            0 => |it, c| get_field(it, c, 0),
            1 => |it, c| get_field(it, c, 1),
            2 => |it, c| get_field(it, c, 2),
            3 => |it, c| get_field(it, c, 3),
            4 => |it, c| get_field(it, c, 4),
            5 => |it, c| get_field(it, c, 5),
            6 => |it, c| get_field(it, c, 6),
            _ => |it, c| get_field(it, c, 7),
        };
        it.def_method(&proto, &format!("get{name}"), 0, f);
        it.def_method(&proto, &format!("getUTC{name}"), 0, f);
    }
    it.def_method(&proto, "getYear", 0, |it, c| {
        let t = this_time(it, c)?;
        Ok(Value::Num(if t.is_nan() { t } else { decompose(t).0 as f64 - 1900.0 }))
    });
    let setters: &[(&str, u8)] = &[("FullYear", 0), ("Month", 1), ("Date", 2), ("Hours", 3), ("Minutes", 4), ("Seconds", 5), ("Milliseconds", 6)];
    for (name, idx) in setters {
        let f: NativeFn = match idx {
            0 => |it, c| set_field(it, c, 0),
            1 => |it, c| set_field(it, c, 1),
            2 => |it, c| set_field(it, c, 2),
            3 => |it, c| set_field(it, c, 3),
            4 => |it, c| set_field(it, c, 4),
            5 => |it, c| set_field(it, c, 5),
            _ => |it, c| set_field(it, c, 6),
        };
        it.def_method(&proto, &format!("set{name}"), 1, f);
        it.def_method(&proto, &format!("setUTC{name}"), 1, f);
    }
    it.def_method(&proto, "setTime", 1, |it, c| {
        let o = this_date(it, c)?;
        let t = time_clip(it.arg_num(c, 0)?);
        o.borrow_mut().class = Class::Date(t);
        Ok(Value::Num(t))
    });
    it.def_method(&proto, "toISOString", 0, |it, c| {
        let t = this_time(it, c)?;
        if t.is_nan() {
            return Err(it.error_ctl("RangeError", "Invalid time value"));
        }
        Ok(Value::string(iso(t)))
    });
    it.def_method(&proto, "toJSON", 1, |it, c| {
        let t = it.to_primitive(&c.this, "number")?;
        if matches!(t, Value::Num(n) if !n.is_finite()) {
            return Ok(Value::Null);
        }
        let f = it.get_str(&c.this, "toISOString")?;
        it.call(&f, c.this.clone(), &[])
    });
    it.def_method(&proto, "toString", 0, |it, c| fmt(it, c, 0));
    it.def_method(&proto, "toDateString", 0, |it, c| fmt(it, c, 1));
    it.def_method(&proto, "toTimeString", 0, |it, c| fmt(it, c, 2));
    it.def_method(&proto, "toUTCString", 0, |it, c| fmt(it, c, 3));
    it.def_method(&proto, "toGMTString", 0, |it, c| fmt(it, c, 3));
    it.def_method(&proto, "toLocaleDateString", 0, |it, c| fmt(it, c, 4));
    it.def_method(&proto, "toLocaleTimeString", 0, |it, c| fmt(it, c, 5));
    it.def_method(&proto, "toLocaleString", 0, |it, c| fmt(it, c, 6));
    let tp = it.wk().to_primitive.clone();
    it.def_sym_method(&proto, &tp, "[Symbol.toPrimitive]", 1, |it, c| {
        let hint = it.arg_str(c, 0)?;
        let o = this_date(it, c)?;
        let order = if &*hint == "number" { ["valueOf", "toString"] } else { ["toString", "valueOf"] };
        for name in order {
            let f = it.get_str(&Value::Obj(o.clone()), name)?;
            if f.as_obj().is_some_and(|x| x.is_callable()) {
                let r = it.call(&f, c.this.clone(), &[])?;
                if !matches!(r, Value::Obj(_)) {
                    return Ok(r);
                }
            }
        }
        it.type_error("Cannot convert object to primitive value")
    });
}

pub fn now(it: &Interp) -> f64 {
    m::floor(it.dom.epoch_ms + it.now)
}

fn date_ctor(it: &mut Interp, c: &Call) -> Res {
    if c.new_target.is_none() {
        let t = now(it);
        return Ok(Value::string(to_string(t, 0)));
    }
    let t = if c.args.is_empty() {
        now(it)
    } else if c.args.len() == 1 {
        let v = match &c.args[0] {
            Value::Obj(o) if matches!(o.borrow().class, Class::Date(_)) => {
                if let Class::Date(t) = o.borrow().class {
                    Value::Num(t)
                } else {
                    Value::Undefined
                }
            }
            v => it.to_primitive(v, "default")?,
        };
        match v {
            Value::Str(s) => parse_date(&s),
            v => time_clip(it.to_number(&v)?),
        }
    } else {
        let mut a = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        for (i, slot) in a.iter_mut().enumerate() {
            if i < c.args.len() {
                *slot = it.to_number(&c.args[i])?;
            }
        }
        if a[0].is_finite() && (0.0..=99.0).contains(&m::trunc(a[0])) {
            a[0] = m::trunc(a[0]) + 1900.0;
        }
        make_time(a[0], a[1], a[2], a[3], a[4], a[5], a[6])
    };
    let proto = it.protos().date.clone();
    let p = it.proto_from_call(c, &proto)?;
    Ok(Value::Obj(Obj::new(Object::new(Some(p), Class::Date(t)))))
}

fn this_date(it: &mut Interp, c: &Call) -> Res<Obj> {
    match &c.this {
        Value::Obj(o) if matches!(o.borrow().class, Class::Date(_)) => Ok(o.clone()),
        _ => it.type_error("this is not a Date object."),
    }
}

fn this_time(it: &mut Interp, c: &Call) -> Res<f64> {
    let o = this_date(it, c)?;
    let b = o.borrow();
    match b.class {
        Class::Date(t) => Ok(t),
        _ => Ok(f64::NAN),
    }
}

fn get_field(it: &mut Interp, c: &Call, idx: u8) -> Res {
    let t = this_time(it, c)?;
    if t.is_nan() {
        return Ok(Value::Num(f64::NAN));
    }
    let (y, mo, d, h, mi, s, ms, wd) = decompose(t);
    Ok(Value::Num([y, mo, d, h, mi, s, ms, wd][idx as usize] as f64))
}

fn set_field(it: &mut Interp, c: &Call, idx: u8) -> Res {
    let o = this_date(it, c)?;
    let t = this_time(it, c)?;
    let mut args = Vec::new();
    for a in c.args {
        args.push(it.to_number(a)?);
    }
    if args.is_empty() {
        args.push(f64::NAN);
    }
    let t = if t.is_nan() {
        if idx == 0 { 0.0 } else { return Ok(Value::Num(f64::NAN)) }
    } else {
        t
    };
    let (y, mo, d, h, mi, s, ms, _) = decompose(t);
    let mut f = [y as f64, mo as f64, d as f64, h as f64, mi as f64, s as f64, ms as f64];
    // each setter accepts several trailing fields
    let max = match idx {
        0 => 3,
        1 => 2,
        2 => 1,
        3 => 4,
        4 => 3,
        5 => 2,
        _ => 1,
    };
    for (k, v) in args.iter().take(max).enumerate() {
        f[idx as usize + k] = *v;
    }
    let nt = make_time(f[0], f[1], f[2], f[3], f[4], f[5], f[6]);
    o.borrow_mut().class = Class::Date(nt);
    Ok(Value::Num(nt))
}

pub fn iso(t: f64) -> String {
    let (y, mo, d, h, mi, s, ms, _) = decompose(t);
    let ys = if (0..=9999).contains(&y) { format!("{y:04}") } else { format!("{}{:06}", if y < 0 { '-' } else { '+' }, y.abs()) };
    format!("{ys}-{:02}-{:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z", mo + 1, d)
}

fn to_string(t: f64, mode: u8) -> String {
    if t.is_nan() {
        return "Invalid Date".into();
    }
    let (y, mo, d, h, mi, s, _, wd) = decompose(t);
    let date = format!("{} {} {d:02} {y:04}", DAYS[wd as usize], MONTHS[mo as usize]);
    let time = format!("{h:02}:{mi:02}:{s:02} GMT+0000 (Coordinated Universal Time)");
    match mode {
        0 => format!("{date} {time}"),
        1 => date,
        2 => time,
        3 => format!("{}, {d:02} {} {y:04} {h:02}:{mi:02}:{s:02} GMT", DAYS[wd as usize], MONTHS[mo as usize]),
        _ => String::new(),
    }
}

fn fmt(it: &mut Interp, c: &Call, mode: u8) -> Res {
    let t = this_time(it, c)?;
    if mode <= 3 {
        return Ok(Value::string(to_string(t, mode)));
    }
    if t.is_nan() {
        return Ok(Value::str("Invalid Date"));
    }
    let (y, mo, d, h, mi, s, _, wd) = decompose(t);
    let opts = c.arg(1);
    let mut date_s = format!("{}/{}/{}", mo + 1, d, y);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let mut time_s = format!("{h12}:{mi:02}:{s:02} {}", if h < 12 { "AM" } else { "PM" });
    if let Value::Obj(_) = &opts {
        let get = |it: &mut Interp, k: &str| -> Res<Option<String>> {
            let v = it.get_str(&opts, k)?;
            Ok(if v.is_undefined() { None } else { Some(it.to_str(&v)?.to_string()) })
        };
        let (wk, yr, mn, dy) = (get(it, "weekday")?, get(it, "year")?, get(it, "month")?, get(it, "day")?);
        if wk.is_some() || yr.is_some() || mn.is_some() || dy.is_some() {
            let mut parts = String::new();
            let month_s = match mn.as_deref() {
                Some("long") => Some(LONG_MONTHS[mo as usize].to_string()),
                Some("short") => Some(MONTHS[mo as usize].to_string()),
                Some("2-digit") => Some(format!("{:02}", mo + 1)),
                Some(_) => Some((mo + 1).to_string()),
                None => None,
            };
            let textual = matches!(mn.as_deref(), Some("long") | Some("short"));
            if let Some(w) = &wk {
                parts.push_str(if w == "long" { LONG_DAYS[wd as usize] } else { DAYS[wd as usize] });
                if textual || yr.is_some() || dy.is_some() {
                    parts.push_str(", ");
                }
            }
            let day_s = dy.as_ref().map(|d2| if d2 == "2-digit" { format!("{d:02}") } else { d.to_string() });
            let year_s = yr.as_ref().map(|y2| if y2 == "2-digit" { format!("{:02}", y % 100) } else { y.to_string() });
            if textual {
                if let Some(ms) = &month_s {
                    parts.push_str(ms);
                }
                if let Some(ds) = &day_s {
                    parts.push(' ');
                    parts.push_str(ds);
                }
                if let Some(ys) = &year_s {
                    parts.push_str(if day_s.is_some() { ", " } else { " " });
                    parts.push_str(ys);
                }
            } else {
                let mut seg = Vec::new();
                if let Some(ms) = month_s {
                    seg.push(ms);
                }
                if let Some(ds) = day_s {
                    seg.push(ds);
                }
                if let Some(ys) = year_s {
                    seg.push(ys);
                }
                parts.push_str(&seg.join("/"));
            }
            date_s = parts;
        }
        let (hr, mnu) = (get(it, "hour")?, get(it, "minute")?);
        if hr.is_some() || mnu.is_some() {
            let h12o = get(it, "hour12")?.map(|v| v == "true");
            let two = hr.as_deref() == Some("2-digit");
            let use12 = h12o.unwrap_or(true);
            let hh = if use12 { h12 } else { h };
            let hs = if two || !use12 { format!("{hh:02}") } else { hh.to_string() };
            time_s = format!("{hs}:{mi:02}");
            if get(it, "second")?.is_some() {
                time_s.push_str(&format!(":{s:02}"));
            }
            if use12 {
                time_s.push_str(if h < 12 { " AM" } else { " PM" });
            }
        }
    }
    Ok(Value::string(match mode {
        4 => date_s,
        5 => time_s,
        _ => format!("{date_s}, {time_s}"),
    }))
}
