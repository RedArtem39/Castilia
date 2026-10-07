//! Minimal WHATWG-style URL parsing and resolution (no_std, no tables).
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Url {
    pub scheme: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: String,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
    /// Scheme has an authority section (`scheme://`).
    pub has_authority: bool,
}

fn special_port(scheme: &str) -> Option<&'static str> {
    match scheme {
        "http" | "ws" => Some("80"),
        "https" | "wss" => Some("443"),
        "ftp" => Some("21"),
        _ => None,
    }
}

fn is_special(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

fn pct_encode(s: &str, keep: impl Fn(char) -> bool) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if keep(ch) {
            out.push(ch);
        } else {
            let mut b = [0u8; 4];
            for byte in ch.encode_utf8(&mut b).bytes() {
                out.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    out
}

fn path_char(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_control() && !matches!(c, ' ' | '"' | '<' | '>' | '`' | '#' | '?' | '{' | '}')
}

fn query_char(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_control() && !matches!(c, ' ' | '"' | '<' | '>' | '#' | '\'')
}

fn frag_char(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_control() && !matches!(c, ' ' | '"' | '<' | '>' | '`')
}

fn remove_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').collect();
    let n = segs.len();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == n;
        match *seg {
            "." | "%2e" | "%2E" => {
                if last {
                    out.push("");
                }
            }
            ".." | ".%2e" | "%2e." | "%2e%2e" | ".%2E" | "%2E." | "%2E%2E" => {
                if out.len() > 1 {
                    out.pop();
                }
                if last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let r = out.join("/");
    if r.is_empty() { "/".to_string() } else { r }
}

impl Url {
    pub fn parse(input: &str, base: Option<&Url>) -> Option<Url> {
        let input = input.trim_matches(|c: char| c <= ' ');
        let input: String = input.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
        let s = input.as_str();
        // scheme
        let mut scheme_end = None;
        for (i, c) in s.char_indices() {
            if i == 0 {
                if !c.is_ascii_alphabetic() {
                    break;
                }
            } else if c == ':' {
                scheme_end = Some(i);
                break;
            } else if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
                break;
            }
        }
        if let Some(e) = scheme_end {
            let scheme = s[..e].to_ascii_lowercase();
            let rest = &s[e + 1..];
            // `http:foo` relative-with-scheme against same-scheme base
            if is_special(&scheme) && scheme != "file" && !rest.starts_with('/') && !rest.starts_with('\\') {
                if let Some(b) = base {
                    if b.scheme == scheme {
                        return Url::relative(rest, b);
                    }
                }
            }
            return Url::absolute(scheme, rest);
        }
        let b = base?;
        Url::relative(s, b)
    }

    fn split_tail(rest: &str) -> (&str, Option<String>, Option<String>) {
        let (before_frag, frag) = match rest.find('#') {
            Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
            None => (rest, None),
        };
        let (path, query) = match before_frag.find('?') {
            Some(i) => (&before_frag[..i], Some(before_frag[i + 1..].to_string())),
            None => (before_frag, None),
        };
        (path, query, frag)
    }

    fn absolute(scheme: String, rest: &str) -> Option<Url> {
        let special = is_special(&scheme);
        let rest_norm: String = if special { rest.replace('\\', "/") } else { rest.to_string() };
        let rest = rest_norm.as_str();
        let mut u = Url { scheme: scheme.clone(), ..Default::default() };
        if rest.starts_with("//") || (special && scheme != "file") {
            let r = rest.trim_start_matches('/');
            let r = if special { r } else { &rest[2..] };
            let end = r.find(['/', '?', '#']).unwrap_or(r.len());
            let auth = &r[..end];
            u.has_authority = true;
            u.parse_authority(auth)?;
            let (path, q, f) = Url::split_tail(&r[end..]);
            u.set_path(path);
            u.query = q.map(|q| pct_encode(&q, query_char));
            u.fragment = f.map(|f| pct_encode(&f, frag_char));
            if special && u.host.is_empty() && scheme != "file" {
                return None;
            }
            Some(u)
        } else if scheme == "file" {
            u.has_authority = true;
            let (path, q, f) = Url::split_tail(rest);
            u.set_path(path);
            u.query = q;
            u.fragment = f;
            Some(u)
        } else {
            // opaque path (mailto:, data:, javascript:, ...)
            let (path, q, f) = Url::split_tail(rest);
            u.path = pct_encode(path, |c| c.is_ascii() && !c.is_ascii_control() || false);
            u.query = q;
            u.fragment = f.map(|f| pct_encode(&f, frag_char));
            Some(u)
        }
    }

    fn parse_authority(&mut self, auth: &str) -> Option<()> {
        let (userinfo, hostport) = match auth.rfind('@') {
            Some(i) => (Some(&auth[..i]), &auth[i + 1..]),
            None => (None, auth),
        };
        if let Some(ui) = userinfo {
            let (u, p) = ui.split_once(':').unwrap_or((ui, ""));
            self.username = u.to_string();
            self.password = p.to_string();
        }
        let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
            let end = rest.find(']')?;
            let host = format!("[{}]", &rest[..end]);
            let after = &rest[end + 1..];
            (host, after.strip_prefix(':').unwrap_or("").to_string())
        } else {
            match hostport.rfind(':') {
                Some(i) => (hostport[..i].to_string(), hostport[i + 1..].to_string()),
                None => (hostport.to_string(), String::new()),
            }
        };
        if !port.is_empty() && !port.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        self.host = if is_special(&self.scheme) { host.to_ascii_lowercase() } else { host };
        let port = port.trim_start_matches('0');
        self.port = if special_port(&self.scheme) == Some(port) || (port.is_empty() && !hostport.ends_with(":0")) { String::new() } else if port.is_empty() { "0".into() } else { port.to_string() };
        Some(())
    }

    fn set_path(&mut self, path: &str) {
        let p = if path.is_empty() && is_special(&self.scheme) { "/" } else { path };
        let enc = pct_encode(p, |c| path_char(c) || c == '%');
        self.path = if enc.starts_with('/') || !is_special(&self.scheme) && !self.has_authority { remove_dots(&enc) } else { remove_dots(&format!("/{enc}")) };
        if !self.has_authority && !p.starts_with('/') {
            self.path = enc;
        }
    }

    fn relative(rest: &str, base: &Url) -> Option<Url> {
        let special = is_special(&base.scheme);
        let rest_n: String = if special { rest.replace('\\', "/") } else { rest.to_string() };
        let rest = rest_n.as_str();
        if !base.has_authority && !base.path.starts_with('/') {
            // opaque base: only fragment-only references work
            if let Some(f) = rest.strip_prefix('#') {
                let mut u = base.clone();
                u.fragment = Some(pct_encode(f, frag_char));
                return Some(u);
            }
            return None;
        }
        if rest.starts_with("//") {
            return Url::absolute(base.scheme.clone(), rest);
        }
        let mut u = base.clone();
        u.fragment = None;
        if rest.is_empty() {
            return Some(u);
        }
        let (path, q, f) = Url::split_tail(rest);
        if path.is_empty() {
            if q.is_some() {
                u.query = q.map(|q| pct_encode(&q, query_char));
            }
        } else if path.starts_with('/') {
            u.set_path(path);
            u.query = q.map(|q| pct_encode(&q, query_char));
        } else {
            let dir = match base.path.rfind('/') {
                Some(i) => &base.path[..=i],
                None => "/",
            };
            u.set_path(&format!("{dir}{path}"));
            u.query = q.map(|q| pct_encode(&q, query_char));
        }
        u.fragment = f.map(|f| pct_encode(&f, frag_char));
        Some(u)
    }

    pub fn hostname(&self) -> &str {
        &self.host
    }

    pub fn host_with_port(&self) -> String {
        if self.port.is_empty() { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }

    pub fn origin(&self) -> String {
        if matches!(self.scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp") {
            format!("{}://{}", self.scheme, self.host_with_port())
        } else {
            "null".to_string()
        }
    }

    pub fn search(&self) -> String {
        match &self.query {
            Some(q) if !q.is_empty() => format!("?{q}"),
            _ => String::new(),
        }
    }

    pub fn hash(&self) -> String {
        match &self.fragment {
            Some(f) if !f.is_empty() => format!("#{f}"),
            _ => String::new(),
        }
    }

    pub fn href(&self) -> String {
        let mut s = format!("{}:", self.scheme);
        if self.has_authority {
            s.push_str("//");
            if !self.username.is_empty() || !self.password.is_empty() {
                s.push_str(&self.username);
                if !self.password.is_empty() {
                    s.push(':');
                    s.push_str(&self.password);
                }
                s.push('@');
            }
            s.push_str(&self.host_with_port());
        }
        s.push_str(&self.path);
        if let Some(q) = &self.query {
            s.push('?');
            s.push_str(q);
        }
        if let Some(f) = &self.fragment {
            s.push('#');
            s.push_str(f);
        }
        s
    }
}

pub fn resolve(base: &str, rel: &str) -> Option<String> {
    let b = Url::parse(base, None);
    Url::parse(rel, b.as_ref()).map(|u| u.href())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_resolve() {
        let u = Url::parse("HTTPS://User:pw@Example.COM:443/a/./b/../c?x=1#frag", None).unwrap();
        assert_eq!(u.href(), "https://User:pw@example.com/a/c?x=1#frag");
        assert_eq!(u.origin(), "https://example.com");
        assert_eq!(resolve("http://a.com/x/y/z.html", "../q?w#e").unwrap(), "http://a.com/x/q?w#e");
        assert_eq!(resolve("http://a.com/x/y", "//b.com/p").unwrap(), "http://b.com/p");
        assert_eq!(resolve("http://a.com/x/y?q", "#h").unwrap(), "http://a.com/x/y?q#h");
        assert_eq!(resolve("http://a.com/x/y?q", "?z").unwrap(), "http://a.com/x/y?z");
        assert_eq!(resolve("http://a.com", "foo").unwrap(), "http://a.com/foo");
        assert_eq!(resolve("http://a.com/a/b", "/c d").unwrap(), "http://a.com/c%20d");
        assert_eq!(resolve("http://a.com/", "mailto:x@y.z").unwrap(), "mailto:x@y.z");
        assert_eq!(resolve("http://a.com/", "data:text/plain,hi").unwrap(), "data:text/plain,hi");
        assert!(Url::parse("foo", None).is_none());
    }
}
