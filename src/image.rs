//! Image header probing: pixel dimensions without decoding (hosts decode and paint).
use alloc::{string::String, vec::Vec};

pub fn probe_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) && bytes.len() >= 24 {
        let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 10 {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some((w, h));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        return probe_jpeg(bytes);
    }
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return probe_webp(bytes);
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        let w = i32::from_le_bytes(bytes[18..22].try_into().ok()?);
        let h = i32::from_le_bytes(bytes[22..26].try_into().ok()?);
        return Some((w.unsigned_abs(), h.unsigned_abs()));
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() >= 8 {
        // ICO: use the first entry (0 means 256)
        let w = if bytes[6] == 0 { 256 } else { bytes[6] as u32 };
        let h = if bytes[7] == 0 { 256 } else { bytes[7] as u32 };
        return Some((w, h));
    }
    probe_svg(bytes)
}

fn probe_jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        if (0xd0..=0xd9).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

fn probe_webp(b: &[u8]) -> Option<(u32, u32)> {
    match &b[12..16] {
        b"VP8 " if b.len() >= 30 => {
            let w = (u16::from_le_bytes([b[26], b[27]]) & 0x3fff) as u32;
            let h = (u16::from_le_bytes([b[28], b[29]]) & 0x3fff) as u32;
            Some((w, h))
        }
        b"VP8L" if b.len() >= 25 => {
            let bits = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" if b.len() >= 30 => {
            let w = (b[24] as u32 | (b[25] as u32) << 8 | (b[26] as u32) << 16) + 1;
            let h = (b[27] as u32 | (b[28] as u32) << 8 | (b[29] as u32) << 16) + 1;
            Some((w, h))
        }
        _ => None,
    }
}

fn probe_svg(b: &[u8]) -> Option<(u32, u32)> {
    let text = core::str::from_utf8(&b[..b.len().min(4096)]).ok().or_else(|| {
        // truncated in the middle of a UTF-8 sequence
        let mut n = b.len().min(4096);
        while n > 0 && core::str::from_utf8(&b[..n]).is_err() {
            n -= 1;
        }
        core::str::from_utf8(&b[..n]).ok()
    })?;
    let start = text.find("<svg")?;
    let end = text[start..].find('>').map_or(text.len(), |e| start + e);
    let tag = &text[start..end];
    let attr = |name: &str| -> Option<&str> {
        let mut from = 0;
        while let Some(p) = tag[from..].find(name) {
            let at = from + p;
            let before = tag[..at].chars().next_back();
            let after = &tag[at + name.len()..];
            if before.is_some_and(|c| c.is_whitespace()) && after.trim_start().starts_with('=') {
                let rest = after.trim_start()[1..].trim_start();
                let q = rest.chars().next()?;
                if q == '"' || q == '\'' {
                    let inner = &rest[1..];
                    return inner.find(q).map(|e| &inner[..e]);
                }
            }
            from = at + name.len();
        }
        None
    };
    let num = |v: &str| -> Option<f32> {
        let v = v.trim();
        if v.ends_with('%') {
            return None;
        }
        let n: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        n.parse().ok()
    };
    let w = attr("width").and_then(num);
    let h = attr("height").and_then(num);
    if let (Some(w), Some(h)) = (w, h) {
        return Some((w as u32, h as u32));
    }
    let vb: Vec<f32> = attr("viewBox")?
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if vb.len() == 4 && vb[2] > 0.0 && vb[3] > 0.0 {
        return match (w, h) {
            (Some(w), None) => Some((w as u32, (w * vb[3] / vb[2]) as u32)),
            (None, Some(h)) => Some(((h * vb[2] / vb[3]) as u32, h as u32)),
            _ => Some((vb[2] as u32, vb[3] as u32)),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_gif_svg() {
        let mut png = alloc::vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&[0, 0, 0, 20, 0, 0, 0, 10]);
        assert_eq!(probe_size(&png), Some((20, 10)));
        let gif = b"GIF89a\x05\x00\x07\x00\x00\x00\x00";
        assert_eq!(probe_size(gif), Some((5, 7)));
        assert_eq!(probe_size(b"<svg xmlns='x' width=\"30\" height=\"40\"></svg>"), Some((30, 40)));
        assert_eq!(probe_size(b"<svg viewBox='0 0 100 50' width='200'>"), Some((200, 100)));
    }
}
