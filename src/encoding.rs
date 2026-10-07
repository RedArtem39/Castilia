//! Text decoding for network responses: BOM, `Content-Type` charset, `<meta charset>` sniffing and a
//! handful of single-byte legacy encodings. Everything else falls back to UTF-8 / windows-1252.
use alloc::string::{String, ToString};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charset {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1252,
    Windows1251,
    Koi8R,
    Iso8859_5,
    Iso8859_15,
}

/// Map a charset label (as written in HTTP headers or `<meta>`) to a supported charset.
pub fn charset_from_label(label: &str) -> Option<Charset> {
    let l = label.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase();
    Some(match l.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" | "x-unicode20utf8" => Charset::Utf8,
        "utf-16" | "utf-16le" | "ucs-2" | "unicode" | "csunicode" | "iso-10646-ucs-2" => Charset::Utf16Le,
        "utf-16be" | "unicodefffe" => Charset::Utf16Be,
        "iso-8859-1" | "latin1" | "l1" | "iso8859-1" | "windows-1252" | "cp1252" | "ascii" | "us-ascii" | "x-cp1252" | "iso-8859-1:1987" | "latin-1" => Charset::Windows1252,
        "windows-1251" | "cp1251" | "x-cp1251" => Charset::Windows1251,
        "koi8-r" | "koi8" | "koi8-u" | "cskoi8r" | "koi" => Charset::Koi8R,
        "iso-8859-5" | "cyrillic" | "iso8859-5" => Charset::Iso8859_5,
        "iso-8859-15" | "latin9" | "l9" | "iso8859-15" => Charset::Iso8859_15,
        _ => return None,
    })
}

/// Extract `charset=...` from a `Content-Type` header value.
pub fn charset_from_content_type(ct: &str) -> Option<Charset> {
    let lower = ct.to_ascii_lowercase();
    let i = lower.find("charset=")?;
    let rest = &ct[i + 8..];
    let end = rest.find([';', ' ']).unwrap_or(rest.len());
    charset_from_label(&rest[..end])
}

/// Look for `<meta charset=..>` / `<meta http-equiv=content-type content="..charset=..">` in the first 2KB.
pub fn sniff_meta_charset(bytes: &[u8]) -> Option<Charset> {
    let head = &bytes[..bytes.len().min(2048)];
    let text: String = head.iter().map(|&b| if b < 0x80 { (b as char).to_ascii_lowercase() } else { ' ' }).collect();
    let mut from = 0;
    while let Some(i) = text[from..].find("<meta") {
        let start = from + i;
        let end = text[start..].find('>').map_or(text.len(), |e| start + e);
        let tag = &text[start..end];
        if let Some(c) = tag.find("charset") {
            let rest = tag[c + 7..].trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start().trim_start_matches(|c| c == '"' || c == '\'');
                let e = rest.find(|c: char| matches!(c, '"' | '\'' | ' ' | ';' | '/' | '>')).unwrap_or(rest.len());
                if let Some(cs) = charset_from_label(&rest[..e]) {
                    // a meta tag cannot declare UTF-16 for an ASCII-compatible stream
                    return Some(if matches!(cs, Charset::Utf16Le | Charset::Utf16Be) { Charset::Utf8 } else { cs });
                }
            }
        }
        from = end;
    }
    None
}

const W1252: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122,
    0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

const W1251: [u16; 64] = [
    0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409, 0x2039, 0x040A, 0x040C, 0x040B, 0x040F, 0x0452, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x0098, 0x2122,
    0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F, 0x00A0, 0x040E, 0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9, 0x0404, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x0407, 0x00B0, 0x00B1, 0x0406,
    0x0456, 0x0491, 0x00B5, 0x00B6, 0x00B7, 0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457,
];

const KOI8_HIGH: [u16; 64] = [
    0x2500, 0x2502, 0x250C, 0x2510, 0x2514, 0x2518, 0x251C, 0x2524, 0x252C, 0x2534, 0x253C, 0x2580, 0x2584, 0x2588, 0x258C, 0x2590, 0x2591, 0x2592, 0x2593, 0x2320, 0x25A0, 0x2219, 0x221A, 0x2248, 0x2264, 0x2265,
    0x00A0, 0x2321, 0x00B0, 0x00B2, 0x00B7, 0x00F7, 0x2550, 0x2551, 0x2552, 0x0451, 0x2553, 0x2554, 0x2555, 0x2556, 0x2557, 0x2558, 0x2559, 0x255A, 0x255B, 0x255C, 0x255D, 0x255E, 0x255F, 0x2560, 0x2561,
    0x0401, 0x2562, 0x2563, 0x2564, 0x2565, 0x2566, 0x2567, 0x2568, 0x2569, 0x256A, 0x256B, 0x256C, 0x00A9,
];

const KOI8_LOWER: &str = "юабцдефгхийклмнопярстужвьызшэщчъ";
const KOI8_UPPER: &str = "ЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";

fn decode_single(bytes: &[u8], cs: Charset) -> String {
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 4);
    let koi_l: alloc::vec::Vec<char> = KOI8_LOWER.chars().collect();
    let koi_u: alloc::vec::Vec<char> = KOI8_UPPER.chars().collect();
    for &b in bytes {
        let c = if b < 0x80 {
            b as char
        } else {
            match cs {
                Charset::Windows1252 => {
                    if b < 0xA0 { char::from_u32(W1252[(b - 0x80) as usize] as u32).unwrap_or('\u{fffd}') } else { b as char }
                }
                Charset::Windows1251 => {
                    if b < 0xC0 { char::from_u32(W1251[(b - 0x80) as usize] as u32).unwrap_or('\u{fffd}') } else { char::from_u32(0x0410 + (b - 0xC0) as u32).unwrap_or('\u{fffd}') }
                }
                Charset::Koi8R => match b {
                    0x80..=0xBF => char::from_u32(KOI8_HIGH[(b - 0x80) as usize] as u32).unwrap_or('\u{fffd}'),
                    0xC0..=0xDF => koi_l[(b - 0xC0) as usize],
                    _ => koi_u[(b - 0xE0) as usize],
                },
                Charset::Iso8859_5 => match b {
                    0xA1..=0xAC => char::from_u32(0x0401 + (b - 0xA1) as u32).unwrap_or('\u{fffd}'),
                    0xAE..=0xAF => char::from_u32(0x040E + (b - 0xAE) as u32).unwrap_or('\u{fffd}'),
                    0xB0..=0xEF => char::from_u32(0x0410 + (b - 0xB0) as u32).unwrap_or('\u{fffd}'),
                    0xF0 => '№',
                    0xF1..=0xFC => char::from_u32(0x0451 + (b - 0xF1) as u32).unwrap_or('\u{fffd}'),
                    0xFD => '§',
                    0xFE..=0xFF => char::from_u32(0x045E + (b - 0xFE) as u32).unwrap_or('\u{fffd}'),
                    _ => b as char,
                },
                Charset::Iso8859_15 => match b {
                    0xA4 => '€',
                    0xA6 => 'Š',
                    0xA8 => 'š',
                    0xB4 => 'Ž',
                    0xB8 => 'ž',
                    0xBC => 'Œ',
                    0xBD => 'œ',
                    0xBE => 'Ÿ',
                    _ => b as char,
                },
                _ => b as char,
            }
        };
        out.push(c);
    }
    out
}

fn decode_utf16(bytes: &[u8], le: bool) -> String {
    let units = bytes.chunks_exact(2).map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) });
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
}

/// Decode `bytes` using (in order) a BOM, the transport hint, a `<meta>` declaration, then UTF-8 with a
/// windows-1252 fallback for invalid sequences.
pub fn decode(bytes: &[u8], transport: Option<Charset>) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).to_string();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(&bytes[2..], true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(&bytes[2..], false);
    }
    let cs = transport.or_else(|| sniff_meta_charset(bytes));
    match cs {
        Some(Charset::Utf8) => String::from_utf8_lossy(bytes).to_string(),
        Some(Charset::Utf16Le) => decode_utf16(bytes, true),
        Some(Charset::Utf16Be) => decode_utf16(bytes, false),
        Some(cs) => decode_single(bytes, cs),
        None => match core::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => decode_single(bytes, Charset::Windows1252),
        },
    }
}

/// Decode a stylesheet or script: BOM / transport charset, else UTF-8 (lossy fallback to windows-1252).
pub fn decode_resource(bytes: &[u8], transport: Option<Charset>) -> String {
    if let Some(cs) = transport {
        if cs != Charset::Utf8 {
            return decode(bytes, Some(cs));
        }
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    // `@charset "windows-1251";` at the very start of a stylesheet
    if let Some(rest) = b.strip_prefix(b"@charset \"") {
        if let Some(end) = rest.iter().position(|&c| c == b'"') {
            if let Ok(label) = core::str::from_utf8(&rest[..end]) {
                if let Some(cs) = charset_from_label(label) {
                    return decode(bytes, Some(cs));
                }
            }
        }
    }
    match core::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => decode_single(b, Charset::Windows1252),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_legacy_encodings() {
        assert_eq!(decode(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2], Some(Charset::Windows1251)), "Привет");
        assert_eq!(decode(&[0xF0, 0xD2, 0xC9, 0xD7, 0xC5, 0xD4], Some(Charset::Koi8R)), "Привет");
        assert_eq!(decode(b"<meta charset=windows-1251><p>\xCF\xF0", None), "<meta charset=windows-1251><p>Пр");
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, b'i', 0], None), "hi");
        assert_eq!(decode(&[0x80, 0x93], None), "€“");
        assert_eq!(decode("héllo".as_bytes(), None), "héllo");
        assert_eq!(charset_from_content_type("text/html; charset=UTF-8"), Some(Charset::Utf8));
        assert_eq!(decode_resource(b"@charset \"windows-1251\"; a{content:\"\xCF\"}", None), "@charset \"windows-1251\"; a{content:\"П\"}");
    }
}
