//! Table layout: auto/fixed column sizing, row/col spans, border-spacing and collapsed borders.
use crate::{css::*, layout::*};
use alloc::{vec, vec::Vec};

struct Cell<'a> {
    bx: Bx,
    s: &'a Style,
    row: usize,
    col: usize,
    rspan: usize,
    cspan: usize,
    min: f32,
    max: f32,
    spec_px: Option<f32>,
    spec_pct: Option<f32>,
    out: Option<Out>,
    height: f32,
}

struct Row<'a> {
    bx: Option<Bx>,
    group: Option<Bx>,
    s: Option<&'a Style>,
    height: f32,
    y: f32,
}

struct Structure<'a> {
    captions: Vec<Bx>,
    rows: Vec<Row<'a>>,
    cells: Vec<Cell<'a>>,
    cols: usize,
    col_specs: Vec<(Option<f32>, Option<f32>)>,
}

fn span_attr(v: Option<&str>, default: usize, max: usize) -> usize {
    v.and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(default).clamp(1, max)
}

impl<'a> Ctx<'a> {
    fn table_collect(&mut self, bx: Bx, cw: f32) -> Structure<'a> {
        let mut st = Structure { captions: Vec::new(), rows: Vec::new(), cells: Vec::new(), cols: 0, col_specs: Vec::new() };
        let mut pending_cells: Vec<Bx> = Vec::new();
        let mut row_cells: Vec<(usize, Vec<Bx>)> = Vec::new(); // row index -> cells
        let children = self.children(bx);
        let mut flat: Vec<(Bx, Option<Bx>)> = Vec::new(); // (row-ish box, group)
        let _ = &mut pending_cells;
        for c in children {
            if c.pseudo == 0 && self.doc.nodes[c.id].tag().is_none() {
                continue;
            }
            let s = self.style(c);
            match s.display {
                Display::None => {}
                Display::TableCaption => st.captions.push(c),
                Display::TableColumn => {
                    let mut cols: Vec<Bx> = Vec::new();
                    if self.tag(c) == Some("colgroup") {
                        for g in self.children(c) {
                            if self.style(g).display == Display::TableColumn {
                                cols.push(g);
                            }
                        }
                        if cols.is_empty() {
                            cols.push(c);
                        }
                    } else {
                        cols.push(c);
                    }
                    for col in cols {
                        let cs = self.style(col);
                        let span = span_attr(self.attr(col, "span"), 1, 1000);
                        let px = cs.width.resolve(0.0).filter(|_| !cs.width.has_percent());
                        let pct = match &cs.width {
                            Dim::Len(_, p) if *p != 0.0 => Some(*p),
                            _ => None,
                        };
                        for _ in 0..span {
                            st.col_specs.push((px, pct));
                        }
                    }
                }
                Display::TableRowGroup => {
                    for r in self.children(c) {
                        if r.pseudo == 0 && self.doc.nodes[r.id].tag().is_none() {
                            continue;
                        }
                        match self.style(r).display {
                            Display::TableRow => flat.push((r, Some(c))),
                            Display::TableCell => flat.push((r, Some(c))),
                            Display::None => {}
                            _ => {}
                        }
                    }
                }
                Display::TableRow | Display::TableCell => flat.push((c, None)),
                _ => {
                    // stray content: anonymous full-width cell
                    if c.pseudo != 0 || self.doc.nodes[c.id].tag().is_some() {
                        flat.push((c, None));
                    }
                }
            }
        }
        // Group consecutive cells into anonymous rows.
        let mut anon: Vec<Bx> = Vec::new();
        let mut list: Vec<(Option<Bx>, Option<Bx>, Vec<Bx>)> = Vec::new();
        for (b, g) in flat {
            match self.style(b).display {
                Display::TableRow => {
                    if !anon.is_empty() {
                        list.push((None, None, core::mem::take(&mut anon)));
                    }
                    let cells: Vec<Bx> = self
                        .children(b)
                        .into_iter()
                        .filter(|c| {
                            if c.pseudo == 0 && self.doc.nodes[c.id].tag().is_none() {
                                return false;
                            }
                            self.style(*c).display != Display::None
                        })
                        .collect();
                    list.push((Some(b), g, cells));
                }
                _ => anon.push(b),
            }
        }
        if !anon.is_empty() {
            list.push((None, None, anon));
        }
        let mut occ: Vec<Vec<bool>> = Vec::new();
        for (ri, (rb, grp, cells)) in list.into_iter().enumerate() {
            st.rows.push(Row { bx: rb, group: grp, s: rb.map(|b| self.style(b)), height: 0.0, y: 0.0 });
            let _ = &mut row_cells;
            let mut col = 0;
            for cb in cells {
                while occ.get(ri).and_then(|r| r.get(col)).copied().unwrap_or(false) {
                    col += 1;
                }
                let cs = self.style(cb);
                let is_cell = cs.display == Display::TableCell;
                let cspan = if is_cell { span_attr(self.attr(cb, "colspan"), 1, 1000) } else { 1 };
                let rspan = if is_cell { span_attr(self.attr(cb, "rowspan"), 1, 65534) } else { 1 };
                for r in ri..ri + rspan.min(200) {
                    while occ.len() <= r {
                        occ.push(Vec::new());
                    }
                    if occ[r].len() < col + cspan {
                        occ[r].resize(col + cspan, false);
                    }
                    for c in col..col + cspan {
                        occ[r][c] = true;
                    }
                }
                let (_, p, b, _) = self.edges(cs, cw);
                let hpb = p.h() + b.h();
                let (cmin, cmax) = self.intrinsic_content(cb);
                let (cmin, cmax) = (cmin + hpb, cmax + hpb);
                let spec_px = match &cs.width {
                    Dim::Len(px, pct) if *pct == 0.0 => Some(if cs.border_box { *px } else { *px + hpb }),
                    _ => None,
                };
                let spec_pct = match &cs.width {
                    Dim::Len(_, pct) if *pct != 0.0 => Some(*pct),
                    _ => None,
                };
                st.cells.push(Cell { bx: cb, s: cs, row: ri, col, rspan, cspan, min: cmin, max: cmax.max(cmin), spec_px, spec_pct, out: None, height: 0.0 });
                col += cspan;
            }
        }
        st.cols = st.cells.iter().map(|c| c.col + c.cspan).max().unwrap_or(0).max(st.col_specs.len());
        // rowspans reaching past the last row
        let nrows = st.rows.len();
        for c in st.cells.iter_mut() {
            c.rspan = c.rspan.min(nrows - c.row);
        }
        st
    }

    /// (min, max) content widths for each column plus spec info.
    #[allow(clippy::type_complexity)]
    fn table_columns(&self, st: &Structure) -> (Vec<f32>, Vec<f32>, Vec<Option<f32>>, Vec<Option<f32>>) {
        let n = st.cols;
        let mut min = vec![0.0f32; n];
        let mut max = vec![0.0f32; n];
        let mut spec_px: Vec<Option<f32>> = vec![None; n];
        let mut spec_pct: Vec<Option<f32>> = vec![None; n];
        for (i, (px, pct)) in st.col_specs.iter().enumerate() {
            if i < n {
                spec_px[i] = *px;
                spec_pct[i] = *pct;
            }
        }
        for c in st.cells.iter().filter(|c| c.cspan == 1) {
            min[c.col] = min[c.col].max(c.min);
            max[c.col] = max[c.col].max(c.max);
            if let Some(px) = c.spec_px {
                spec_px[c.col] = Some(spec_px[c.col].map_or(px, |v| v.max(px)));
            }
            if let Some(p) = c.spec_pct {
                spec_pct[c.col] = Some(spec_pct[c.col].map_or(p, |v| v.max(p)));
            }
        }
        for c in st.cells.iter().filter(|c| c.cspan > 1) {
            let end = (c.col + c.cspan).min(n);
            let have_min: f32 = min[c.col..end].iter().sum();
            if c.min > have_min {
                let add = (c.min - have_min) / (end - c.col) as f32;
                for v in &mut min[c.col..end] {
                    *v += add;
                }
            }
            let have_max: f32 = max[c.col..end].iter().sum();
            if c.max > have_max {
                let add = (c.max - have_max) / (end - c.col) as f32;
                for v in &mut max[c.col..end] {
                    *v += add;
                }
            }
        }
        for i in 0..n {
            max[i] = max[i].max(min[i]);
        }
        (min, max, spec_px, spec_pct)
    }

    fn table_spacing(&self, s: &Style) -> (f32, f32) {
        if s.border_collapse { (0.0, 0.0) } else { s.border_spacing }
    }

    pub fn table_intrinsic(&mut self, bx: Bx, s: &'a Style) -> (f32, f32) {
        let st = self.table_collect(bx, 0.0);
        let (min, max, spec_px, _) = self.table_columns(&st);
        let (hs, _) = self.table_spacing(s);
        let n = st.cols as f32;
        let spacing = hs * (n + 1.0);
        let mut mn = min.iter().sum::<f32>() + spacing;
        let mut mx: f32 = (0..st.cols).map(|i| spec_px[i].map_or(max[i], |p| p.max(min[i]))).sum::<f32>() + spacing;
        for &cap in &st.captions {
            let (a, b) = self.intrinsic(cap);
            mn = mn.max(a);
            mx = mx.max(b);
        }
        if st.cols == 0 {
            return (0.0, 0.0);
        }
        (mn, mx.max(mn))
    }

    fn distribute_widths(&self, st: &Structure, s: &Style, avail: f32) -> Vec<f32> {
        let n = st.cols;
        let (min, max, spec_px, spec_pct) = self.table_columns(st);
        if n == 0 {
            return Vec::new();
        }
        if s.table_fixed {
            // equal shares unless widths are specified
            let mut widths: Vec<Option<f32>> = (0..n).map(|i| spec_px[i].or(spec_pct[i].map(|p| avail * p / 100.0))).collect();
            let fixed: f32 = widths.iter().flatten().sum();
            let auto = widths.iter().filter(|w| w.is_none()).count();
            let each = if auto > 0 { ((avail - fixed) / auto as f32).max(0.0) } else { 0.0 };
            for w in widths.iter_mut() {
                if w.is_none() {
                    *w = Some(each);
                }
            }
            return widths.into_iter().map(|w| w.unwrap_or(0.0)).collect();
        }
        let mut pref: Vec<f32> = Vec::with_capacity(n);
        let mut is_auto: Vec<bool> = Vec::with_capacity(n);
        for i in 0..n {
            if let Some(p) = spec_pct[i] {
                pref.push((avail * p / 100.0).max(min[i]));
                is_auto.push(false);
            } else if let Some(px) = spec_px[i] {
                pref.push(px.max(min[i]));
                is_auto.push(false);
            } else {
                pref.push(max[i]);
                is_auto.push(true);
            }
        }
        let total: f32 = pref.iter().sum();
        let mut widths = pref.clone();
        if total <= avail {
            let extra = avail - total;
            let basis: Vec<usize> = if is_auto.iter().any(|a| *a) { (0..n).filter(|&i| is_auto[i]).collect() } else { (0..n).collect() };
            let weight: f32 = basis.iter().map(|&i| pref[i].max(0.0)).sum();
            for &i in &basis {
                widths[i] += if weight > 0.0 { extra * pref[i] / weight } else { extra / basis.len() as f32 };
            }
        } else {
            let sum_min: f32 = min.iter().sum();
            if sum_min >= avail {
                widths = min.clone();
            } else {
                let room = avail - sum_min;
                let want: f32 = (0..n).map(|i| pref[i] - min[i]).sum();
                for i in 0..n {
                    widths[i] = min[i] + if want > 0.0 { (pref[i] - min[i]) * room / want } else { 0.0 };
                }
            }
        }
        widths
    }

    pub fn layout_table(&mut self, bx: Bx, s: &'a Style, content_w: f32, _border_w: f32) -> (Out, f32) {
        let mut st = self.table_collect(bx, content_w);
        let mut result = Out::default();
        let (hs, vs) = self.table_spacing(s);
        let n = st.cols;
        let nrows = st.rows.len();
        // captions
        let mut top_h = 0.0f32;
        let mut bottom_caps: Vec<Bx> = Vec::new();
        for &cap in &st.captions {
            if self.style(cap).caption_bottom || s.caption_bottom {
                bottom_caps.push(cap);
                continue;
            }
            let args = Args { avail_w: content_w, cb_h: None, forced_w: Some(content_w), forced_h: None, shrink: false };
            let out = self.layout_box(cap, &args, None, false);
            let h = out.h;
            result.absorb(out, 0.0, top_h);
            top_h += h;
        }
        if n == 0 || nrows == 0 {
            let mut bottom = top_h;
            for cap in bottom_caps {
                let args = Args { avail_w: content_w, cb_h: None, forced_w: Some(content_w), forced_h: None, shrink: false };
                let out = self.layout_box(cap, &args, None, false);
                let h = out.h;
                result.absorb(out, 0.0, bottom);
                bottom += h;
            }
            return (result, bottom);
        }
        let avail = (content_w - hs * (n as f32 + 1.0)).max(0.0);
        let widths = self.distribute_widths(&st, s, avail);
        let mut col_x = Vec::with_capacity(n);
        let mut x = hs;
        for w in &widths {
            col_x.push(x);
            x += w + hs;
        }
        let collapse = s.border_collapse;
        // lay cells out at their column width
        for idx in 0..st.cells.len() {
            let (bx_c, col, cspan, row) = {
                let c = &st.cells[idx];
                (c.bx, c.col, c.cspan, c.row)
            };
            let end = (col + cspan).min(n);
            let w: f32 = widths[col..end].iter().sum::<f32>() + hs * (end - col).saturating_sub(1) as f32;
            if collapse {
                let skip_left = col > 0;
                let skip_top = row > 0;
                self.cell_border_skip = Some([skip_top, false, false, skip_left]);
            }
            let args = Args { avail_w: w, cb_h: None, forced_w: Some(w), forced_h: None, shrink: false };
            let out = self.layout_box(bx_c, &args, None, false);
            self.cell_border_skip = None;
            st.cells[idx].height = out.h;
            st.cells[idx].out = Some(out);
        }
        // row heights
        for ri in 0..nrows {
            let mut h = 0.0f32;
            let mut asc = 0.0f32;
            let mut desc = 0.0f32;
            for c in st.cells.iter().filter(|c| c.row == ri && c.rspan == 1) {
                h = h.max(c.height);
                if matches!(c.s.vertical_align, VerticalAlign::Baseline) {
                    let b = c.out.as_ref().and_then(|o| o.baseline).unwrap_or(0.0);
                    asc = asc.max(b);
                    desc = desc.max(c.height - b);
                }
            }
            h = h.max(asc + desc);
            if let Some(rs) = st.rows[ri].s {
                if let Some(rh) = rs.height.resolve(0.0).filter(|_| !rs.height.has_percent()) {
                    h = h.max(rh);
                }
            }
            st.rows[ri].height = h;
        }
        for c in st.cells.iter().filter(|c| c.rspan > 1) {
            let end = (c.row + c.rspan).min(nrows);
            let have: f32 = st.rows[c.row..end].iter().map(|r| r.height).sum::<f32>() + vs * (end - c.row - 1) as f32;
            if c.height > have {
                st.rows[end - 1].height += c.height - have;
            }
        }
        // specified table height distributes extra space to rows
        let total_rows: f32 = st.rows.iter().map(|r| r.height).sum::<f32>() + vs * (nrows as f32 + 1.0);
        if let Some(th) = s.height.resolve(0.0).filter(|_| !s.height.has_percent()) {
            let (_, p, b, _) = self.edges(s, content_w);
            let inner = if s.border_box { th - p.v() - b.v() } else { th };
            let inner = inner - top_h;
            if inner > total_rows && nrows > 0 {
                let extra = (inner - total_rows) / nrows as f32;
                for r in st.rows.iter_mut() {
                    r.height += extra;
                }
            }
        }
        let mut y = vs;
        for r in st.rows.iter_mut() {
            r.y = y;
            y += r.height + vs;
        }
        let grid_h = y;
        // paint row and row-group backgrounds
        let table_w = content_w;
        let mut painted_groups: Vec<Bx> = Vec::new();
        for ri in 0..nrows {
            if let Some(g) = st.rows[ri].group
                && !painted_groups.contains(&g)
            {
                painted_groups.push(g);
                let first = ri;
                let last = (first..nrows).take_while(|&k| st.rows[k].group == Some(g)).last().unwrap_or(first);
                let gs = self.style(g);
                let rect = Rect { x: 0.0, y: top_h + st.rows[first].y - vs.min(0.0), width: table_w, height: st.rows[last].y + st.rows[last].height - st.rows[first].y };
                let mut p = Paint::default();
                self.paint_decorations(&mut p, gs, rect, false, [false; 4]);
                p.hits.push(HitRegion { rect, node: g.id });
                result.paint.append(p, 0.0, 0.0);
            }
            if let (Some(rb), Some(rs)) = (st.rows[ri].bx, st.rows[ri].s) {
                let rect = Rect { x: hs.min(0.0).max(0.0), y: top_h + st.rows[ri].y, width: table_w, height: st.rows[ri].height };
                let mut p = Paint::default();
                if rs.visibility_visible {
                    self.paint_decorations(&mut p, rs, rect, false, [false; 4]);
                }
                p.hits.push(HitRegion { rect, node: rb.id });
                result.paint.append(p, 0.0, 0.0);
            }
        }
        // place cells
        let mut first_baseline: Option<f32> = None;
        for c in st.cells.iter_mut() {
            let Some(out) = c.out.take() else { continue };
            let end = (c.row + c.rspan).min(nrows);
            let cell_h: f32 = st.rows[c.row..end].iter().map(|r| r.height).sum::<f32>() + vs * (end - c.row - 1) as f32;
            let extra = (cell_h - c.height).max(0.0);
            let mut out = out;
            // The cell box fills its row; vertical alignment moves the content.
            let dy = match c.s.vertical_align {
                VerticalAlign::Top | VerticalAlign::TextTop => 0.0,
                VerticalAlign::Bottom | VerticalAlign::TextBottom => extra,
                VerticalAlign::Baseline => {
                    let asc: f32 = st.rows[c.row..end].iter().map(|_| 0.0f32).sum();
                    let _ = asc;
                    0.0
                }
                _ => extra / 2.0,
            };
            let baseline_shift = if matches!(c.s.vertical_align, VerticalAlign::Baseline) && c.rspan == 1 {
                // align first baselines within the row
                let row_asc = {
                    let mut a = 0.0f32;
                    // computed lazily below
                    a += 0.0;
                    a
                };
                row_asc
            } else {
                0.0
            };
            let _ = baseline_shift;
            // Stretch the cell box to the row height; content is shifted by `dy` inside.
            let stretch = extra;
            let (px, py) = (col_x[c.col], top_h + st.rows[c.row].y);
            if stretch > 0.0 {
                out = self.stretch_cell(out, c.s, stretch, dy);
            }
            if first_baseline.is_none() {
                first_baseline = out.baseline.map(|b| b + py);
            }
            result.absorb(out, px, py);
        }
        let mut bottom = top_h + grid_h;
        for cap in bottom_caps {
            let args = Args { avail_w: content_w, cb_h: None, forced_w: Some(content_w), forced_h: None, shrink: false };
            let out = self.layout_box(cap, &args, None, false);
            let h = out.h;
            result.absorb(out, 0.0, bottom);
            bottom += h;
        }
        result.baseline = first_baseline;
        (result, bottom)
    }

    /// Grow a laid-out cell to the row height. Background/border commands that cover the
    /// old height are rewritten; content is shifted by `shift` for vertical alignment.
    fn stretch_cell(&mut self, mut out: Out, _s: &Style, extra: f32, shift: f32) -> Out {
        let old_h = out.h;
        let new_h = old_h + extra;
        // The first decorations belong to the cell box itself (pushed before content).
        let mut seen_box_cmds = true;
        let mut content_start = 0;
        for (i, c) in out.paint.cmds.iter_mut().enumerate() {
            match c {
                Command::FillRect { rect, .. } | Command::Gradient { rect, .. } | Command::Shadow { rect, .. } | Command::Border { rect, .. }
                    if seen_box_cmds && (rect.height - old_h).abs() < 0.01 && rect.y == 0.0 =>
                {
                    rect.height = new_h;
                    content_start = i + 1;
                }
                Command::Image { rect, .. } if seen_box_cmds && (rect.height - old_h).abs() < 0.01 && rect.y == 0.0 => {
                    rect.height = new_h;
                    content_start = i + 1;
                }
                _ => seen_box_cmds = false,
            }
        }
        if let Some(first) = out.paint.hits.first_mut() {
            first.rect.height = new_h;
        }
        if shift != 0.0 {
            for c in out.paint.cmds.iter_mut().skip(content_start) {
                translate_command(c, 0.0, shift);
            }
            for h in out.paint.hits.iter_mut().skip(1) {
                h.rect.y += shift;
            }
            for (_, p) in out.layers.iter_mut() {
                p.translate(0.0, shift);
            }
            for p in out.abs.iter_mut() {
                p.y += shift;
            }
            if let Some(b) = out.baseline.as_mut() {
                *b += shift;
            }
        }
        out.h = new_h;
        out
    }
}

fn translate_command(c: &mut Command, dx: f32, dy: f32) {
    let mut p = Paint { cmds: vec![core::mem::replace(c, Command::PopClip)], hits: Vec::new() };
    p.translate(dx, dy);
    *c = p.cmds.pop().unwrap_or(Command::PopClip);
}
