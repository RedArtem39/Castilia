use crate::dom::{Document, NodeId};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Flex,
    Inline,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    Px(f32),
    Percent(f32),
}

impl Length {
    pub fn resolve(self, base: f32) -> f32 {
        match self {
            Self::Px(value) => value,
            Self::Percent(value) => base * value / 100.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub display: Display,
    pub color: u32,
    pub background: Option<u32>,
    pub font_size: f32,
    pub margin: Edges,
    pub padding: Edges,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub gap: f32,
}

impl Style {
    fn for_node(tag: Option<&str>, parent: Option<&Self>) -> Self {
        let mut style = Self {
            display: Display::Inline,
            color: parent.map_or(0x181818, |parent| parent.color),
            background: None,
            font_size: parent.map_or(16.0, |parent| parent.font_size),
            margin: Edges::default(),
            padding: Edges::default(),
            width: None,
            height: None,
            gap: 0.0,
        };
        if matches!(
            tag,
            Some(
                "html"
                    | "body"
                    | "center"
                    | "div"
                    | "section"
                    | "article"
                    | "main"
                    | "header"
                    | "footer"
                    | "nav"
                    | "p"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "ul"
                    | "ol"
                    | "li"
                    | "pre"
                    | "table"
                    | "thead"
                    | "tbody"
                    | "tfoot"
                    | "tr"
                    | "caption"
            )
        ) {
            style.display = Display::Block;
        }
        match tag {
            Some("body") => {
                style.margin = Edges {
                    top: 8.0,
                    right: 8.0,
                    bottom: 8.0,
                    left: 8.0,
                }
            }
            Some("p") | Some("ul") | Some("ol") => {
                style.margin.top = 8.0;
                style.margin.bottom = 8.0;
            }
            Some("h1") => {
                style.font_size = 32.0;
                style.margin.top = 16.0;
                style.margin.bottom = 12.0;
            }
            Some("h2") => {
                style.font_size = 24.0;
                style.margin.top = 14.0;
                style.margin.bottom = 10.0;
            }
            Some("h3") => {
                style.font_size = 19.0;
                style.margin.top = 12.0;
                style.margin.bottom = 8.0;
            }
            Some("a") => style.color = 0x0645ad,
            Some("head" | "style" | "script" | "title" | "meta" | "link") => {
                style.display = Display::None
            }
            _ => {}
        }
        style
    }
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selector: String,
    pub declarations: Vec<Declaration>,
    pub specificity: u32,
    pub order: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CssAudit {
    pub ignored_selectors: usize,
    pub ignored_declarations: usize,
    pub at_rules: usize,
}

/// Approximate feature count for diagnostics, not a standards conformance score.
pub fn audit(source: &str) -> CssAudit {
    let source = remove_comments(source);
    let mut result = CssAudit::default();
    audit_blocks(&source, &mut result);
    result
}

fn audit_blocks(mut source: &str, result: &mut CssAudit) {
    while let Some((header, body, rest)) = next_block(source) {
        if header.starts_with("@media") && media_matches(header, 800.0).is_some() {
            audit_blocks(body, result);
        } else if header.starts_with('@') {
            result.at_rules += 1;
        } else {
            result.ignored_selectors += header
                .split(',')
                .filter(|part| specificity(part.trim()).is_none())
                .count();
            result.ignored_declarations += parse_declarations(body)
                .iter()
                .filter(|d| !supported_property(&d.name, &d.value))
                .count();
        }
        source = rest;
    }
}

fn supported_property(name: &str, value: &str) -> bool {
    match name {
        "display" => matches!(value, "block" | "flex" | "inline" | "inline-block" | "none"),
        "color" | "background" | "background-color" => {
            parse_color(value).is_some() || value == "transparent"
        }
        "font-size" => {
            px(value).is_some()
                || value
                    .strip_suffix("em")
                    .is_some_and(|n| n.parse::<f32>().is_ok())
        }
        "margin" | "padding" => parse_edges(value).is_some(),
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" => px(value).is_some(),
        "width" | "height" => parse_length(value).is_some(),
        "gap" => px(value).is_some(),
        _ => false,
    }
}

pub fn parse(source: &str) -> Vec<Rule> {
    parse_with_viewport(source, f32::INFINITY)
}

pub fn parse_with_viewport(source: &str, viewport_width: f32) -> Vec<Rule> {
    let source = remove_comments(source);
    let mut rules = Vec::new();
    parse_blocks(&source, viewport_width, &mut rules);
    rules
}

fn parse_blocks(mut source: &str, viewport_width: f32, rules: &mut Vec<Rule>) {
    while let Some((header, body, rest)) = next_block(source) {
        if header.starts_with("@media") {
            if media_matches(header, viewport_width) == Some(true) {
                parse_blocks(body, viewport_width, rules);
            }
        } else if !header.starts_with('@') {
            let declarations = parse_declarations(body);
            for selector in header.split(',').map(str::trim) {
                if let Some(specificity) = specificity(selector) {
                    rules.push(Rule {
                        selector: selector.to_string(),
                        declarations: declarations.clone(),
                        specificity,
                        order: rules.len(),
                    });
                }
            }
        }
        source = rest;
    }
}

fn next_block(source: &str) -> Option<(&str, &str, &str)> {
    let open = source.find('{')?;
    let bytes = source.as_bytes();
    let mut depth = 1;
    let mut quote = None;
    let mut index = open + 1;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index += 2;
                continue;
            }
            if byte == delimiter {
                quote = None;
            }
        } else {
            match byte {
                b'\'' | b'"' => quote = Some(byte),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((
                            source[..open].trim(),
                            &source[open + 1..index],
                            &source[index + 1..],
                        ));
                    }
                }
                _ => {}
            }
        }
        index += 1;
    }
    None
}

fn media_matches(header: &str, width: f32) -> Option<bool> {
    let query = header.strip_prefix("@media")?.trim();
    let mut any = false;
    for alternative in query.split(',') {
        let mut rest = alternative.trim();
        if let Some(tail) = rest.strip_prefix("screen") {
            rest = tail.trim();
        } else if let Some(tail) = rest.strip_prefix("all") {
            rest = tail.trim();
        } else if rest.starts_with("print") {
            continue;
        }
        let mut matches = true;
        if rest.is_empty() {
            any = true;
            continue;
        }
        while !rest.is_empty() {
            rest = rest.trim_start();
            if let Some(tail) = rest.strip_prefix("and") {
                rest = tail.trim_start();
            }
            let inner = rest.strip_prefix('(')?;
            let close = inner.find(')')?;
            let (name, value) = inner[..close].split_once(':')?;
            let threshold = px(value.trim())?;
            matches &= match name.trim() {
                "max-width" => width <= threshold,
                "min-width" => width >= threshold,
                _ => return None,
            };
            rest = inner[close + 1..].trim();
        }
        any |= matches;
    }
    Some(any)
}

fn remove_comments(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map(|end| &rest[start + 2 + end + 2..])
            .unwrap_or("");
    }
    out.push_str(rest);
    out
}

fn parse_declarations(body: &str) -> Vec<Declaration> {
    body.split(';')
        .filter_map(|part| {
            let (name, value) = part.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            let mut value = value.trim();
            let important = value.len() >= 10
                && value
                    .get(value.len() - 10..)
                    .is_some_and(|end| end.eq_ignore_ascii_case("!important"));
            if important {
                value = value[..value.len() - 10].trim_end();
            }
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(Declaration {
                name,
                value: value.to_string(),
                important,
            })
        })
        .collect()
}

fn specificity(selector: &str) -> Option<u32> {
    if selector.is_empty() || selector.contains(['>', '+', '~', '[', ':', '*']) {
        return None;
    }
    let mut score = 0;
    for part in selector.split_whitespace() {
        if !part.split(['#', '.']).next().unwrap_or("").is_empty() {
            score += 1;
        }
        score += part.bytes().filter(|&byte| byte == b'#').count() as u32 * 100;
        score += part.bytes().filter(|&byte| byte == b'.').count() as u32 * 10;
    }
    Some(score)
}

fn matches_selector(doc: &Document, node: NodeId, selector: &str) -> bool {
    let mut parts = selector.split_whitespace().rev();
    let Some(last) = parts.next() else {
        return false;
    };
    if !matches_compound(doc, node, last) {
        return false;
    }
    let mut ancestor = doc.nodes[node].parent;
    for part in parts {
        let mut found = false;
        while let Some(id) = ancestor {
            ancestor = doc.nodes[id].parent;
            if matches_compound(doc, id, part) {
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

fn matches_compound(doc: &Document, node: NodeId, selector: &str) -> bool {
    let item = &doc.nodes[node];
    let Some(tag) = item.tag() else {
        return false;
    };
    let bytes = selector.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'#' && bytes[i] != b'.' {
        i += 1;
    }
    if i > 0 && !tag.eq_ignore_ascii_case(&selector[..i]) {
        return false;
    }
    while i < bytes.len() {
        let marker = bytes[i];
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != b'#' && bytes[i] != b'.' {
            i += 1;
        }
        if start == i {
            return false;
        }
        let value = &selector[start..i];
        if marker == b'#' && item.attribute("id") != Some(value) {
            return false;
        }
        if marker == b'.'
            && !item
                .attribute("class")
                .is_some_and(|classes| classes.split_whitespace().any(|class| class == value))
        {
            return false;
        }
    }
    true
}

pub fn compute_styles(doc: &Document, rules: &[Rule]) -> Vec<Style> {
    let mut styles = Vec::with_capacity(doc.nodes.len());
    for (id, node) in doc.nodes.iter().enumerate() {
        let parent = node.parent.and_then(|parent| styles.get(parent));
        let mut style = Style::for_node(node.tag(), parent);
        let mut declarations: Vec<(bool, u32, usize, usize, &Declaration)> = Vec::new();
        for rule in rules {
            if matches_selector(doc, id, &rule.selector) {
                for (index, declaration) in rule.declarations.iter().enumerate() {
                    declarations.push((
                        declaration.important,
                        rule.specificity,
                        rule.order,
                        index,
                        declaration,
                    ));
                }
            }
        }
        let inline = node
            .attribute("style")
            .map(parse_declarations)
            .unwrap_or_default();
        for (index, declaration) in inline.iter().enumerate() {
            declarations.push((
                declaration.important,
                1_000_000,
                rules.len(),
                index,
                declaration,
            ));
        }
        declarations.sort_by_key(|entry| (entry.0, entry.1, entry.2, entry.3));
        for (_, _, _, _, declaration) in declarations {
            apply(&mut style, declaration, parent);
        }
        styles.push(style);
    }
    styles
}

fn parse_length(value: &str) -> Option<Length> {
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse().ok().map(Length::Px);
    }
    if let Some(percent) = value.strip_suffix('%') {
        return percent.trim().parse().ok().map(Length::Percent);
    }
    if value == "0" {
        return Some(Length::Px(0.0));
    }
    None
}

fn px(value: &str) -> Option<f32> {
    match parse_length(value)? {
        Length::Px(value) => Some(value),
        Length::Percent(_) => None,
    }
}

fn parse_color(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            3 => u32::from_str_radix(hex, 16).ok().map(|n| {
                ((n & 0xf00) << 12)
                    | ((n & 0xf00) << 8)
                    | ((n & 0x0f0) << 8)
                    | ((n & 0x0f0) << 4)
                    | ((n & 0x00f) << 4)
                    | (n & 0x00f)
            }),
            6 => u32::from_str_radix(hex, 16).ok(),
            _ => None,
        };
    }
    match value.to_ascii_lowercase().as_str() {
        "black" => Some(0),
        "white" => Some(0xffffff),
        "red" => Some(0xff0000),
        "green" => Some(0x008000),
        "blue" => Some(0x0000ff),
        "gray" | "grey" => Some(0x808080),
        "yellow" => Some(0xffff00),
        "navy" => Some(0x000080),
        "transparent" => None,
        _ => None,
    }
}

fn parse_edges(value: &str) -> Option<Edges> {
    let values: Vec<_> = value
        .split_whitespace()
        .map(px)
        .collect::<Option<Vec<_>>>()?;
    let edges = match values.as_slice() {
        [a] => Edges {
            top: *a,
            right: *a,
            bottom: *a,
            left: *a,
        },
        [a, b] => Edges {
            top: *a,
            right: *b,
            bottom: *a,
            left: *b,
        },
        [a, b, c] => Edges {
            top: *a,
            right: *b,
            bottom: *c,
            left: *b,
        },
        [a, b, c, d] => Edges {
            top: *a,
            right: *b,
            bottom: *c,
            left: *d,
        },
        _ => return None,
    };
    Some(edges)
}

fn apply(style: &mut Style, declaration: &Declaration, parent: Option<&Style>) {
    let value = declaration.value.as_str();
    match declaration.name.as_str() {
        "display" => {
            if let Some(display) = match value {
                "block" => Some(Display::Block),
                "flex" => Some(Display::Flex),
                "inline" | "inline-block" => Some(Display::Inline),
                "none" => Some(Display::None),
                _ => None,
            } {
                style.display = display;
            }
        }
        "color" => {
            if let Some(color) = parse_color(value) {
                style.color = color;
            }
        }
        "background" | "background-color" => style.background = parse_color(value),
        "font-size" => {
            let base = parent.map_or(16.0, |parent| parent.font_size);
            if let Some(size) = px(value).or_else(|| {
                value
                    .strip_suffix("em")
                    .and_then(|n| n.parse::<f32>().ok())
                    .map(|n| n * base)
            }) {
                style.font_size = size;
            }
        }
        "margin" => {
            if let Some(edges) = parse_edges(value) {
                style.margin = edges;
            }
        }
        "padding" => {
            if let Some(edges) = parse_edges(value) {
                style.padding = edges;
            }
        }
        "margin-top" => {
            if let Some(n) = px(value) {
                style.margin.top = n;
            }
        }
        "margin-right" => {
            if let Some(n) = px(value) {
                style.margin.right = n;
            }
        }
        "margin-bottom" => {
            if let Some(n) = px(value) {
                style.margin.bottom = n;
            }
        }
        "margin-left" => {
            if let Some(n) = px(value) {
                style.margin.left = n;
            }
        }
        "padding-top" => {
            if let Some(n) = px(value) {
                style.padding.top = n;
            }
        }
        "padding-right" => {
            if let Some(n) = px(value) {
                style.padding.right = n;
            }
        }
        "padding-bottom" => {
            if let Some(n) = px(value) {
                style.padding.bottom = n;
            }
        }
        "padding-left" => {
            if let Some(n) = px(value) {
                style.padding.left = n;
            }
        }
        "width" => style.width = parse_length(value),
        "height" => style.height = parse_length(value),
        "gap" => {
            if let Some(gap) = px(value) {
                style.gap = gap.max(0.0);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;
    #[test]
    fn class_overrides_tag_and_inherits_color() {
        let doc = html::parse(
            "<style>p { color: red } .note { color: #123456 }</style><p class='note'><span>x</span></p>",
        );
        let rules = parse("p { color: red } .note { color: #123456 }");
        let styles = compute_styles(&doc, &rules);
        let p = doc.nodes[doc.body].children[0];
        assert_eq!(styles[p].color, 0x123456);
        assert_eq!(styles[doc.nodes[p].children[0]].color, 0x123456);
    }

    #[test]
    fn descendant_selector_and_important_override() {
        let doc = html::parse("<main><p class='note' style='color: blue'>text</p></main>");
        let styles = compute_styles(
            &doc,
            &parse("main .note { color: red !important } main p { color: green }"),
        );
        let main = doc.nodes[doc.body].children[0];
        let p = doc.nodes[main].children[0];
        assert_eq!(styles[p].color, 0xff0000);
    }

    #[test]
    fn type_plus_class_beats_class_alone() {
        let doc = html::parse("<div class='x'>text</div>");
        let rules = parse("div.x { color: blue } .x { color: red }");
        let styles = compute_styles(&doc, &rules);
        assert_eq!(styles[doc.nodes[doc.body].children[0]].color, 0x0000ff);
    }

    #[test]
    fn media_rules_follow_viewport_width() {
        let source = "p { color: blue } @media screen and (max-width: 500px) { p { color: red } }";
        let doc = html::parse("<p>text</p>");
        let p = doc.nodes[doc.body].children[0];
        assert_eq!(
            compute_styles(&doc, &parse_with_viewport(source, 400.0))[p].color,
            0xff0000
        );
        assert_eq!(
            compute_styles(&doc, &parse_with_viewport(source, 800.0))[p].color,
            0x0000ff
        );
    }
}
