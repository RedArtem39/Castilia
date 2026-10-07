//! Style engine: stylesheet parsing, selector indexing, cascade and computed values.
use crate::{
    color::{self, Color},
    css_values::*,
    dom::{Document, NodeId, NodeKind},
    selector::{self, Complex, PseudoElement},
};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    rc::Rc,
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub use crate::css_values::{Dim, Gradient};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    None,
    Contents,
    Block,
    FlowRoot,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    ListItem,
    Table,
    InlineTable,
    TableRowGroup,
    TableRow,
    TableCell,
    TableCaption,
    TableColumn,
}

impl Display {
    /// Block-level boxes participate in block formatting as blocks.
    pub fn is_block_level(self) -> bool {
        matches!(
            self,
            Display::Block
                | Display::FlowRoot
                | Display::Flex
                | Display::Grid
                | Display::ListItem
                | Display::Table
                | Display::TableCaption
        )
    }
    pub fn is_table_internal(self) -> bool {
        matches!(
            self,
            Display::TableRowGroup | Display::TableRow | Display::TableCell | Display::TableColumn
        )
    }
    /// Computed display after blockification (flex/grid items, floats, absolutes).
    pub fn blockify(self) -> Display {
        match self {
            Display::Inline | Display::InlineBlock => Display::Block,
            Display::InlineFlex => Display::Flex,
            Display::InlineGrid => Display::Grid,
            Display::InlineTable => Display::Table,
            d => d,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float {
    None,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
}

impl WhiteSpace {
    pub fn collapses_spaces(self) -> bool {
        matches!(self, WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine)
    }
    pub fn wraps(self) -> bool {
        matches!(self, WhiteSpace::Normal | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }
    pub fn preserves_newlines(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    Len(Dim),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
}

impl Overflow {
    pub fn clips(self) -> bool {
        !matches!(self, Overflow::Visible)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Hidden,
    Solid,
    Dashed,
    Dotted,
    Double,
}

impl BorderStyle {
    pub fn visible(self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    Nowrap,
    Wrap,
    WrapReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Auto,
    Normal,
    Stretch,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamily {
    Serif,
    Sans,
    Monospace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    LowerGreek,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Track {
    Px(f32),
    Pct(f32),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    MinMax(Box<Track>, Box<Track>),
    FitContent(f32),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackList {
    pub before: Vec<Track>,
    /// `repeat(auto-fill | auto-fit, ...)`: (is_fit, tracks)
    pub repeat: Option<(bool, Vec<Track>)>,
    pub after: Vec<Track>,
    /// Named lines: (name, line index counted over `before` tracks only)
    pub names: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GridLine {
    Auto,
    Line(i32),
    Span(u32),
    Name(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgRepeat {
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BgSize {
    Auto,
    Cover,
    Contain,
    Explicit(Dim, Dim),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BgImage {
    None,
    Url(String),
    Gradient(Gradient),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    Text(String),
    Attr(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub display: Display,
    pub position: Position,
    pub float: Float,
    pub clear: Clear,
    pub top: Dim,
    pub right: Dim,
    pub bottom: Dim,
    pub left: Dim,
    pub z_index: Option<i32>,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub visibility_visible: bool,
    pub opacity: f32,
    pub pointer_events: bool,

    pub width: Dim,
    pub height: Dim,
    pub min_width: Dim,
    pub min_height: Dim,
    pub max_width: Dim,
    pub max_height: Dim,
    pub border_box: bool,
    pub aspect_ratio: Option<f32>,
    pub margin: [Dim; 4],
    pub padding: [Dim; 4],
    pub border_width: [f32; 4],
    pub border_style: [BorderStyle; 4],
    /// `None` means `currentcolor`.
    pub border_color: [Option<Color>; 4],
    pub border_radius: f32,
    pub shadows: Vec<Shadow>,

    pub color: Color,
    pub background_color: Color,
    pub background_image: BgImage,
    pub background_repeat: BgRepeat,
    pub background_size: BgSize,
    pub background_position: (Dim, Dim),

    pub font_size: f32,
    /// True while the size still comes from the default / a keyword (enables the 13px monospace quirk).
    pub font_size_keyword: bool,
    pub font_weight: u16,
    pub font_italic: bool,
    pub font_family: FontFamily,
    /// Unitless multiplier or `None` for `normal`.
    pub line_height_number: Option<f32>,
    pub line_height_px: Option<f32>,
    pub text_align: TextAlign,
    pub text_decoration: u8,
    pub text_decoration_own: u8,
    pub text_transform: TextTransform,
    pub text_indent: Dim,
    pub text_overflow_ellipsis: bool,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub white_space: WhiteSpace,
    pub word_break_all: bool,
    pub overflow_wrap_break: bool,
    pub vertical_align: VerticalAlign,
    pub rtl: bool,
    pub line_clamp: Option<u32>,

    pub list_style_type: ListStyleType,
    pub list_style_inside: bool,

    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Dim,
    pub order: i32,
    pub justify_content: Align,
    pub align_items: Align,
    pub align_self: Align,
    pub align_content: Align,
    pub justify_items: Align,
    pub justify_self: Align,
    pub row_gap: Dim,
    pub column_gap: Dim,

    pub grid_columns: TrackList,
    pub grid_rows: TrackList,
    pub grid_auto_columns: Vec<Track>,
    pub grid_auto_rows: Vec<Track>,
    pub grid_auto_flow_column: bool,
    pub grid_auto_flow_dense: bool,
    pub grid_areas: Vec<Vec<String>>,
    pub grid_column: (GridLine, GridLine),
    pub grid_row: (GridLine, GridLine),

    pub border_collapse: bool,
    pub border_spacing: (f32, f32),
    pub table_fixed: bool,
    pub caption_bottom: bool,

    pub content: Option<Vec<ContentItem>>,
    pub translate: (Dim, Dim),
    pub transform_scale: f32,

    pub vars: Rc<BTreeMap<String, String>>,
}

impl Style {
    pub fn initial() -> Style {
        Style {
            display: Display::Inline,
            position: Position::Static,
            float: Float::None,
            clear: Clear::None,
            top: Dim::Auto,
            right: Dim::Auto,
            bottom: Dim::Auto,
            left: Dim::Auto,
            z_index: None,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            visibility_visible: true,
            opacity: 1.0,
            pointer_events: true,
            width: Dim::Auto,
            height: Dim::Auto,
            min_width: Dim::Auto,
            min_height: Dim::Auto,
            max_width: Dim::Auto,
            max_height: Dim::Auto,
            border_box: false,
            aspect_ratio: None,
            margin: [Dim::ZERO, Dim::ZERO, Dim::ZERO, Dim::ZERO],
            padding: [Dim::ZERO, Dim::ZERO, Dim::ZERO, Dim::ZERO],
            border_width: [0.0; 4],
            border_style: [BorderStyle::None; 4],
            border_color: [None; 4],
            border_radius: 0.0,
            shadows: Vec::new(),
            color: color::BLACK,
            background_color: color::TRANSPARENT,
            background_image: BgImage::None,
            background_repeat: BgRepeat::Repeat,
            background_size: BgSize::Auto,
            background_position: (Dim::pct(0.0), Dim::pct(0.0)),
            font_size: 16.0,
            font_size_keyword: true,
            font_weight: 400,
            font_italic: false,
            font_family: FontFamily::Serif,
            line_height_number: None,
            line_height_px: None,
            text_align: TextAlign::Left,
            text_decoration: 0,
            text_decoration_own: 0,
            text_transform: TextTransform::None,
            text_indent: Dim::ZERO,
            text_overflow_ellipsis: false,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            white_space: WhiteSpace::Normal,
            word_break_all: false,
            overflow_wrap_break: false,
            vertical_align: VerticalAlign::Baseline,
            rtl: false,
            line_clamp: None,
            list_style_type: ListStyleType::Disc,
            list_style_inside: false,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Nowrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Dim::Auto,
            order: 0,
            justify_content: Align::Normal,
            align_items: Align::Normal,
            align_self: Align::Auto,
            align_content: Align::Normal,
            justify_items: Align::Normal,
            justify_self: Align::Auto,
            row_gap: Dim::ZERO,
            column_gap: Dim::ZERO,
            grid_columns: TrackList::default(),
            grid_rows: TrackList::default(),
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_auto_flow_column: false,
            grid_auto_flow_dense: false,
            grid_areas: Vec::new(),
            grid_column: (GridLine::Auto, GridLine::Auto),
            grid_row: (GridLine::Auto, GridLine::Auto),
            border_collapse: false,
            border_spacing: (0.0, 0.0),
            table_fixed: false,
            caption_bottom: false,
            content: None,
            translate: (Dim::ZERO, Dim::ZERO),
            transform_scale: 1.0,
            vars: Rc::new(BTreeMap::new()),
        }
    }

    /// Initial values with inherited properties copied from the parent.
    pub fn inherit_from(parent: &Style) -> Style {
        let mut s = Style::initial();
        s.color = parent.color;
        s.font_size = parent.font_size;
        s.font_size_keyword = parent.font_size_keyword;
        s.font_weight = parent.font_weight;
        s.font_italic = parent.font_italic;
        s.font_family = parent.font_family;
        s.line_height_number = parent.line_height_number;
        s.line_height_px = parent.line_height_px;
        s.text_align = parent.text_align;
        s.text_indent = parent.text_indent.clone();
        s.text_transform = parent.text_transform;
        s.text_overflow_ellipsis = false;
        s.letter_spacing = parent.letter_spacing;
        s.word_spacing = parent.word_spacing;
        s.white_space = parent.white_space;
        s.word_break_all = parent.word_break_all;
        s.overflow_wrap_break = parent.overflow_wrap_break;
        s.visibility_visible = parent.visibility_visible;
        s.pointer_events = parent.pointer_events;
        s.rtl = parent.rtl;
        s.list_style_type = parent.list_style_type;
        s.list_style_inside = parent.list_style_inside;
        s.border_collapse = parent.border_collapse;
        s.border_spacing = parent.border_spacing;
        s.caption_bottom = parent.caption_bottom;
        s.text_decoration = parent.text_decoration;
        s.vars = parent.vars.clone();
        s
    }

    pub fn is_positioned(&self) -> bool {
        self.position != Position::Static
    }

    pub fn is_out_of_flow(&self) -> bool {
        matches!(self.position, Position::Absolute | Position::Fixed)
    }

    /// The used line height in px for this element's own font.
    pub fn line_height(&self, normal: f32) -> f32 {
        if let Some(px) = self.line_height_px {
            px
        } else if let Some(n) = self.line_height_number {
            n * self.font_size
        } else {
            normal
        }
    }

    pub fn is_bfc_root(&self) -> bool {
        self.float != Float::None
            || self.is_out_of_flow()
            || self.overflow_x.clips()
            || self.overflow_y.clips()
            || matches!(
                self.display,
                Display::InlineBlock
                    | Display::FlowRoot
                    | Display::Flex
                    | Display::InlineFlex
                    | Display::Grid
                    | Display::InlineGrid
                    | Display::Table
                    | Display::InlineTable
                    | Display::TableCell
                    | Display::TableCaption
            )
    }
}

// ---------------------------------------------------------------------------
// Stylesheet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

/// A chain of `@media` conditions that must all hold.
#[derive(Debug)]
pub struct MediaChain {
    parent: Option<Rc<MediaChain>>,
    query: String,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selector: Complex,
    pub declarations: Rc<Vec<Declaration>>,
    pub order: usize,
    /// 0 for the UA sheet, 1 for author rules.
    pub origin: u8,
    /// Index of the cascade layer; `u16::MAX` for unlayered rules.
    pub layer: u16,
    pub media: Option<Rc<MediaChain>>,
}

#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    pub rules: Vec<Rule>,
    /// `@import` targets: (url, media query).
    pub imports: Vec<(String, String)>,
    layers: Vec<String>,
    audit: CssAudit,
}

/// Approximate feature count for diagnostics, not a standards conformance score.
#[derive(Debug, Clone, Copy, Default)]
pub struct CssAudit {
    pub ignored_selectors: usize,
    pub ignored_declarations: usize,
    pub at_rules: usize,
}

pub fn audit(source: &str) -> CssAudit {
    parse(source).audit
}

impl StyleSheet {
    pub fn audit(&self) -> CssAudit {
        self.audit
    }
}

pub fn parse(source: &str) -> StyleSheet {
    parse_origin(source, 1)
}

fn parse_origin(source: &str, origin: u8) -> StyleSheet {
    let source = remove_comments(source);
    let mut sheet = StyleSheet::default();
    parse_rules(&source, &mut sheet, origin, u16::MAX, None);
    sheet
}

fn remove_comments(source: &str) -> String {
    if !source.contains("/*") {
        return source.to_string();
    }
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut quote = 0u8;
    let mut last = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == quote {
                quote = 0;
            }
        } else if b == b'"' || b == b'\'' {
            quote = b;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            out.push_str(&source[last..i]);
            let end = source[i + 2..].find("*/").map_or(source.len(), |e| i + 2 + e + 2);
            out.push(' ');
            i = end;
            last = end;
            continue;
        }
        i += 1;
    }
    if last < source.len() {
        out.push_str(&source[last.min(source.len())..]);
    }
    out
}

/// Find the end of a `{ ... }` block starting at `open` (index of `{`).
fn block_end(source: &str, open: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut quote = 0u8;
    let mut paren = 0;
    let mut i = open;
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == quote {
                quote = 0;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = b,
                b'(' => paren += 1,
                b')' => paren -= 1,
                b'{' if paren <= 0 => depth += 1,
                b'}' if paren <= 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    bytes.len()
}

fn parse_rules(source: &str, sheet: &mut StyleSheet, origin: u8, layer: u16, media: Option<Rc<MediaChain>>) {
    let mut rest = source.trim_start();
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix("<!--").or_else(|| rest.strip_prefix("-->")) {
            rest = stripped.trim_start();
            continue;
        }
        // find prelude end: first top-level '{' or ';'
        let mut prelude_end = None;
        {
            let bytes = rest.as_bytes();
            let mut quote = 0u8;
            let mut paren = 0;
            let mut i = 0;
            while i < bytes.len() {
                let b = bytes[i];
                if quote != 0 {
                    if b == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b == quote {
                        quote = 0;
                    }
                } else {
                    match b {
                        b'"' | b'\'' => quote = b,
                        b'(' => paren += 1,
                        b')' => paren -= 1,
                        b'{' | b';' if paren <= 0 => {
                            prelude_end = Some(i);
                            break;
                        }
                        b'}' if paren <= 0 => {
                            // stray closing brace: drop it
                            prelude_end = Some(i);
                            break;
                        }
                        _ => {}
                    }
                }
                i += 1;
            }
        }
        let Some(pe) = prelude_end else { break };
        let prelude = rest[..pe].trim();
        match rest.as_bytes()[pe] {
            b'}' => {
                rest = rest[pe + 1..].trim_start();
                continue;
            }
            b';' => {
                handle_statement_at_rule(prelude, sheet);
                rest = rest[pe + 1..].trim_start();
                continue;
            }
            _ => {}
        }
        let end = block_end(rest, pe);
        let body = &rest[pe + 1..end.min(rest.len())];
        rest = if end < rest.len() { rest[end + 1..].trim_start() } else { "" };
        if let Some(at) = prelude.strip_prefix('@') {
            let name_end = at.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(at.len());
            let name = at[..name_end].to_ascii_lowercase();
            let params = at[name_end..].trim();
            match name.as_str() {
                "media" => {
                    let chain = Rc::new(MediaChain { parent: media.clone(), query: params.to_string() });
                    parse_rules(body, sheet, origin, layer, Some(chain));
                }
                "supports" => {
                    if eval_supports(params) {
                        parse_rules(body, sheet, origin, layer, media.clone());
                    }
                }
                "layer" => {
                    let full = params.to_string();
                    let idx = match sheet.layers.iter().position(|l| *l == full) {
                        Some(i) => i,
                        None => {
                            sheet.layers.push(full);
                            sheet.layers.len() - 1
                        }
                    } as u16;
                    parse_rules(body, sheet, origin, idx.min(60000), media.clone());
                }
                "document" | "-moz-document" | "scope" | "container" => {
                    sheet.audit.at_rules += 1;
                }
                _ => sheet.audit.at_rules += 1,
            }
            continue;
        }
        parse_style_rule(prelude, body, sheet, origin, layer, &media);
    }
}

fn handle_statement_at_rule(prelude: &str, sheet: &mut StyleSheet) {
    let Some(at) = prelude.strip_prefix('@') else { return };
    let name_end = at.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(at.len());
    let name = at[..name_end].to_ascii_lowercase();
    let params = at[name_end..].trim();
    match name.as_str() {
        "import" => {
            let parts = split_ws(params);
            if let Some(first) = parts.first() {
                let url = parse_url(first).or_else(|| parse_css_string(first));
                if let Some(url) = url {
                    let media = parts[1..]
                        .iter()
                        .filter(|p| !p.starts_with("layer") && !p.starts_with("supports"))
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" ");
                    sheet.imports.push((url, media));
                }
            }
        }
        "layer" => {
            for l in params.split(',') {
                let l = l.trim().to_string();
                if !l.is_empty() && !sheet.layers.contains(&l) {
                    sheet.layers.push(l);
                }
            }
        }
        "charset" | "namespace" => {}
        _ => sheet.audit.at_rules += 1,
    }
}

enum BodyItem {
    Decl(Declaration),
    Nested(String, String),
}

fn split_body(body: &str) -> Vec<BodyItem> {
    let mut items = Vec::new();
    let bytes = body.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut quote = 0u8;
    let mut paren = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if quote != 0 {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => quote = b,
            b'(' => paren += 1,
            b')' => paren -= 1,
            b';' if paren <= 0 => {
                push_decl(&body[start..i], &mut items);
                start = i + 1;
            }
            b'{' if paren <= 0 => {
                let end = block_end(body, i);
                let prelude = body[start..i].trim().to_string();
                let inner = body[i + 1..end.min(body.len())].to_string();
                // A custom property may hold a `{}` block; keep it as a declaration.
                if prelude.starts_with("--") && prelude.contains(':') {
                    push_decl(&body[start..(end + 1).min(body.len())], &mut items);
                } else {
                    items.push(BodyItem::Nested(prelude, inner));
                }
                i = end;
                start = end + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < body.len() {
        push_decl(&body[start..], &mut items);
    }
    items
}

fn push_decl(part: &str, items: &mut Vec<BodyItem>) {
    let Some((name, value)) = part.split_once(':') else { return };
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return;
    }
    let name = if name.starts_with("--") { name.to_string() } else { name.to_ascii_lowercase() };
    let mut value = value.trim();
    let mut important = false;
    if let Some(idx) = value.rfind('!') {
        if value[idx + 1..].trim().eq_ignore_ascii_case("important") {
            important = true;
            value = value[..idx].trim_end();
        }
    }
    if value.is_empty() && !name.starts_with("--") {
        return;
    }
    items.push(BodyItem::Decl(Declaration { name, value: value.to_string(), important }));
}

pub fn parse_declarations(body: &str) -> Vec<Declaration> {
    split_body(body)
        .into_iter()
        .filter_map(|i| match i {
            BodyItem::Decl(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn parse_style_rule(prelude: &str, body: &str, sheet: &mut StyleSheet, origin: u8, layer: u16, media: &Option<Rc<MediaChain>>) {
    let Some(list) = selector::parse_list(prelude) else {
        sheet.audit.ignored_selectors += 1;
        return;
    };
    let items = split_body(body);
    let mut decls = Vec::new();
    let mut nested = Vec::new();
    for item in items {
        match item {
            BodyItem::Decl(d) => decls.push(d),
            BodyItem::Nested(p, b) => nested.push((p, b)),
        }
    }
    let decls = Rc::new(decls);
    for complex in list {
        sheet.rules.push(Rule {
            selector: complex,
            declarations: decls.clone(),
            order: sheet.rules.len(),
            origin,
            layer,
            media: media.clone(),
        });
    }
    // CSS nesting: `&` stands for `:is(parent)`.
    for (p, b) in nested {
        let parent = prelude.to_string();
        if let Some(at) = p.strip_prefix('@') {
            let name_end = at.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(at.len());
            let name = at[..name_end].to_ascii_lowercase();
            let params = at[name_end..].trim();
            let wrapped = alloc::format!("{parent} {{ {b} }}");
            match name.as_str() {
                "media" => {
                    let chain = Rc::new(MediaChain { parent: media.clone(), query: params.to_string() });
                    parse_rules(&wrapped, sheet, origin, layer, Some(chain));
                }
                "supports" if eval_supports(params) => parse_rules(&wrapped, sheet, origin, layer, media.clone()),
                _ => {}
            }
            continue;
        }
        let combined: Vec<String> = split_commas(&p)
            .into_iter()
            .map(|sel| {
                let is_parent = alloc::format!(":is({parent})");
                if sel.contains('&') {
                    sel.replace('&', &is_parent)
                } else {
                    alloc::format!("{is_parent} {sel}")
                }
            })
            .collect();
        parse_style_rule(&combined.join(", "), &b, sheet, origin, layer, media);
    }
}

// ---- @supports -----------------------------------------------------------

fn supported_property(name: &str) -> bool {
    matches!(
        name,
        "display" | "position" | "float" | "clear" | "top" | "right" | "bottom" | "left" | "inset" | "z-index"
            | "overflow" | "overflow-x" | "overflow-y" | "visibility" | "opacity" | "width" | "height"
            | "min-width" | "min-height" | "max-width" | "max-height" | "box-sizing" | "margin" | "padding"
            | "border" | "border-radius" | "color" | "background" | "background-color" | "background-image"
            | "font" | "font-size" | "font-weight" | "font-style" | "font-family" | "line-height" | "text-align"
            | "text-decoration" | "text-transform" | "text-indent" | "letter-spacing" | "word-spacing"
            | "white-space" | "vertical-align" | "list-style" | "flex" | "flex-direction" | "flex-wrap"
            | "flex-grow" | "flex-shrink" | "flex-basis" | "flex-flow" | "justify-content" | "align-items"
            | "align-self" | "align-content" | "order" | "gap" | "row-gap" | "column-gap" | "grid"
            | "grid-template-columns" | "grid-template-rows" | "grid-template-areas" | "grid-area"
            | "grid-column" | "grid-row" | "grid-auto-flow" | "grid-gap" | "place-items" | "place-content"
            | "place-self" | "aspect-ratio" | "table-layout" | "border-collapse" | "border-spacing"
            | "content" | "pointer-events" | "box-shadow" | "word-break" | "overflow-wrap" | "text-overflow"
            | "inset-inline" | "inset-block" | "margin-inline" | "margin-block" | "padding-inline"
            | "padding-block" | "-webkit-line-clamp" | "transform" | "translate"
    ) || name.starts_with("--")
}

fn eval_supports(cond: &str) -> bool {
    let c = cond.trim();
    if let Some(rest) = c.strip_prefix("not") {
        return !eval_supports(rest.trim_start());
    }
    // split on top-level ` or ` / ` and `
    for (word, is_or) in [(" or ", true), (" and ", false)] {
        let mut parts = Vec::new();
        let mut depth = 0;
        let mut start = 0;
        let bytes = c.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth == 0 && c[i..].starts_with(word) {
                parts.push(&c[start..i]);
                start = i + word.len();
                i += word.len();
                continue;
            }
            i += 1;
        }
        if !parts.is_empty() {
            parts.push(&c[start..]);
            return if is_or {
                parts.iter().any(|p| eval_supports(p))
            } else {
                parts.iter().all(|p| eval_supports(p))
            };
        }
    }
    if let Some(inner) = c.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        if let Some((name, value)) = inner.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_ascii_lowercase();
            if name == "display" {
                return matches!(
                    value.as_str(),
                    "block" | "inline" | "inline-block" | "flex" | "inline-flex" | "grid" | "inline-grid" | "none"
                        | "table" | "contents" | "flow-root" | "list-item"
                );
            }
            if name == "position" {
                return matches!(value.as_str(), "static" | "relative" | "absolute" | "fixed" | "sticky" | "-webkit-sticky");
            }
            return supported_property(&name);
        }
        return eval_supports(inner);
    }
    if let Some(inner) = strip_function(c, "selector") {
        return selector::parse_list(inner).is_some();
    }
    false
}

// ---- media queries -------------------------------------------------------

fn media_matches(query: &str, vp: Viewport) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    split_commas(query).into_iter().any(|alt| media_alternative(alt, vp))
}

fn media_alternative(alt: &str, vp: Viewport) -> bool {
    let lower = alt.trim().to_ascii_lowercase();
    let mut rest = lower.as_str();
    let mut negate = false;
    if let Some(r) = rest.strip_prefix("not ") {
        negate = true;
        rest = r.trim_start();
    } else if let Some(r) = rest.strip_prefix("only ") {
        rest = r.trim_start();
    }
    let mut ok = true;
    // media type
    let first_len = rest.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(rest.len());
    let first = &rest[..first_len];
    if !first.is_empty() {
        match first {
            "screen" | "all" => rest = rest[first_len..].trim_start(),
            "print" | "speech" | "tty" | "tv" | "projection" | "handheld" | "braille" | "embossed" | "aural" => {
                ok = false;
                rest = rest[first_len..].trim_start();
            }
            _ if first.starts_with('(') => {}
            _ => ok = false,
        }
    }
    if let Some(r) = rest.strip_prefix("and ") {
        rest = r.trim_start();
    }
    // conditions: (a) and (b) ...
    let conds = split_media_conditions(rest);
    for cond in conds {
        if !media_condition(&cond, vp) {
            ok = false;
        }
    }
    ok != negate
}

fn split_media_conditions(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = None;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0
                    && let Some(s) = start.take()
                {
                    out.push(rest[s + 1..i].trim().to_string());
                }
            }
            _ => {}
        }
    }
    out
}

fn media_condition(cond: &str, vp: Viewport) -> bool {
    let ctx = LenCtx { font_size: 16.0, root_font_size: 16.0, vw: vp.width, vh: vp.height };
    if cond.starts_with('(') || cond.starts_with("not ") {
        // nested condition list, e.g. "(a) or (b)"
        return false;
    }
    let len = |v: &str| parse_px(v.trim(), &ctx);
    if let Some((name, value)) = cond.split_once(':') {
        let name = name.trim();
        let value = value.trim();
        return match name {
            "width" => len(value).is_some_and(|v| (vp.width - v).abs() < 0.5),
            "min-width" => len(value).is_some_and(|v| vp.width >= v),
            "max-width" => len(value).is_some_and(|v| vp.width <= v),
            "height" => len(value).is_some_and(|v| (vp.height - v).abs() < 0.5),
            "min-height" => len(value).is_some_and(|v| vp.height >= v),
            "max-height" => len(value).is_some_and(|v| vp.height <= v),
            "orientation" => (value == "landscape") == (vp.width >= vp.height),
            "prefers-color-scheme" => value == "light",
            "prefers-reduced-motion" => value == "no-preference",
            "prefers-contrast" => value == "no-preference",
            "prefers-reduced-transparency" => value == "no-preference",
            "forced-colors" | "inverted-colors" => value == "none",
            "hover" | "any-hover" => value == "hover",
            "pointer" | "any-pointer" => value == "fine",
            "scripting" => value == "enabled",
            "update" => value == "fast",
            "display-mode" => value == "browser",
            "color" => value.parse::<i32>().is_ok_and(|v| v <= 8),
            "min-color" => value.parse::<i32>().is_ok_and(|v| v <= 8),
            "min-resolution" | "-webkit-min-device-pixel-ratio" | "min--moz-device-pixel-ratio" => {
                let v = value.trim_end_matches("dppx").trim_end_matches('x');
                v.parse::<f32>().is_ok_and(|v| v <= 1.0)
            }
            "max-resolution" | "-webkit-max-device-pixel-ratio" => {
                let v = value.trim_end_matches("dppx").trim_end_matches('x');
                v.parse::<f32>().is_ok_and(|v| v >= 1.0)
            }
            "aspect-ratio" | "min-aspect-ratio" | "max-aspect-ratio" => {
                let (a, b) = value.split_once('/').unwrap_or((value, "1"));
                match (a.trim().parse::<f32>(), b.trim().parse::<f32>()) {
                    (Ok(a), Ok(b)) if b > 0.0 => {
                        let want = a / b;
                        let have = vp.width / vp.height;
                        match name {
                            "min-aspect-ratio" => have >= want,
                            "max-aspect-ratio" => have <= want,
                            _ => (have - want).abs() < 0.01,
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        };
    }
    // boolean context and range syntax
    match cond {
        "color" | "hover" | "any-hover" | "pointer" | "any-pointer" | "scripting" => return true,
        "monochrome" | "grid" => return false,
        _ => {}
    }
    for op in ["<=", ">=", "<", ">", "="] {
        if cond.contains(op) {
            // could be `a op feature op b`
            let parts: Vec<&str> = cond.split(op).collect();
            let feature_value = |name: &str| match name.trim() {
                "width" => Some(vp.width),
                "height" => Some(vp.height),
                "aspect-ratio" => Some(vp.width / vp.height),
                _ => None,
            };
            let cmp = |a: f32, op: &str, b: f32| match op {
                "<=" => a <= b,
                ">=" => a >= b,
                "<" => a < b,
                ">" => a > b,
                _ => (a - b).abs() < 0.5,
            };
            // Re-split to support double comparisons
            let mut tokens: Vec<&str> = Vec::new();
            let mut ops: Vec<&str> = Vec::new();
            let mut i = 0;
            let bytes = cond.as_bytes();
            let mut start = 0;
            while i < bytes.len() {
                let two = cond.get(i..i + 2);
                let one = cond.get(i..i + 1);
                if matches!(two, Some("<=") | Some(">=")) {
                    tokens.push(cond[start..i].trim());
                    ops.push(&cond[i..i + 2]);
                    i += 2;
                    start = i;
                } else if matches!(one, Some("<") | Some(">") | Some("=")) {
                    tokens.push(cond[start..i].trim());
                    ops.push(&cond[i..i + 1]);
                    i += 1;
                    start = i;
                } else {
                    i += 1;
                }
            }
            tokens.push(cond[start..].trim());
            let _ = parts;
            let eval_token = |t: &str| feature_value(t).or_else(|| len(t));
            return match tokens.len() {
                2 => match (eval_token(tokens[0]), eval_token(tokens[1])) {
                    (Some(a), Some(b)) => cmp(a, ops[0], b),
                    _ => false,
                },
                3 => match (eval_token(tokens[0]), eval_token(tokens[1]), eval_token(tokens[2])) {
                    (Some(a), Some(b), Some(c)) => cmp(a, ops[0], b) && cmp(b, ops[1], c),
                    _ => false,
                },
                _ => false,
            };
        }
    }
    false
}

fn chain_matches(chain: &Option<Rc<MediaChain>>, vp: Viewport, memo: &mut BTreeMap<usize, bool>) -> bool {
    let Some(c) = chain else { return true };
    let key = Rc::as_ptr(c) as usize;
    if let Some(&v) = memo.get(&key) {
        return v;
    }
    let v = media_matches(&c.query, vp) && chain_matches(&c.parent, vp, memo);
    memo.insert(key, v);
    v
}

// ---------------------------------------------------------------------------
// Cascade
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct RuleIndex {
    by_id: BTreeMap<String, Vec<u32>>,
    by_class: BTreeMap<String, Vec<u32>>,
    by_tag: BTreeMap<String, Vec<u32>>,
    universal: Vec<u32>,
}

/// Rules from all sheets that apply at a given viewport, ready for lookup.
pub struct ActiveRules<'a> {
    rules: Vec<&'a Rule>,
    index: RuleIndex,
}

impl<'a> ActiveRules<'a> {
    pub fn new(sheets: &[&'a StyleSheet], vp: Viewport) -> Self {
        let mut rules = Vec::new();
        let mut memo = BTreeMap::new();
        let mut order = 0usize;
        let _ = &mut order;
        for sheet in sheets {
            for rule in &sheet.rules {
                if chain_matches(&rule.media, vp, &mut memo) {
                    rules.push(rule);
                }
            }
        }
        let mut index = RuleIndex::default();
        for (i, rule) in rules.iter().enumerate() {
            let last = rule.selector.compounds.last().expect("selector has a compound");
            let i = i as u32;
            if let Some(id) = &last.id {
                index.by_id.entry(id.clone()).or_default().push(i);
            } else if let Some(class) = last.classes.first() {
                index.by_class.entry(class.clone()).or_default().push(i);
            } else if let Some(tag) = &last.tag {
                index.by_tag.entry(tag.clone()).or_default().push(i);
            } else {
                index.universal.push(i);
            }
        }
        Self { rules, index }
    }

    fn candidates(&self, doc: &Document, node: NodeId) -> Vec<u32> {
        let n = &doc.nodes[node];
        let mut out: Vec<u32> = self.index.universal.clone();
        if let Some(id) = n.attribute("id")
            && let Some(v) = self.index.by_id.get(id)
        {
            out.extend_from_slice(v);
        }
        if let Some(classes) = n.attribute("class") {
            for c in classes.split_ascii_whitespace() {
                if let Some(v) = self.index.by_class.get(c) {
                    out.extend_from_slice(v);
                }
            }
        }
        if let Some(tag) = n.tag()
            && let Some(v) = self.index.by_tag.get(tag)
        {
            out.extend_from_slice(v);
        }
        out
    }
}

type Cascaded<'a> = Vec<((u8, u32, u32, usize, usize), &'a Declaration)>;

fn cascade_key(important: bool, origin: u8, layer: u16, specificity: u32, order: usize, index: usize) -> (u8, u32, u32, usize, usize) {
    // Author unlayered > author layered > hints > UA; reversed for !important.
    let rank: u32 = match origin {
        0 => 0,
        2 => 1,
        _ => {
            if layer == u16::MAX {
                70000
            } else {
                2 + layer as u32
            }
        }
    };
    let rank = if important { 100_000 - rank } else { rank };
    (important as u8, rank, specificity, order, index)
}

/// Computed styles for the whole document, indexed by node id.
pub struct Styles {
    pub nodes: Vec<Style>,
    pub before: Vec<Option<Style>>,
    pub after: Vec<Option<Style>>,
}

impl core::ops::Index<usize> for Styles {
    type Output = Style;
    fn index(&self, i: usize) -> &Style {
        &self.nodes[i]
    }
}

const UA_CSS: &str = include_str!("ua.css");

pub fn compute_styles(doc: &Document, author: &StyleSheet, vp: Viewport) -> Styles {
    let ua = parse_origin(UA_CSS, 0);
    let active = ActiveRules::new(&[&ua, author], vp);
    compute_with_rules(doc, &active, vp)
}

pub fn compute_with_rules(doc: &Document, active: &ActiveRules, vp: Viewport) -> Styles {
    let n = doc.nodes.len();
    let mut out = Styles { nodes: vec![Style::initial(); 0], before: vec![None; n], after: vec![None; n] };
    out.nodes = (0..n).map(|_| Style::initial()).collect();
    for s in &mut out.nodes {
        s.display = Display::None;
    }
    let mut computed = vec![false; n];
    // Iterative DFS from the root so parents are always computed first.
    let mut stack = vec![doc.root];
    let mut root_font = 16.0;
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        match &node.kind {
            NodeKind::Document => {
                let mut s = Style::initial();
                s.display = Display::Block;
                out.nodes[id] = s;
            }
            NodeKind::Text(_) => {
                let parent = node.parent.map(|p| &out.nodes[p]);
                let mut s = parent.map_or_else(Style::initial, Style::inherit_from);
                s.display = Display::Inline;
                out.nodes[id] = s;
            }
            NodeKind::Element { .. } => {
                let parent = node.parent.map(|p| out.nodes[p].clone());
                let (style, before, after) = compute_element(doc, id, parent.as_ref(), active, vp, root_font);
                if doc.nodes[id].tag() == Some("html") {
                    root_font = style.font_size;
                }
                out.nodes[id] = style;
                out.before[id] = before;
                out.after[id] = after;
            }
        }
        computed[id] = true;
        for &c in node.children.iter().rev() {
            if !computed[c] {
                stack.push(c);
            }
        }
    }
    out
}

fn compute_element(
    doc: &Document,
    id: NodeId,
    parent: Option<&Style>,
    active: &ActiveRules,
    vp: Viewport,
    root_font: f32,
) -> (Style, Option<Style>, Option<Style>) {
    let node = &doc.nodes[id];
    let mut main: Cascaded = Vec::new();
    let mut before: Cascaded = Vec::new();
    let mut after: Cascaded = Vec::new();
    let hints = crate::presentation::hints(doc, id);
    for (i, d) in hints.iter().enumerate() {
        main.push((cascade_key(false, 2, 0, 0, 0, i), d));
    }
    for r in active.candidates(doc, id) {
        let rule = active.rules[r as usize];
        if !selector::matches_complex_ignoring_pseudo_element(doc, id, &rule.selector) {
            continue;
        }
        let target = match rule.selector.pseudo_element {
            None => &mut main,
            Some(PseudoElement::Before) => &mut before,
            Some(PseudoElement::After) => &mut after,
            Some(_) => continue,
        };
        for (i, d) in rule.declarations.iter().enumerate() {
            target.push((
                cascade_key(d.important, rule.origin, rule.layer, rule.selector.specificity, rule.order, i),
                d,
            ));
        }
    }
    let inline = node.attribute("style").map(parse_declarations).unwrap_or_default();
    for (i, d) in inline.iter().enumerate() {
        main.push((cascade_key(d.important, 1, u16::MAX, u32::MAX, usize::MAX - 1, i), d));
    }
    main.sort_by_key(|e| e.0);
    let style = build_style(parent, &main, vp, root_font, node.tag(), false);
    let make_pseudo = |mut list: Cascaded, is_before: bool| -> Option<Style> {
        let _ = is_before;
        if list.is_empty() {
            return None;
        }
        list.sort_by_key(|e| e.0);
        let s = build_style(Some(&style), &list, vp, root_font, None, true);
        s.content.as_ref()?;
        Some(s)
    };
    let b = make_pseudo(before, true);
    let a = make_pseudo(after, false);
    (style, b, a)
}

fn build_style(parent: Option<&Style>, decls: &Cascaded, vp: Viewport, root_font: f32, tag: Option<&str>, pseudo: bool) -> Style {
    let mut style = parent.map_or_else(Style::initial, Style::inherit_from);
    if pseudo {
        style.display = Display::Inline;
    }
    let _ = tag;
    // Custom properties first.
    let mut vars: Option<BTreeMap<String, String>> = None;
    for (_, d) in decls {
        if d.name.starts_with("--") {
            let v = vars.get_or_insert_with(|| (*style.vars).clone());
            v.insert(d.name.clone(), d.value.clone());
        }
    }
    if let Some(v) = vars {
        style.vars = Rc::new(v);
    }
    let vars = style.vars.clone();
    let init = Style::initial();
    let resolve = |d: &Declaration| -> Option<String> {
        substitute_vars(&d.value, &vars, 0)
    };
    // font-size and the `font` shorthand first so em units see the right size.
    let parent_font = parent.map_or(16.0, |p| p.font_size);
    for (_, d) in decls {
        if d.name == "font-size" || d.name == "font" {
            if let Some(v) = resolve(d) {
                apply(&mut style, &d.name, &v, parent, &init, parent_font, root_font, vp);
            }
        }
    }
    for (_, d) in decls {
        if d.name.starts_with("--") || d.name == "font-size" {
            continue;
        }
        if let Some(v) = resolve(d) {
            apply(&mut style, &d.name, &v, parent, &init, parent_font, root_font, vp);
        }
    }
    // text-decoration propagates to inline descendants.
    let parent_deco = parent.map_or(0, |p| p.text_decoration);
    style.text_decoration = style.text_decoration_own;
    let isolates = style.float != Float::None
        || style.is_out_of_flow()
        || matches!(
            style.display,
            Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable
        );
    if !isolates {
        style.text_decoration |= parent_deco;
    }
    // Blockification and display adjustments.
    if (style.float != Float::None || style.is_out_of_flow()) && style.display != Display::None && style.display != Display::Contents {
        style.display = style.display.blockify();
    }
    style
}

fn edge4(v: &[Dim]) -> Option<[Dim; 4]> {
    match v {
        [a] => Some([a.clone(), a.clone(), a.clone(), a.clone()]),
        [a, b] => Some([a.clone(), b.clone(), a.clone(), b.clone()]),
        [a, b, c] => Some([a.clone(), b.clone(), c.clone(), b.clone()]),
        [a, b, c, d] => Some([a.clone(), b.clone(), c.clone(), d.clone()]),
        _ => None,
    }
}

fn parse_dims(value: &str, ctx: &LenCtx) -> Option<Vec<Dim>> {
    split_ws(value).into_iter().map(|t| parse_dim(t, ctx)).collect()
}

fn border_style_kw(v: &str) -> Option<BorderStyle> {
    Some(match v {
        "none" => BorderStyle::None,
        "hidden" => BorderStyle::Hidden,
        "solid" | "ridge" | "groove" | "inset" | "outset" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        _ => return None,
    })
}

fn parse_color_in(v: &str, current: Color) -> Option<Option<Color>> {
    if v.eq_ignore_ascii_case("currentcolor") {
        return Some(None);
    }
    let _ = current;
    color::parse_color(v).map(Some)
}

fn parse_align(v: &str) -> Option<Align> {
    Some(match v {
        "auto" => Align::Auto,
        "normal" => Align::Normal,
        "stretch" => Align::Stretch,
        "start" | "self-start" | "left" => Align::Start,
        "end" | "self-end" | "right" => Align::End,
        "flex-start" => Align::FlexStart,
        "flex-end" => Align::FlexEnd,
        "center" => Align::Center,
        "baseline" | "first baseline" | "last baseline" => Align::Baseline,
        "space-between" => Align::SpaceBetween,
        "space-around" => Align::SpaceAround,
        "space-evenly" => Align::SpaceEvenly,
        _ => return None,
    })
}

fn parse_tracks(value: &str, ctx: &LenCtx) -> Option<TrackList> {
    let mut list = TrackList::default();
    let v = value.trim();
    if v == "none" || v.is_empty() {
        return Some(list);
    }
    let mut seen_repeat = false;
    for tok in split_ws(v) {
        if let Some(name) = tok.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            if !seen_repeat {
                for n in name.split_whitespace() {
                    list.names.push((n.to_string(), list.before.len()));
                }
            }
            continue;
        }
        if let Some(inner) = strip_function(tok, "repeat") {
            let args = split_commas(inner);
            if args.len() < 2 {
                return None;
            }
            let mut inner_tracks = Vec::new();
            for t in split_ws(args[1]) {
                if t.starts_with('[') {
                    continue;
                }
                inner_tracks.push(parse_track(t, ctx)?);
            }
            match args[0].trim() {
                "auto-fill" | "auto-fit" => {
                    list.repeat = Some((args[0].trim() == "auto-fit", inner_tracks));
                    seen_repeat = true;
                }
                n => {
                    let count: usize = n.parse().ok()?;
                    for _ in 0..count.min(1000) {
                        let target = if seen_repeat { &mut list.after } else { &mut list.before };
                        target.extend(inner_tracks.iter().cloned());
                    }
                }
            }
            continue;
        }
        let t = parse_track(tok, ctx)?;
        if seen_repeat {
            list.after.push(t);
        } else {
            list.before.push(t);
        }
    }
    Some(list)
}

fn parse_track(tok: &str, ctx: &LenCtx) -> Option<Track> {
    let t = tok.trim();
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "auto" => return Some(Track::Auto),
        "min-content" => return Some(Track::MinContent),
        "max-content" => return Some(Track::MaxContent),
        _ => {}
    }
    if let Some(fr) = lower.strip_suffix("fr") {
        return fr.trim().parse::<f32>().ok().map(Track::Fr);
    }
    if let Some(inner) = strip_function(t, "minmax") {
        let args = split_commas(inner);
        if args.len() != 2 {
            return None;
        }
        return Some(Track::MinMax(Box::new(parse_track(args[0], ctx)?), Box::new(parse_track(args[1], ctx)?)));
    }
    if let Some(inner) = strip_function(t, "fit-content") {
        return parse_px(inner, ctx).map(Track::FitContent);
    }
    match parse_dim(t, ctx)? {
        Dim::Len(px, pct) if pct == 0.0 => Some(Track::Px(px)),
        Dim::Len(px, pct) if px == 0.0 => Some(Track::Pct(pct)),
        Dim::Len(px, _) => Some(Track::Px(px)),
        _ => None,
    }
}

fn parse_grid_line(v: &str) -> GridLine {
    let v = v.trim();
    if v == "auto" || v.is_empty() {
        return GridLine::Auto;
    }
    if let Some(n) = v.strip_prefix("span") {
        let n = n.trim();
        return GridLine::Span(n.parse().unwrap_or(1));
    }
    if let Ok(n) = v.parse::<i32>() {
        return GridLine::Line(n);
    }
    GridLine::Name(v.to_string())
}

fn parse_grid_pair(v: &str) -> (GridLine, GridLine) {
    let parts: Vec<&str> = v.split('/').collect();
    let start = parse_grid_line(parts[0]);
    let end = if parts.len() > 1 {
        parse_grid_line(parts[1])
    } else if let GridLine::Name(n) = &start {
        GridLine::Name(n.clone())
    } else {
        GridLine::Auto
    };
    (start, end)
}

fn parse_font_family(v: &str) -> FontFamily {
    for part in split_commas(v) {
        let p = part.trim().trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase();
        match p.as_str() {
            "monospace" | "ui-monospace" | "courier" | "courier new" | "consolas" | "menlo" | "monaco" | "sf mono"
            | "lucida console" | "dejavu sans mono" | "liberation mono" | "fira code" | "fira mono" | "source code pro"
            | "roboto mono" | "jetbrains mono" | "cascadia code" => return FontFamily::Monospace,
            "serif" | "ui-serif" | "times" | "times new roman" | "georgia" | "garamond" | "palatino" | "cambria"
            | "book antiqua" | "baskerville" | "liberation serif" | "dejavu serif" | "noto serif" => return FontFamily::Serif,
            "sans-serif" | "system-ui" | "ui-sans-serif" | "-apple-system" | "blinkmacsystemfont" | "arial" | "helvetica"
            | "helvetica neue" | "verdana" | "tahoma" | "trebuchet ms" | "segoe ui" | "roboto" | "open sans" | "lato"
            | "inter" | "ubuntu" | "liberation sans" | "dejavu sans" | "noto sans" | "calibri" | "sans" | "cantarell"
            | "oxygen" | "fira sans" | "source sans pro" | "montserrat" | "poppins" | "nunito" | "arimo" => {
                return FontFamily::Sans
            }
            _ => {}
        }
    }
    FontFamily::Serif
}

fn font_size_keyword(v: &str, parent_font: f32) -> Option<f32> {
    Some(match v {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "xxx-large" => 48.0,
        "larger" => parent_font * 1.2,
        "smaller" => parent_font / 1.2,
        _ => return None,
    })
}

fn parse_font_weight(v: &str, parent: u16) -> Option<u16> {
    Some(match v {
        "normal" => 400,
        "bold" => 700,
        "bolder" => {
            if parent < 400 {
                400
            } else if parent < 600 {
                700
            } else {
                900
            }
        }
        "lighter" => {
            if parent > 700 {
                700
            } else if parent > 500 {
                400
            } else {
                100
            }
        }
        n => n.parse::<f32>().ok()?.clamp(1.0, 1000.0) as u16,
    })
}

fn parse_shadows(value: &str, ctx: &LenCtx, current: Color) -> Vec<Shadow> {
    let mut out = Vec::new();
    if value.trim() == "none" {
        return out;
    }
    for layer in split_commas(value) {
        let mut nums = Vec::new();
        let mut col = None;
        let mut inset = false;
        for tok in split_ws(layer) {
            if tok == "inset" {
                inset = true;
            } else if let Some(px) = parse_px(tok, ctx) {
                nums.push(px);
            } else if tok.eq_ignore_ascii_case("currentcolor") {
                col = Some(current);
            } else if let Some(c) = color::parse_color(tok) {
                col = Some(c);
            }
        }
        if nums.len() >= 2 {
            out.push(Shadow {
                dx: nums[0],
                dy: nums[1],
                blur: nums.get(2).copied().unwrap_or(0.0).max(0.0),
                spread: nums.get(3).copied().unwrap_or(0.0),
                color: col.unwrap_or(current),
                inset,
            });
        }
    }
    out
}

fn parse_content(value: &str) -> Option<Vec<ContentItem>> {
    let v = value.trim();
    if v == "none" || v == "normal" {
        return None;
    }
    let mut items = Vec::new();
    for tok in split_ws(v) {
        if let Some(s) = parse_css_string(tok) {
            items.push(ContentItem::Text(s));
        } else if let Some(inner) = strip_function(tok, "attr") {
            items.push(ContentItem::Attr(inner.trim().to_ascii_lowercase()));
        } else if tok == "open-quote" {
            items.push(ContentItem::Text("\u{201c}".to_string()));
        } else if tok == "close-quote" {
            items.push(ContentItem::Text("\u{201d}".to_string()));
        } else if tok == "no-open-quote" || tok == "no-close-quote" {
        } else {
            // counters, images: unsupported pieces render nothing
        }
    }
    Some(items)
}

struct BgParse {
    color: Option<Color>,
    image: Option<BgImage>,
    repeat: Option<BgRepeat>,
    size: Option<BgSize>,
    position: Option<(Dim, Dim)>,
}

fn parse_position(tokens: &[&str], ctx: &LenCtx) -> Option<(Dim, Dim)> {
    let kw = |t: &str| -> Option<Dim> {
        Some(match t {
            "left" | "top" => Dim::pct(0.0),
            "center" => Dim::pct(50.0),
            "right" | "bottom" => Dim::pct(100.0),
            _ => return None,
        })
    };
    match tokens {
        [a] => match *a {
            "top" | "bottom" => Some((Dim::pct(50.0), kw(a)?)),
            _ => Some((kw(a).or_else(|| parse_dim(a, ctx))?, Dim::pct(50.0))),
        },
        [a, b] => {
            let vertical_first = matches!(*a, "top" | "bottom") || matches!(*b, "left" | "right");
            let (x, y) = if vertical_first { (b, a) } else { (a, b) };
            Some((kw(x).or_else(|| parse_dim(x, ctx))?, kw(y).or_else(|| parse_dim(y, ctx))?))
        }
        _ => None,
    }
}

fn parse_background_layer(layer: &str, ctx: &LenCtx, current: Color) -> BgParse {
    let mut out = BgParse { color: None, image: None, repeat: None, size: None, position: None };
    let mut pos_tokens: Vec<&str> = Vec::new();
    let mut size_tokens: Vec<&str> = Vec::new();
    let mut after_slash = false;
    for tok in split_ws(layer) {
        let lower = tok.to_ascii_lowercase();
        if tok == "/" {
            after_slash = true;
            continue;
        }
        if let Some(rest) = tok.strip_prefix('/') {
            after_slash = true;
            if !rest.is_empty() {
                size_tokens.push(rest);
            }
            continue;
        }
        if after_slash {
            if matches!(lower.as_str(), "cover" | "contain" | "auto") || parse_dim(tok, ctx).is_some() {
                size_tokens.push(tok);
                continue;
            }
            after_slash = false;
        }
        match lower.as_str() {
            "none" => out.image = Some(BgImage::None),
            "repeat" => out.repeat = Some(BgRepeat::Repeat),
            "no-repeat" => out.repeat = Some(BgRepeat::NoRepeat),
            "repeat-x" => out.repeat = Some(BgRepeat::RepeatX),
            "repeat-y" => out.repeat = Some(BgRepeat::RepeatY),
            "space" | "round" => out.repeat = Some(BgRepeat::Repeat),
            "scroll" | "fixed" | "local" | "border-box" | "padding-box" | "content-box" | "text" => {}
            "currentcolor" => out.color = Some(current),
            "left" | "right" | "top" | "bottom" | "center" => pos_tokens.push(tok),
            _ => {
                if let Some(url) = parse_url(tok) {
                    out.image = Some(BgImage::Url(url));
                } else if let Some(g) = parse_gradient(tok, ctx) {
                    out.image = Some(BgImage::Gradient(g));
                } else if let Some(c) = color::parse_color(tok) {
                    out.color = Some(c);
                } else if parse_dim(tok, ctx).is_some() {
                    pos_tokens.push(tok);
                }
            }
        }
    }
    if !pos_tokens.is_empty() {
        out.position = parse_position(&pos_tokens, ctx);
    }
    if !size_tokens.is_empty() {
        out.size = Some(match size_tokens[0] {
            "cover" => BgSize::Cover,
            "contain" => BgSize::Contain,
            first => {
                let w = parse_dim(first, ctx).unwrap_or(Dim::Auto);
                let h = size_tokens.get(1).and_then(|t| parse_dim(t, ctx)).unwrap_or(Dim::Auto);
                if w.is_auto() && h.is_auto() { BgSize::Auto } else { BgSize::Explicit(w, h) }
            }
        });
    }
    out
}

/// Copy a single property from `src` (used for `inherit`, `initial` and `unset`).
fn copy_property(dst: &mut Style, src: &Style, name: &str) {
    macro_rules! cp {
        ($($field:ident),+) => {{ $( dst.$field = src.$field.clone(); )+ }};
    }
    match name {
        "display" => cp!(display),
        "position" => cp!(position),
        "float" => cp!(float),
        "clear" => cp!(clear),
        "top" => cp!(top),
        "right" => cp!(right),
        "bottom" => cp!(bottom),
        "left" => cp!(left),
        "z-index" => cp!(z_index),
        "overflow" => {
            cp!(overflow_x);
            cp!(overflow_y)
        }
        "overflow-x" => cp!(overflow_x),
        "overflow-y" => cp!(overflow_y),
        "visibility" => cp!(visibility_visible),
        "opacity" => cp!(opacity),
        "width" => cp!(width),
        "height" => cp!(height),
        "min-width" => cp!(min_width),
        "min-height" => cp!(min_height),
        "max-width" => cp!(max_width),
        "max-height" => cp!(max_height),
        "box-sizing" => cp!(border_box),
        "margin" => cp!(margin),
        "margin-top" => dst.margin[0] = src.margin[0].clone(),
        "margin-right" => dst.margin[1] = src.margin[1].clone(),
        "margin-bottom" => dst.margin[2] = src.margin[2].clone(),
        "margin-left" => dst.margin[3] = src.margin[3].clone(),
        "padding" => cp!(padding),
        "padding-top" => dst.padding[0] = src.padding[0].clone(),
        "padding-right" => dst.padding[1] = src.padding[1].clone(),
        "padding-bottom" => dst.padding[2] = src.padding[2].clone(),
        "padding-left" => dst.padding[3] = src.padding[3].clone(),
        "border" => {
            cp!(border_width);
            cp!(border_style);
            cp!(border_color)
        }
        "border-width" => cp!(border_width),
        "border-style" => cp!(border_style),
        "border-color" => cp!(border_color),
        "border-radius" => cp!(border_radius),
        "color" => cp!(color),
        "background" => {
            cp!(background_color);
            cp!(background_image);
            cp!(background_repeat);
            cp!(background_size);
            cp!(background_position)
        }
        "background-color" => cp!(background_color),
        "background-image" => cp!(background_image),
        "font-size" => cp!(font_size),
        "font-weight" => cp!(font_weight),
        "font-style" => cp!(font_italic),
        "font-family" => cp!(font_family),
        "line-height" => {
            cp!(line_height_number);
            cp!(line_height_px)
        }
        "text-align" => cp!(text_align),
        "text-decoration" | "text-decoration-line" => cp!(text_decoration_own),
        "text-transform" => cp!(text_transform),
        "text-indent" => cp!(text_indent),
        "letter-spacing" => cp!(letter_spacing),
        "word-spacing" => cp!(word_spacing),
        "white-space" => cp!(white_space),
        "vertical-align" => cp!(vertical_align),
        "list-style-type" => cp!(list_style_type),
        "flex-direction" => cp!(flex_direction),
        "flex-wrap" => cp!(flex_wrap),
        "flex-grow" => cp!(flex_grow),
        "flex-shrink" => cp!(flex_shrink),
        "flex-basis" => cp!(flex_basis),
        "order" => cp!(order),
        "justify-content" => cp!(justify_content),
        "align-items" => cp!(align_items),
        "align-self" => cp!(align_self),
        "align-content" => cp!(align_content),
        "gap" => {
            cp!(row_gap);
            cp!(column_gap)
        }
        "row-gap" => cp!(row_gap),
        "column-gap" => cp!(column_gap),
        "content" => cp!(content),
        "border-collapse" => cp!(border_collapse),
        "pointer-events" => cp!(pointer_events),
        _ => {}
    }
}

const INHERITED: &[&str] = &[
    "color", "font-size", "font-weight", "font-style", "font-family", "line-height", "text-align", "text-indent",
    "text-transform", "letter-spacing", "word-spacing", "white-space", "visibility", "list-style-type",
    "border-collapse", "pointer-events",
];

#[allow(clippy::too_many_arguments)]
fn apply(style: &mut Style, name: &str, value: &str, parent: Option<&Style>, init: &Style, parent_font: f32, root_font: f32, vp: Viewport) {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let lv = lower.as_str();
    match lv {
        "inherit" => {
            if let Some(p) = parent {
                copy_property(style, p, name);
            }
            return;
        }
        "initial" => {
            copy_property(style, init, name);
            return;
        }
        "unset" | "revert" | "revert-layer" => {
            let src = if INHERITED.contains(&name) { parent.unwrap_or(init) } else { init };
            copy_property(style, src, name);
            return;
        }
        _ => {}
    }
    let ctx = LenCtx { font_size: style.font_size, root_font_size: root_font, vw: vp.width, vh: vp.height };
    let current = style.color;
    let dim = |v: &str| parse_dim(v, &ctx);
    match name {
        "display" => {
            let d = match lv {
                "block" | "-webkit-box" | "-webkit-flex-box" | "run-in" => Display::Block,
                "flow-root" => Display::FlowRoot,
                "inline" | "ruby" | "ruby-text" | "ruby-base" | "contents-inline" => Display::Inline,
                "inline-block" | "-webkit-inline-box" => Display::InlineBlock,
                "flex" | "-webkit-flex" | "-ms-flexbox" => Display::Flex,
                "inline-flex" | "-webkit-inline-flex" => Display::InlineFlex,
                "grid" | "-ms-grid" | "subgrid" => Display::Grid,
                "inline-grid" => Display::InlineGrid,
                "list-item" => Display::ListItem,
                "table" => Display::Table,
                "inline-table" => Display::InlineTable,
                "table-row-group" | "table-header-group" | "table-footer-group" => Display::TableRowGroup,
                "table-row" => Display::TableRow,
                "table-cell" => Display::TableCell,
                "table-caption" => Display::TableCaption,
                "table-column" | "table-column-group" => Display::TableColumn,
                "none" => Display::None,
                "contents" => Display::Contents,
                _ => {
                    // Two-value syntax, e.g. `block flex`.
                    let parts: Vec<&str> = lv.split_whitespace().collect();
                    match parts.as_slice() {
                        ["block", "flex"] => Display::Flex,
                        ["inline", "flex"] => Display::InlineFlex,
                        ["block", "grid"] => Display::Grid,
                        ["inline", "grid"] => Display::InlineGrid,
                        ["block", "flow-root"] => Display::FlowRoot,
                        ["inline", "flow-root"] => Display::InlineBlock,
                        _ => return,
                    }
                }
            };
            style.display = d;
        }
        "position" => {
            style.position = match lv {
                "static" => Position::Static,
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                "sticky" | "-webkit-sticky" => Position::Sticky,
                _ => return,
            }
        }
        "float" => {
            style.float = match lv {
                "left" | "inline-start" => Float::Left,
                "right" | "inline-end" => Float::Right,
                "none" => Float::None,
                _ => return,
            }
        }
        "clear" => {
            style.clear = match lv {
                "left" | "inline-start" => Clear::Left,
                "right" | "inline-end" => Clear::Right,
                "both" => Clear::Both,
                "none" => Clear::None,
                _ => return,
            }
        }
        "top" => {
            if let Some(d) = dim(value) {
                style.top = d
            }
        }
        "right" => {
            if let Some(d) = dim(value) {
                style.right = d
            }
        }
        "bottom" => {
            if let Some(d) = dim(value) {
                style.bottom = d
            }
        }
        "left" => {
            if let Some(d) = dim(value) {
                style.left = d
            }
        }
        "inset" => {
            if let Some(v) = parse_dims(value, &ctx).and_then(|v| edge4(&v)) {
                let [t, r, b, l] = v;
                style.top = t;
                style.right = r;
                style.bottom = b;
                style.left = l;
            }
        }
        "inset-inline" | "inset-inline-start" | "inset-inline-end" | "inset-block" | "inset-block-start" | "inset-block-end" => {
            if let Some(v) = parse_dims(value, &ctx) {
                let a = v.first().cloned().unwrap_or(Dim::Auto);
                let b = v.get(1).cloned().unwrap_or(a.clone());
                match name {
                    "inset-inline" => {
                        style.left = a;
                        style.right = b
                    }
                    "inset-inline-start" => style.left = a,
                    "inset-inline-end" => style.right = a,
                    "inset-block" => {
                        style.top = a;
                        style.bottom = b
                    }
                    "inset-block-start" => style.top = a,
                    _ => style.bottom = a,
                }
            }
        }
        "z-index" => style.z_index = lv.parse::<i32>().ok(),
        "overflow" | "overflow-x" | "overflow-y" => {
            let parse = |t: &str| match t {
                "visible" => Some(Overflow::Visible),
                "hidden" | "clip" => Some(Overflow::Hidden),
                "scroll" => Some(Overflow::Scroll),
                "auto" | "overlay" => Some(Overflow::Auto),
                _ => None,
            };
            let parts: Vec<&str> = lv.split_whitespace().collect();
            match (name, parts.as_slice()) {
                ("overflow", [a]) => {
                    if let Some(o) = parse(a) {
                        style.overflow_x = o;
                        style.overflow_y = o;
                    }
                }
                ("overflow", [a, b]) => {
                    if let (Some(x), Some(y)) = (parse(a), parse(b)) {
                        style.overflow_x = x;
                        style.overflow_y = y;
                    }
                }
                ("overflow-x", [a]) => {
                    if let Some(o) = parse(a) {
                        style.overflow_x = o
                    }
                }
                ("overflow-y", [a]) => {
                    if let Some(o) = parse(a) {
                        style.overflow_y = o
                    }
                }
                _ => {}
            }
        }
        "visibility" => style.visibility_visible = !matches!(lv, "hidden" | "collapse"),
        "opacity" => {
            let v = if let Some(p) = lv.strip_suffix('%') { p.trim().parse::<f32>().ok().map(|v| v / 100.0) } else { lv.parse::<f32>().ok() };
            if let Some(v) = v {
                style.opacity = v.clamp(0.0, 1.0);
            }
        }
        "pointer-events" => style.pointer_events = lv != "none",
        "width" | "inline-size" => {
            if let Some(d) = dim(value) {
                style.width = d
            }
        }
        "height" | "block-size" => {
            if let Some(d) = dim(value) {
                style.height = d
            }
        }
        "min-width" | "min-inline-size" => {
            if let Some(d) = dim(value) {
                style.min_width = d
            }
        }
        "min-height" | "min-block-size" => {
            if let Some(d) = dim(value) {
                style.min_height = d
            }
        }
        "max-width" | "max-inline-size" => {
            if let Some(d) = dim(value) {
                style.max_width = d
            }
        }
        "max-height" | "max-block-size" => {
            if let Some(d) = dim(value) {
                style.max_height = d
            }
        }
        "box-sizing" => style.border_box = lv == "border-box",
        "aspect-ratio" => {
            style.aspect_ratio = if lv == "auto" {
                None
            } else {
                let nums: Vec<&str> = lv.split('/').collect();
                let num = |s: &str| s.split_whitespace().find_map(|t| t.parse::<f32>().ok());
                match (nums.first().and_then(|s| num(s)), nums.get(1).and_then(|s| num(s))) {
                    (Some(a), Some(b)) if b > 0.0 => Some(a / b),
                    (Some(a), None) if a > 0.0 => Some(a),
                    _ => style.aspect_ratio,
                }
            }
        }
        "margin" => {
            if let Some(v) = parse_dims(value, &ctx).and_then(|v| edge4(&v)) {
                style.margin = v;
            }
        }
        "padding" => {
            if let Some(v) = parse_dims(value, &ctx).and_then(|v| edge4(&v)) {
                style.padding = v.map(|d| if matches!(d, Dim::Len(a, b) if a < 0.0 || b < 0.0) { Dim::ZERO } else { d });
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "margin-block-start" | "margin-block-end"
        | "margin-inline-start" | "margin-inline-end" => {
            if let Some(d) = dim(value) {
                let i = match name {
                    "margin-top" | "margin-block-start" => 0,
                    "margin-right" | "margin-inline-end" => 1,
                    "margin-bottom" | "margin-block-end" => 2,
                    _ => 3,
                };
                style.margin[i] = d;
            }
        }
        "margin-inline" | "margin-block" => {
            if let Some(v) = parse_dims(value, &ctx) {
                let a = v.first().cloned().unwrap_or(Dim::ZERO);
                let b = v.get(1).cloned().unwrap_or(a.clone());
                if name == "margin-inline" {
                    style.margin[3] = a;
                    style.margin[1] = b;
                } else {
                    style.margin[0] = a;
                    style.margin[2] = b;
                }
            }
        }
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "padding-block-start" | "padding-block-end"
        | "padding-inline-start" | "padding-inline-end" => {
            if let Some(d) = dim(value) {
                let i = match name {
                    "padding-top" | "padding-block-start" => 0,
                    "padding-right" | "padding-inline-end" => 1,
                    "padding-bottom" | "padding-block-end" => 2,
                    _ => 3,
                };
                style.padding[i] = d;
            }
        }
        "padding-inline" | "padding-block" => {
            if let Some(v) = parse_dims(value, &ctx) {
                let a = v.first().cloned().unwrap_or(Dim::ZERO);
                let b = v.get(1).cloned().unwrap_or(a.clone());
                if name == "padding-inline" {
                    style.padding[3] = a;
                    style.padding[1] = b;
                } else {
                    style.padding[0] = a;
                    style.padding[2] = b;
                }
            }
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" | "border-block" | "border-inline"
        | "border-block-start" | "border-block-end" | "border-inline-start" | "border-inline-end" => {
            let sides: &[usize] = match name {
                "border" => &[0, 1, 2, 3],
                "border-top" | "border-block-start" => &[0],
                "border-right" | "border-inline-end" => &[1],
                "border-bottom" | "border-block-end" => &[2],
                "border-left" | "border-inline-start" => &[3],
                "border-block" => &[0, 2],
                _ => &[1, 3],
            };
            let mut width = 3.0;
            let mut bstyle = BorderStyle::None;
            let mut bcolor: Option<Color> = None;
            let mut color_set = false;
            if lv != "none" && lv != "0" {
                for tok in split_ws(value) {
                    let tl = tok.to_ascii_lowercase();
                    if let Some(s) = border_style_kw(&tl) {
                        bstyle = s;
                    } else if let Some(px) = parse_px(tok, &ctx) {
                        width = px.max(0.0);
                    } else if let Some(c) = parse_color_in(tok, current) {
                        bcolor = c;
                        color_set = true;
                    }
                }
            } else {
                width = 0.0;
            }
            let _ = color_set;
            for &s in sides {
                style.border_width[s] = width;
                style.border_style[s] = bstyle;
                style.border_color[s] = bcolor;
            }
        }
        "border-width" | "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            if name == "border-width" {
                if let Some(v) = parse_dims(value, &ctx).and_then(|v| edge4(&v)) {
                    for i in 0..4 {
                        if let Dim::Len(px, _) = v[i] {
                            style.border_width[i] = px.max(0.0);
                        }
                    }
                }
            } else if let Some(Dim::Len(px, _)) = dim(value) {
                let i = match name {
                    "border-top-width" => 0,
                    "border-right-width" => 1,
                    "border-bottom-width" => 2,
                    _ => 3,
                };
                style.border_width[i] = px.max(0.0);
            }
        }
        "border-style" | "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            let styles: Vec<BorderStyle> = lv.split_whitespace().filter_map(border_style_kw).collect();
            if name == "border-style" {
                let e = match styles.as_slice() {
                    [a] => Some([*a, *a, *a, *a]),
                    [a, b] => Some([*a, *b, *a, *b]),
                    [a, b, c] => Some([*a, *b, *c, *b]),
                    [a, b, c, d] => Some([*a, *b, *c, *d]),
                    _ => None,
                };
                if let Some(e) = e {
                    style.border_style = e;
                }
            } else if let Some(s) = styles.first() {
                let i = match name {
                    "border-top-style" => 0,
                    "border-right-style" => 1,
                    "border-bottom-style" => 2,
                    _ => 3,
                };
                style.border_style[i] = *s;
            }
        }
        "border-color" | "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            let colors: Vec<Option<Color>> = split_ws(value).into_iter().filter_map(|t| parse_color_in(t, current)).collect();
            if name == "border-color" {
                let e = match colors.as_slice() {
                    [a] => Some([*a, *a, *a, *a]),
                    [a, b] => Some([*a, *b, *a, *b]),
                    [a, b, c] => Some([*a, *b, *c, *b]),
                    [a, b, c, d] => Some([*a, *b, *c, *d]),
                    _ => None,
                };
                if let Some(e) = e {
                    style.border_color = e;
                }
            } else if let Some(c) = colors.first() {
                let i = match name {
                    "border-top-color" => 0,
                    "border-right-color" => 1,
                    "border-bottom-color" => 2,
                    _ => 3,
                };
                style.border_color[i] = *c;
            }
        }
        "border-radius" | "border-top-left-radius" | "border-top-right-radius" | "border-bottom-left-radius"
        | "border-bottom-right-radius" => {
            // One uniform radius: use the largest horizontal value.
            let first = value.split('/').next().unwrap_or(value);
            let vals: Vec<f32> = split_ws(first)
                .into_iter()
                .filter_map(|t| match parse_dim(t, &ctx) {
                    Some(Dim::Len(px, pct)) => Some(if pct != 0.0 { pct * 0.5 * 1000.0 } else { px }),
                    _ => None,
                })
                .collect();
            if let Some(m) = vals.iter().copied().reduce(f32::max) {
                if name == "border-radius" {
                    style.border_radius = m.max(0.0);
                } else {
                    style.border_radius = style.border_radius.max(m.max(0.0));
                }
            }
        }
        "box-shadow" => style.shadows = parse_shadows(value, &ctx, current),
        "color" => {
            if lv == "currentcolor" {
                // keep
            } else if let Some(c) = color::parse_color(value) {
                style.color = c;
            }
        }
        "background-color" => {
            if lv == "currentcolor" {
                style.background_color = style.color;
            } else if let Some(c) = color::parse_color(value) {
                style.background_color = c;
            }
        }
        "background" => {
            let layers = split_commas(value);
            let mut image = None;
            let mut result_color = None;
            let mut repeat = BgRepeat::Repeat;
            let mut size = BgSize::Auto;
            let mut pos = (Dim::pct(0.0), Dim::pct(0.0));
            for (i, layer) in layers.iter().enumerate() {
                let p = parse_background_layer(layer, &ctx, current);
                if i + 1 == layers.len() {
                    result_color = p.color;
                }
                if image.is_none() && matches!(p.image, Some(BgImage::Url(_)) | Some(BgImage::Gradient(_))) {
                    image = p.image;
                    repeat = p.repeat.unwrap_or(BgRepeat::Repeat);
                    size = p.size.unwrap_or(BgSize::Auto);
                    pos = p.position.unwrap_or((Dim::pct(0.0), Dim::pct(0.0)));
                }
            }
            style.background_color = result_color.unwrap_or(color::TRANSPARENT);
            style.background_image = image.unwrap_or(BgImage::None);
            style.background_repeat = repeat;
            style.background_size = size;
            style.background_position = pos;
        }
        "background-image" => {
            let mut found = BgImage::None;
            for layer in split_commas(value) {
                if let Some(url) = parse_url(layer) {
                    found = BgImage::Url(url);
                    break;
                }
                if let Some(g) = parse_gradient(layer, &ctx) {
                    found = BgImage::Gradient(g);
                    break;
                }
            }
            style.background_image = found;
        }
        "background-repeat" => {
            style.background_repeat = match lv.split(',').next().unwrap_or("").trim() {
                "no-repeat" => BgRepeat::NoRepeat,
                "repeat-x" => BgRepeat::RepeatX,
                "repeat-y" => BgRepeat::RepeatY,
                "repeat" | "space" | "round" => BgRepeat::Repeat,
                v if v.starts_with("no-repeat") => BgRepeat::NoRepeat,
                _ => return,
            }
        }
        "background-size" => {
            let first = split_commas(value).first().copied().unwrap_or("auto");
            let toks = split_ws(first);
            style.background_size = match toks.first().copied().unwrap_or("auto") {
                "cover" => BgSize::Cover,
                "contain" => BgSize::Contain,
                f => {
                    let w = parse_dim(f, &ctx).unwrap_or(Dim::Auto);
                    let h = toks.get(1).and_then(|t| parse_dim(t, &ctx)).unwrap_or(Dim::Auto);
                    if w.is_auto() && h.is_auto() { BgSize::Auto } else { BgSize::Explicit(w, h) }
                }
            };
        }
        "background-position" => {
            let first = split_commas(value).first().copied().unwrap_or("");
            let toks = split_ws(first);
            if let Some(p) = parse_position(&toks, &ctx) {
                style.background_position = p;
            }
        }
        "font" => {
            if matches!(lv, "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar") {
                style.font_size = 13.333;
                style.font_family = FontFamily::Sans;
                return;
            }
            let toks = split_ws(value);
            let mut weight = 400;
            let mut italic = false;
            let mut i = 0;
            while i < toks.len() {
                let t = toks[i].to_ascii_lowercase();
                match t.as_str() {
                    "normal" | "small-caps" | "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed"
                    | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded" => {}
                    "italic" | "oblique" => italic = true,
                    "bold" | "bolder" | "lighter" => weight = parse_font_weight(&t, 400).unwrap_or(400),
                    n if n.parse::<u16>().is_ok() && toks[i].len() == 3 => weight = n.parse().unwrap_or(400),
                    _ => break,
                }
                i += 1;
            }
            if i >= toks.len() {
                return;
            }
            let size_tok = toks[i];
            let (size_part, lh_part) = match size_tok.split_once('/') {
                Some((s, l)) => (s, Some(l)),
                None => (size_tok, None),
            };
            let size_ctx = LenCtx { font_size: parent_font, ..ctx };
            let size = font_size_keyword(&size_part.to_ascii_lowercase(), parent_font).or_else(|| {
                parse_dim(size_part, &size_ctx).and_then(|d| d.resolve(parent_font))
            });
            let Some(size) = size else { return };
            style.font_size = size.max(0.0);
            style.font_size_keyword = font_size_keyword(&size_part.to_ascii_lowercase(), parent_font).is_some();
            style.font_weight = weight;
            style.font_italic = italic;
            let mut lh = lh_part.map(|s| s.to_string());
            let mut family_start = i + 1;
            if lh.is_none() && toks.get(i + 1).is_some_and(|t| t.starts_with('/')) {
                let t = toks[i + 1];
                if t.len() > 1 {
                    lh = Some(t[1..].to_string());
                } else if let Some(n) = toks.get(i + 2) {
                    lh = Some(n.to_string());
                    family_start = i + 3;
                }
                if t.len() > 1 {
                    family_start = i + 2;
                }
            }
            if let Some(lh) = lh {
                apply_line_height(style, &lh, &LenCtx { font_size: style.font_size, ..ctx });
            } else {
                style.line_height_number = None;
                style.line_height_px = None;
            }
            style.font_family = parse_font_family(&toks[family_start.min(toks.len())..].join(" "));
        }
        "font-size" => {
            let size_ctx = LenCtx { font_size: parent_font, ..ctx };
            let size = font_size_keyword(lv, parent_font).or_else(|| match parse_dim(value, &size_ctx) {
                Some(Dim::Len(px, pct)) => Some(px + parent_font * pct / 100.0),
                Some(d @ Dim::Func(..)) => d.resolve(parent_font),
                _ => None,
            });
            if let Some(s) = size {
                style.font_size = s.max(0.0);
                style.font_size_keyword = match lv {
                    "larger" | "smaller" => parent.is_none_or(|p| p.font_size_keyword),
                    _ => font_size_keyword(lv, parent_font).is_some(),
                };
            }
        }
        "font-weight" => {
            if let Some(w) = parse_font_weight(lv, parent.map_or(400, |p| p.font_weight)) {
                style.font_weight = w;
            }
        }
        "font-style" => style.font_italic = lv.starts_with("italic") || lv.starts_with("oblique"),
        "font-family" => {
            let family = parse_font_family(value);
            // Blink scales the default size when only the generic `monospace` family is given.
            let only_generic = lv.trim() == "monospace";
            let parent_mono = parent.is_some_and(|p| p.font_family == FontFamily::Monospace);
            if style.font_size_keyword {
                if family == FontFamily::Monospace && !parent_mono && only_generic {
                    style.font_size *= 13.0 / 16.0;
                } else if family != FontFamily::Monospace && parent_mono {
                    style.font_size *= 16.0 / 13.0;
                }
            }
            style.font_family = family;
        }
        "line-height" => apply_line_height(style, value, &ctx),
        "text-align" | "text-align-last" => {
            if name == "text-align" {
                style.text_align = match lv {
                    "left" | "start" | "-webkit-left" => {
                        if style.rtl && lv == "start" { TextAlign::Right } else { TextAlign::Left }
                    }
                    "right" | "end" | "-webkit-right" => {
                        if style.rtl && lv == "end" { TextAlign::Left } else { TextAlign::Right }
                    }
                    "center" | "-webkit-center" | "-moz-center" => TextAlign::Center,
                    "justify" => TextAlign::Justify,
                    _ => return,
                }
            }
        }
        "direction" => style.rtl = lv == "rtl",
        "text-decoration" | "text-decoration-line" => {
            let mut flags = 0;
            for t in lv.split_whitespace() {
                match t {
                    "underline" => flags |= 1,
                    "line-through" => flags |= 2,
                    "overline" => flags |= 4,
                    _ => {}
                }
            }
            style.text_decoration_own = flags;
        }
        "text-transform" => {
            style.text_transform = match lv {
                "uppercase" => TextTransform::Uppercase,
                "lowercase" => TextTransform::Lowercase,
                "capitalize" => TextTransform::Capitalize,
                "none" => TextTransform::None,
                _ => return,
            }
        }
        "text-indent" => {
            if let Some(d) = split_ws(value).first().and_then(|t| dim(t)) {
                style.text_indent = d;
            }
        }
        "text-overflow" => style.text_overflow_ellipsis = lv.contains("ellipsis"),
        "letter-spacing" => {
            style.letter_spacing = if lv == "normal" { 0.0 } else { parse_px(value, &ctx).unwrap_or(style.letter_spacing) }
        }
        "word-spacing" => {
            style.word_spacing = if lv == "normal" { 0.0 } else { parse_px(value, &ctx).unwrap_or(style.word_spacing) }
        }
        "white-space" => {
            style.white_space = match lv {
                "normal" => WhiteSpace::Normal,
                "nowrap" => WhiteSpace::Nowrap,
                "pre" => WhiteSpace::Pre,
                "pre-wrap" | "break-spaces" => WhiteSpace::PreWrap,
                "pre-line" => WhiteSpace::PreLine,
                _ => return,
            }
        }
        "word-break" => style.word_break_all = lv == "break-all" || lv == "break-word",
        "overflow-wrap" | "word-wrap" => style.overflow_wrap_break = matches!(lv, "break-word" | "anywhere"),
        "-webkit-line-clamp" | "line-clamp" => style.line_clamp = lv.parse::<u32>().ok(),
        "vertical-align" => {
            style.vertical_align = match lv {
                "baseline" => VerticalAlign::Baseline,
                "top" => VerticalAlign::Top,
                "middle" => VerticalAlign::Middle,
                "bottom" => VerticalAlign::Bottom,
                "text-top" => VerticalAlign::TextTop,
                "text-bottom" => VerticalAlign::TextBottom,
                "sub" => VerticalAlign::Sub,
                "super" => VerticalAlign::Super,
                _ => match dim(value) {
                    Some(d @ Dim::Len(..)) => VerticalAlign::Len(d),
                    _ => return,
                },
            }
        }
        "list-style" | "list-style-type" => {
            for t in lv.split_whitespace() {
                if let Some(ty) = match t {
                    "none" => Some(ListStyleType::None),
                    "disc" => Some(ListStyleType::Disc),
                    "circle" => Some(ListStyleType::Circle),
                    "square" => Some(ListStyleType::Square),
                    "decimal" => Some(ListStyleType::Decimal),
                    "decimal-leading-zero" => Some(ListStyleType::DecimalLeadingZero),
                    "lower-alpha" | "lower-latin" => Some(ListStyleType::LowerAlpha),
                    "upper-alpha" | "upper-latin" => Some(ListStyleType::UpperAlpha),
                    "lower-roman" => Some(ListStyleType::LowerRoman),
                    "upper-roman" => Some(ListStyleType::UpperRoman),
                    "lower-greek" => Some(ListStyleType::LowerGreek),
                    _ => None,
                } {
                    style.list_style_type = ty;
                } else if t == "inside" {
                    style.list_style_inside = true;
                } else if t == "outside" {
                    style.list_style_inside = false;
                }
            }
        }
        "list-style-position" => style.list_style_inside = lv == "inside",
        "flex" => {
            match lv {
                "none" => {
                    style.flex_grow = 0.0;
                    style.flex_shrink = 0.0;
                    style.flex_basis = Dim::Auto;
                }
                "auto" => {
                    style.flex_grow = 1.0;
                    style.flex_shrink = 1.0;
                    style.flex_basis = Dim::Auto;
                }
                "initial" => {
                    style.flex_grow = 0.0;
                    style.flex_shrink = 1.0;
                    style.flex_basis = Dim::Auto;
                }
                _ => {
                    let toks = split_ws(value);
                    let mut nums = Vec::new();
                    let mut basis = None;
                    for t in &toks {
                        if let Ok(n) = t.parse::<f32>() {
                            nums.push(n);
                        } else if let Some(d) = dim(t) {
                            basis = Some(d);
                        }
                    }
                    style.flex_grow = nums.first().copied().unwrap_or(1.0);
                    style.flex_shrink = nums.get(1).copied().unwrap_or(1.0);
                    style.flex_basis = basis.unwrap_or(if nums.is_empty() { Dim::Auto } else { Dim::pct(0.0) });
                }
            }
        }
        "flex-grow" => {
            if let Some(n) = parse_number(value) {
                style.flex_grow = n.max(0.0)
            }
        }
        "flex-shrink" => {
            if let Some(n) = parse_number(value) {
                style.flex_shrink = n.max(0.0)
            }
        }
        "flex-basis" => {
            if lv == "content" {
                style.flex_basis = Dim::MaxContent;
            } else if let Some(d) = dim(value) {
                style.flex_basis = d;
            }
        }
        "flex-direction" => {
            style.flex_direction = match lv {
                "row" => FlexDirection::Row,
                "row-reverse" => FlexDirection::RowReverse,
                "column" => FlexDirection::Column,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => return,
            }
        }
        "flex-wrap" => {
            style.flex_wrap = match lv {
                "nowrap" => FlexWrap::Nowrap,
                "wrap" => FlexWrap::Wrap,
                "wrap-reverse" => FlexWrap::WrapReverse,
                _ => return,
            }
        }
        "flex-flow" => {
            for t in lv.split_whitespace() {
                match t {
                    "row" => style.flex_direction = FlexDirection::Row,
                    "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
                    "column" => style.flex_direction = FlexDirection::Column,
                    "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
                    "nowrap" => style.flex_wrap = FlexWrap::Nowrap,
                    "wrap" => style.flex_wrap = FlexWrap::Wrap,
                    "wrap-reverse" => style.flex_wrap = FlexWrap::WrapReverse,
                    _ => {}
                }
            }
        }
        "order" => {
            if let Ok(n) = lv.parse::<i32>() {
                style.order = n
            }
        }
        "justify-content" => {
            if let Some(a) = parse_align(lv.trim_start_matches("safe ").trim_start_matches("unsafe ")) {
                style.justify_content = a
            }
        }
        "align-items" => {
            if let Some(a) = parse_align(lv.trim_start_matches("safe ").trim_start_matches("unsafe ")) {
                style.align_items = a
            }
        }
        "align-self" => {
            if let Some(a) = parse_align(lv.trim_start_matches("safe ").trim_start_matches("unsafe ")) {
                style.align_self = a
            }
        }
        "align-content" => {
            if let Some(a) = parse_align(lv) {
                style.align_content = a
            }
        }
        "justify-items" => {
            if let Some(a) = parse_align(lv) {
                style.justify_items = a
            }
        }
        "justify-self" => {
            if let Some(a) = parse_align(lv) {
                style.justify_self = a
            }
        }
        "place-items" | "place-content" | "place-self" => {
            let parts: Vec<&str> = lv.split_whitespace().collect();
            let a = parts.first().and_then(|p| parse_align(p));
            let b = parts.get(1).and_then(|p| parse_align(p)).or(a);
            if let (Some(a), Some(b)) = (a, b) {
                match name {
                    "place-items" => {
                        style.align_items = a;
                        style.justify_items = b;
                    }
                    "place-content" => {
                        style.align_content = a;
                        style.justify_content = b;
                    }
                    _ => {
                        style.align_self = a;
                        style.justify_self = b;
                    }
                }
            }
        }
        "gap" | "grid-gap" => {
            let toks = split_ws(value);
            if let Some(r) = toks.first().and_then(|t| dim(t)) {
                style.row_gap = r.clone();
                style.column_gap = toks.get(1).and_then(|t| dim(t)).unwrap_or(r);
            }
        }
        "row-gap" | "grid-row-gap" => {
            if let Some(d) = dim(value) {
                style.row_gap = d
            }
        }
        "column-gap" | "grid-column-gap" => {
            if let Some(d) = dim(value) {
                style.column_gap = d
            }
        }
        "grid-template-columns" => {
            if let Some(t) = parse_tracks(value, &ctx) {
                style.grid_columns = t
            }
        }
        "grid-template-rows" => {
            if let Some(t) = parse_tracks(value, &ctx) {
                style.grid_rows = t
            }
        }
        "grid-template" | "grid" => {
            // `rows / columns` form, optionally with area strings.
            if let Some((r, c)) = value.split_once('/') {
                if !r.contains('"') {
                    if let Some(t) = parse_tracks(r, &ctx) {
                        style.grid_rows = t
                    }
                }
                if let Some(t) = parse_tracks(c, &ctx) {
                    style.grid_columns = t
                }
            }
        }
        "grid-template-areas" => {
            style.grid_areas = if lv == "none" {
                Vec::new()
            } else {
                let mut rows = Vec::new();
                let mut rest = value;
                while let Some(s) = rest.find('"') {
                    let Some(e) = rest[s + 1..].find('"') else { break };
                    rows.push(rest[s + 1..s + 1 + e].split_whitespace().map(|x| x.to_string()).collect());
                    rest = &rest[s + 1 + e + 1..];
                }
                rows
            };
        }
        "grid-auto-columns" => {
            style.grid_auto_columns = split_ws(value).into_iter().filter_map(|t| parse_track(t, &ctx)).collect()
        }
        "grid-auto-rows" => style.grid_auto_rows = split_ws(value).into_iter().filter_map(|t| parse_track(t, &ctx)).collect(),
        "grid-auto-flow" => {
            style.grid_auto_flow_column = lv.contains("column");
            style.grid_auto_flow_dense = lv.contains("dense");
        }
        "grid-column" => style.grid_column = parse_grid_pair(value),
        "grid-row" => style.grid_row = parse_grid_pair(value),
        "grid-column-start" => style.grid_column.0 = parse_grid_line(value),
        "grid-column-end" => style.grid_column.1 = parse_grid_line(value),
        "grid-row-start" => style.grid_row.0 = parse_grid_line(value),
        "grid-row-end" => style.grid_row.1 = parse_grid_line(value),
        "grid-area" => {
            let parts: Vec<&str> = value.split('/').collect();
            if parts.len() == 1 {
                let n = GridLine::Name(parts[0].trim().to_string());
                if let GridLine::Name(_) = parse_grid_line(parts[0]) {
                    style.grid_row = (n.clone(), n.clone());
                    style.grid_column = (n.clone(), n);
                } else {
                    style.grid_row = (parse_grid_line(parts[0]), GridLine::Auto);
                }
            } else {
                style.grid_row = (parse_grid_line(parts[0]), parts.get(2).map_or(GridLine::Auto, |p| parse_grid_line(p)));
                style.grid_column = (parse_grid_line(parts[1]), parts.get(3).map_or(GridLine::Auto, |p| parse_grid_line(p)));
            }
        }
        "border-collapse" => style.border_collapse = lv == "collapse",
        "border-spacing" => {
            let toks = split_ws(value);
            if let Some(h) = toks.first().and_then(|t| parse_px(t, &ctx)) {
                style.border_spacing = (h, toks.get(1).and_then(|t| parse_px(t, &ctx)).unwrap_or(h));
            }
        }
        "table-layout" => style.table_fixed = lv == "fixed",
        "caption-side" => style.caption_bottom = lv == "bottom",
        "content" => style.content = parse_content(value),
        "translate" => {
            let toks = split_ws(value);
            if let Some(x) = toks.first().and_then(|t| dim(t)) {
                style.translate = (x, toks.get(1).and_then(|t| dim(t)).unwrap_or(Dim::ZERO));
            }
        }
        "transform" => {
            let mut tx = Dim::ZERO;
            let mut ty = Dim::ZERO;
            let mut scale = 1.0;
            if lv != "none" {
                for f in split_ws(value) {
                    if let Some(i) = strip_function(f, "translate") {
                        let a = split_commas(i);
                        tx = a.first().and_then(|t| dim(t)).unwrap_or(Dim::ZERO);
                        ty = a.get(1).and_then(|t| dim(t)).unwrap_or(Dim::ZERO);
                    } else if let Some(i) = strip_function(f, "translatex") {
                        tx = dim(i).unwrap_or(Dim::ZERO);
                    } else if let Some(i) = strip_function(f, "translatey") {
                        ty = dim(i).unwrap_or(Dim::ZERO);
                    } else if let Some(i) = strip_function(f, "translate3d") {
                        let a = split_commas(i);
                        tx = a.first().and_then(|t| dim(t)).unwrap_or(Dim::ZERO);
                        ty = a.get(1).and_then(|t| dim(t)).unwrap_or(Dim::ZERO);
                    } else if let Some(i) = strip_function(f, "scale") {
                        scale = split_commas(i).first().and_then(|t| parse_number(t)).unwrap_or(1.0);
                    }
                }
            }
            style.translate = (tx, ty);
            style.transform_scale = scale;
        }
        _ => {}
    }
}

fn apply_line_height(style: &mut Style, value: &str, ctx: &LenCtx) {
    let v = value.trim().to_ascii_lowercase();
    if v == "normal" {
        style.line_height_number = None;
        style.line_height_px = None;
    } else if let Ok(n) = v.parse::<f32>() {
        style.line_height_number = Some(n.max(0.0));
        style.line_height_px = None;
    } else if let Some(Dim::Len(px, pct)) = parse_dim(&v, ctx) {
        style.line_height_px = Some((px + ctx.font_size * pct / 100.0).max(0.0));
        style.line_height_number = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn styles(html_src: &str, css: &str) -> (Document, Styles) {
        let doc = html::parse(html_src);
        let sheet = parse(css);
        let s = compute_styles(&doc, &sheet, Viewport::new(800.0, 600.0));
        (doc, s)
    }

    #[test]
    fn class_overrides_tag_and_inherits_color() {
        let (doc, s) = styles("<p class='note'><span>x</span></p>", "p { color: red } .note { color: #123456 }");
        let p = doc.nodes[doc.body].children[0];
        assert_eq!(s[p].color, 0xff123456);
        assert_eq!(s[doc.nodes[p].children[0]].color, 0xff123456);
    }

    #[test]
    fn descendant_selector_and_important_override() {
        let (doc, s) = styles(
            "<main><p class='note' style='color: blue'>text</p></main>",
            "main .note { color: red !important } main p { color: green }",
        );
        let main = doc.nodes[doc.body].children[0];
        let p = doc.nodes[main].children[0];
        assert_eq!(s[p].color, 0xffff0000);
    }

    #[test]
    fn media_rules_follow_viewport_width() {
        let doc = html::parse("<p>text</p>");
        let p = doc.nodes[doc.body].children[0];
        let sheet = parse("p { color: blue } @media screen and (max-width: 500px) { p { color: red } }");
        let narrow = compute_styles(&doc, &sheet, Viewport::new(400.0, 600.0));
        let wide = compute_styles(&doc, &sheet, Viewport::new(800.0, 600.0));
        assert_eq!(narrow[p].color, 0xffff0000);
        assert_eq!(wide[p].color, 0xff0000ff);
    }

    #[test]
    fn variables_calc_and_nesting() {
        let (doc, s) = styles(
            "<div class='a'><p>x</p></div>",
            ":root { --gap: 10px; --c: #0f0 } .a { padding: calc(var(--gap) * 2); color: var(--c) } .a { & p { margin-left: var(--gap) } }",
        );
        let div = doc.nodes[doc.body].children[0];
        let p = doc.nodes[div].children[0];
        assert_eq!(s[div].padding[0], Dim::px(20.0));
        assert_eq!(s[div].color, 0xff00ff00);
        assert_eq!(s[p].margin[3], Dim::px(10.0));
    }

    #[test]
    fn layers_and_pseudo_elements() {
        let (doc, s) = styles(
            "<p class='x'>t</p>",
            "@layer base { .x { color: red } } p { color: blue } .x::before { content: '>'; color: green }",
        );
        let p = doc.nodes[doc.body].children[0];
        assert_eq!(s[p].color, 0xff0000ff);
        assert_eq!(s.before[p].as_ref().unwrap().content, Some(vec![ContentItem::Text(">".to_string())]));
    }

    #[test]
    fn shorthand_properties() {
        let (doc, s) = styles(
            "<div style='font: italic bold 20px/30px Arial, sans-serif; border: 2px dashed #333; background: url(a.png) no-repeat center / cover #eee; margin: 0 auto'>x</div>",
            "",
        );
        let d = doc.nodes[doc.body].children[0];
        assert_eq!(s[d].font_size, 20.0);
        assert_eq!(s[d].font_weight, 700);
        assert!(s[d].font_italic);
        assert_eq!(s[d].line_height_px, Some(30.0));
        assert_eq!(s[d].border_width, [2.0; 4]);
        assert_eq!(s[d].border_style[0], BorderStyle::Dashed);
        assert_eq!(s[d].background_image, BgImage::Url("a.png".to_string()));
        assert_eq!(s[d].background_size, BgSize::Cover);
        assert_eq!(s[d].background_color, 0xffeeeeee);
        assert_eq!(s[d].margin[1], Dim::Auto);
    }
}
