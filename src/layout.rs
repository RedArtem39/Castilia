//! Layout: turns the styled DOM into a platform-independent display list.
//!
//! Coordinates are CSS pixels. Colors are `0xAARRGGBB`. Every box is laid out in its own
//! local coordinate space (origin = border-box top-left) and translated into place by its
//! parent, which keeps flex/grid/table re-layout simple.
use crate::{
    color::{self, Color},
    css::*,
    dom::{Document, NodeId, NodeKind},
    math,
};
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// Font selection passed to the host for measuring and painting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Font {
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    pub family: FontFamily,
}

impl Font {
    pub fn of(style: &Style) -> Font {
        Font { size: style.font_size, weight: style.font_weight, italic: style.font_italic, family: style.font_family }
    }
    pub fn is_bold(&self) -> bool {
        self.weight >= 600
    }
}

/// Text and image measurement supplied by the host. Only `text_width` and `line_height`
/// are required; the other methods have reasonable defaults.
pub trait FontMetrics {
    /// Advance width of `text` in the default (regular, sans) font at `size` px.
    fn text_width(&self, text: &str, size: f32) -> f32;
    /// Height of one line at `size` px with `line-height: normal`.
    fn line_height(&self, size: f32) -> f32;
    /// Width using the full font description. Override to measure bold/italic/mono accurately.
    fn text_width_font(&self, text: &str, font: &Font) -> f32 {
        if font.family == FontFamily::Monospace {
            return text.chars().count() as f32 * font.size * 0.6;
        }
        let w = self.text_width(text, font.size);
        if font.is_bold() { w * 1.06 } else { w }
    }
    /// Distance from the top of the content area to the baseline.
    fn ascent(&self, font: &Font) -> f32 {
        self.line_height(font.size) * 0.8
    }
    /// `line-height: normal` for this font. Defaults to the size-only `line_height`.
    fn line_height_font(&self, font: &Font) -> f32 {
        self.line_height(font.size)
    }
    /// Average advance used to size `<input size=N>` and `<textarea cols=N>`.
    fn avg_char_width(&self, font: &Font) -> f32 {
        let w = self.text_width_font("0", font);
        if font.family == FontFamily::Monospace { w } else { w * 1.035 }
    }
    /// Widest advance, used for the extra room Blink gives text inputs.
    fn max_char_width(&self, font: &Font) -> f32 {
        let avg = self.avg_char_width(font);
        if font.family == FontFamily::Monospace { avg } else { avg + font.size * 1.77 }
    }
    /// Descent of the font (distance below the baseline to the bottom of the content area).
    fn descent(&self, font: &Font) -> f32 {
        (self.line_height_font(font) - self.ascent(font)).max(0.0)
    }
    /// Intrinsic pixel size of an image once the host has fetched and probed it.
    fn image_size(&self, _src: &str) -> Option<(f32, f32)> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    FillRect {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    /// Borders of a box. `styles`: 0 none, 1 solid, 2 dashed, 3 dotted, 4 double. Order: top, right, bottom, left.
    Border {
        rect: Rect,
        widths: [f32; 4],
        colors: [Color; 4],
        styles: [u8; 4],
        radius: f32,
    },
    /// `y` is the top of the font's content area; the baseline is `y + ascent`.
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        color: Color,
        link: Option<String>,
        weight: u16,
        italic: bool,
        family: FontFamily,
        /// bit 0 underline, bit 1 line-through, bit 2 overline
        decoration: u8,
        width: f32,
        letter_spacing: f32,
    },
    Image {
        rect: Rect,
        src: String,
        link: Option<String>,
        opacity: f32,
    },
    /// ARGB stops with fractional positions already resolved to 0..=1.
    Gradient {
        rect: Rect,
        angle: f32,
        radial: bool,
        stops: Vec<(f32, Color)>,
        radius: f32,
    },
    Shadow {
        rect: Rect,
        radius: f32,
        dx: f32,
        dy: f32,
        blur: f32,
        spread: f32,
        color: Color,
        inset: bool,
    },
    /// Clip subsequent commands to `rect` until the matching `PopClip`.
    PushClip {
        rect: Rect,
        radius: f32,
    },
    PopClip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
    /// Canvas background (propagated from `html`/`body`), `0` when transparent.
    pub background: Color,
    pub commands: Vec<Command>,
    pub hit_regions: Vec<HitRegion>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitRegion {
    pub rect: Rect,
    pub node: NodeId,
}

impl DisplayList {
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_regions.iter().rev().find(|r| r.rect.contains(x, y)).map(|r| r.node)
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A layout box: a DOM node or one of its generated `::before` / `::after` boxes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Bx {
    pub id: NodeId,
    /// 0 = the node itself, 1 = `::before`, 2 = `::after`
    pub pseudo: u8,
}

impl Bx {
    pub fn node(id: NodeId) -> Bx {
        Bx { id, pseudo: 0 }
    }
    pub fn key(self) -> usize {
        self.id * 3 + self.pseudo as usize
    }
}

#[derive(Default, Clone)]
pub(crate) struct Paint {
    pub cmds: Vec<Command>,
    pub hits: Vec<HitRegion>,
}

fn translate_cmd(c: &mut Command, dx: f32, dy: f32) {
    match c {
        Command::FillRect { rect, .. }
        | Command::Border { rect, .. }
        | Command::Image { rect, .. }
        | Command::Gradient { rect, .. }
        | Command::Shadow { rect, .. }
        | Command::PushClip { rect, .. } => {
            rect.x += dx;
            rect.y += dy;
        }
        Command::Text { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Command::PopClip => {}
    }
}

fn scale_alpha(c: Color, f: f32) -> Color {
    color::with_opacity(c, f)
}

impl Paint {
    pub fn translate(&mut self, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for c in &mut self.cmds {
            translate_cmd(c, dx, dy);
        }
        for h in &mut self.hits {
            h.rect.x += dx;
            h.rect.y += dy;
        }
    }
    pub fn append(&mut self, mut other: Paint, dx: f32, dy: f32) {
        other.translate(dx, dy);
        self.cmds.append(&mut other.cmds);
        self.hits.append(&mut other.hits);
    }
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty() && self.hits.is_empty()
    }
    pub fn apply_opacity(&mut self, f: f32) {
        if f >= 1.0 {
            return;
        }
        for c in &mut self.cmds {
            match c {
                Command::FillRect { color, .. } => *color = scale_alpha(*color, f),
                Command::Text { color, .. } => *color = scale_alpha(*color, f),
                Command::Border { colors, .. } => {
                    for col in colors.iter_mut() {
                        *col = scale_alpha(*col, f)
                    }
                }
                Command::Image { opacity, .. } => *opacity *= f,
                Command::Gradient { stops, .. } => {
                    for s in stops.iter_mut() {
                        s.1 = scale_alpha(s.1, f)
                    }
                }
                Command::Shadow { color, .. } => *color = scale_alpha(*color, f),
                _ => {}
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct Pending {
    pub bx: Bx,
    pub x: f32,
    pub y: f32,
    pub fixed: bool,
}

#[derive(Default)]
pub(crate) struct Out {
    pub w: f32,
    pub h: f32,
    pub ml: f32,
    pub mr: f32,
    /// Top / bottom margins after collapsing with first/last child (may escape).
    pub mt: f32,
    pub mb: f32,
    pub paint: Paint,
    pub layers: Vec<(i32, Paint)>,
    pub abs: Vec<Pending>,
    pub baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    pub shift: (f32, f32),
    pub collapses_through: bool,
    /// Stacking order if this box paints as its own layer.
    pub z: Option<i32>,
    pub float_bottom: f32,
}

impl Out {
    /// Move `child` into `self` at offset (dx, dy), carrying layers and pending absolutes.
    pub fn absorb(&mut self, mut child: Out, dx: f32, dy: f32) {
        let dx = dx + child.shift.0;
        let dy = dy + child.shift.1;
        let mut paint = core::mem::take(&mut child.paint);
        if let Some(z) = child.z {
            // Own layer: bubble up with all of the child's content.
            let mut layers = core::mem::take(&mut child.layers);
            let mut p = Paint::default();
            for (lz, lp) in layers.iter_mut() {
                if *lz < 0 {
                    p.append(core::mem::take(lp), 0.0, 0.0);
                }
            }
            p.append(paint, 0.0, 0.0);
            for (lz, lp) in layers.iter_mut() {
                if *lz >= 0 {
                    p.append(core::mem::take(lp), 0.0, 0.0);
                }
            }
            p.translate(dx, dy);
            self.layers.push((z, p));
        } else {
            self.paint.append(paint, dx, dy);
            for (z, mut lp) in child.layers.drain(..) {
                lp.translate(dx, dy);
                self.layers.push((z, lp));
            }
        }
        for mut p in child.abs.drain(..) {
            p.x += dx;
            p.y += dy;
            self.abs.push(p);
        }
    }
}

/// Floats in BFC coordinates.
#[derive(Default)]
pub(crate) struct Floats {
    pub list: Vec<FloatBox>,
}

#[derive(Clone, Copy)]
pub(crate) struct FloatBox {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    pub left: bool,
}

impl Floats {
    /// Horizontal bounds available to a line occupying `[y, y + h)` inside `[lo, hi]`.
    pub fn avail(&self, y: f32, h: f32, lo: f32, hi: f32) -> (f32, f32) {
        let mut l = lo;
        let mut r = hi;
        let h = h.max(0.01);
        for f in &self.list {
            if f.y1 > y && f.y0 < y + h {
                if f.left {
                    l = l.max(f.x1);
                } else {
                    r = r.min(f.x0);
                }
            }
        }
        (l, r.max(l))
    }
    pub fn next_bottom(&self, y: f32, h: f32) -> Option<f32> {
        let h = h.max(0.01);
        self.list
            .iter()
            .filter(|f| f.y1 > y && f.y0 < y + h)
            .map(|f| f.y1)
            .fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.min(b))))
    }
    pub fn bottom(&self, side: Clear) -> f32 {
        self.list
            .iter()
            .filter(|f| match side {
                Clear::Left => f.left,
                Clear::Right => !f.left,
                _ => true,
            })
            .map(|f| f.y1)
            .fold(0.0, f32::max)
    }
    pub fn last_top(&self) -> f32 {
        self.list.iter().map(|f| f.y0).fold(f32::NEG_INFINITY, f32::max)
    }
}

pub(crate) struct Args {
    pub avail_w: f32,
    pub cb_h: Option<f32>,
    pub forced_w: Option<f32>,
    pub forced_h: Option<f32>,
    pub shrink: bool,
}

impl Args {
    pub fn block(avail_w: f32, cb_h: Option<f32>) -> Args {
        Args { avail_w, cb_h, forced_w: None, forced_h: None, shrink: false }
    }
}

pub(crate) struct Ctx<'a> {
    pub doc: &'a Document,
    pub st: &'a Styles,
    pub fonts: &'a dyn FontMetrics,
    pub vp: Viewport,
    pub intr: Vec<Option<(f32, f32)>>,
    pub depth: usize,
    pub anon: Vec<Style>,
    /// Set by the table layout for the next cell box: borders to drop in collapsed mode.
    pub cell_border_skip: Option<[bool; 4]>,
}

pub(crate) const MAX_DEPTH: usize = 90;

pub(crate) fn collapse(a: f32, b: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        a.max(b)
    } else if a <= 0.0 && b <= 0.0 {
        a.min(b)
    } else {
        a + b
    }
}

pub(crate) struct Box4 {
    pub t: f32,
    pub r: f32,
    pub b: f32,
    pub l: f32,
}

impl Box4 {
    pub fn h(&self) -> f32 {
        self.l + self.r
    }
    pub fn v(&self) -> f32 {
        self.t + self.b
    }
}

/// Lay out the document. `viewport` supplies the width available to the root box.
pub fn layout(doc: &Document, styles: &Styles, viewport: Viewport, fonts: &dyn FontMetrics) -> DisplayList {
    let mut ctx = Ctx {
        doc,
        st: styles,
        fonts,
        vp: viewport,
        intr: vec![None; doc.nodes.len() * 3],
        depth: 0,
        anon: Vec::new(),
        cell_border_skip: None,
    };
    ctx.layout_document()
}

pub(crate) fn text_of(doc: &Document, id: NodeId) -> Option<&str> {
    match &doc.nodes[id].kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

impl<'a> Ctx<'a> {
    pub fn style(&self, bx: Bx) -> &'a Style {
        let st: &'a Styles = self.st;
        match bx.pseudo {
            1 => st.before[bx.id].as_ref().unwrap_or(&st.nodes[bx.id]),
            2 => st.after[bx.id].as_ref().unwrap_or(&st.nodes[bx.id]),
            _ => &st.nodes[bx.id],
        }
    }

    pub fn tag(&self, bx: Bx) -> Option<&'a str> {
        if bx.pseudo != 0 {
            return None;
        }
        let doc: &'a Document = self.doc;
        doc.nodes[bx.id].tag()
    }

    pub fn attr(&self, bx: Bx, name: &str) -> Option<&'a str> {
        if bx.pseudo != 0 {
            return None;
        }
        let doc: &'a Document = self.doc;
        doc.nodes[bx.id].attribute(name)
    }

    /// Children as layout boxes, including generated content. Honors closed `<details>`.
    pub fn children(&self, bx: Bx) -> Vec<Bx> {
        if bx.pseudo != 0 {
            return Vec::new();
        }
        let node = &self.doc.nodes[bx.id];
        let mut out = Vec::with_capacity(node.children.len() + 2);
        if self.st.before[bx.id].is_some() {
            out.push(Bx { id: bx.id, pseudo: 1 });
        }
        let closed_details = node.tag() == Some("details") && node.attribute("open").is_none();
        let mut summary_seen = false;
        for &c in &node.children {
            if closed_details {
                if self.doc.nodes[c].tag() == Some("summary") && !summary_seen {
                    summary_seen = true;
                    out.push(Bx::node(c));
                }
                continue;
            }
            out.push(Bx::node(c));
        }
        if self.st.after[bx.id].is_some() {
            out.push(Bx { id: bx.id, pseudo: 2 });
        }
        out
    }

    pub fn pseudo_text(&self, bx: Bx) -> String {
        let style = self.style(bx);
        let mut s = String::new();
        if let Some(items) = &style.content {
            for item in items {
                match item {
                    ContentItem::Text(t) => s.push_str(t),
                    ContentItem::Attr(a) => {
                        if let Some(v) = self.doc.nodes[bx.id].attribute(a) {
                            s.push_str(v)
                        }
                    }
                }
            }
        }
        s
    }

    pub fn font(&self, style: &Style) -> Font {
        Font::of(style)
    }

    /// (ascent, descent, used line height) of the element's primary font.
    pub fn vmetrics(&self, s: &Style) -> (f32, f32, f32) {
        let font = Font::of(s);
        let asc = self.fonts.ascent(&font);
        let desc = self.fonts.descent(&font);
        let lh = s.line_height(self.fonts.line_height_font(&font));
        (asc, desc, lh)
    }

    pub fn normal_line_height(&self, style: &Style) -> f32 {
        self.fonts.line_height_font(&Font::of(style))
    }

    pub fn used_line_height(&self, style: &Style) -> f32 {
        style.line_height(self.normal_line_height(style))
    }

    pub fn text_w(&self, text: &str, style: &Style) -> f32 {
        let mut w = self.fonts.text_width_font(text, &Font::of(style));
        if style.letter_spacing != 0.0 {
            w += style.letter_spacing * text.chars().count() as f32;
        }
        w
    }

    pub fn edges(&self, style: &Style, base: f32) -> (Box4, Box4, Box4, [bool; 2]) {
        let m = |d: &Dim| d.resolve(base).unwrap_or(0.0);
        let margin = Box4 { t: m(&style.margin[0]), r: m(&style.margin[1]), b: m(&style.margin[2]), l: m(&style.margin[3]) };
        let padding = Box4 {
            t: m(&style.padding[0]).max(0.0),
            r: m(&style.padding[1]).max(0.0),
            b: m(&style.padding[2]).max(0.0),
            l: m(&style.padding[3]).max(0.0),
        };
        let bw = |i: usize| if style.border_style[i].visible() { style.border_width[i] } else { 0.0 };
        let border = Box4 { t: bw(0), r: bw(1), b: bw(2), l: bw(3) };
        let auto = [style.margin[3].is_auto(), style.margin[1].is_auto()];
        (margin, padding, border, auto)
    }

    // -----------------------------------------------------------------------
    // Document
    // -----------------------------------------------------------------------

    fn layout_document(&mut self) -> DisplayList {
        let doc = self.doc;
        let html = doc.nodes[doc.root].children.iter().copied().find(|&c| doc.nodes[c].tag() == Some("html"));
        let vw = self.vp.width.max(0.0);
        let mut background = color::TRANSPARENT;
        let mut out = Out::default();
        if let Some(html) = html {
            background = self.st.nodes[html].background_color;
            if background == 0 {
                background = self.st.nodes[doc.body].background_color;
            }
            let mut fl = Floats::default();
            let args = Args::block(vw, Some(self.vp.height));
            let mut o = self.layout_box(Bx::node(html), &args, Some((&mut fl, 0.0, 0.0)), true);
            out.h = o.h + o.mt.max(0.0) + o.mb.max(0.0);
            let abs = core::mem::take(&mut o.abs);
            out.absorb(o, 0.0, 0.0);
            // Remaining absolutes belong to the initial containing block.
            let cb = Rect { x: 0.0, y: 0.0, width: vw, height: self.vp.height };
            self.place_pending(&mut out, abs, cb, false);
        }
        let mut layers = core::mem::take(&mut out.layers);
        layers.sort_by_key(|l| l.0);
        let mut ordered: Vec<Paint> = Vec::new();
        for (z, p) in layers.iter_mut() {
            if *z < 0 {
                ordered.push(core::mem::take(p));
            }
        }
        ordered.push(core::mem::take(&mut out.paint));
        for (z, p) in layers.iter_mut() {
            if *z >= 0 {
                ordered.push(core::mem::take(p));
            }
        }
        let mut commands = Vec::new();
        let mut hits = Vec::new();
        for p in ordered {
            commands.extend(p.cmds);
            hits.extend(p.hits);
        }
        DisplayList { width: vw, height: math::ceil32(out.h.max(0.0)), background, commands, hit_regions: hits }
    }

    /// Lay out pending absolutely positioned boxes inside the containing block `cb`
    /// (expressed in `out`'s coordinate space) and add them on top of `out`.
    pub fn place_pending(&mut self, out: &mut Out, pending: Vec<Pending>, cb: Rect, keep_fixed: bool) {
        for p in pending {
            if p.fixed && keep_fixed {
                out.abs.push(p);
                continue;
            }
            let cbr = if p.fixed { Rect { x: 0.0, y: 0.0, width: self.vp.width, height: self.vp.height } } else { cb };
            let (child, (x, y)) = self.layout_abs(p.bx, cbr, p.x, p.y);
            out.absorb(child, x, y);
        }
    }

    /// Returns the laid out box and its border-box origin in the coordinate space of `cb`.
    pub fn layout_abs(&mut self, bx: Bx, cb: Rect, static_x: f32, static_y: f32) -> (Out, (f32, f32)) {
        let s = self.style(bx);
        let (margin, padding, border, auto) = self.edges(s, cb.width);
        let left = s.left.resolve(cb.width);
        let right = s.right.resolve(cb.width);
        let top = s.top.resolve(cb.height);
        let bottom = s.bottom.resolve(cb.height);
        let _ = (&padding, &border);
        let mut args = Args { avail_w: cb.width, cb_h: Some(cb.height), forced_w: None, forced_h: None, shrink: true };
        let replaced = self.is_replaced(bx);
        if s.width.is_auto() && !replaced {
            if let (Some(l), Some(r)) = (left, right) {
                args.forced_w = Some((cb.width - l - r - margin.l - margin.r).max(0.0));
                args.shrink = false;
            } else {
                let used = left.unwrap_or(0.0) + right.unwrap_or(0.0);
                args.avail_w = (cb.width - used).max(0.0);
            }
        }
        if s.height.is_auto() && !replaced && let (Some(t), Some(b)) = (top, bottom) {
            args.forced_h = Some((cb.height - t - b - margin.t - margin.b).max(0.0));
        }
        let out = self.layout_box(bx, &args, None, false);
        let (ow, oh) = (out.w, out.h);
        let mut ml = margin.l;
        let x = if let Some(l) = left {
            if auto[0] && auto[1] && let Some(r) = right {
                ml = ((cb.width - l - r - ow) / 2.0).max(0.0);
            }
            cb.x + l + ml
        } else if let Some(r) = right {
            cb.x + cb.width - r - margin.r - ow
        } else {
            static_x + margin.l
        };
        let y = if let Some(t) = top {
            cb.y + t + margin.t
        } else if let Some(b) = bottom {
            cb.y + cb.height - b - margin.b - oh
        } else {
            static_y + margin.t
        };
        (out, (x, y))
    }

    pub fn is_replaced(&self, bx: Bx) -> bool {
        matches!(
            self.tag(bx),
            Some("img" | "input" | "select" | "textarea" | "video" | "canvas" | "iframe" | "embed" | "object" | "svg" | "progress" | "meter" | "audio")
        )
    }

    /// Boxes that behave as one unit inside a line.
    pub fn is_atomic_inline(&self, bx: Bx) -> bool {
        let s = self.style(bx);
        matches!(s.display, Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable | Display::TableCell)
            || (s.display == Display::Inline && (self.is_replaced(bx) || self.tag(bx) == Some("button")))
    }

    // -----------------------------------------------------------------------
    // Boxes
    // -----------------------------------------------------------------------

    /// Lay out a box. `fl` carries the parent's float context and the BFC coordinates of this
    /// box's border-box origin when the box takes part in its parent's formatting context.
    pub fn layout_box(&mut self, bx: Bx, a: &Args, fl: Option<(&mut Floats, f32, f32)>, is_root: bool) -> Out {
        if self.depth >= MAX_DEPTH {
            return Out::default();
        }
        let s = self.style(bx);
        self.depth += 1;
        let out = self.layout_box_inner(bx, s, a, fl, is_root);
        self.depth -= 1;
        out
    }

    fn block_width(&mut self, bx: Bx, s: &Style, a: &Args, margin: &Box4, hpb: f32, is_table: bool) -> f32 {
        match &s.width {
            Dim::Len(..) | Dim::Func(..) => match s.width.resolve(a.avail_w) {
                Some(v) => {
                    let v = v.max(0.0);
                    if s.border_box { v } else { v + hpb }
                }
                None => a.avail_w,
            },
            Dim::MinContent => self.intrinsic(bx).0,
            Dim::MaxContent => self.intrinsic(bx).1,
            Dim::FitContent => {
                let (mn, mx) = self.intrinsic(bx);
                mx.min(mn.max(a.avail_w - margin.h()))
            }
            Dim::Auto => {
                let inline_shrink =
                    matches!(s.display, Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable);
                if a.shrink || s.float != Float::None || inline_shrink || is_table {
                    let (mn, mx) = self.intrinsic(bx);
                    let avail = (a.avail_w - margin.l.max(0.0) - margin.r.max(0.0)).max(0.0);
                    mx.min(mn.max(avail))
                } else {
                    (a.avail_w - margin.l - margin.r).max(0.0)
                }
            }
        }
    }

    fn layout_box_inner(&mut self, bx: Bx, s: &'a Style, a: &Args, fl: Option<(&mut Floats, f32, f32)>, is_root: bool) -> Out {
        let skip = self.cell_border_skip.take().unwrap_or([false; 4]);
        let (margin, padding, mut border, auto) = self.edges(s, a.avail_w);
        if skip[0] {
            border.t = 0.0;
        }
        if skip[1] {
            border.r = 0.0;
        }
        if skip[2] {
            border.b = 0.0;
        }
        if skip[3] {
            border.l = 0.0;
        }
        let hpb = padding.h() + border.h();
        let vpb = padding.v() + border.v();
        let replaced = self.is_replaced(bx);
        let rsize = if replaced { Some(self.replaced_size(bx, s, a, hpb, vpb)) } else { None };
        let is_table = matches!(s.display, Display::Table | Display::InlineTable);

        // ---- width ----
        let mut border_w = if let Some(fw) = a.forced_w {
            fw.max(0.0)
        } else if let Some((rw, _)) = rsize {
            rw + hpb
        } else {
            self.block_width(bx, s, a, &margin, hpb, is_table)
        };
        if a.forced_w.is_none() {
            let max = match &s.max_width {
                Dim::MinContent => Some(self.intrinsic(bx).0),
                Dim::MaxContent => Some(self.intrinsic(bx).1),
                d => d.resolve(a.avail_w).map(|v| if s.border_box { v } else { v + hpb }),
            };
            if let Some(m) = max {
                border_w = border_w.min(m);
            }
            let min = match &s.min_width {
                Dim::MinContent => Some(self.intrinsic(bx).0),
                Dim::MaxContent => Some(self.intrinsic(bx).1),
                d => d.resolve(a.avail_w).map(|v| if s.border_box { v } else { v + hpb }),
            };
            if let Some(m) = min {
                border_w = border_w.max(m);
            }
        }
        let border_w = border_w.max(hpb);
        let content_w = border_w - hpb;

        // horizontal margins (auto margins center / push blocks)
        let (mut ml, mut mr) = (margin.l, margin.r);
        let block_level = s.display.is_block_level();
        if block_level && s.float == Float::None && !s.is_out_of_flow() && a.forced_w.is_none() && (auto[0] || auto[1]) {
            let fixed_l = if auto[0] { 0.0 } else { margin.l };
            let fixed_r = if auto[1] { 0.0 } else { margin.r };
            let free = (a.avail_w - border_w - fixed_l - fixed_r).max(0.0);
            if auto[0] && auto[1] {
                ml = free / 2.0;
                mr = free / 2.0;
            } else if auto[0] {
                ml = free;
            } else {
                mr = free;
            }
        }

        // ---- height ----
        let origin_x = border.l + padding.l;
        let origin_y = border.t + padding.t;
        let height_resolved = match &s.height {
            Dim::Len(_, pct) if *pct != 0.0 && a.cb_h.is_none() => None,
            d => d.resolve(a.cb_h.unwrap_or(0.0)),
        };
        let content_h_known: Option<f32> = if let Some(fh) = a.forced_h {
            Some((fh - vpb).max(0.0))
        } else if let Some(h) = height_resolved {
            let h = h.max(0.0);
            Some(if s.border_box { (h - vpb).max(0.0) } else { h })
        } else if let Some((_, rh)) = rsize {
            Some(rh)
        } else if let Some(r) = s.aspect_ratio
            && r > 0.0
            && s.display != Display::Inline
        {
            Some(content_w / r)
        } else {
            None
        };

        // ---- content ----
        let mut content = Out::default();
        let mut info = FlowInfo::default();
        let mut own_floats = Floats::default();
        let is_bfc = s.is_bfc_root() || is_root || fl.is_none();
        let collapse_top = !is_bfc && padding.t == 0.0 && border.t == 0.0;
        let collapse_bottom = !is_bfc
            && padding.b == 0.0
            && border.b == 0.0
            && content_h_known.is_none()
            && s.min_height.resolve(0.0).unwrap_or(0.0) == 0.0;
        let mut content_h;
        if rsize.is_some() {
            content_h = content_h_known.unwrap_or(0.0);
            content = self.paint_replaced(bx, s, content_w, content_h);
        } else {
            match s.display {
                Display::Flex | Display::InlineFlex => {
                    let (o, h) = self.layout_flex(bx, s, content_w, content_h_known);
                    content_h = h;
                    info.first_baseline = o.baseline;
                    content = o;
                }
                Display::Grid | Display::InlineGrid => {
                    let (o, h) = self.layout_grid(bx, s, content_w, content_h_known);
                    content_h = h;
                    info.first_baseline = o.baseline;
                    content = o;
                }
                Display::Table | Display::InlineTable => {
                    let (o, h) = self.layout_table(bx, s, content_w, border_w);
                    content_h = h;
                    info.first_baseline = o.baseline;
                    content = o;
                }
                _ => {
                    let (floats, fox, foy): (&mut Floats, f32, f32) = match fl {
                        Some((f, x, y)) if !is_bfc => (f, x + origin_x, y + origin_y),
                        _ => (&mut own_floats, 0.0, 0.0),
                    };
                    let fa = FlowArgs { cw: content_w, collapse_top, collapse_bottom, cb_h: content_h_known };
                    let r = self.layout_flow(bx, s, &fa, floats, fox, foy);
                    content_h = r.h;
                    if is_bfc {
                        content_h = content_h.max(own_floats.bottom(Clear::Both));
                    }
                    info.esc_top = r.esc_top;
                    info.esc_bottom = r.esc_bottom;
                    info.empty = r.empty;
                    info.first_baseline = r.first_baseline;
                    info.last_baseline = r.last_baseline;
                    content.paint = r.paint;
                    content.layers = r.layers;
                    content.abs = r.abs;
                    if s.display == Display::ListItem {
                        self.add_list_marker(bx, s, &mut content.paint, r.first_baseline);
                    }
                }
            }
            if let Some(h) = content_h_known {
                content_h = h;
            }
        }
        let mut border_h = content_h + vpb;
        let cbh = a.cb_h.unwrap_or(0.0);
        let resolve_h = |d: &Dim| match d {
            Dim::Len(_, pct) if *pct != 0.0 && a.cb_h.is_none() => None,
            d => d.resolve(cbh),
        };
        if let Some(m) = resolve_h(&s.max_height) {
            border_h = border_h.min(if s.border_box { m } else { m + vpb });
        }
        if let Some(m) = resolve_h(&s.min_height) {
            border_h = border_h.max(if s.border_box { m } else { m + vpb });
        }
        if let Some(fh) = a.forced_h {
            border_h = fh;
        }
        let border_h = border_h.max(vpb);

        // ---- assemble ----
        let mut out = Out { w: border_w, h: border_h, ml, mr, ..Default::default() };
        let rect = Rect { x: 0.0, y: 0.0, width: border_w, height: border_h };
        let propagate_bg = is_root || (self.tag(bx) == Some("body") && self.html_bg_none());
        if s.visibility_visible {
            self.paint_decorations(&mut out.paint, s, rect, propagate_bg, skip);
        }
        let clip = (s.overflow_x.clips() || s.overflow_y.clips()) && !is_root && !matches!(s.display, Display::Contents);
        if clip {
            let inner = Rect {
                x: border.l,
                y: border.t,
                width: (border_w - border.h()).max(0.0),
                height: (border_h - border.v()).max(0.0),
            };
            out.paint.cmds.push(Command::PushClip { rect: inner, radius: (s.border_radius - border.l.max(border.t)).max(0.0) });
        }
        if s.pointer_events {
            out.paint.hits.push(HitRegion { rect, node: bx.id });
        }
        let mut abs = core::mem::take(&mut content.abs);
        let content_layers = core::mem::take(&mut content.layers);
        out.paint.append(content.paint, origin_x, origin_y);
        for (z, mut p) in content_layers {
            p.translate(origin_x, origin_y);
            out.layers.push((z, p));
        }
        for p in abs.iter_mut() {
            p.x += origin_x;
            p.y += origin_y;
        }
        if clip {
            out.paint.cmds.push(Command::PopClip);
        }
        out.baseline = info.first_baseline.map(|b| b + origin_y);
        out.last_baseline = info.last_baseline.or(info.first_baseline).map(|b| b + origin_y);

        // vertical margins, collapsing through the box edges
        out.mt = if collapse_top { collapse(margin.t, info.esc_top) } else { margin.t };
        out.mb = if collapse_bottom { collapse(margin.b, info.esc_bottom) } else { margin.b };
        out.collapses_through = !is_bfc
            && info.empty
            && border_h <= 0.0
            && content_h_known.is_none()
            && matches!(s.display, Display::Block | Display::ListItem)
            && rsize.is_none();
        if out.collapses_through {
            out.mt = collapse(out.mt, out.mb);
            out.mb = out.mt;
        }

        // positioned boxes are containing blocks for their absolute descendants
        let positioned = s.is_positioned();
        if positioned && !is_root {
            let cb = Rect {
                x: border.l,
                y: border.t,
                width: (border_w - border.h()).max(0.0),
                height: (border_h - border.v()).max(0.0),
            };
            let (own, rest): (Vec<_>, Vec<_>) = abs.into_iter().partition(|p| !p.fixed);
            abs = rest;
            self.place_pending(&mut out, own, cb, false);
        }
        out.abs.append(&mut abs);

        if s.opacity < 1.0 {
            out.paint.apply_opacity(s.opacity);
            for (_, p) in out.layers.iter_mut() {
                p.apply_opacity(s.opacity);
            }
        }
        if positioned && s.z_index.is_some() {
            out.z = s.z_index;
        }
        let mut shift = (0.0, 0.0);
        if s.position == Position::Relative {
            let l = s.left.resolve(a.avail_w);
            let r = s.right.resolve(a.avail_w);
            let t = s.top.resolve(cbh);
            let b = s.bottom.resolve(cbh);
            shift.0 += l.or(r.map(|v| -v)).unwrap_or(0.0);
            shift.1 += t.or(b.map(|v| -v)).unwrap_or(0.0);
        }
        shift.0 += s.translate.0.resolve(border_w).unwrap_or(0.0);
        shift.1 += s.translate.1.resolve(border_h).unwrap_or(0.0);
        out.shift = shift;
        out
    }

    fn html_bg_none(&self) -> bool {
        let doc = self.doc;
        doc.nodes[doc.root]
            .children
            .iter()
            .copied()
            .find(|&c| doc.nodes[c].tag() == Some("html"))
            .is_some_and(|h| self.st.nodes[h].background_color == 0)
    }

    pub fn paint_decorations(&mut self, paint: &mut Paint, s: &Style, rect: Rect, propagate_bg: bool, skip: [bool; 4]) {
        for sh in s.shadows.iter().filter(|x| !x.inset) {
            paint.cmds.push(Command::Shadow {
                rect,
                radius: s.border_radius,
                dx: sh.dx,
                dy: sh.dy,
                blur: sh.blur,
                spread: sh.spread,
                color: sh.color,
                inset: false,
            });
        }
        let (_, _, mut border, _) = self.edges(s, 0.0);
        for (i, w) in [&mut border.t, &mut border.r, &mut border.b, &mut border.l].into_iter().enumerate() {
            if skip[i] {
                *w = 0.0;
            }
        }
        if !propagate_bg {
            if color::alpha(s.background_color) > 0 {
                paint.cmds.push(Command::FillRect { rect, color: s.background_color, radius: s.border_radius });
            }
            self.paint_background_image(paint, s, rect, &border);
        }
        for sh in s.shadows.iter().filter(|x| x.inset) {
            paint.cmds.push(Command::Shadow {
                rect,
                radius: s.border_radius,
                dx: sh.dx,
                dy: sh.dy,
                blur: sh.blur,
                spread: sh.spread,
                color: sh.color,
                inset: true,
            });
        }
        if border.t > 0.0 || border.r > 0.0 || border.b > 0.0 || border.l > 0.0 {
            let mut widths = [border.t, border.r, border.b, border.l];
            let mut styles = [0u8; 4];
            let mut colors = [0u32; 4];
            for i in 0..4 {
                styles[i] = match s.border_style[i] {
                    BorderStyle::None | BorderStyle::Hidden => 0,
                    BorderStyle::Solid => 1,
                    BorderStyle::Dashed => 2,
                    BorderStyle::Dotted => 3,
                    BorderStyle::Double => 4,
                };
                if styles[i] == 0 {
                    widths[i] = 0.0;
                }
                colors[i] = s.border_color[i].unwrap_or(s.color);
            }
            paint.cmds.push(Command::Border { rect, widths, colors, styles, radius: s.border_radius });
        }
    }

    fn paint_background_image(&mut self, paint: &mut Paint, s: &Style, rect: Rect, border: &Box4) {
        let pad_rect = Rect {
            x: rect.x + border.l,
            y: rect.y + border.t,
            width: (rect.width - border.h()).max(0.0),
            height: (rect.height - border.v()).max(0.0),
        };
        match &s.background_image {
            BgImage::None => {}
            BgImage::Gradient(g) => {
                let n = g.stops.len();
                let mut stops: Vec<(f32, Color)> = Vec::with_capacity(n);
                for (i, (c, p)) in g.stops.iter().enumerate() {
                    let pos = p.unwrap_or(if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 });
                    stops.push((pos.clamp(0.0, 1.0), *c));
                }
                paint.cmds.push(Command::Gradient { rect: pad_rect, angle: g.angle, radial: g.radial, stops, radius: s.border_radius });
            }
            BgImage::Url(url) => {
                if pad_rect.width <= 0.0 || pad_rect.height <= 0.0 {
                    return;
                }
                let natural = self.fonts.image_size(url);
                let (iw, ih) = match (&s.background_size, natural) {
                    (BgSize::Cover | BgSize::Contain, Some((w, h))) if w > 0.0 && h > 0.0 => {
                        let sx = pad_rect.width / w;
                        let sy = pad_rect.height / h;
                        let k = if matches!(s.background_size, BgSize::Cover) { sx.max(sy) } else { sx.min(sy) };
                        (w * k, h * k)
                    }
                    (BgSize::Cover | BgSize::Contain, _) => (pad_rect.width, pad_rect.height),
                    (BgSize::Explicit(w, h), nat) => {
                        let rw = w.resolve(pad_rect.width);
                        let rh = h.resolve(pad_rect.height);
                        match (rw, rh, nat) {
                            (Some(a), Some(b), _) => (a, b),
                            (Some(a), None, Some((nw, nh))) if nw > 0.0 => (a, a * nh / nw),
                            (None, Some(b), Some((nw, nh))) if nh > 0.0 => (b * nw / nh, b),
                            (Some(a), None, None) => (a, a),
                            (None, Some(b), None) => (b, b),
                            _ => nat.unwrap_or((pad_rect.width, pad_rect.height)),
                        }
                    }
                    (_, Some((w, h))) => (w, h),
                    (_, None) => {
                        paint.cmds.push(Command::Image { rect: pad_rect, src: url.clone(), link: None, opacity: 1.0 });
                        return;
                    }
                };
                if iw <= 0.0 || ih <= 0.0 {
                    return;
                }
                let px = s.background_position.0.resolve(pad_rect.width - iw).unwrap_or(0.0);
                let py = s.background_position.1.resolve(pad_rect.height - ih).unwrap_or(0.0);
                let (rx, ry) = match s.background_repeat {
                    BgRepeat::Repeat => (true, true),
                    BgRepeat::RepeatX => (true, false),
                    BgRepeat::RepeatY => (false, true),
                    BgRepeat::NoRepeat => (false, false),
                };
                let start_x = if rx { px - math::ceil32(px / iw) * iw } else { px };
                let start_y = if ry { py - math::ceil32(py / ih) * ih } else { py };
                let cols = if rx { math::ceil32((pad_rect.width - start_x) / iw).max(1.0) as usize } else { 1 };
                let rows = if ry { math::ceil32((pad_rect.height - start_y) / ih).max(1.0) as usize } else { 1 };
                if cols * rows > 400 {
                    paint.cmds.push(Command::Image { rect: pad_rect, src: url.clone(), link: None, opacity: 1.0 });
                    return;
                }
                paint.cmds.push(Command::PushClip { rect: pad_rect, radius: s.border_radius });
                for r in 0..rows {
                    for c in 0..cols {
                        paint.cmds.push(Command::Image {
                            rect: Rect { x: pad_rect.x + start_x + c as f32 * iw, y: pad_rect.y + start_y + r as f32 * ih, width: iw, height: ih },
                            src: url.clone(),
                            link: None,
                            opacity: 1.0,
                        });
                    }
                }
                paint.cmds.push(Command::PopClip);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct FlowInfo {
    pub esc_top: f32,
    pub esc_bottom: f32,
    pub empty: bool,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
}

pub(crate) struct FlowArgs {
    pub cw: f32,
    pub collapse_top: bool,
    pub collapse_bottom: bool,
    pub cb_h: Option<f32>,
}

pub(crate) struct Flow {
    pub h: f32,
    pub esc_top: f32,
    pub esc_bottom: f32,
    pub empty: bool,
    pub paint: Paint,
    pub layers: Vec<(i32, Paint)>,
    pub abs: Vec<Pending>,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
}

/// An item of the flattened child stream of a block container.
pub(crate) enum FlowItem {
    Inline(crate::layout_inline::Item),
    Block(Bx),
    Float(Bx),
    Abs(Bx),
}

impl<'a> Ctx<'a> {
    /// Block formatting of `bx`'s in-flow children.
    pub fn layout_flow(&mut self, bx: Bx, s: &'a Style, fa: &FlowArgs, fl: &mut Floats, fox: f32, foy: f32) -> Flow {
        let items = self.flatten(bx);
        self.layout_flow_items(items, s, fa, fl, fox, foy)
    }

    pub fn layout_flow_items(&mut self, items: Vec<FlowItem>, s: &'a Style, fa: &FlowArgs, fl: &mut Floats, fox: f32, foy: f32) -> Flow {
        let cw = fa.cw;
        let mut flow = Flow {
            h: 0.0,
            esc_top: 0.0,
            esc_bottom: 0.0,
            empty: true,
            paint: Paint::default(),
            layers: Vec::new(),
            abs: Vec::new(),
            first_baseline: None,
            last_baseline: None,
        };
        let mut float_paint = Paint::default();
        let mut cursor = 0.0f32;
        let mut pending = 0.0f32;
        let mut at_top = true;
        let mut baseline_set = false;
        let mut run: Vec<crate::layout_inline::Item> = Vec::new();

        macro_rules! flush_run {
            () => {
                if !run.is_empty() {
                    let items = core::mem::take(&mut run);
                    let escapes = at_top && fa.collapse_top;
                    let top_pad = if escapes { 0.0 } else { pending };
                    let y0 = cursor + top_pad;
                    let r = self.layout_inline_run(items, s, cw, fa.cb_h, foy + y0, fl, fox);
                    flow.paint.append(r.paint, 0.0, y0);
                    float_paint.append(r.float_paint, 0.0, y0);
                    for (z, mut p) in r.layers {
                        p.translate(0.0, y0);
                        flow.layers.push((z, p));
                    }
                    for mut p in r.abs {
                        p.y += y0;
                        flow.abs.push(p);
                    }
                    if !r.empty {
                        if escapes {
                            flow.esc_top = collapse(flow.esc_top, pending);
                        }
                        if !baseline_set {
                            flow.first_baseline = r.first_baseline.map(|b| b + y0);
                            baseline_set = true;
                        }
                        flow.last_baseline = r.last_baseline.map(|b| b + y0);
                        cursor = y0 + r.h;
                        pending = 0.0;
                        at_top = false;
                        flow.empty = false;
                    }
                }
            };
        }

        for item in items {
            match item {
                FlowItem::Inline(i) => run.push(i),
                FlowItem::Abs(b) => {
                    if run.iter().all(|i| i.is_collapsible_space()) {
                        run.clear();
                        let y = if at_top && fa.collapse_top { cursor } else { cursor + pending };
                        flow.abs.push(Pending { bx: b, x: 0.0, y, fixed: self.style(b).position == Position::Fixed });
                    } else {
                        run.push(crate::layout_inline::Item::Abs(b));
                    }
                }
                FlowItem::Float(b) => {
                    if run.iter().all(|i| i.is_collapsible_space()) {
                        run.clear();
                        let y = if at_top && fa.collapse_top { cursor } else { cursor + pending };
                        let (o, x, fy) = self.place_float(b, cw, fa.cb_h, fl, fox, foy + y);
                        let mut tmp = Out::default();
                        tmp.absorb(o, x, fy - foy);
                        float_paint.append(tmp.paint, 0.0, 0.0);
                        flow.layers.extend(tmp.layers);
                        flow.abs.extend(tmp.abs);
                    } else {
                        run.push(crate::layout_inline::Item::Float(b));
                    }
                }
                FlowItem::Block(b) => {
                    flush_run!();
                    let cs = self.style(b);
                    if cs.clear != Clear::None {
                        let target = fl.bottom(cs.clear) - foy;
                        if target > cursor + pending {
                            cursor = target;
                            pending = 0.0;
                            at_top = false;
                            flow.empty = false;
                        }
                    }
                    let (cm, _, _, cauto) = self.edges(cs, cw);
                    let est_top = cursor + collapse(pending, cm.t);
                    let est_left = if cauto[0] { 0.0 } else { cm.l };
                    let mut args = Args::block(cw, fa.cb_h);
                    let mut x_off = 0.0;
                    // Boxes that establish a BFC do not overlap floats: shrink beside them.
                    if cs.is_bfc_root() && cs.float == Float::None && !fl.list.is_empty() {
                        let (l, r) = fl.avail(foy + est_top, 1.0, fox, fox + cw);
                        if l > fox || r < fox + cw {
                            args.avail_w = (r - l).max(0.0);
                            x_off = l - fox;
                        }
                    }
                    let out = self.layout_box(b, &args, Some((&mut *fl, fox + est_left + x_off, foy + est_top)), false);
                    if out.collapses_through && out.paint.cmds.is_empty() && out.abs.is_empty() && out.layers.is_empty() {
                        pending = collapse(pending, out.mt);
                        let at = cursor + pending;
                        flow.paint.hits.extend(out.paint.hits.into_iter().map(|mut h| {
                            h.rect.y += at;
                            h
                        }));
                        if at_top && fa.collapse_top {
                            flow.esc_top = pending;
                        }
                        continue;
                    }
                    let gap_before = if at_top && fa.collapse_top {
                        flow.esc_top = collapse(flow.esc_top, out.mt);
                        0.0
                    } else {
                        collapse(pending, out.mt)
                    };
                    let y = cursor + gap_before;
                    let x = out.ml + x_off;
                    if !baseline_set && out.baseline.is_some() {
                        flow.first_baseline = out.baseline.map(|bl| bl + y + out.shift.1);
                        baseline_set = true;
                    }
                    if out.last_baseline.is_some() {
                        flow.last_baseline = out.last_baseline.map(|bl| bl + y + out.shift.1);
                    }
                    let (h, mb) = (out.h, out.mb);
                    let mut sub = Out::default();
                    sub.absorb(out, 0.0, 0.0);
                    flow.paint.append(sub.paint, x, y);
                    for (z, mut p) in sub.layers {
                        p.translate(x, y);
                        flow.layers.push((z, p));
                    }
                    for mut p in sub.abs {
                        p.x += x;
                        p.y += y;
                        flow.abs.push(p);
                    }
                    cursor = y + h;
                    pending = mb;
                    at_top = false;
                    flow.empty = false;
                }
            }
        }
        flush_run!();
        if fa.collapse_bottom && !at_top {
            flow.esc_bottom = pending;
            flow.h = cursor;
        } else if fa.collapse_bottom {
            flow.esc_top = collapse(flow.esc_top, pending);
            flow.h = 0.0;
        } else {
            flow.h = cursor + pending;
        }
        flow.paint.append(float_paint, 0.0, 0.0);
        flow
    }

    /// Lay out a float and register it. Returns the box and its border-box origin in flow
    /// coordinates (x relative to the flow's content box, `y` in BFC coordinates).
    pub fn place_float(&mut self, b: Bx, cw: f32, cb_h: Option<f32>, fl: &mut Floats, fox: f32, y_bfc: f32) -> (Out, f32, f32) {
        let s = self.style(b);
        let args = Args { avail_w: cw, cb_h, forced_w: None, forced_h: None, shrink: true };
        let out = self.layout_box(b, &args, None, false);
        let (m, _, _, _) = self.edges(s, cw);
        let outer_w = out.w + m.l + m.r;
        let outer_h = out.h + m.t + m.b;
        let is_left = s.float == Float::Left;
        let mut y = y_bfc.max(fl.last_top());
        if s.clear != Clear::None {
            y = y.max(fl.bottom(s.clear));
        }
        let mut guard = 0;
        let x = loop {
            let (l, r) = fl.avail(y, outer_h, fox, fox + cw);
            let fits = outer_w <= r - l + 0.01;
            let next = if fits || guard > 200 { None } else { fl.next_bottom(y, outer_h) };
            match next {
                Some(nb) if nb > y => {
                    y = nb;
                    guard += 1;
                }
                _ => break if is_left { l } else { (r - outer_w).max(l) },
            }
        };
        fl.list.push(FloatBox { x0: x, x1: x + outer_w, y0: y, y1: y + outer_h, left: is_left });
        (out, x - fox + m.l, y + m.t)
    }
}
