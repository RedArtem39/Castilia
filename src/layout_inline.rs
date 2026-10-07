//! Inline formatting: flattening children into items, line breaking, line boxes and
//! intrinsic (min/max-content) widths.
use crate::{
    color,
    css::*,
    layout::*,
    math,
};
use alloc::{string::String, vec::Vec};

#[derive(Clone)]
pub(crate) enum Item {
    Text(Bx, String),
    Open(Bx),
    Close(Bx),
    Atomic(Bx),
    Break,
    Float(Bx),
    Abs(Bx),
}

impl Item {
    pub fn is_collapsible_space(&self) -> bool {
        match self {
            Item::Text(_, t) => t.chars().all(is_ws),
            Item::Open(_) | Item::Close(_) => true,
            _ => false,
        }
    }
}

pub(crate) fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

#[derive(Clone)]
enum Atom {
    Word { text: String, w: f32, bx: Bx, break_after: bool, wrap: bool, split_ok: bool, shift: f32, mode: u8 },
    Space { w: f32, bx: Bx, wrap: bool, shift: f32, mode: u8 },
    Newline,
    Atomic { bx: Bx, shift: f32, mode: u8 },
    Open(Bx),
    Close(Bx),
    Float(Bx),
    Abs(Bx),
}

struct Frag {
    kind: FragKind,
    w: f32,
    break_after: bool,
    wrap: bool,
    shift: f32,
    mode: u8,
    split_ok: bool,
}

enum FragKind {
    Text { text: String, bx: Bx },
    Space { bx: Bx },
    Atomic { out: Out, bx: Bx, ml: f32, mt: f32, mb: f32 },
    Open { bx: Bx, first: bool, ml: f32, decor: f32 },
    Close { bx: Bx, last: bool, mr: f32, decor: f32 },
}

pub(crate) struct InlineOut {
    pub h: f32,
    pub empty: bool,
    pub paint: Paint,
    pub float_paint: Paint,
    pub layers: Vec<(i32, Paint)>,
    pub abs: Vec<Pending>,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    pub lines: usize,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

fn capitalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_start = true;
    for c in text.chars() {
        if at_start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            at_start = false;
        } else {
            out.push(c);
            if is_ws(c) {
                at_start = true;
            } else if c.is_alphanumeric() {
                at_start = false;
            }
        }
    }
    out
}

impl<'a> Ctx<'a> {
    /// Link target of the nearest `a[href]` ancestor.
    pub fn link_of(&self, bx: Bx) -> Option<String> {
        let doc: &'a crate::dom::Document = self.doc;
        let mut cur = Some(bx.id);
        while let Some(id) = cur {
            if doc.nodes[id].tag() == Some("a")
                && let Some(h) = doc.nodes[id].attribute("href")
            {
                return Some(h.into());
            }
            cur = doc.nodes[id].parent;
        }
        None
    }

    // -----------------------------------------------------------------------
    // Flattening
    // -----------------------------------------------------------------------

    pub fn flatten(&self, bx: Bx) -> Vec<FlowItem> {
        let mut out = Vec::new();
        self.flatten_into(bx, &mut out, 0);
        out
    }

    fn flatten_into(&self, bx: Bx, out: &mut Vec<FlowItem>, depth: usize) {
        if depth > 200 {
            return;
        }
        if bx.pseudo == 0
            && let Some(t) = text_of(self.doc, bx.id)
        {
            if !t.is_empty() {
                out.push(FlowItem::Inline(Item::Text(bx, t.into())));
            }
            return;
        }
        if bx.pseudo == 0 {
            let s = self.style(bx);
            if s.display == Display::ListItem
                && s.list_style_inside
                && let Some(m) = self.marker_text(bx, s)
            {
                out.push(FlowItem::Inline(Item::Text(bx, alloc::format!("{m} "))));
            }
        }
        if bx.pseudo != 0 {
            let t = self.pseudo_text(bx);
            if !t.is_empty() {
                out.push(FlowItem::Inline(Item::Text(bx, t)));
            }
            return;
        }
        for c in self.children(bx) {
            if c.pseudo == 0
                && let Some(t) = text_of(self.doc, c.id)
            {
                if !t.is_empty() {
                    out.push(FlowItem::Inline(Item::Text(c, t.into())));
                }
                continue;
            }
            if c.pseudo == 0 && self.doc.nodes[c.id].tag().is_none() {
                continue;
            }
            let s = self.style(c);
            match s.display {
                Display::None => continue,
                Display::Contents => {
                    self.flatten_into(c, out, depth + 1);
                    continue;
                }
                _ => {}
            }
            if s.is_out_of_flow() {
                out.push(FlowItem::Abs(c));
                continue;
            }
            if s.float != Float::None {
                out.push(FlowItem::Float(c));
                continue;
            }
            if self.tag(c) == Some("br") {
                out.push(FlowItem::Inline(Item::Break));
                continue;
            }
            if self.tag(c) == Some("wbr") {
                continue;
            }
            if self.is_atomic_inline(c) {
                out.push(FlowItem::Inline(Item::Atomic(c)));
                continue;
            }
            match s.display {
                Display::Inline => {
                    out.push(FlowItem::Inline(Item::Open(c)));
                    self.flatten_into(c, out, depth + 1);
                    out.push(FlowItem::Inline(Item::Close(c)));
                }
                Display::TableColumn => {}
                _ => out.push(FlowItem::Block(c)),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Atoms
    // -----------------------------------------------------------------------

    fn needs_marks(&self, s: &Style) -> bool {
        let pad = |i: usize| s.padding[i].resolve(0.0).unwrap_or(0.0) != 0.0 || s.padding[i].has_percent();
        let mar = |i: usize| s.margin[i].resolve(0.0).unwrap_or(0.0) != 0.0;
        pad(1) || pad(3) || mar(1) || mar(3) || s.border_width[1] > 0.0 || s.border_width[3] > 0.0
            || color::alpha(s.background_color) > 0
            || !matches!(s.background_image, BgImage::None)
            || s.border_width[0] > 0.0 || s.border_width[2] > 0.0
            || pad(0) || pad(2)
    }

    fn valign_of(&self, s: &Style, parent_size: f32) -> (f32, u8) {
        match &s.vertical_align {
            VerticalAlign::Baseline => (0.0, 0),
            VerticalAlign::Sub => (-(parent_size / 5.0 + 1.0), 0),
            VerticalAlign::Super => (parent_size / 3.0 + 1.0, 0),
            VerticalAlign::Len(d) => {
                let lh = self.used_line_height(s);
                (d.resolve(lh).unwrap_or(0.0), 0)
            }
            VerticalAlign::Top | VerticalAlign::TextTop => (0.0, 1),
            VerticalAlign::Bottom | VerticalAlign::TextBottom => (0.0, 2),
            VerticalAlign::Middle => (0.0, 3),
        }
    }

    fn make_atoms(&mut self, items: &[Item], block: &Style) -> Vec<Atom> {
        let mut atoms = Vec::new();
        let mut stack: Vec<(f32, u8)> = Vec::new();
        let (mut cum_shift, mut cum_mode) = (0.0f32, 0u8);
        let mut stack_sizes: Vec<f32> = Vec::new();
        let _ = block;
        for item in items {
            match item {
                Item::Open(bx) => {
                    let s = self.style(*bx);
                    let parent_size = stack_sizes.last().copied().unwrap_or(block.font_size);
                    let (sh, mode) = self.valign_of(s, parent_size);
                    stack.push((sh, mode));
                    stack_sizes.push(s.font_size);
                    cum_shift += sh;
                    if mode != 0 {
                        cum_mode = mode;
                    }
                    if self.needs_marks(s) {
                        atoms.push(Atom::Open(*bx));
                    }
                }
                Item::Close(bx) => {
                    let s = self.style(*bx);
                    if let Some((sh, _)) = stack.pop() {
                        cum_shift -= sh;
                        cum_mode = stack.iter().rev().map(|x| x.1).find(|&m| m != 0).unwrap_or(0);
                    }
                    stack_sizes.pop();
                    if self.needs_marks(s) {
                        atoms.push(Atom::Close(*bx));
                    }
                }
                Item::Break => atoms.push(Atom::Newline),
                Item::Float(bx) => atoms.push(Atom::Float(*bx)),
                Item::Abs(bx) => atoms.push(Atom::Abs(*bx)),
                Item::Atomic(bx) => {
                    let s = self.style(*bx);
                    let parent_size = stack_sizes.last().copied().unwrap_or(block.font_size);
                    let (sh, mode) = self.valign_of(s, parent_size);
                    atoms.push(Atom::Atomic { bx: *bx, shift: cum_shift + sh, mode: if mode != 0 { mode } else { cum_mode } });
                }
                Item::Text(bx, text) => {
                    self.text_atoms(*bx, text, cum_shift, cum_mode, &mut atoms);
                }
            }
        }
        atoms
    }

    fn text_atoms(&mut self, bx: Bx, raw: &str, shift: f32, mode: u8, atoms: &mut Vec<Atom>) {
        let s = self.style(bx);
        let text_owned;
        let text: &str = match s.text_transform {
            TextTransform::None => raw,
            TextTransform::Uppercase => {
                text_owned = raw.to_uppercase();
                &text_owned
            }
            TextTransform::Lowercase => {
                text_owned = raw.to_lowercase();
                &text_owned
            }
            TextTransform::Capitalize => {
                text_owned = capitalize(raw);
                &text_owned
            }
        };
        let ws = s.white_space;
        let wrap = ws.wraps();
        let space_w = self.text_w(" ", s) + s.word_spacing;
        let split_ok = s.overflow_wrap_break || s.word_break_all;
        let mut push_word = |this: &mut Self, atoms: &mut Vec<Atom>, word: &str| {
            // Split into break opportunities.
            let mut piece = String::new();
            let chars: Vec<char> = word.chars().collect();
            let mut i = 0;
            let flush = |this: &mut Self, atoms: &mut Vec<Atom>, piece: &mut String, break_after: bool| {
                if piece.is_empty() {
                    return;
                }
                let w = this.text_w(piece, s);
                atoms.push(Atom::Word { text: core::mem::take(piece), w, bx, break_after: break_after && wrap, wrap, split_ok, shift, mode });
            };
            while i < chars.len() {
                let c = chars[i];
                piece.push(c);
                let next = chars.get(i + 1).copied();
                let break_here = if s.word_break_all {
                    true
                } else if is_cjk(c) {
                    !next.is_some_and(|n| matches!(n, '、' | '。' | '，' | '．' | '：' | '；' | '！' | '？' | ')' | '）' | '」' | '』' | '】' | '”' | '’'))
                } else if c == '-' || c == '\u{2010}' || c == '\u{2013}' || c == '\u{2014}' {
                    next.is_some_and(|n| n.is_alphanumeric()) && piece.chars().count() > 1
                } else if c == '/' {
                    false
                } else {
                    false
                };
                if break_here && next.is_some() {
                    flush(this, atoms, &mut piece, true);
                }
                i += 1;
            }
            flush(this, atoms, &mut piece, true);
        };
        if ws.collapses_spaces() {
            let mut word = String::new();
            let mut pending_space = false;
            let mut chars = text.chars().peekable();
            while let Some(c) = chars.next() {
                if is_ws(c) {
                    if c == '\n' && ws == WhiteSpace::PreLine {
                        if !word.is_empty() {
                            let w = core::mem::take(&mut word);
                            push_word(self, atoms, &w);
                        }
                        pending_space = false;
                        atoms.push(Atom::Newline);
                        // collapse following spaces
                        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                            chars.next();
                        }
                        continue;
                    }
                    if !word.is_empty() {
                        let w = core::mem::take(&mut word);
                        push_word(self, atoms, &w);
                    }
                    pending_space = true;
                } else {
                    if pending_space {
                        atoms.push(Atom::Space { w: space_w, bx, wrap, shift, mode });
                        pending_space = false;
                    }
                    word.push(c);
                }
            }
            if !word.is_empty() {
                push_word(self, atoms, &word);
            }
            if pending_space {
                atoms.push(Atom::Space { w: space_w, bx, wrap, shift, mode });
            }
        } else {
            // pre / pre-wrap: keep spaces and newlines
            let mut first = true;
            for line in text.split('\n') {
                if !first {
                    atoms.push(Atom::Newline);
                }
                first = false;
                let line = line.trim_end_matches('\r');
                let expanded;
                let line = if line.contains('\t') {
                    expanded = line.replace('\t', "        ");
                    expanded.as_str()
                } else {
                    line
                };
                if line.is_empty() {
                    continue;
                }
                if ws == WhiteSpace::Pre {
                    let w = self.text_w(line, s);
                    atoms.push(Atom::Word { text: line.into(), w, bx, break_after: false, wrap: false, split_ok: false, shift, mode });
                } else {
                    // pre-wrap: words and runs of spaces, both may wrap
                    let mut cur = String::new();
                    let mut in_space = false;
                    for c in line.chars() {
                        let sp = c == ' ';
                        if !cur.is_empty() && sp != in_space {
                            let w = self.text_w(&cur, s);
                            atoms.push(Atom::Word { text: core::mem::take(&mut cur), w, bx, break_after: true, wrap: true, split_ok, shift, mode });
                        }
                        in_space = sp;
                        cur.push(c);
                    }
                    if !cur.is_empty() {
                        let w = self.text_w(&cur, s);
                        atoms.push(Atom::Word { text: cur, w, bx, break_after: true, wrap: true, split_ok, shift, mode });
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Intrinsic widths of an inline run
    // -----------------------------------------------------------------------

    pub fn inline_intrinsic(&mut self, items: &[Item], block: &Style) -> (f32, f32) {
        let atoms = self.make_atoms(items, block);
        let (mut min, mut max) = (0.0f32, 0.0f32);
        let (mut seg, mut line) = (0.0f32, 0.0f32);
        let mut prev_space = true;
        for atom in &atoms {
            match atom {
                Atom::Word { w, break_after, .. } => {
                    seg += w;
                    line += w;
                    prev_space = false;
                    if *break_after {
                        min = min.max(seg);
                        seg = 0.0;
                    }
                }
                Atom::Space { w, wrap, .. } => {
                    if !prev_space {
                        line += w;
                        if *wrap {
                            min = min.max(seg);
                            seg = 0.0;
                        } else {
                            seg += w;
                        }
                    }
                    prev_space = true;
                }
                Atom::Newline => {
                    min = min.max(seg);
                    max = max.max(line);
                    seg = 0.0;
                    line = 0.0;
                    prev_space = true;
                }
                Atom::Atomic { bx, .. } => {
                    let s = self.style(*bx);
                    let (mn, mx) = self.intrinsic(*bx);
                    let (m, _, _, _) = self.edges(s, 0.0);
                    seg += mn + m.h();
                    line += mx + m.h();
                    min = min.max(seg);
                    seg = 0.0;
                    prev_space = false;
                }
                Atom::Float(bx) => {
                    let s = self.style(*bx);
                    let (mn, mx) = self.intrinsic(*bx);
                    let (m, _, _, _) = self.edges(s, 0.0);
                    min = min.max(mn + m.h());
                    line += mx + m.h();
                }
                Atom::Open(bx) => {
                    let s = self.style(*bx);
                    let (m, p, b, _) = self.edges(s, 0.0);
                    let v = m.l + p.l + b.l;
                    seg += v;
                    line += v;
                }
                Atom::Close(bx) => {
                    let s = self.style(*bx);
                    let (m, p, b, _) = self.edges(s, 0.0);
                    let v = m.r + p.r + b.r;
                    seg += v;
                    line += v;
                }
                Atom::Abs(_) => {}
            }
        }
        (min.max(seg), max.max(line))
    }

    /// Border-box (min-content, max-content) widths of `bx`, margins excluded.
    pub fn intrinsic(&mut self, bx: Bx) -> (f32, f32) {
        if let Some(v) = self.intr[bx.key()] {
            return v;
        }
        if self.depth >= MAX_DEPTH {
            return (0.0, 0.0);
        }
        self.depth += 1;
        let v = self.intrinsic_uncached(bx);
        self.depth -= 1;
        self.intr[bx.key()] = Some(v);
        v
    }

    /// Like `intrinsic` but ignoring the box's own `width` (table cells use it as a preference).
    pub fn intrinsic_content(&mut self, bx: Bx) -> (f32, f32) {
        if self.depth >= MAX_DEPTH {
            return (0.0, 0.0);
        }
        self.depth += 1;
        let (mn, mx) = self.intrinsic_impl(bx, false);
        self.depth -= 1;
        let s = self.style(bx);
        let (_, p, b, _) = self.edges(s, 0.0);
        let hpb = p.h() + b.h();
        // `intrinsic_impl` returns border-box values; callers add padding themselves.
        ((mn - hpb).max(0.0), (mx - hpb).max(0.0))
    }

    fn intrinsic_uncached(&mut self, bx: Bx) -> (f32, f32) {
        self.intrinsic_impl(bx, true)
    }

    fn intrinsic_impl(&mut self, bx: Bx, honor_width: bool) -> (f32, f32) {
        let s = self.style(bx);
        let (_, padding, border, _) = self.edges(s, 0.0);
        let hpb = if s.padding[1].has_percent() || s.padding[3].has_percent() { border.h() } else { padding.h() + border.h() };
        let fixed = match &s.width {
            Dim::Len(px, pct) if *pct == 0.0 && honor_width => Some(if s.border_box { *px } else { *px + hpb }),
            _ => None,
        };
        let (mut mn, mut mx) = if let Some(w) = fixed {
            (w, w)
        } else if self.is_replaced(bx) {
            let a = Args::block(0.0, None);
            let a = Args { shrink: true, ..a };
            let (w, _) = self.replaced_size(bx, s, &a, hpb, 0.0);
            (w + hpb, w + hpb)
        } else {
            let (cmn, cmx) = match s.display {
                Display::Flex | Display::InlineFlex => self.flex_intrinsic(bx, s),
                Display::Grid | Display::InlineGrid => self.grid_intrinsic(bx, s),
                Display::Table | Display::InlineTable => self.table_intrinsic(bx, s),
                _ => self.flow_intrinsic(bx, s),
            };
            (cmn + hpb, cmx + hpb)
        };
        // px min/max constraints
        let px = |d: &Dim| match d {
            Dim::Len(v, pct) if *pct == 0.0 => Some(if s.border_box { *v } else { *v + hpb }),
            _ => None,
        };
        if let Some(m) = px(&s.max_width) {
            mn = mn.min(m);
            mx = mx.min(m);
        }
        if let Some(m) = px(&s.min_width) {
            mn = mn.max(m);
            mx = mx.max(m);
        }
        (mn, mx.max(mn))
    }

    fn flow_intrinsic(&mut self, bx: Bx, s: &'a Style) -> (f32, f32) {
        let items = self.flatten(bx);
        let (mut mn, mut mx) = (0.0f32, 0.0f32);
        let mut run: Vec<Item> = Vec::new();
        let mut flush = |this: &mut Self, run: &mut Vec<Item>, mn: &mut f32, mx: &mut f32| {
            if run.is_empty() {
                return;
            }
            let (a, b) = this.inline_intrinsic(run, s);
            *mn = mn.max(a);
            *mx = mx.max(b);
            run.clear();
        };
        for item in items {
            match item {
                FlowItem::Inline(i) => run.push(i),
                FlowItem::Float(b) => run.push(Item::Float(b)),
                FlowItem::Abs(_) => {}
                FlowItem::Block(b) => {
                    flush(self, &mut run, &mut mn, &mut mx);
                    let cs = self.style(b);
                    let (m, _, _, _) = self.edges(cs, 0.0);
                    let (a, c) = self.intrinsic(b);
                    mn = mn.max(a + m.h());
                    mx = mx.max(c + m.h());
                }
            }
        }
        flush(self, &mut run, &mut mn, &mut mx);
        (mn, mx)
    }

    // -----------------------------------------------------------------------
    // Line layout
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn layout_inline_run(&mut self, items: Vec<Item>, block: &'a Style, cw: f32, cb_h: Option<f32>, y_bfc: f32, fl: &mut Floats, fox: f32) -> InlineOut {
        if block.text_balance && block.white_space.wraps() && fl.list.is_empty() && !items.iter().any(|i| matches!(i, Item::Float(_))) {
            return self.layout_balanced(items, block, cw, cb_h, y_bfc, fl, fox);
        }
        self.layout_inline_run_at(items, block, cw, cb_h, y_bfc, fl, fox, f32::INFINITY)
    }

    /// `text-wrap: balance` (Blink: at most 6 lines): the narrowest wrap width that keeps the line
    /// count, found by bisection. Lines are still aligned within the full content width.
    #[allow(clippy::too_many_arguments)]
    fn layout_balanced(&mut self, items: Vec<Item>, block: &'a Style, cw: f32, cb_h: Option<f32>, y_bfc: f32, fl: &mut Floats, fox: f32) -> InlineOut {
        let full = self.layout_inline_run_at(items.clone(), block, cw, cb_h, y_bfc, fl, fox, f32::INFINITY);
        let lines = full.lines;
        if !(2..=6).contains(&lines) {
            return full;
        }
        let (mut lo, mut hi) = (cw / lines as f32, cw);
        for _ in 0..10 {
            if hi - lo < 1.0 {
                break;
            }
            let mid = (lo + hi) / 2.0;
            let mut scratch = Floats::default();
            let probe = self.layout_inline_run_at(items.clone(), block, cw, cb_h, y_bfc, &mut scratch, fox, mid);
            if probe.lines <= lines {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        if hi >= cw - 0.5 {
            return full;
        }
        self.layout_inline_run_at(items, block, cw, cb_h, y_bfc, fl, fox, math::ceil32(hi))
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_inline_run_at(&mut self, items: Vec<Item>, block: &'a Style, cw: f32, cb_h: Option<f32>, y_bfc: f32, fl: &mut Floats, fox: f32, wrap_limit: f32) -> InlineOut {
        let atoms = self.make_atoms(&items, block);
        let mut run = Run::new(block, cw, cb_h, y_bfc, fox);
        run.wrap_limit = wrap_limit;
        let (sa, sd, lh) = self.vmetrics(block);
        run.strut_content = sa + sd;
        run.strut_asc = sa;
        // Blink: half-leading is floored; the descent absorbs the remainder so the box is exactly `lh`
        let half = math::floor32((lh - run.strut_content) / 2.0);
        run.strut_above = sa + half;
        run.strut_below = lh - run.strut_above;
        let _ = (&mut run.first_line,);
        run.max_lines = block.line_clamp.filter(|_| block.overflow_x.clips() || block.overflow_y.clips()).map(|n| n.max(1) as usize);
        self.begin_line(&mut run, fl);
        for atom in atoms {
            if run.stopped {
                break;
            }
            match atom {
                Atom::Float(bx) => {
                    if run.has_content {
                        run.pending_floats.push(bx);
                    } else {
                        self.run_place_float(&mut run, bx, fl);
                        self.refresh_line(&mut run, fl);
                    }
                }
                Atom::Abs(bx) => {
                    let s = self.style(bx);
                    run.abs.push(Pending { bx, x: run.line_left + run.line_w, y: run.y, fixed: s.position == Position::Fixed });
                }
                Atom::Newline => {
                    self.finish_line(&mut run, true, false, fl);
                }
                Atom::Open(bx) => {
                    let s = self.style(bx);
                    let (m, p, b, _) = self.edges(s, cw);
                    let decor = p.l + b.l;
                    let f = Frag { kind: FragKind::Open { bx, first: true, ml: m.l, decor }, w: m.l + decor, break_after: false, wrap: true, shift: 0.0, mode: 0, split_ok: false };
                    run.line_w += f.w;
                    run.frags.push(f);
                }
                Atom::Close(bx) => {
                    let s = self.style(bx);
                    let (m, p, b, _) = self.edges(s, cw);
                    let decor = p.r + b.r;
                    let f = Frag { kind: FragKind::Close { bx, last: true, mr: m.r, decor }, w: m.r + decor, break_after: false, wrap: true, shift: 0.0, mode: 0, split_ok: false };
                    run.line_w += f.w;
                    run.frags.push(f);
                }
                Atom::Space { w, bx, wrap, shift, mode } => {
                    let s = self.style(bx);
                    if !run.has_content && s.white_space.collapses_spaces() {
                        continue;
                    }
                    if run.frags.iter().rev().find(|f| !matches!(f.kind, FragKind::Open { .. } | FragKind::Close { .. })).is_some_and(|f| matches!(f.kind, FragKind::Space { .. })) {
                        continue;
                    }
                    let f = Frag { kind: FragKind::Space { bx }, w, break_after: wrap, wrap, shift, mode, split_ok: false };
                    run.line_w += f.w;
                    run.frags.push(f);
                }
                Atom::Word { text, w, bx, break_after, wrap, split_ok, shift, mode } => {
                    let f = Frag { kind: FragKind::Text { text, bx }, w, break_after, wrap, shift, mode, split_ok };
                    self.push_content(&mut run, f, fl);
                }
                Atom::Atomic { bx, shift, mode } => {
                    let s = self.style(bx);
                    let (m, _, _, _) = self.edges(s, cw);
                    let args = Args { avail_w: (cw - m.h()).max(0.0), cb_h, forced_w: None, forced_h: None, shrink: true };
                    let out = self.layout_box(bx, &args, None, false);
                    let w = out.w + m.l + m.r;
                    let f = Frag { kind: FragKind::Atomic { out, bx, ml: m.l, mt: m.t, mb: m.b }, w, break_after: true, wrap: true, shift, mode, split_ok: false };
                    // a break opportunity exists before atomic inline boxes
                    if let Some(last) = run.frags.last_mut() {
                        last.break_after = last.wrap || matches!(last.kind, FragKind::Atomic { .. });
                    }
                    self.push_content(&mut run, f, fl);
                }
            }
        }
        if run.stopped {
            run.frags.clear();
        } else if run.has_content || run.frags.iter().any(|f| matches!(f.kind, FragKind::Atomic { .. })) {
            self.finish_line(&mut run, false, true, fl);
        } else {
            run.frags.clear();
            for bx in core::mem::take(&mut run.pending_floats) {
                self.run_place_float(&mut run, bx, fl);
            }
        }
        InlineOut {
            h: run.y,
            empty: run.lines == 0,
            paint: run.paint,
            float_paint: run.float_paint,
            layers: run.layers,
            abs: run.abs,
            first_baseline: run.first_baseline,
            last_baseline: run.last_baseline,
            lines: run.lines,
        }
    }

    fn refresh_line(&mut self, run: &mut Run<'a>, fl: &mut Floats) {
        let (l, r) = fl.avail(run.y_bfc + run.y, run.strut_content.max(1.0), run.fox, run.fox + run.cw);
        let indent = if run.first_line { run.indent } else { 0.0 };
        run.line_left = (l - run.fox) + indent.max(0.0);
        run.line_avail = (r - l - indent).max(0.0);
        if indent < 0.0 {
            run.line_left += indent;
            run.line_avail -= indent;
        }
        run.narrowed = (r - l) < run.cw - 0.01;
        run.balance_cut = (run.line_avail - run.wrap_limit).max(0.0);
        run.line_avail -= run.balance_cut;
    }

    fn begin_line(&mut self, run: &mut Run<'a>, fl: &mut Floats) {
        for bx in core::mem::take(&mut run.pending_floats) {
            self.run_place_float(run, bx, fl);
        }
        self.refresh_line(run, fl);
    }

    fn run_place_float(&mut self, run: &mut Run<'a>, bx: Bx, fl: &mut Floats) {
        let (o, x, y_bfc) = self.place_float(bx, run.cw, run.cb_h, fl, run.fox, run.y_bfc + run.y);
        let mut tmp = Out::default();
        tmp.absorb(o, x, y_bfc - run.y_bfc);
        run.float_paint.append(tmp.paint, 0.0, 0.0);
        run.layers.extend(tmp.layers);
        run.abs.extend(tmp.abs);
    }

    fn push_content(&mut self, run: &mut Run<'a>, f: Frag, fl: &mut Floats) {
        let mut f = f;
        loop {
            let fits = run.line_w + f.w <= run.line_avail + 0.01;
            if fits {
                break;
            }
            if !run.has_content {
                // Nothing on the line yet: float-narrowed lines slide down past the float.
                if run.narrowed && let Some(nb) = fl.next_bottom(run.y_bfc + run.y, run.strut_content.max(1.0)) {
                    let nb_local = nb - run.y_bfc;
                    if nb_local > run.y {
                        run.y = nb_local;
                        self.begin_line(run, fl);
                        continue;
                    }
                }
                // Too wide for an empty line: optionally split the word.
                if f.split_ok && f.wrap && let FragKind::Text { text, bx } = &f.kind {
                    let s = self.style(*bx);
                    if let Some((head, tail)) = self.split_to_fit(text, s, run.line_avail) {
                        let hw = self.text_w(&head, s);
                        let tw = f.w - hw;
                        let bxv = *bx;
                        let head_frag = Frag { kind: FragKind::Text { text: head, bx: bxv }, w: hw, break_after: true, wrap: true, shift: f.shift, mode: f.mode, split_ok: true };
                        run.line_w += head_frag.w;
                        run.frags.push(head_frag);
                        run.has_content = true;
                        self.finish_line(run, false, false, fl);
                        f = Frag { kind: FragKind::Text { text: tail, bx: bxv }, w: tw.max(0.0), break_after: f.break_after, wrap: true, shift: f.shift, mode: f.mode, split_ok: true };
                        continue;
                    }
                }
                break;
            }
            // Find the last break opportunity on this line.
            let candidate = run.frags.iter().rposition(|x| x.break_after && x.wrap);
            let Some(i) = candidate else {
                if f.split_ok && f.wrap && let FragKind::Text { text, bx } = &f.kind {
                    let s = self.style(*bx);
                    let room = (run.line_avail - run.line_w).max(0.0);
                    if let Some((head, tail)) = self.split_to_fit(text, s, room) {
                        let hw = self.text_w(&head, s);
                        let tw = f.w - hw;
                        let bxv = *bx;
                        let head_frag = Frag { kind: FragKind::Text { text: head, bx: bxv }, w: hw, break_after: true, wrap: true, shift: f.shift, mode: f.mode, split_ok: true };
                        run.line_w += head_frag.w;
                        run.frags.push(head_frag);
                        self.finish_line(run, false, false, fl);
                        f = Frag { kind: FragKind::Text { text: tail, bx: bxv }, w: tw.max(0.0), break_after: f.break_after, wrap: true, shift: f.shift, mode: f.mode, split_ok: true };
                        continue;
                    }
                }
                break;
            };
            if f.wrap || matches!(f.kind, FragKind::Atomic { .. }) {
                let tail = run.frags.split_off(i + 1);
                run.line_w = run.frags.iter().map(|x| x.w).sum();
                self.finish_line(run, false, false, fl);
                for t in tail {
                    run.line_w += t.w;
                    if !matches!(t.kind, FragKind::Open { .. } | FragKind::Close { .. } | FragKind::Space { .. }) {
                        run.has_content = true;
                    }
                    // leading spaces never start a line
                    if matches!(t.kind, FragKind::Space { .. }) && !run.has_content {
                        run.line_w -= t.w;
                        continue;
                    }
                    run.frags.push(t);
                }
                continue;
            }
            break;
        }
        run.line_w += f.w;
        run.has_content = true;
        run.frags.push(f);
    }

    /// Largest prefix of `text` that fits in `room` (at least one char), plus the remainder.
    fn split_to_fit(&self, text: &str, s: &Style, room: f32) -> Option<(String, String)> {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        if chars.len() < 2 {
            return None;
        }
        let mut best = 1;
        let mut lo = 1;
        let mut hi = chars.len() - 1;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let end = chars[mid].0;
            if self.text_w(&text[..end], s) <= room {
                best = mid;
                lo = mid + 1;
            } else {
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            }
        }
        let end = chars[best].0;
        Some((text[..end].into(), text[end..].into()))
    }

    fn finish_line(&mut self, run: &mut Run<'a>, forced: bool, last: bool, fl: &mut Floats) {
        // drop trailing spaces
        while let Some(f) = run.frags.last() {
            if matches!(f.kind, FragKind::Space { .. }) {
                run.frags.pop();
            } else {
                break;
            }
        }
        let has_box = run.frags.iter().any(|f| matches!(f.kind, FragKind::Text { .. } | FragKind::Atomic { .. }));
        if !has_box && !forced {
            run.frags.clear();
            run.line_w = 0.0;
            run.has_content = false;
            return;
        }
        let block = run.block;
        let frags = core::mem::take(&mut run.frags);
        // continuation of boxes still open at the end of this line
        let mut open: Vec<Bx> = Vec::new();
        for f in &frags {
            match &f.kind {
                FragKind::Open { bx, .. } => open.push(*bx),
                FragKind::Close { bx, .. } => {
                    if let Some(p) = open.iter().rposition(|b| b == bx) {
                        open.remove(p);
                    }
                }
                _ => {}
            }
        }
        // metrics
        let mut asc_max = run.strut_above;
        let mut desc_max = run.strut_below;
        struct M {
            above: f32,
            below: f32,
        }
        let mut metrics: Vec<M> = Vec::with_capacity(frags.len());
        for f in &frags {
            let m = match &f.kind {
                FragKind::Text { bx, .. } | FragKind::Space { bx } => {
                    let s = self.style(*bx);
                    let (asc, desc, lh) = self.vmetrics(s);
                    let half = math::floor32((lh - asc - desc) / 2.0);
                    let mut m = M { above: asc + half, below: lh - (asc + half) };
                    // Emoji come from a fallback font (Noto Color Emoji: ascent .928em, descent .244em)
                    // whose metrics also size the line when `line-height: normal`.
                    if let FragKind::Text { text, .. } = &f.kind {
                        if s.line_height_number.is_none() && s.line_height_px.is_none() && text.chars().any(is_emoji) {
                            m.above = m.above.max(math::round32(0.928 * s.font_size));
                            m.below = m.below.max(math::round32(0.244 * s.font_size));
                        }
                    }
                    m
                }
                FragKind::Atomic { out, mt, mb, .. } => {
                    // boxes without a baseline (replaced elements) sit on the bottom *margin* edge
                    let above = out.baseline.map_or(out.h + mb, |b| b.min(out.h)) + mt;
                    M { above, below: out.h + mt + mb - above }
                }
                _ => M { above: 0.0, below: 0.0 },
            };
            metrics.push(m);
        }
        for (f, m) in frags.iter().zip(&metrics) {
            if matches!(f.kind, FragKind::Open { .. } | FragKind::Close { .. }) || f.mode == 1 || f.mode == 2 {
                continue;
            }
            let (a, b) = if f.mode == 3 {
                let h = m.above + m.below;
                (h / 2.0 + block.font_size * 0.25 + f.shift, h / 2.0 - block.font_size * 0.25 - f.shift)
            } else {
                (m.above + f.shift, m.below - f.shift)
            };
            asc_max = asc_max.max(a);
            desc_max = desc_max.max(b);
        }
        for (f, m) in frags.iter().zip(&metrics) {
            if f.mode == 1 || f.mode == 2 {
                let h = m.above + m.below;
                let total = asc_max + desc_max;
                if h > total {
                    if f.mode == 1 {
                        desc_max += h - total;
                    } else {
                        asc_max += h - total;
                    }
                }
            }
        }
        let line_h = asc_max + desc_max;
        let line_top = run.y;
        let baseline = line_top + asc_max;

        // alignment
        let trimmed_w: f32 = frags.iter().map(|f| f.w).sum();
        let avail = run.line_avail + run.balance_cut;
        let mut offset = 0.0;
        let mut extra_per_space = 0.0;
        let free = (avail - trimmed_w).max(0.0);
        match block.text_align {
            TextAlign::Left => {
                if block.rtl {
                    offset = free;
                }
            }
            TextAlign::Right => offset = free,
            TextAlign::Center => offset = free / 2.0,
            TextAlign::Justify => {
                if !forced && !last {
                    let n = frags.iter().filter(|f| matches!(f.kind, FragKind::Space { .. })).count();
                    if n > 0 {
                        extra_per_space = free / n as f32;
                    }
                }
            }
        }
        if block.text_overflow_ellipsis && trimmed_w > avail + 0.5 && (block.overflow_x.clips()) {
            // handled below by truncating text
        }

        // emit
        let mut x = run.line_left + offset;
        struct Span {
            bx: Bx,
            x0: f32,
            first: bool,
            shift: f32,
        }
        let mut spans: Vec<Span> = Vec::new();
        let mut finished_spans: Vec<(Span, f32, bool)> = Vec::new();
        let mut text_runs: Vec<TextRun> = Vec::new();
        let mut cur: Option<TextRun> = None;
        let mut pending_space: Option<(f32, Bx)> = None;
        let mut atomics: Vec<(Out, f32, f32, bool)> = Vec::new();
        let mut line_paint = Paint::default();
        let _ = &mut line_paint;
        let clamp_last = !last && run.max_lines.is_some_and(|m| run.lines + 1 >= m);
        let ellipsis_limit = if clamp_last || (block.text_overflow_ellipsis && block.overflow_x.clips() && trimmed_w > avail + 0.5) { Some(avail) } else { None };
        let last_text_idx = frags.iter().rposition(|f| matches!(f.kind, FragKind::Text { .. }));
        let mut used = 0.0f32;
        let mut truncated = false;
        let mut text_hits: Vec<HitRegion> = Vec::new();
        for (fi, (f, m)) in frags.into_iter().zip(metrics).enumerate() {
            let w = match &f.kind {
                FragKind::Space { .. } => f.w + extra_per_space,
                _ => f.w,
            };
            let item_top = |mode: u8, shift: f32, above: f32, below: f32, block_font: f32| -> f32 {
                // top of the item's margin box
                match mode {
                    1 => line_top,
                    2 => line_top + line_h - (above + below),
                    3 => baseline - (above + below) / 2.0 - block_font * 0.25 - shift,
                    _ => baseline - shift - above,
                }
            };
            match f.kind {
                FragKind::Open { bx, first, ml, decor } => {
                    if let Some(t) = cur.take() {
                        text_runs.push(t);
                    }
                    pending_space = None;
                    x += ml;
                    spans.push(Span { bx, x0: x, first, shift: 0.0 });
                    x += decor;
                }
                FragKind::Close { bx, last: is_last, mr, decor } => {
                    if let Some(t) = cur.take() {
                        text_runs.push(t);
                    }
                    pending_space = None;
                    x += decor;
                    if let Some(p) = spans.iter().rposition(|s| s.bx == bx) {
                        let sp = spans.remove(p);
                        finished_spans.push((sp, x, is_last));
                    }
                    x += mr;
                }
                FragKind::Space { bx } => {
                    if truncated {
                        continue;
                    }
                    pending_space = Some((w, bx));
                    // The space is only painted if more text of the same style follows.
                    if let Some(c) = cur.as_mut() {
                        if c.bx == bx && c.shift == f.shift && extra_per_space == 0.0 {
                            c.pending_space = w;
                        } else {
                            text_runs.push(cur.take().unwrap());
                        }
                    }
                    x += w;
                    used += w;
                }
                FragKind::Text { text, bx } => {
                    if truncated {
                        continue;
                    }
                    let s = self.style(bx);
                    let mut text = text;
                    let mut w = w;
                    if let Some(limit) = ellipsis_limit {
                        let ell = self.text_w("\u{2026}", s);
                        if clamp_last && Some(fi) == last_text_idx && used + w + ell <= limit {
                            // the clamped line is not full: the ellipsis simply follows the text
                            text.push('\u{2026}');
                            w += ell;
                            truncated = true;
                        } else if used + w > limit || (clamp_last && Some(fi) == last_text_idx) {
                            // cut characters until the ellipsis fits
                            let mut chars: Vec<char> = text.chars().collect();
                            let room = limit - used - ell;
                            while !chars.is_empty() {
                                let t: String = chars.iter().collect();
                                if self.text_w(&t, s) <= room {
                                    break;
                                }
                                chars.pop();
                            }
                            let mut t: String = chars.iter().collect();
                            t.push('\u{2026}');
                            w = self.text_w(&t, s);
                            text = t;
                            truncated = true;
                        }
                    }
                    let mut can_merge = false;
                    if let Some(c) = cur.as_mut() {
                        if c.bx == bx && c.shift == f.shift && c.mode == f.mode {
                            can_merge = true;
                            if c.pending_space > 0.0 {
                                c.text.push(' ');
                                c.w += c.pending_space;
                                c.pending_space = 0.0;
                            }
                            c.text.push_str(&text);
                            c.w += w;
                        }
                    }
                    if !can_merge {
                        if let Some(t) = cur.take() {
                            text_runs.push(t);
                        }
                        cur = Some(TextRun { bx, x, text, w, shift: f.shift, mode: f.mode, pending_space: 0.0 });
                    }
                    x += w;
                    used += w;
                    pending_space = None;
                }
                FragKind::Atomic { out, ml, mt, mb, bx } => {
                    if let Some(t) = cur.take() {
                        text_runs.push(t);
                    }
                    pending_space = None;
                    let above = m.above;
                    let below = m.below;
                    let top = item_top(f.mode, f.shift, above, below, block.font_size);
                    atomics.push((out, x + ml, top + mt, true));
                    let _ = (mb, bx);
                    x += w;
                    used += w;
                }
            }
        }
        let _ = pending_space;
        if let Some(t) = cur.take() {
            text_runs.push(t);
        }
        // spans still open at the line end
        for sp in spans.drain(..) {
            finished_spans.push((sp, x, false));
        }
        // inline box backgrounds and borders first
        for (sp, x1, is_last) in finished_spans {
            let s = self.style(sp.bx);
            if s.visibility_visible {
                let (asc, desc, _) = self.vmetrics(s);
                let ch = asc + desc;
                let (_, p, b, _) = self.edges(s, run.cw);
                let (shift, _) = self.valign_of(s, block.font_size);
                let top = baseline - shift - asc - p.t - b.t;
                let rect = Rect { x: sp.x0, y: top, width: (x1 - sp.x0).max(0.0), height: ch + p.t + p.b + b.t + b.b };
                self.paint_decorations_sides(&mut run.paint, s, rect, [sp.first, is_last]);
            }
        }
        for t in text_runs {
            let s = self.style(t.bx);
            if !s.visibility_visible {
                continue;
            }
            let (asc, desc, lh) = self.vmetrics(s);
            let ch = asc + desc;
            let y = if t.mode == 3 {
                baseline - t.shift - asc
            } else if t.mode == 1 {
                line_top + (lh - ch) / 2.0
            } else if t.mode == 2 {
                line_top + line_h - ch - (lh - ch) / 2.0
            } else {
                baseline - t.shift - asc
            };
            let link = self.link_of(t.bx);
            let hit_node = if t.bx.pseudo != 0 { t.bx.id } else { self.doc.nodes[t.bx.id].parent.unwrap_or(t.bx.id) };
            for sh in s.text_shadows.iter().rev() {
                run.paint.cmds.push(Command::Text {
                    x: t.x + sh.dx,
                    y: y + sh.dy,
                    text: t.text.clone(),
                    size: s.font_size,
                    color: sh.color,
                    link: None,
                    weight: s.font_weight,
                    italic: s.font_italic,
                    family: s.font_family,
                    family_names: s.font_names.clone(),
                    decoration: 0,
                    width: t.w,
                    letter_spacing: s.letter_spacing,
                    blur: sh.blur,
                });
            }
            run.paint.cmds.push(Command::Text {
                x: t.x,
                y,
                text: t.text,
                size: s.font_size,
                color: s.color,
                link,
                weight: s.font_weight,
                italic: s.font_italic,
                family: s.font_family,
                    family_names: s.font_names.clone(),
                decoration: s.text_decoration,
                width: t.w,
                letter_spacing: s.letter_spacing,
                blur: 0.0,
            });
            if s.pointer_events {
                text_hits.push(HitRegion { rect: Rect { x: t.x, y, width: t.w, height: ch }, node: hit_node });
            }
        }
        run.paint.hits.extend(text_hits);
        for (out, ax, ay, _) in atomics {
            let mut tmp = Out::default();
            tmp.absorb(out, 0.0, 0.0);
            run.paint.append(tmp.paint, ax, ay);
            for (z, mut p) in tmp.layers {
                p.translate(ax, ay);
                run.layers.push((z, p));
            }
            for mut p in tmp.abs {
                p.x += ax;
                p.y += ay;
                run.abs.push(p);
            }
        }
        if run.first_baseline.is_none() {
            run.first_baseline = Some(baseline);
        }
        run.last_baseline = Some(baseline);
        run.lines += 1;
        if clamp_last {
            run.stopped = true;
        }
        run.y += line_h;
        run.first_line = false;
        run.line_w = 0.0;
        run.has_content = false;
        // start the next line, carrying open inline boxes forward
        self.begin_line(run, fl);
        for bx in open {
            run.frags.push(Frag { kind: FragKind::Open { bx, first: false, ml: 0.0, decor: 0.0 }, w: 0.0, break_after: false, wrap: true, shift: 0.0, mode: 0, split_ok: false });
        }
    }

    pub fn paint_decorations_sides(&mut self, paint: &mut Paint, s: &Style, rect: Rect, sides: [bool; 2]) {
        let mut tmp = Paint::default();
        self.paint_decorations(&mut tmp, s, rect, false, [false; 4]);
        for c in tmp.cmds.iter_mut() {
            if let Command::Border { widths, styles, .. } = c {
                if !sides[0] {
                    widths[3] = 0.0;
                    styles[3] = 0;
                }
                if !sides[1] {
                    widths[1] = 0.0;
                    styles[1] = 0;
                }
            }
        }
        paint.cmds.extend(tmp.cmds);
    }
}

struct TextRun {
    bx: Bx,
    x: f32,
    text: String,
    w: f32,
    shift: f32,
    mode: u8,
    pending_space: f32,
}

struct Run<'a> {
    block: &'a Style,
    cw: f32,
    cb_h: Option<f32>,
    y_bfc: f32,
    fox: f32,
    y: f32,
    frags: Vec<Frag>,
    line_w: f32,
    line_left: f32,
    line_avail: f32,
    has_content: bool,
    narrowed: bool,
    first_line: bool,
    indent: f32,
    pending_floats: Vec<Bx>,
    paint: Paint,
    float_paint: Paint,
    layers: Vec<(i32, Paint)>,
    abs: Vec<Pending>,
    lines: usize,
    first_baseline: Option<f32>,
    last_baseline: Option<f32>,
    strut_content: f32,
    strut_asc: f32,
    strut_above: f32,
    strut_below: f32,
    /// `-webkit-line-clamp`: lines kept; the last one ends with an ellipsis when content was dropped.
    max_lines: Option<usize>,
    stopped: bool,
    /// Wrap width for `text-wrap: balance` (infinite otherwise) and how much it narrowed the line.
    wrap_limit: f32,
    balance_cut: f32,
}

impl<'a> Run<'a> {
    fn new(block: &'a Style, cw: f32, cb_h: Option<f32>, y_bfc: f32, fox: f32) -> Run<'a> {
        let indent = block.text_indent.resolve(cw).unwrap_or(0.0);
        Run {
            block,
            cw,
            cb_h,
            y_bfc,
            fox,
            y: 0.0,
            frags: Vec::new(),
            line_w: 0.0,
            line_left: 0.0,
            line_avail: cw,
            has_content: false,
            narrowed: false,
            first_line: true,
            indent,
            pending_floats: Vec::new(),
            paint: Paint::default(),
            float_paint: Paint::default(),
            layers: Vec::new(),
            abs: Vec::new(),
            lines: 0,
            first_baseline: None,
            last_baseline: None,
            strut_content: 0.0,
            strut_asc: 0.0,
            strut_above: 0.0,
            strut_below: 0.0,
            max_lines: None,
            stopped: false,
            wrap_limit: f32::INFINITY,
            balance_cut: 0.0,
        }
    }
}

#[allow(dead_code)]
fn _unused(_: f32) -> f32 {
    math::floor32(0.0)
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B50 | 0x2B55 | 0x231A..=0x23FF)
}
