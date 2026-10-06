use crate::dom::{Document, NodeId, NodeKind};
use alloc::{string::String, vec::Vec};

/// Tolerant HTML subset. Unknown elements are retained in the DOM.
pub fn parse(source: &str) -> Document {
    let mut doc = Document::new();
    let mut stack = Vec::from([doc.body]);
    let mut input = source;
    let mut in_head = false;
    let mut body_started = false;

    while !input.is_empty() {
        if input.starts_with("<!--") {
            input = input.find("-->").map(|end| &input[end + 3..]).unwrap_or("");
            continue;
        }
        if input.starts_with("<!") || input.starts_with("<?") {
            input = input.find('>').map(|end| &input[end + 1..]).unwrap_or("");
            continue;
        }
        if input.starts_with("</")
            && let Some(end) = input.find('>')
        {
            let tag = input[2..end]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            match tag.as_str() {
                "head" => {
                    in_head = false;
                    stack.clear();
                    stack.push(doc.body);
                }
                "body" | "html" => {
                    stack.clear();
                    stack.push(doc.body);
                }
                _ => {
                    if let Some(position) = stack
                        .iter()
                        .rposition(|&id| doc.nodes[id].tag() == Some(tag.as_str()))
                    {
                        stack.truncate(position);
                        if stack.is_empty() {
                            stack.push(doc.body);
                        }
                    }
                }
            }
            input = &input[end + 1..];
            continue;
        }
        if input.starts_with('<')
            && let Some((tag, attributes, rest, self_closing)) = start_tag(input)
        {
            input = rest;
            match tag.as_str() {
                "html" => {
                    if let NodeKind::Element {
                        attributes: existing,
                        ..
                    } = &mut doc.nodes[1].kind
                    {
                        *existing = attributes;
                    }
                    continue;
                }
                "head" if !body_started => {
                    if let NodeKind::Element {
                        attributes: existing,
                        ..
                    } = &mut doc.nodes[doc.head].kind
                    {
                        *existing = attributes;
                    }
                    stack.clear();
                    stack.push(doc.head);
                    in_head = true;
                    continue;
                }
                "body" => {
                    if let NodeKind::Element {
                        attributes: existing,
                        ..
                    } = &mut doc.nodes[doc.body].kind
                    {
                        *existing = attributes;
                    }
                    stack.clear();
                    stack.push(doc.body);
                    in_head = false;
                    body_started = true;
                    continue;
                }
                _ => {}
            }
            if matches!(tag.as_str(), "style" | "script" | "title" | "meta" | "link")
                && !body_started
                && stack.len() == 1
            {
                stack.clear();
                stack.push(doc.head);
                in_head = true;
            } else if in_head
                && !matches!(
                    tag.as_str(),
                    "style" | "script" | "title" | "meta" | "link" | "base"
                )
            {
                stack.clear();
                stack.push(doc.body);
                in_head = false;
                body_started = true;
            }
            if matches!(
                tag.as_str(),
                "p" | "div" | "section" | "article" | "h1" | "h2" | "h3" | "ul" | "ol"
            ) {
                close_open(&mut stack, &doc, "p");
            }
            if tag == "li" {
                close_open(&mut stack, &doc, "li");
            }
            let parent = *stack.last().unwrap_or(&doc.body);
            let id = doc.push(
                NodeKind::Element {
                    tag: tag.clone(),
                    attributes,
                },
                Some(parent),
            );
            if matches!(tag.as_str(), "style" | "script" | "title") {
                let closing = alloc::format!("</{}", tag);
                if let Some(end) = find_ascii_case_insensitive(input, &closing) {
                    doc.push(
                        NodeKind::Text(if tag == "style" || tag == "script" {
                            String::from(&input[..end])
                        } else {
                            decode_entities(&input[..end])
                        }),
                        Some(id),
                    );
                    input = input[end..]
                        .find('>')
                        .map(|n| &input[end + n + 1..])
                        .unwrap_or("");
                } else {
                    doc.push(
                        NodeKind::Text(if tag == "style" || tag == "script" {
                            String::from(input)
                        } else {
                            decode_entities(input)
                        }),
                        Some(id),
                    );
                    input = "";
                }
            } else if !self_closing && !is_void(&tag) {
                stack.push(id);
            }
            continue;
        }
        let end = if let Some(after_lt) = input.strip_prefix('<') {
            after_lt.find('<').map(|n| n + 1).unwrap_or(input.len())
        } else {
            input.find('<').unwrap_or(input.len())
        };
        let text = decode_entities(&input[..end]);
        if !text.trim().is_empty() || stack.len() > 1 {
            let parent = *stack.last().unwrap_or(&doc.body);
            if !in_head {
                body_started = true;
            }
            doc.push(NodeKind::Text(text), Some(parent));
        }
        input = &input[end..];
    }
    doc
}

fn close_open(stack: &mut Vec<NodeId>, doc: &Document, tag: &str) {
    if let Some(position) = stack
        .iter()
        .rposition(|&id| doc.nodes[id].tag() == Some(tag))
    {
        stack.truncate(position);
        if stack.is_empty() {
            stack.push(doc.body);
        }
    }
}

fn is_void(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

type StartTag<'a> = (String, Vec<(String, String)>, &'a str, bool);

fn start_tag(input: &str) -> Option<StartTag<'_>> {
    let bytes = input.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'<' || !bytes[1].is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b':')) {
        i += 1;
    }
    let tag = input[1..i].to_ascii_lowercase();
    let mut attributes = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'>' {
            return Some((tag, attributes, &input[i + 1..], false));
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'>') {
            return Some((tag, attributes, &input[i + 2..], true));
        }
        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
        {
            i += 1;
        }
        if i == start {
            i += 1;
            continue;
        }
        let name = input[start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if let Some(&quote @ (b'\'' | b'"')) = bytes.get(i) {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = decode_entities(&input[start..i]);
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = decode_entities(&input[start..i]);
            }
        }
        if !attributes.iter().any(|(key, _)| key == &name) {
            attributes.push((name, value));
        }
    }
    None
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn decode_entities(input: &str) -> String {
    let mut output = String::new();
    let mut rest = input;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(end) = rest.find(';')
            && end <= 32
        {
            let entity = &rest[1..end];
            let numeric = if entity.starts_with("#x") || entity.starts_with("#X") {
                u32::from_str_radix(&entity[2..], 16).ok()
            } else if let Some(number) = entity.strip_prefix('#') {
                number.parse::<u32>().ok()
            } else {
                None
            };
            if let Some(number) = numeric {
                output.push(
                    char::from_u32(number)
                        .filter(|&c| c != '\0')
                        .unwrap_or(char::REPLACEMENT_CHARACTER),
                );
                rest = &rest[end + 1..];
                continue;
            }
            if let Ok(index) = crate::entities::NAMED.binary_search_by(|entry| entry.0.cmp(entity))
            {
                output.push_str(crate::entities::NAMED[index].1);
                rest = &rest[end + 1..];
                continue;
            }
        }
        output.push('&');
        rest = &rest[1..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repairs_missing_paragraph_end_tags() {
        let doc = parse("<p>one<p>two");
        assert_eq!(doc.nodes[doc.body].children.len(), 2);
        assert_eq!(
            doc.nodes[doc.body]
                .children
                .iter()
                .map(|&id| doc.nodes[id].tag())
                .collect::<Vec<_>>(),
            [Some("p"), Some("p")]
        );
    }

    #[test]
    fn parses_attributes_and_entities() {
        let doc = parse("<a href='a&amp;b'>A &#x26; B</a>");
        let a = doc.nodes[doc.body].children[0];
        assert_eq!(doc.nodes[a].attribute("href"), Some("a&b"));
        assert_eq!(
            doc.nodes[doc.nodes[a].children[0]].kind,
            NodeKind::Text(String::from("A & B"))
        );
    }

    #[test]
    fn preserves_raw_inline_script_text() {
        let doc = parse("<head><script>document.body.textContent = '<ok>';</script></head>");
        let script = doc.nodes[doc.head].children[0];
        assert_eq!(doc.nodes[script].tag(), Some("script"));
        let text = doc.nodes[script].children[0];
        assert_eq!(
            doc.nodes[text].kind,
            NodeKind::Text(String::from("document.body.textContent = '<ok>';"))
        );
    }

    #[test]
    fn accepts_utf8_text() {
        let doc = parse("<p>Привет, світ!</p>");
        let p = doc.nodes[doc.body].children[0];
        assert_eq!(
            doc.nodes[doc.nodes[p].children[0]].kind,
            NodeKind::Text(String::from("Привет, світ!"))
        );
    }

    #[test]
    fn decodes_long_and_multicodepoint_named_entities() {
        let doc = parse("<p>&CounterClockwiseContourIntegral; &NotEqualTilde;</p>");
        let p = doc.nodes[doc.body].children[0];
        let NodeKind::Text(text) = &doc.nodes[doc.nodes[p].children[0]].kind else {
            panic!("missing text");
        };
        assert!(text.starts_with('∳'));
        assert!(text.chars().count() > 3);
    }
}
