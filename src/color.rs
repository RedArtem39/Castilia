//! CSS colors. Colors are packed as `0xAARRGGBB`; alpha 0 is fully transparent.
use crate::math;
use alloc::vec::Vec;

pub type Color = u32;
pub const TRANSPARENT: Color = 0;
pub const BLACK: Color = 0xff00_0000;
pub const WHITE: Color = 0xffff_ffff;

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

pub fn alpha(c: Color) -> u8 {
    (c >> 24) as u8
}

pub fn rgb_part(c: Color) -> u32 {
    c & 0x00ff_ffff
}

/// Multiply the alpha channel by `factor` (0..=1).
pub fn with_opacity(c: Color, factor: f32) -> Color {
    if factor >= 1.0 {
        return c;
    }
    let a = (alpha(c) as f32 * factor.max(0.0) + 0.5) as u32;
    (a.min(255) << 24) | rgb_part(c)
}

/// Source-over blend of `top` onto `bottom` (both ARGB).
pub fn blend(top: Color, bottom: Color) -> Color {
    let ta = alpha(top) as f32 / 255.0;
    let ba = alpha(bottom) as f32 / 255.0;
    let oa = ta + ba * (1.0 - ta);
    if oa <= 0.0 {
        return 0;
    }
    let ch = |shift: u32| {
        let t = ((top >> shift) & 0xff) as f32;
        let b = ((bottom >> shift) & 0xff) as f32;
        ((t * ta + b * ba * (1.0 - ta)) / oa + 0.5) as u32
    };
    ((oa * 255.0 + 0.5) as u32) << 24 | ch(16).min(255) << 16 | ch(8).min(255) << 8 | ch(0).min(255)
}

fn hex_digit(b: u8) -> Option<u32> {
    (b as char).to_digit(16)
}

fn clamp_byte(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    math::round32(v.clamp(0.0, 255.0)) as u8
}

fn parse_alpha(token: &str) -> Option<u8> {
    let token = token.trim();
    let v = if let Some(p) = token.strip_suffix('%') {
        p.trim().parse::<f32>().ok()? / 100.0
    } else {
        token.parse::<f32>().ok()?
    };
    Some(clamp_byte(v * 255.0))
}

fn parse_channel(token: &str) -> Option<u8> {
    let token = token.trim();
    if let Some(p) = token.strip_suffix('%') {
        let v = p.trim().parse::<f32>().ok()?;
        Some(clamp_byte(v * 2.55))
    } else {
        Some(clamp_byte(token.parse::<f32>().ok()?))
    }
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

fn hsl(h: f32, s: f32, l: f32, a: u8) -> Color {
    let h = (h % 360.0 + 360.0) % 360.0 / 360.0;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (
            hue_to_rgb(p, q, h + 1.0 / 3.0),
            hue_to_rgb(p, q, h),
            hue_to_rgb(p, q, h - 1.0 / 3.0),
        )
    };
    rgba(clamp_byte(r * 255.0), clamp_byte(g * 255.0), clamp_byte(b * 255.0), a)
}

fn parse_hue(token: &str) -> Option<f32> {
    let t = token.trim();
    if let Some(v) = t.strip_suffix("deg") {
        v.trim().parse().ok()
    } else if let Some(v) = t.strip_suffix("grad") {
        v.trim().parse::<f32>().ok().map(|g| g * 0.9)
    } else if let Some(v) = t.strip_suffix("rad") {
        v.trim().parse::<f32>().ok().map(|r| r * 180.0 / core::f32::consts::PI)
    } else if let Some(v) = t.strip_suffix("turn") {
        v.trim().parse::<f32>().ok().map(|r| r * 360.0)
    } else {
        t.parse().ok()
    }
}

/// Split color function arguments on commas, spaces and `/`.
fn function_args(inner: &str) -> Vec<&str> {
    inner
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let b = hex.as_bytes();
        let d: Option<Vec<u32>> = b.iter().map(|&c| hex_digit(c)).collect();
        let d = d?;
        return match d.len() {
            3 => Some(rgba((d[0] * 17) as u8, (d[1] * 17) as u8, (d[2] * 17) as u8, 255)),
            4 => Some(rgba(
                (d[0] * 17) as u8,
                (d[1] * 17) as u8,
                (d[2] * 17) as u8,
                (d[3] * 17) as u8,
            )),
            6 => Some(rgba(
                (d[0] * 16 + d[1]) as u8,
                (d[2] * 16 + d[3]) as u8,
                (d[4] * 16 + d[5]) as u8,
                255,
            )),
            8 => Some(rgba(
                (d[0] * 16 + d[1]) as u8,
                (d[2] * 16 + d[3]) as u8,
                (d[4] * 16 + d[5]) as u8,
                (d[6] * 16 + d[7]) as u8,
            )),
            _ => None,
        };
    }
    let lower = value.to_ascii_lowercase();
    if let Some(open) = lower.find('(') {
        let name = lower[..open].trim();
        let inner = lower[open + 1..].strip_suffix(')')?;
        let args = function_args(inner);
        return match name {
            "rgb" | "rgba" => {
                if args.len() < 3 {
                    return None;
                }
                let a = if args.len() >= 4 { parse_alpha(args[3])? } else { 255 };
                Some(rgba(parse_channel(args[0])?, parse_channel(args[1])?, parse_channel(args[2])?, a))
            }
            "hsl" | "hsla" => {
                if args.len() < 3 {
                    return None;
                }
                let a = if args.len() >= 4 { parse_alpha(args[3])? } else { 255 };
                let s = args[1].trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                let l = args[2].trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                Some(hsl(parse_hue(args[0])?, s, l, a))
            }
            "hwb" => {
                if args.len() < 3 {
                    return None;
                }
                let w = args[1].trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                let bl = args[2].trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                let base = hsl(parse_hue(args[0])?, 1.0, 0.5, 255);
                let scale = 1.0 - w - bl;
                let mix = |shift: u32| {
                    let c = ((base >> shift) & 0xff) as f32 / 255.0;
                    clamp_byte((c * scale + w) * 255.0)
                };
                let a = if args.len() >= 4 { parse_alpha(args[3])? } else { 255 };
                Some(rgba(mix(16), mix(8), mix(0), a))
            }
            "color-mix" => parse_color_mix(inner),
            _ => None,
        };
    }
    named_color(&lower)
}

/// `color-mix(in srgb, A p%, B q%)` in sRGB space, which is the common usage.
fn parse_color_mix(inner: &str) -> Option<Color> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(inner[start..].trim());
    if parts.len() != 3 || !parts[0].starts_with("in ") {
        return None;
    }
    let split = |part: &str| -> Option<(Color, Option<f32>)> {
        let part = part.trim();
        if let Some(idx) = part.rfind(' ')
            && let Some(p) = part[idx..].trim().strip_suffix('%')
            && let Ok(p) = p.parse::<f32>()
        {
            return Some((parse_color(part[..idx].trim())?, Some(p / 100.0)));
        }
        Some((parse_color(part)?, None))
    };
    let (a, pa) = split(parts[1])?;
    let (b, pb) = split(parts[2])?;
    let (pa, pb) = match (pa, pb) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, 1.0 - x),
        (None, Some(y)) => (1.0 - y, y),
        (None, None) => (0.5, 0.5),
    };
    let total = pa + pb;
    if total <= 0.0 {
        return None;
    }
    let (pa, pb) = (pa / total, pb / total);
    let mix = |shift: u32| {
        let x = ((a >> shift) & 0xff) as f32;
        let y = ((b >> shift) & 0xff) as f32;
        clamp_byte(x * pa + y * pb)
    };
    Some(rgba(mix(16), mix(8), mix(0), mix(24)))
}

pub fn named_color(name: &str) -> Option<Color> {
    if name == "transparent" {
        return Some(TRANSPARENT);
    }
    NAMED
        .binary_search_by(|entry| entry.0.cmp(name))
        .ok()
        .map(|i| 0xff00_0000 | NAMED[i].1)
}

/// Sorted for binary search.
static NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_table_is_sorted() {
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_color("#fff"), Some(0xffffffff));
        assert_eq!(parse_color("#12345678"), Some(0x78123456));
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(0xffff0000));
        assert_eq!(parse_color("rgb(255 0 0 / 50%)"), Some(0x80ff0000));
        assert_eq!(parse_color("rgba(0,0,0,.5)"), Some(0x80000000));
        assert_eq!(parse_color("hsl(120, 100%, 25%)"), Some(0xff008000));
        assert_eq!(parse_color("RebeccaPurple"), Some(0xff663399));
        assert_eq!(parse_color("nonsense"), None);
    }
}
