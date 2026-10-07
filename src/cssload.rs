//! Stylesheet post-processing for loaded CSS: `@import` inlining and absolute `url()` references.
use crate::js::url::resolve;
use alloc::{
    format,
    string::{String, ToString},
};

const MAX_DEPTH: usize = 4;
const MAX_IMPORTS: usize = 24;

/// Inline `@import`s (fetched through `fetch`, which receives absolute URLs) and make every `url()`
/// reference absolute relative to `base`.
pub fn process(css: &str, base: &str, fetch: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let mut budget = MAX_IMPORTS;
    process_inner(css, base, fetch, 0, &mut budget)
}

fn process_inner(css: &str, base: &str, fetch: &mut dyn FnMut(&str) -> Option<String>, depth: usize, budget: &mut usize) -> String {
    let css = css.strip_prefix('\u{feff}').unwrap_or(css);
    let mut out = String::new();
    let mut rest = css;
    // leading @charset and @import rules
    loop {
        let t = rest.trim_start();
        let t = skip_comments(t);
        if t.starts_with("@charset") {
            match t.find(';') {
                Some(e) => {
                    rest = &t[e + 1..];
                    continue;
                }
                None => break,
            }
        }
        if t.len() >= 7 && t[..7].eq_ignore_ascii_case("@import") {
            let Some(end) = find_rule_end(t) else { break };
            let rule = &t[7..end];
            rest = &t[end + 1..];
            if depth >= MAX_DEPTH || *budget == 0 {
                continue;
            }
            *budget -= 1;
            if let Some((url, tail)) = parse_import(rule) {
                if let Some(abs) = resolve(base, &url) {
                    if let Some(text) = fetch(&abs) {
                        let inner = process_inner(&text, &abs, fetch, depth + 1, budget);
                        out.push_str(&wrap_import(&inner, tail.trim()));
                    }
                }
            }
            continue;
        }
        rest = t;
        break;
    }
    out.push_str(&absolutize_urls(rest, base));
    out
}

fn skip_comments(mut s: &str) -> &str {
    loop {
        let t = s.trim_start();
        if let Some(r) = t.strip_prefix("/*") {
            match r.find("*/") {
                Some(e) => s = &r[e + 2..],
                None => return "",
            }
        } else {
            return t;
        }
    }
}

/// Index of the `;` ending an at-rule that starts at 0 (ignores `;` inside strings/parens).
fn find_rule_end(s: &str) -> Option<usize> {
    let mut quote = None;
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth -= 1,
                ';' if depth <= 0 => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// Split `url(x) media...` or `"x" layer(a) supports(..) media` into (url, remaining conditions).
fn parse_import(rule: &str) -> Option<(String, &str)> {
    let r = rule.trim_start();
    if r.len() >= 4 && r[..4].eq_ignore_ascii_case("url(") {
        let close = r.find(')')?;
        let inner = r[4..close].trim().trim_matches(|c| c == '"' || c == '\'');
        return Some((inner.to_string(), &r[close + 1..]));
    }
    let q = r.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let end = r[1..].find(q)? + 1;
    Some((r[1..end].to_string(), &r[end + 1..]))
}

fn wrap_import(css: &str, cond: &str) -> String {
    let mut layer: Option<String> = None;
    let mut media = String::new();
    let mut c = cond.trim();
    // layer / layer(name) / supports(...) prefixes
    loop {
        if let Some(r) = c.strip_prefix("layer(") {
            if let Some(e) = r.find(')') {
                layer = Some(r[..e].trim().to_string());
                c = r[e + 1..].trim_start();
                continue;
            }
        }
        if c.starts_with("layer") && (c.len() == 5 || c[5..].starts_with(char::is_whitespace)) {
            layer = Some(String::new());
            c = c[5..].trim_start();
            continue;
        }
        if let Some(r) = c.strip_prefix("supports(") {
            let mut depth = 1;
            let mut end = None;
            for (i, ch) in r.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(e) = end {
                c = r[e + 1..].trim_start();
                continue;
            }
        }
        break;
    }
    if !c.is_empty() && !c.eq_ignore_ascii_case("all") {
        media = c.to_string();
    }
    let mut s = css.to_string();
    if !media.is_empty() {
        s = format!("@media {media} {{\n{s}\n}}\n");
    }
    if let Some(l) = layer {
        s = if l.is_empty() { format!("@layer {{\n{s}\n}}\n") } else { format!("@layer {l} {{\n{s}\n}}\n") };
    }
    s.push('\n');
    s
}

/// Rewrite `url(...)` tokens to absolute URLs (data: and fragment references stay as-is).
pub fn absolutize_urls(css: &str, base: &str) -> String {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut last = 0;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if (c == b'u' || c == b'U') && css[i..].len() >= 4 && css[i..i + 4].eq_ignore_ascii_case("url(") && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'-' || b[i - 1] == b'_')) {
            let start = i + 4;
            let inner_start = start + css[start..].len() - css[start..].trim_start().len();
            let (value, end) = if matches!(b.get(inner_start), Some(b'"') | Some(b'\'')) {
                let q = b[inner_start];
                let mut j = inner_start + 1;
                while j < b.len() && b[j] != q {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let v = &css[inner_start + 1..j.min(b.len())];
                let after = css[(j + 1).min(b.len())..].find(')').map_or(b.len(), |e| j + 1 + e + 1);
                (v, after)
            } else {
                match css[start..].find(')') {
                    Some(e) => (css[start..start + e].trim(), start + e + 1),
                    None => break,
                }
            };
            out.push_str(&css[last..i]);
            let v = value.trim();
            if v.is_empty() || v.starts_with("data:") || v.starts_with('#') || v.starts_with("about:") || v.starts_with("blob:") {
                out.push_str(&css[i..end]);
            } else {
                match resolve(base, v) {
                    Some(abs) => out.push_str(&format!("url(\"{}\")", abs.replace('"', "%22"))),
                    None => out.push_str(&css[i..end]),
                }
            }
            last = end;
            i = end;
            continue;
        }
        i += 1;
    }
    out.push_str(&css[last.min(css.len())..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_become_absolute() {
        let css = ".a{background:url(img/a.png) no-repeat}.b{background:url( \"../b.png\" )}.c{background:url(data:image/png;base64,AAA=)}/* url(x) */.d{content:\"url(y)\"}";
        let out = absolutize_urls(css, "http://h.com/css/main.css");
        assert!(out.contains("url(\"http://h.com/css/img/a.png\")"), "{out}");
        assert!(out.contains("url(\"http://h.com/b.png\")"), "{out}");
        assert!(out.contains("url(data:image/png;base64,AAA=)"));
        assert!(out.contains("/* url(x) */"));
        assert!(out.contains("\"url(y)\""));
    }

    #[test]
    fn imports_are_inlined() {
        let mut calls = alloc::vec::Vec::new();
        let mut fetch = |u: &str| {
            calls.push(u.to_string());
            match u {
                "http://h.com/a.css" => Some("@import 'b.css'; .a{background:url(x.png)}".to_string()),
                "http://h.com/b.css" => Some(".b{color:red}".to_string()),
                _ => None,
            }
        };
        let out = process("@charset \"utf-8\";\n@import url(a.css) screen and (min-width: 10px);\n@import \"b.css\" layer(base);\n.main{color:blue}", "http://h.com/s.css", &mut fetch);
        assert!(out.contains("@media screen and (min-width: 10px)"), "{out}");
        assert!(out.contains("@layer base"), "{out}");
        assert!(out.contains(".a{background:url(\"http://h.com/x.png\")}"), "{out}");
        assert!(out.contains(".main{color:blue}"));
        assert_eq!(calls.len(), 3);
    }
}
