//! Flexbox layout (CSS Flexible Box Layout Module Level 1).
use crate::{css::*, layout::*};
use alloc::vec::Vec;

struct FItem<'a> {
    bx: Bx,
    s: &'a Style,
    /// margins: top, right, bottom, left (auto resolved to 0)
    mg: [f32; 4],
    mauto: [bool; 4],
    pb_main: f32,
    pb_cross: f32,
    base: f32,
    hypo: f32,
    minm: f32,
    maxm: f32,
    target: f32,
    frozen: bool,
    out: Out,
    cross: f32,
    off_main: f32,
    off_cross: f32,
}

fn ws_only(s: &str) -> bool {
    s.chars().all(crate::layout_inline::is_ws)
}

impl<'a> Ctx<'a> {
    fn flex_items(&self, bx: Bx, items: &mut Vec<Bx>, abs: &mut Vec<Bx>, depth: usize) {
        if depth > 50 {
            return;
        }
        for c in self.children(bx) {
            if c.pseudo == 0
                && let Some(t) = text_of(self.doc, c.id)
            {
                if !ws_only(t) {
                    items.push(c);
                }
                continue;
            }
            if c.pseudo == 0 && self.doc.nodes[c.id].tag().is_none() {
                continue;
            }
            let s = self.style(c);
            match s.display {
                Display::None => {}
                Display::Contents => self.flex_items(c, items, abs, depth + 1),
                _ if s.is_out_of_flow() => abs.push(c),
                _ => items.push(c),
            }
        }
    }

    pub fn flex_intrinsic(&mut self, bx: Bx, s: &'a Style) -> (f32, f32) {
        let mut items = Vec::new();
        let mut abs = Vec::new();
        self.flex_items(bx, &mut items, &mut abs, 0);
        let row = matches!(s.flex_direction, FlexDirection::Row | FlexDirection::RowReverse);
        let gap = if row { s.column_gap.resolve(0.0).unwrap_or(0.0) } else { 0.0 };
        let (mut mn, mut mx) = (0.0f32, 0.0f32);
        let mut count = 0;
        for c in items {
            let cs = self.style(c);
            let (m, _, _, _) = self.edges(cs, 0.0);
            let (a, b) = self.intrinsic(c);
            let (a, b) = (a + m.h(), b + m.h());
            count += 1;
            if row {
                mx += b;
                if s.flex_wrap == FlexWrap::Nowrap {
                    mn += a;
                } else {
                    mn = mn.max(a);
                }
            } else {
                mn = mn.max(a);
                mx = mx.max(b);
            }
        }
        if row && count > 1 {
            let g = gap * (count - 1) as f32;
            mx += g;
            if s.flex_wrap == FlexWrap::Nowrap {
                mn += g;
            }
        }
        (mn, mx)
    }

    /// Returns the content (in content-box coordinates) and its height.
    pub fn layout_flex(&mut self, bx: Bx, s: &'a Style, cw: f32, ch_known: Option<f32>) -> (Out, f32) {
        let mut item_bx = Vec::new();
        let mut abs_bx = Vec::new();
        self.flex_items(bx, &mut item_bx, &mut abs_bx, 0);
        let mut result = Out::default();
        for b in abs_bx {
            let fixed = self.style(b).position == Position::Fixed;
            result.abs.push(Pending { bx: b, x: 0.0, y: 0.0, fixed });
        }
        let row = matches!(s.flex_direction, FlexDirection::Row | FlexDirection::RowReverse);
        let reverse = matches!(s.flex_direction, FlexDirection::RowReverse | FlexDirection::ColumnReverse) ^ (row && s.rtl);
        let wrap = s.flex_wrap != FlexWrap::Nowrap;
        let wrap_reverse = s.flex_wrap == FlexWrap::WrapReverse;
        let main_avail: Option<f32> = if row { Some(cw) } else { ch_known };
        let cross_avail: Option<f32> = if row { ch_known } else { Some(cw) };
        let gap_main = if row { s.column_gap.resolve(cw) } else { s.row_gap.resolve(ch_known.unwrap_or(0.0)) }.unwrap_or(0.0).max(0.0);
        let gap_cross = if row { s.row_gap.resolve(ch_known.unwrap_or(0.0)) } else { s.column_gap.resolve(cw) }.unwrap_or(0.0).max(0.0);
        let (ma, mb_, ca, cb_) = if row { (3usize, 1usize, 0usize, 2usize) } else { (0, 2, 3, 1) };

        // Build items sorted by `order`.
        let mut items: Vec<FItem> = Vec::with_capacity(item_bx.len());
        for b in item_bx {
            let st = self.style(b);
            let (m, p, bd, _) = self.edges(st, cw);
            let mg = [m.t, m.r, m.b, m.l];
            let mauto = [st.margin[0].is_auto(), st.margin[1].is_auto(), st.margin[2].is_auto(), st.margin[3].is_auto()];
            let (hpb, vpb) = (p.h() + bd.h(), p.v() + bd.v());
            items.push(FItem {
                bx: b,
                s: st,
                mg,
                mauto,
                pb_main: if row { hpb } else { vpb },
                pb_cross: if row { vpb } else { hpb },
                base: 0.0,
                hypo: 0.0,
                minm: 0.0,
                maxm: f32::INFINITY,
                target: 0.0,
                frozen: false,
                out: Out::default(),
                cross: 0.0,
                off_main: 0.0,
                off_cross: 0.0,
            });
        }
        items.sort_by_key(|i| i.s.order);

        // Flex base sizes and clamps.
        for i in 0..items.len() {
            let (st, b) = (items[i].s, items[i].bx);
            let pb = items[i].pb_main;
            let to_border = |v: f32| if st.border_box { v } else { v + pb };
            let main_msum = items[i].mg[ma] + items[i].mg[mb_];
            let (min_dim, max_dim, size_dim) = if row { (&st.min_width, &st.max_width, &st.width) } else { (&st.min_height, &st.max_height, &st.height) };
            let pct_base = if row { cw } else { ch_known.unwrap_or(f32::NAN) };
            let resolve = |d: &Dim| -> Option<f32> {
                if d.has_percent() && pct_base.is_nan() {
                    return None;
                }
                d.resolve(pct_base)
            };
            let basis = if !st.flex_basis.is_auto() { &st.flex_basis } else { size_dim };
            let base = match basis {
                Dim::Len(..) | Dim::Func(..) => resolve(basis).map(|v| to_border(v.max(0.0))),
                Dim::MinContent if row => Some(self.intrinsic(b).0),
                Dim::MaxContent | Dim::FitContent if row => Some(self.intrinsic(b).1),
                _ => None,
            };
            let base = match base {
                Some(v) => v,
                None => {
                    if row {
                        self.intrinsic(b).1
                    } else {
                        // content height at the item's cross size
                        let cross_w = match cross_avail {
                            Some(w) => (w - items[i].mg[3] - items[i].mg[1]).max(0.0),
                            None => cw,
                        };
                        let stretch = self.item_align(st, s) == Align::Stretch && st.width.is_auto();
                        let args = Args { avail_w: cross_w, cb_h: None, forced_w: if stretch { Some(cross_w) } else { None }, forced_h: None, shrink: !stretch };
                        let o = self.layout_box(b, &args, None, false);
                        o.h
                    }
                }
            };
            let minv = match min_dim {
                Dim::Auto => {
                    if row {
                        if st.overflow_x.clips() {
                            0.0
                        } else {
                            let content_min = self.intrinsic(b).0;
                            match size_dim {
                                Dim::Len(..) | Dim::Func(..) => resolve(size_dim).map_or(content_min, |v| content_min.min(to_border(v))),
                                _ => content_min,
                            }
                        }
                    } else {
                        0.0
                    }
                }
                Dim::MinContent if row => self.intrinsic(b).0,
                Dim::MaxContent if row => self.intrinsic(b).1,
                d => resolve(d).map_or(0.0, |v| to_border(v.max(0.0))),
            };
            let maxv = match max_dim {
                Dim::MinContent if row => self.intrinsic(b).0,
                Dim::MaxContent if row => self.intrinsic(b).1,
                d => resolve(d).map_or(f32::INFINITY, |v| to_border(v.max(0.0))),
            };
            let it = &mut items[i];
            it.base = base.max(it.pb_main);
            it.minm = minv.max(it.pb_main);
            it.maxm = maxv.max(it.minm);
            it.hypo = it.base.clamp(it.minm, it.maxm);
            let _ = main_msum;
        }

        // Collect into lines.
        let mut lines: Vec<Vec<usize>> = Vec::new();
        {
            let mut cur: Vec<usize> = Vec::new();
            let mut used = 0.0f32;
            for i in 0..items.len() {
                let outer = items[i].hypo + items[i].mg[ma] + items[i].mg[mb_];
                if wrap && let Some(avail) = main_avail {
                    if !cur.is_empty() && used + gap_main + outer > avail + 0.01 {
                        lines.push(core::mem::take(&mut cur));
                        used = 0.0;
                    }
                }
                used += if cur.is_empty() { outer } else { gap_main + outer };
                cur.push(i);
            }
            if !cur.is_empty() {
                lines.push(cur);
            }
        }

        // Resolve flexible lengths and lay items out.
        let mut line_cross: Vec<f32> = Vec::new();
        let mut line_main_used: Vec<f32> = Vec::new();
        let mut baseline_info: Vec<(f32, f32)> = Vec::new(); // (max ascent, max descent) of baseline-aligned items
        for line in &lines {
            let gaps = gap_main * line.len().saturating_sub(1) as f32;
            let sum_hypo: f32 = line.iter().map(|&i| items[i].hypo + items[i].mg[ma] + items[i].mg[mb_]).sum();
            let container_main = main_avail.unwrap_or(sum_hypo + gaps);
            let initial_free = container_main - gaps - sum_hypo;
            let growing = initial_free > 0.0;
            for &i in line {
                let it = &mut items[i];
                let factor = if growing { it.s.flex_grow } else { it.s.flex_shrink };
                it.frozen = factor == 0.0 || (growing && it.base > it.hypo) || (!growing && it.base < it.hypo);
                it.target = it.hypo;
                it.frozen |= false;
            }
            loop {
                if line.iter().all(|&i| items[i].frozen) {
                    break;
                }
                let used: f32 = line
                    .iter()
                    .map(|&i| (if items[i].frozen { items[i].target } else { items[i].base }) + items[i].mg[ma] + items[i].mg[mb_])
                    .sum();
                let mut remaining = container_main - gaps - used;
                let sum_factor: f32 = line.iter().filter(|&&i| !items[i].frozen).map(|&i| if growing { items[i].s.flex_grow } else { items[i].s.flex_shrink }).sum();
                if growing && sum_factor < 1.0 {
                    remaining = remaining.min(initial_free * sum_factor);
                }
                let scaled_sum: f32 = line.iter().filter(|&&i| !items[i].frozen).map(|&i| items[i].s.flex_shrink * items[i].base).sum();
                let mut total_violation = 0.0;
                let mut violations: Vec<(usize, f32)> = Vec::new();
                for &i in line {
                    if items[i].frozen {
                        continue;
                    }
                    let it = &items[i];
                    let size = if growing {
                        if sum_factor > 0.0 { it.base + remaining * it.s.flex_grow / sum_factor } else { it.base }
                    } else if scaled_sum > 0.0 {
                        it.base + remaining * (it.s.flex_shrink * it.base) / scaled_sum
                    } else {
                        it.base
                    };
                    let clamped = size.clamp(it.minm, it.maxm).max(it.pb_main);
                    total_violation += clamped - size;
                    violations.push((i, clamped - size));
                    items[i].target = clamped;
                }
                for (i, v) in violations {
                    let freeze = if total_violation == 0.0 { true } else if total_violation > 0.0 { v > 0.0 } else { v < 0.0 };
                    if freeze {
                        items[i].frozen = true;
                    }
                }
            }
            // Lay out each item at its main size to find the cross size.
            let mut asc_max = 0.0f32;
            let mut desc_max = 0.0f32;
            let mut cross_max = 0.0f32;
            for &i in line {
                let st = items[i].s;
                let b = items[i].bx;
                let target = items[i].target;
                let align = self.item_align(st, s);
                let out = if row {
                    let args = Args { avail_w: cw, cb_h: ch_known, forced_w: Some(target), forced_h: None, shrink: false };
                    self.layout_box(b, &args, None, false)
                } else {
                    let cross_w = (cw - items[i].mg[3] - items[i].mg[1]).max(0.0);
                    let stretch = align == Align::Stretch && st.width.is_auto() && !items[i].mauto[1] && !items[i].mauto[3];
                    let args = Args { avail_w: cw, cb_h: ch_known, forced_w: if stretch { Some(cross_w) } else { None }, forced_h: Some(target), shrink: !stretch };
                    self.layout_box(b, &args, None, false)
                };
                let cross_size = if row { out.h } else { out.w };
                items[i].cross = cross_size + items[i].mg[ca] + items[i].mg[cb_];
                if align == Align::Baseline && row {
                    let bl = out.baseline.unwrap_or(out.h);
                    asc_max = asc_max.max(items[i].mg[0] + bl);
                    desc_max = desc_max.max(out.h - bl + items[i].mg[2]);
                } else {
                    cross_max = cross_max.max(items[i].cross);
                }
                items[i].out = out;
            }
            let lc = cross_max.max(asc_max + desc_max);
            line_cross.push(lc);
            baseline_info.push((asc_max, desc_max));
            line_main_used.push(sum_hypo);
            let _ = &mut line_main_used;
        }

        // Cross size of lines (definite container cross size).
        let total_gap_cross = gap_cross * lines.len().saturating_sub(1) as f32;
        if lines.len() == 1 && !wrap && let Some(c) = cross_avail {
            line_cross[0] = c;
        } else if let Some(c) = cross_avail {
            let used: f32 = line_cross.iter().sum::<f32>() + total_gap_cross;
            let free = c - used;
            if free > 0.0 && matches!(s.align_content, Align::Normal | Align::Stretch) && !lines.is_empty() {
                let add = free / lines.len() as f32;
                for l in line_cross.iter_mut() {
                    *l += add;
                }
            }
        }
        let cross_content: f32 = line_cross.iter().sum::<f32>() + total_gap_cross;
        let container_cross = cross_avail.unwrap_or(cross_content);
        // align-content offsets for definite multi-line containers
        let mut cross_start = 0.0f32;
        let mut cross_extra_gap = 0.0f32;
        if let Some(c) = cross_avail {
            let free = c - cross_content;
            let n = lines.len().max(1) as f32;
            match s.align_content {
                Align::Center => cross_start = free / 2.0,
                Align::End | Align::FlexEnd => cross_start = free,
                Align::SpaceBetween if lines.len() > 1 && free > 0.0 => cross_extra_gap = free / (n - 1.0),
                Align::SpaceAround if free > 0.0 => {
                    cross_extra_gap = free / n;
                    cross_start = free / (2.0 * n);
                }
                Align::SpaceEvenly if free > 0.0 => {
                    cross_extra_gap = free / (n + 1.0);
                    cross_start = cross_extra_gap;
                }
                _ => {}
            }
        }

        // Position lines and items.
        let order: Vec<usize> = if wrap_reverse { (0..lines.len()).rev().collect() } else { (0..lines.len()).collect() };
        let mut cross_cursor = cross_start;
        let mut first_baseline: Option<f32> = None;
        let mut total_main_extent = 0.0f32;
        for (pos, &li) in order.iter().enumerate() {
            let line = &lines[li];
            let lc = line_cross[li];
            let gaps = gap_main * line.len().saturating_sub(1) as f32;
            let used: f32 = line.iter().map(|&i| items[i].target + items[i].mg[ma] + items[i].mg[mb_]).sum::<f32>() + gaps;
            let container_main = main_avail.unwrap_or(used);
            let mut free = container_main - used;
            // auto margins on the main axis
            let auto_count: usize = line.iter().map(|&i| items[i].mauto[ma] as usize + items[i].mauto[mb_] as usize).sum();
            if auto_count > 0 && free > 0.0 {
                let each = free / auto_count as f32;
                for &i in line {
                    if items[i].mauto[ma] {
                        items[i].mg[ma] = each;
                    }
                    if items[i].mauto[mb_] {
                        items[i].mg[mb_] = each;
                    }
                }
                free = 0.0;
            }
            let n = line.len() as f32;
            let (mut start, mut between) = (0.0f32, 0.0f32);
            match s.justify_content {
                Align::Center => start = free / 2.0,
                Align::End | Align::FlexEnd => start = free,
                Align::SpaceBetween if free > 0.0 && line.len() > 1 => between = free / (n - 1.0),
                Align::SpaceAround if free > 0.0 => {
                    between = free / n;
                    start = between / 2.0;
                }
                Align::SpaceEvenly if free > 0.0 => {
                    between = free / (n + 1.0);
                    start = between;
                }
                _ => {}
            }
            let mut cursor = start;
            for (k, &i) in line.iter().enumerate() {
                if k > 0 {
                    cursor += gap_main + between;
                }
                cursor += items[i].mg[ma];
                items[i].off_main = cursor;
                cursor += items[i].target + items[i].mg[mb_];
            }
            total_main_extent = total_main_extent.max(cursor);
            // cross-axis alignment
            let (asc, _) = baseline_info[li];
            for &i in line {
                let align = self.item_align(items[i].s, s);
                let outer_cross = items[i].cross;
                let mut free_c = lc - outer_cross;
                let mut m_start = items[i].mg[ca];
                let auto_c = items[i].mauto[ca] as usize + items[i].mauto[cb_] as usize;
                if auto_c > 0 && free_c > 0.0 {
                    let each = free_c / auto_c as f32;
                    if items[i].mauto[ca] {
                        m_start = each;
                    }
                    free_c = 0.0;
                }
                let pos_in_line = if auto_c > 0 {
                    m_start
                } else {
                    match align {
                        Align::End | Align::FlexEnd => free_c + m_start,
                        Align::Center => free_c / 2.0 + m_start,
                        Align::Baseline if row => asc - items[i].out.baseline.unwrap_or(items[i].out.h) + 0.0,
                        _ => m_start,
                    }
                };
                items[i].off_cross = cross_cursor + pos_in_line;
                // stretch: re-lay out with the final cross size when it grew
                if align == Align::Stretch && auto_c == 0 {
                    let st = items[i].s;
                    let target_cross = (lc - items[i].mg[ca] - items[i].mg[cb_]).max(0.0);
                    let cross_dim_auto = if row { st.height.is_auto() } else { st.width.is_auto() };
                    let current = if row { items[i].out.h } else { items[i].out.w };
                    if cross_dim_auto && (target_cross - current).abs() > 0.01 && !self.is_replaced(items[i].bx) {
                        let args = if row {
                            Args { avail_w: cw, cb_h: ch_known, forced_w: Some(items[i].target), forced_h: Some(target_cross), shrink: false }
                        } else {
                            Args { avail_w: cw, cb_h: ch_known, forced_w: Some(target_cross), forced_h: Some(items[i].target), shrink: false }
                        };
                        items[i].out = self.layout_box(items[i].bx, &args, None, false);
                    }
                }
            }
            if pos == 0 && first_baseline.is_none() {
                if let Some(&i) = line.first() {
                    let (x, y) = if row { (items[i].off_main, items[i].off_cross) } else { (items[i].off_cross, items[i].off_main) };
                    let _ = x;
                    first_baseline = items[i].out.baseline.map(|b| b + y);
                }
            }
            cross_cursor += lc + gap_cross + cross_extra_gap;
        }

        // Emit.
        let container_main_final = match (main_avail, row) {
            (Some(m), _) => m,
            (None, _) => total_main_extent,
        };
        for it in items {
            let mut main_pos = it.off_main;
            if reverse {
                main_pos = container_main_final - it.off_main - it.target;
            }
            let (x, y) = if row { (main_pos, it.off_cross) } else { (it.off_cross, main_pos) };
            let _ = (it.hypo, it.base, it.pb_cross, it.frozen);
            result.absorb(it.out, x, y);
        }
        result.baseline = first_baseline;
        let content_h = if row { container_cross } else { container_main_final };
        (result, content_h)
    }

    fn item_align(&self, item: &Style, container: &Style) -> Align {
        let a = if item.align_self == Align::Auto { container.align_items } else { item.align_self };
        match a {
            Align::Normal | Align::Auto => Align::Stretch,
            a => a,
        }
    }
}
