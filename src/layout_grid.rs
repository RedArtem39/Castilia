//! CSS Grid layout (explicit/implicit tracks, auto placement, fr/minmax sizing, areas).
use crate::{css::*, layout::*, math};
use alloc::{string::String, vec, vec::Vec};

#[derive(Clone, Copy, PartialEq)]
enum SizeKind {
    Fixed(f32),
    Min,
    Max,
    Flex(f32),
}

struct TrackState {
    base_kind: SizeKind,
    limit_kind: SizeKind,
    base: f32,
    limit: f32,
    flex: f32,
    auto_max: bool,
}

#[derive(Clone)]
struct Contribution {
    start: usize,
    span: usize,
    min_c: f32,
    max_c: f32,
}

fn track_state(def: &Track, avail: Option<f32>) -> TrackState {
    let kinds = |t: &Track, is_min: bool| -> SizeKind {
        match t {
            Track::Px(p) => SizeKind::Fixed(*p),
            Track::Pct(p) => match avail {
                Some(a) => SizeKind::Fixed(a * p / 100.0),
                None => SizeKind::Max,
            },
            Track::Fr(f) => {
                if is_min {
                    SizeKind::Min
                } else {
                    SizeKind::Flex(*f)
                }
            }
            Track::Auto => {
                if is_min {
                    SizeKind::Min
                } else {
                    SizeKind::Max
                }
            }
            Track::MinContent => SizeKind::Min,
            Track::MaxContent => SizeKind::Max,
            Track::Func(op, args) => SizeKind::Fixed(crate::css::resolve_track_func(*op, args, avail.unwrap_or(0.0)).max(0.0)),
            Track::FitContent(_) => {
                if is_min {
                    SizeKind::Min
                } else {
                    SizeKind::Max
                }
            }
            Track::MinMax(..) => SizeKind::Min,
        }
    };
    let (base_kind, limit_kind, auto_max) = match def {
        Track::MinMax(a, b) => {
            let bk = kinds(a, true);
            let bk = if matches!(**a, Track::MaxContent) { SizeKind::Max } else { bk };
            let lk = kinds(b, false);
            (bk, lk, matches!(**b, Track::Auto))
        }
        Track::Auto => (SizeKind::Min, SizeKind::Max, true),
        Track::FitContent(_) => (SizeKind::Min, SizeKind::Max, false),
        Track::MinContent => (SizeKind::Min, SizeKind::Min, false),
        Track::MaxContent => (SizeKind::Max, SizeKind::Max, false),
        Track::Fr(f) => (SizeKind::Min, SizeKind::Flex(*f), false),
        t => (kinds(t, true), kinds(t, false), false),
    };
    let flex = if let SizeKind::Flex(f) = limit_kind { f } else { 0.0 };
    let base = if let SizeKind::Fixed(p) = base_kind { p } else { 0.0 };
    let limit = match limit_kind {
        SizeKind::Fixed(p) => p,
        _ => 0.0,
    };
    let limit = if let Track::FitContent(p) = def { *p } else { limit };
    TrackState { base_kind, limit_kind, base, limit, flex, auto_max }
}

/// Size tracks given item contributions. With `avail == None` tracks take their
/// max-content size (`prefer_limit`) or min-content size.
fn size_tracks(defs: &[Track], items: &[Contribution], avail: Option<f32>, gap: f32, prefer_limit: bool, collapsed: &[bool]) -> Vec<f32> {
    let n = defs.len();
    if n == 0 {
        return Vec::new();
    }
    let mut st: Vec<TrackState> = defs.iter().map(|d| track_state(d, avail)).collect();
    let fit_cap: Vec<Option<f32>> = defs.iter().map(|d| if let Track::FitContent(p) = d { Some(*p) } else { None }).collect();
    // intrinsic contributions
    let mut limit_needs: Vec<f32> = vec![0.0; n];
    for it in items.iter().filter(|i| i.span == 1) {
        let t = &mut st[it.start];
        match t.base_kind {
            SizeKind::Min => t.base = t.base.max(it.min_c),
            SizeKind::Max => t.base = t.base.max(it.max_c),
            _ => {}
        }
        match t.limit_kind {
            SizeKind::Min => limit_needs[it.start] = limit_needs[it.start].max(it.min_c),
            SizeKind::Max => limit_needs[it.start] = limit_needs[it.start].max(it.max_c),
            _ => {}
        }
    }
    for it in items.iter().filter(|i| i.span > 1) {
        let end = (it.start + it.span).min(n);
        let spanned: Vec<usize> = (it.start..end).collect();
        let inner_gaps = gap * (spanned.len() as f32 - 1.0);
        let growable: Vec<usize> = spanned.iter().copied().filter(|&i| !matches!(st[i].base_kind, SizeKind::Fixed(_))).collect();
        if !growable.is_empty() {
            let have: f32 = spanned.iter().map(|&i| st[i].base).sum::<f32>() + inner_gaps;
            let need = it.min_c - have;
            if need > 0.0 {
                for &i in &growable {
                    st[i].base += need / growable.len() as f32;
                }
            }
        }
        let lim_grow: Vec<usize> = spanned.iter().copied().filter(|&i| matches!(st[i].limit_kind, SizeKind::Min | SizeKind::Max)).collect();
        if !lim_grow.is_empty() {
            let have: f32 = spanned.iter().map(|&i| st[i].base.max(limit_needs[i])).sum::<f32>() + inner_gaps;
            let need = it.max_c - have;
            if need > 0.0 {
                for &i in &lim_grow {
                    limit_needs[i] += need / lim_grow.len() as f32;
                }
            }
        }
    }
    for (i, t) in st.iter_mut().enumerate() {
        if matches!(t.limit_kind, SizeKind::Min | SizeKind::Max) {
            t.limit = t.limit.max(limit_needs[i]);
            if let Some(cap) = fit_cap[i] {
                t.limit = t.limit_cap(cap, limit_needs[i], t.base);
            }
        }
        if matches!(t.limit_kind, SizeKind::Fixed(_)) {
            // minmax(auto, 100px) etc.
        }
        t.limit = t.limit.max(t.base);
        if let SizeKind::Fixed(p) = t.base_kind {
            t.limit = t.limit.max(p);
        }
    }
    let gaps_total: f32 = {
        let visible = collapsed.iter().take(n).filter(|c| !**c).count();
        gap * visible.saturating_sub(1) as f32
    };
    let mut size: Vec<f32> = st.iter().map(|t| t.base).collect();
    for (i, c) in collapsed.iter().enumerate().take(n) {
        if *c {
            size[i] = 0.0;
        }
    }
    match avail {
        Some(a) => {
            // maximize tracks up to their growth limit
            let has_flex = st.iter().any(|t| t.flex > 0.0);
            let mut free = a - gaps_total - size.iter().sum::<f32>();
            let mut guard = 0;
            while free > 0.01 && guard < 50 {
                guard += 1;
                let growable: Vec<usize> = (0..n).filter(|&i| !collapsed.get(i).copied().unwrap_or(false) && st[i].flex == 0.0 && size[i] < st[i].limit - 0.001 && !matches!(st[i].limit_kind, SizeKind::Fixed(_)) || (!collapsed.get(i).copied().unwrap_or(false) && matches!(st[i].limit_kind, SizeKind::Fixed(_)) && size[i] < st[i].limit - 0.001 && !matches!(st[i].base_kind, SizeKind::Fixed(_)))).collect();
                if growable.is_empty() {
                    break;
                }
                let share = free / growable.len() as f32;
                let mut used = 0.0;
                for &i in &growable {
                    let add = share.min(st[i].limit - size[i]);
                    size[i] += add;
                    used += add;
                }
                free -= used;
                if used < 0.001 {
                    break;
                }
            }
            if has_flex {
                let mut frozen: Vec<bool> = st.iter().map(|t| t.flex == 0.0).collect();
                let fixed_sum: f32 = (0..n).filter(|&i| st[i].flex == 0.0).map(|i| size[i]).sum();
                let mut leftover = a - gaps_total - fixed_sum;
                let mut fr = 0.0;
                for _ in 0..20 {
                    let sum_f: f32 = (0..n).filter(|&i| !frozen[i] && !collapsed.get(i).copied().unwrap_or(false)).map(|i| st[i].flex).sum();
                    if sum_f <= 0.0 {
                        break;
                    }
                    fr = leftover.max(0.0) / sum_f.max(1.0);
                    let mut changed = false;
                    for i in 0..n {
                        if !frozen[i] && st[i].base > fr * st[i].flex + 0.001 {
                            frozen[i] = true;
                            leftover -= st[i].base;
                            size[i] = st[i].base;
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }
                for i in 0..n {
                    if !frozen[i] && !collapsed.get(i).copied().unwrap_or(false) {
                        size[i] = (fr * st[i].flex).max(st[i].base);
                    }
                }
            } else {
                // stretch auto tracks
                let free = a - gaps_total - size.iter().sum::<f32>();
                if free > 0.01 {
                    let autos: Vec<usize> = (0..n).filter(|&i| st[i].auto_max && !collapsed.get(i).copied().unwrap_or(false)).collect();
                    if !autos.is_empty() {
                        for &i in &autos {
                            size[i] += free / autos.len() as f32;
                        }
                    }
                }
            }
        }
        None => {
            for i in 0..n {
                if collapsed.get(i).copied().unwrap_or(false) {
                    continue;
                }
                size[i] = if st[i].flex > 0.0 {
                    st[i].base
                } else if prefer_limit {
                    st[i].limit.max(st[i].base)
                } else {
                    st[i].base
                };
            }
            // Indefinite free space (§12.7.1): the flex fraction is the largest of each flexible
            // track's base / flex and, for every item crossing flexible tracks, what one fr must be
            // for the spanned tracks to hold the item's contribution.
            let is_flex = |i: usize| st[i].flex > 0.0 && !collapsed.get(i).copied().unwrap_or(false);
            if (0..n).any(is_flex) {
                let mut fr: f32 = (0..n).filter(|&i| is_flex(i)).map(|i| st[i].base / st[i].flex.max(1.0)).fold(0.0, f32::max);
                for it in items {
                    let end = (it.start + it.span).min(n);
                    let flex_sum: f32 = (it.start..end).filter(|&i| is_flex(i)).map(|i| st[i].flex).sum();
                    if flex_sum <= 0.0 {
                        continue;
                    }
                    let fixed: f32 = (it.start..end).filter(|&i| !is_flex(i)).map(|i| size[i]).sum();
                    let gaps = gap * (end - it.start).saturating_sub(1) as f32;
                    let want = if prefer_limit { it.max_c } else { it.min_c };
                    fr = fr.max((want - fixed - gaps) / flex_sum.max(1.0));
                }
                for i in 0..n {
                    if is_flex(i) {
                        size[i] = size[i].max(fr * st[i].flex);
                    }
                }
            }
        }
    }
    size
}

impl TrackState {
    fn limit_cap(&self, cap: f32, needs: f32, base: f32) -> f32 {
        needs.min(cap).max(base)
    }
}

struct Placed<'a> {
    bx: Bx,
    s: &'a Style,
    col: usize,
    cspan: usize,
    row: usize,
    rspan: usize,
    out: Out,
    ml: f32,
    mt: f32,
}

fn has_funcs(list: &TrackList) -> bool {
    fn f(t: &Track) -> bool {
        match t {
            Track::Func(..) => true,
            Track::MinMax(a, b) => f(a) || f(b),
            _ => false,
        }
    }
    list.before.iter().chain(list.after.iter()).any(f) || list.repeat.as_ref().is_some_and(|(_, ts)| ts.iter().any(f))
}

struct Template {
    cols: Vec<Track>,
    rows: Vec<Track>,
    auto_fit_cols: Vec<bool>,
}

fn track_min_size(t: &Track) -> f32 {
    match t {
        Track::Px(p) => *p,
        Track::MinMax(a, _) => match **a {
            Track::Px(p) => p,
            _ => 0.0,
        },
        Track::FitContent(p) => *p,
        _ => 0.0,
    }
}

/// Replace `min()/max()/clamp()` tracks by fixed sizes once the container size is known.
fn resolve_funcs(t: &Track, avail: Option<f32>) -> Track {
    match t {
        Track::Func(op, args) => Track::Px(crate::css::resolve_track_func(*op, args, avail.unwrap_or(0.0)).max(0.0)),
        Track::MinMax(a, b) => Track::MinMax(alloc::boxed::Box::new(resolve_funcs(a, avail)), alloc::boxed::Box::new(resolve_funcs(b, avail))),
        other => other.clone(),
    }
}

fn expand(list: &TrackList, avail: Option<f32>, gap: f32) -> (Vec<Track>, Vec<bool>) {
    let resolved;
    let list = if has_funcs(list) {
        resolved = TrackList {
            before: list.before.iter().map(|t| resolve_funcs(t, avail)).collect(),
            repeat: list.repeat.as_ref().map(|(f, ts)| (*f, ts.iter().map(|t| resolve_funcs(t, avail)).collect())),
            after: list.after.iter().map(|t| resolve_funcs(t, avail)).collect(),
            ..list.clone()
        };
        &resolved
    } else {
        list
    };
    let mut out: Vec<Track> = list.before.clone();
    let mut fit = vec![false; out.len()];
    if let Some((is_fit, tracks)) = &list.repeat {
        let unit_min: f32 = tracks.iter().map(track_min_size).sum::<f32>() + gap * (tracks.len() as f32 - 1.0).max(0.0);
        let fixed: f32 = list.before.iter().chain(list.after.iter()).map(track_min_size).sum::<f32>() + gap * (list.before.len() + list.after.len()) as f32;
        let count = match avail {
            Some(a) if unit_min + gap > 0.0 => (math::floor32((a - fixed + gap) / (unit_min + gap)).max(1.0)) as usize,
            _ => 1,
        };
        for _ in 0..count.min(500) {
            for t in tracks {
                out.push(t.clone());
                fit.push(*is_fit);
            }
        }
    }
    for t in &list.after {
        out.push(t.clone());
        fit.push(false);
    }
    (out, fit)
}

struct Occupancy {
    cells: Vec<Vec<bool>>,
    cols: usize,
}

impl Occupancy {
    fn is_free(&self, r: usize, c: usize, rs: usize, cs: usize) -> bool {
        for rr in r..r + rs {
            for cc in c..c + cs {
                if self.cells.get(rr).and_then(|row| row.get(cc)).copied().unwrap_or(false) {
                    return false;
                }
            }
        }
        true
    }
    fn mark(&mut self, r: usize, c: usize, rs: usize, cs: usize) {
        while self.cells.len() < r + rs {
            self.cells.push(vec![false; self.cols.max(c + cs)]);
        }
        for rr in r..r + rs {
            if self.cells[rr].len() < c + cs {
                self.cells[rr].resize(c + cs, false);
            }
            for cc in c..c + cs {
                self.cells[rr][cc] = true;
            }
        }
    }
}

fn area_lookup(areas: &[Vec<String>], name: &str) -> Option<(usize, usize, usize, usize)> {
    let mut r0 = usize::MAX;
    let mut r1 = 0;
    let mut c0 = usize::MAX;
    let mut c1 = 0;
    for (r, row) in areas.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell == name {
                r0 = r0.min(r);
                r1 = r1.max(r + 1);
                c0 = c0.min(c);
                c1 = c1.max(c + 1);
            }
        }
    }
    if r0 == usize::MAX { None } else { Some((r0, r1, c0, c1)) }
}

/// Resolve a start/end pair to (start index, span) when explicit.
fn resolve_axis(pair: &(GridLine, GridLine), explicit_count: usize, areas: &[Vec<String>], list: &TrackList, is_col: bool) -> (Option<usize>, usize) {
    let line_idx = |g: &GridLine, end: bool| -> Option<i64> {
        match g {
            GridLine::Line(n) if *n > 0 => Some(*n as i64 - 1),
            GridLine::Line(n) if *n < 0 => Some(explicit_count as i64 + 1 + *n as i64 - 1 + 1 - 1),
            GridLine::Name(name) => {
                if let Some((r0, r1, c0, c1)) = area_lookup(areas, name) {
                    return Some(if is_col { if end { c1 } else { c0 } } else if end { r1 } else { r0 } as i64);
                }
                let wanted_start = alloc::format!("{name}-start");
                let wanted_end = alloc::format!("{name}-end");
                list.names.iter().find(|(n, _)| if end { *n == wanted_end || n == name } else { *n == wanted_start || n == name }).map(|(_, i)| *i as i64)
            }
            _ => None,
        }
    };
    let (a, b) = pair;
    match (a, b) {
        (GridLine::Span(n), GridLine::Span(_)) => (None, (*n).max(1) as usize),
        (GridLine::Span(n), g) => match line_idx(g, true) {
            Some(e) => ((e - *n as i64).max(0) as usize, *n as usize).into_pair(),
            None => (None, (*n).max(1) as usize),
        },
        (g, GridLine::Span(n)) => match line_idx(g, false) {
            Some(s) => (Some(s.max(0) as usize), (*n).max(1) as usize),
            None => (None, (*n).max(1) as usize),
        },
        (g1, g2) => match (line_idx(g1, false), line_idx(g2, true)) {
            (Some(s), Some(e)) => {
                let (s, e) = if e < s { (e, s) } else { (s, e) };
                (Some(s.max(0) as usize), ((e - s).max(1)) as usize)
            }
            (Some(s), None) => (Some(s.max(0) as usize), 1),
            (None, Some(e)) => (Some((e - 1).max(0) as usize), 1),
            (None, None) => (None, 1),
        },
    }
}

trait IntoPair {
    fn into_pair(self) -> (Option<usize>, usize);
}
impl IntoPair for (usize, usize) {
    fn into_pair(self) -> (Option<usize>, usize) {
        (Some(self.0), self.1)
    }
}

impl<'a> Ctx<'a> {
    fn grid_children(&self, bx: Bx) -> (Vec<Bx>, Vec<Bx>) {
        let mut items = Vec::new();
        let mut abs = Vec::new();
        self.grid_collect(bx, &mut items, &mut abs, 0);
        (items, abs)
    }

    fn grid_collect(&self, bx: Bx, items: &mut Vec<Bx>, abs: &mut Vec<Bx>, depth: usize) {
        if depth > 50 {
            return;
        }
        for c in self.children(bx) {
            if c.pseudo == 0
                && let Some(t) = text_of(self.doc, c.id)
            {
                if !t.chars().all(crate::layout_inline::is_ws) {
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
                Display::Contents => self.grid_collect(c, items, abs, depth + 1),
                _ if s.is_out_of_flow() => abs.push(c),
                _ => items.push(c),
            }
        }
    }

    /// Resolve explicit tracks and place items. Returns (items as (bx, col, cspan, row, rspan), template).
    #[allow(clippy::type_complexity)]
    fn grid_place(&mut self, s: &'a Style, items: &[Bx], avail_w: Option<f32>, avail_h: Option<f32>, col_gap: f32, row_gap: f32) -> (Vec<(Bx, usize, usize, usize, usize)>, Template, usize, usize) {
        let (mut cols, mut fit_cols) = expand(&s.grid_columns, avail_w, col_gap);
        let (mut rows, _) = expand(&s.grid_rows, avail_h, row_gap);
        // `grid-template-areas` defines explicit tracks too
        if !s.grid_areas.is_empty() {
            let ncols = s.grid_areas.iter().map(|r| r.len()).max().unwrap_or(0);
            while cols.len() < ncols {
                cols.push(Track::Auto);
                fit_cols.push(false);
            }
            while rows.len() < s.grid_areas.len() {
                rows.push(Track::Auto);
            }
        }
        let explicit_cols = cols.len();
        let explicit_rows = rows.len();
        let col_major = s.grid_auto_flow_column;
        let mut occ = Occupancy { cells: Vec::new(), cols: explicit_cols.max(1) };
        let mut placed: Vec<Option<(usize, usize, usize, usize)>> = vec![None; items.len()];
        struct Spec {
            col: Option<usize>,
            cspan: usize,
            row: Option<usize>,
            rspan: usize,
        }
        let mut specs: Vec<Spec> = Vec::new();
        for &b in items {
            let st = self.style(b);
            let (c, cs) = resolve_axis(&st.grid_column, explicit_cols, &s.grid_areas, &s.grid_columns, true);
            let (r, rs) = resolve_axis(&st.grid_row, explicit_rows, &s.grid_areas, &s.grid_rows, false);
            specs.push(Spec { col: c, cspan: cs, row: r, rspan: rs });
        }
        // 1. fully explicit
        for (i, sp) in specs.iter().enumerate() {
            if let (Some(c), Some(r)) = (sp.col, sp.row) {
                occ.mark(r, c, sp.rspan, sp.cspan);
                placed[i] = Some((c, sp.cspan, r, sp.rspan));
            }
        }
        // 2. auto placement
        let (mut cur_r, mut cur_c) = (0usize, 0usize);
        for (i, sp) in specs.iter().enumerate() {
            if placed[i].is_some() {
                continue;
            }
            let limit_c = explicit_cols.max(1);
            if !col_major {
                if let Some(r) = sp.row {
                    // fixed row, auto column
                    let mut c = if s.grid_auto_flow_dense { 0 } else { 0 };
                    while !occ.is_free(r, c, sp.rspan, sp.cspan) {
                        c += 1;
                    }
                    occ.mark(r, c, sp.rspan, sp.cspan);
                    placed[i] = Some((c, sp.cspan, r, sp.rspan));
                    continue;
                }
                if let Some(c) = sp.col {
                    let mut r = if s.grid_auto_flow_dense { 0 } else { cur_r };
                    while !occ.is_free(r, c, sp.rspan, sp.cspan) {
                        r += 1;
                    }
                    occ.mark(r, c, sp.rspan, sp.cspan);
                    placed[i] = Some((c, sp.cspan, r, sp.rspan));
                    continue;
                }
                if s.grid_auto_flow_dense {
                    cur_r = 0;
                    cur_c = 0;
                }
                loop {
                    if cur_c + sp.cspan > limit_c.max(sp.cspan) {
                        cur_c = 0;
                        cur_r += 1;
                    }
                    if occ.is_free(cur_r, cur_c, sp.rspan, sp.cspan) {
                        break;
                    }
                    cur_c += 1;
                }
                occ.mark(cur_r, cur_c, sp.rspan, sp.cspan);
                placed[i] = Some((cur_c, sp.cspan, cur_r, sp.rspan));
                cur_c += sp.cspan;
            } else {
                // column-major flow: fill down the rows first
                let limit_r = explicit_rows.max(1);
                if let Some(c) = sp.col {
                    let mut r = 0;
                    while !occ.is_free(r, c, sp.rspan, sp.cspan) {
                        r += 1;
                    }
                    occ.mark(r, c, sp.rspan, sp.cspan);
                    placed[i] = Some((c, sp.cspan, r, sp.rspan));
                    continue;
                }
                loop {
                    if cur_r + sp.rspan > limit_r.max(sp.rspan) {
                        cur_r = 0;
                        cur_c += 1;
                    }
                    if occ.is_free(cur_r, cur_c, sp.rspan, sp.cspan) {
                        break;
                    }
                    cur_r += 1;
                }
                occ.mark(cur_r, cur_c, sp.rspan, sp.cspan);
                placed[i] = Some((cur_c, sp.cspan, cur_r, sp.rspan));
                cur_r += sp.rspan;
            }
        }
        let result: Vec<(Bx, usize, usize, usize, usize)> = items.iter().zip(placed).map(|(b, p)| {
            let (c, cs, r, rs) = p.unwrap_or((0, 1, 0, 1));
            (*b, c, cs, r, rs)
        }).collect();
        let max_c = result.iter().map(|x| x.1 + x.2).max().unwrap_or(0).max(explicit_cols);
        let max_r = result.iter().map(|x| x.3 + x.4).max().unwrap_or(0).max(explicit_rows);
        // implicit tracks
        let auto_c = if s.grid_auto_columns.is_empty() { vec![Track::Auto] } else { s.grid_auto_columns.clone() };
        let auto_r = if s.grid_auto_rows.is_empty() { vec![Track::Auto] } else { s.grid_auto_rows.clone() };
        let mut k = 0;
        while cols.len() < max_c {
            cols.push(auto_c[k % auto_c.len()].clone());
            fit_cols.push(false);
            k += 1;
        }
        k = 0;
        while rows.len() < max_r {
            rows.push(auto_r[k % auto_r.len()].clone());
            k += 1;
        }
        (result, Template { cols, rows, auto_fit_cols: fit_cols }, max_c, max_r)
    }

    pub fn grid_intrinsic(&mut self, bx: Bx, s: &'a Style) -> (f32, f32) {
        let (items, _) = self.grid_children(bx);
        let col_gap = s.column_gap.resolve(0.0).unwrap_or(0.0);
        let row_gap = s.row_gap.resolve(0.0).unwrap_or(0.0);
        let (placed, tpl, _, _) = self.grid_place(s, &items, None, None, col_gap, row_gap);
        let mut contribs = Vec::new();
        for (b, c, cs, _, _) in &placed {
            let st = self.style(*b);
            let (m, _, _, _) = self.edges(st, 0.0);
            let (mn, mx) = self.intrinsic(*b);
            contribs.push(Contribution { start: *c, span: *cs, min_c: mn + m.h(), max_c: mx + m.h() });
        }
        let collapsed = vec![false; tpl.cols.len()];
        let gaps = col_gap * tpl.cols.len().saturating_sub(1) as f32;
        let max_sizes = size_tracks(&tpl.cols, &contribs, None, col_gap, true, &collapsed);
        let min_sizes = size_tracks(&tpl.cols, &contribs, None, col_gap, false, &collapsed);
        (min_sizes.iter().sum::<f32>() + gaps, max_sizes.iter().sum::<f32>() + gaps)
    }

    pub fn layout_grid(&mut self, bx: Bx, s: &'a Style, cw: f32, ch_known: Option<f32>) -> (Out, f32) {
        let (item_bx, abs_bx) = self.grid_children(bx);
        let mut result = Out::default();
        for b in abs_bx {
            let fixed = self.style(b).position == Position::Fixed;
            result.abs.push(Pending { bx: b, x: 0.0, y: 0.0, fixed });
        }
        let col_gap = s.column_gap.resolve(cw).unwrap_or(0.0).max(0.0);
        let row_gap = s.row_gap.resolve(ch_known.unwrap_or(0.0)).unwrap_or(0.0).max(0.0);
        let (placed, tpl, ncols, nrows) = self.grid_place(s, &item_bx, Some(cw), ch_known, col_gap, row_gap);
        if placed.is_empty() {
            return (result, ch_known.unwrap_or(0.0));
        }
        // collapse empty auto-fit columns
        let mut collapsed = vec![false; ncols];
        for (c, f) in tpl.auto_fit_cols.iter().enumerate() {
            if *f && c < ncols && !placed.iter().any(|p| c >= p.1 && c < p.1 + p.2) {
                collapsed[c] = true;
            }
        }
        // column contributions
        let mut contribs = Vec::new();
        for (b, c, cs, _, _) in &placed {
            let st = self.style(*b);
            let (m, _, _, _) = self.edges(st, cw);
            let (mn, mx) = self.intrinsic(*b);
            contribs.push(Contribution { start: *c, span: *cs, min_c: mn + m.h(), max_c: mx + m.h() });
        }
        let col_sizes = size_tracks(&tpl.cols, &contribs, Some(cw), col_gap, true, &collapsed);
        let mut col_pos = Vec::with_capacity(ncols);
        let mut x = 0.0f32;
        let mut first = true;
        for (i, w) in col_sizes.iter().enumerate() {
            if !collapsed[i] {
                if !first {
                    x += col_gap;
                }
                first = false;
            }
            col_pos.push(x);
            x += w;
        }
        let tracks_w = x;
        let span_w = |c: usize, cs: usize| -> f32 {
            let end = (c + cs).min(col_sizes.len());
            let mut w: f32 = col_sizes[c..end].iter().sum();
            let visible = (c..end).filter(|&i| !collapsed[i]).count();
            w += col_gap * visible.saturating_sub(1) as f32;
            w
        };
        // lay out items at their column width to size rows
        let mut cells: Vec<Placed> = Vec::new();
        for (b, c, cs, r, rs) in placed {
            let st = self.style(b);
            let (m, _, _, mauto_dummy) = self.edges(st, cw);
            let _ = mauto_dummy;
            let area_w = span_w(c, cs);
            let justify = if st.justify_self == Align::Auto { s.justify_items } else { st.justify_self };
            let replaced = self.is_replaced(b);
            let stretch_w = matches!(justify, Align::Normal | Align::Stretch | Align::Auto) && st.width.is_auto() && !replaced;
            let args = if stretch_w {
                Args { avail_w: cw, cb_h: None, forced_w: Some((area_w - m.l - m.r).max(0.0)), forced_h: None, shrink: false }
            } else {
                Args { avail_w: (area_w - m.l - m.r).max(0.0), cb_h: None, forced_w: None, forced_h: None, shrink: true }
            };
            let out = self.layout_box(b, &args, None, false);
            cells.push(Placed { bx: b, s: st, col: c, cspan: cs, row: r, rspan: rs, out, ml: m.l, mt: m.t });
        }
        let mut row_contribs = Vec::new();
        for cell in &cells {
            let (m, _, _, _) = self.edges(cell.s, cw);
            let h = cell.out.h + m.t + m.b;
            row_contribs.push(Contribution { start: cell.row, span: cell.rspan, min_c: h, max_c: h });
        }
        let row_collapsed = vec![false; nrows];
        let row_sizes = size_tracks(&tpl.rows, &row_contribs, ch_known, row_gap, true, &row_collapsed);
        let mut row_pos = Vec::with_capacity(nrows);
        let mut y = 0.0f32;
        for (i, h) in row_sizes.iter().enumerate() {
            if i > 0 {
                y += row_gap;
            }
            row_pos.push(y);
            y += h;
        }
        let tracks_h = y;
        // content alignment inside the container
        let mut x_off = 0.0;
        let free_w = cw - tracks_w;
        if free_w > 0.0 {
            match s.justify_content {
                Align::Center => x_off = free_w / 2.0,
                Align::End | Align::FlexEnd => x_off = free_w,
                _ => {}
            }
        }
        let mut y_off = 0.0;
        if let Some(h) = ch_known {
            let free_h = h - tracks_h;
            if free_h > 0.0 {
                match s.align_content {
                    Align::Center => y_off = free_h / 2.0,
                    Align::End | Align::FlexEnd => y_off = free_h,
                    _ => {}
                }
            }
        }
        let mut first_baseline = None;
        for cell in cells {
            let st = cell.s;
            let (m, _, _, auto) = self.edges(st, cw);
            let _ = auto;
            let area_w = span_w(cell.col, cell.cspan);
            let end_r = (cell.row + cell.rspan).min(row_sizes.len());
            let area_h: f32 = row_sizes[cell.row..end_r].iter().sum::<f32>() + row_gap * (end_r - cell.row).saturating_sub(1) as f32;
            let ax = col_pos[cell.col] + x_off;
            let ay = row_pos[cell.row] + y_off;
            let align = if st.align_self == Align::Auto { s.align_items } else { st.align_self };
            let justify = if st.justify_self == Align::Auto { s.justify_items } else { st.justify_self };
            let replaced = self.is_replaced(cell.bx);
            let mut out = cell.out;
            // vertical stretch
            if matches!(align, Align::Normal | Align::Stretch | Align::Auto) && st.height.is_auto() && !replaced {
                let target = (area_h - m.t - m.b).max(0.0);
                if (target - out.h).abs() > 0.01 && target > out.h {
                    let stretch_w = matches!(justify, Align::Normal | Align::Stretch | Align::Auto) && st.width.is_auto();
                    let args = Args {
                        avail_w: cw,
                        cb_h: Some(target),
                        forced_w: if stretch_w { Some((area_w - m.l - m.r).max(0.0)) } else { Some(out.w) },
                        forced_h: Some(target),
                        shrink: false,
                    };
                    out = self.layout_box(cell.bx, &args, None, false);
                }
            }
            let dx = match justify {
                Align::Center => ((area_w - out.w - m.l - m.r) / 2.0).max(0.0),
                Align::End | Align::FlexEnd => (area_w - out.w - m.l - m.r).max(0.0),
                _ => 0.0,
            };
            let dy = match align {
                Align::Center => ((area_h - out.h - m.t - m.b) / 2.0).max(0.0),
                Align::End | Align::FlexEnd => (area_h - out.h - m.t - m.b).max(0.0),
                _ => 0.0,
            };
            let (px, py) = (ax + cell.ml + dx, ay + cell.mt + dy);
            if first_baseline.is_none() {
                first_baseline = out.baseline.map(|b| b + py);
            }
            result.absorb(out, px, py);
        }
        result.baseline = first_baseline;
        let content_h = ch_known.unwrap_or(tracks_h);
        let _: Option<String> = None;
        (result, content_h)
    }
}
