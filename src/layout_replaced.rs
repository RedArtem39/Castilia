//! Replaced elements (images, form controls, embeds) and list markers.
use crate::{
    color::{self, Color},
    css::*,
    dom::NodeKind,
    layout::*,
    math,
};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

const TEXT_INPUTS: &[&str] = &["", "text", "search", "email", "url", "tel", "password", "number", "date", "datetime-local", "month", "week", "time"];

pub(crate) fn percent_encode_svg(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len() + 32);
    for c in svg.chars() {
        match c {
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            '"' => out.push_str("'"),
            '\n' | '\r' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn to_roman(mut n: i32, upper: bool) -> String {
    if !(1..4000).contains(&n) {
        return n.to_string();
    }
    let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper { s.to_uppercase() } else { s }
}

fn to_alpha(n: i32, upper: bool) -> String {
    if n < 1 {
        return n.to_string();
    }
    let mut n = n - 1;
    let mut s = Vec::new();
    loop {
        s.push((b'a' + (n % 26) as u8) as char);
        n = n / 26 - 1;
        if n < 0 {
            break;
        }
    }
    s.reverse();
    let t: String = s.into_iter().collect();
    if upper { t.to_uppercase() } else { t }
}

impl<'a> Ctx<'a> {
    pub fn image_src(&self, bx: Bx) -> Option<String> {
        for name in ["src", "data-src", "data-lazy-src", "data-original"] {
            if let Some(v) = self.attr(bx, name)
                && !v.trim().is_empty()
            {
                return Some(v.trim().to_string());
            }
        }
        if let Some(set) = self.attr(bx, "srcset") {
            let first = set.split(',').next()?.split_whitespace().next()?;
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
        None
    }

    fn char_width(&self, s: &Style) -> f32 {
        self.fonts.avg_char_width(&Font::of(s))
    }

    fn control_text(&self, bx: Bx) -> String {
        match self.tag(bx) {
            Some("input") => {
                let ty = self.attr(bx, "type").unwrap_or("text").to_ascii_lowercase();
                match ty.as_str() {
                    "submit" => self.attr(bx, "value").unwrap_or("Submit").to_string(),
                    "reset" => self.attr(bx, "value").unwrap_or("Reset").to_string(),
                    "button" => self.attr(bx, "value").unwrap_or("").to_string(),
                    "password" => {
                        let n = self.attr(bx, "value").map_or(0, |v| v.chars().count());
                        "\u{2022}".repeat(n)
                    }
                    _ => self.attr(bx, "value").unwrap_or("").to_string(),
                }
            }
            _ => String::new(),
        }
    }

    fn selected_option_text(&self, bx: Bx) -> String {
        let doc = self.doc;
        let mut options = Vec::new();
        let mut stack: Vec<usize> = doc.nodes[bx.id].children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if doc.nodes[id].tag() == Some("option") {
                options.push(id);
            } else {
                stack.extend(doc.nodes[id].children.iter().rev().copied());
            }
        }
        let chosen = options.iter().copied().find(|&o| doc.nodes[o].attribute("selected").is_some()).or(options.first().copied());
        chosen.and_then(|o| doc.text_content(o)).map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
    }

    /// Content-box size of a replaced element.
    pub fn replaced_size(&mut self, bx: Bx, s: &Style, a: &Args, hpb: f32, vpb: f32) -> (f32, f32) {
        let tag = self.tag(bx).unwrap_or("");
        let cb_h = a.cb_h;
        let style_w = if a.shrink && s.width.has_percent() { None } else { s.width.resolve(a.avail_w) }.map(|v| if s.border_box { (v - hpb).max(0.0) } else { v.max(0.0) });
        let style_h = match &s.height {
            Dim::Len(_, pct) if *pct != 0.0 && cb_h.is_none() => None,
            d => d.resolve(cb_h.unwrap_or(0.0)).map(|v| if s.border_box { (v - vpb).max(0.0) } else { v.max(0.0) }),
        };
        // intrinsic size and ratio
        let mut natural: Option<(f32, f32)> = None;
        let mut default_box = (300.0, 150.0);
        match tag {
            "img" => {
                if let Some(src) = self.image_src(bx) {
                    natural = self.fonts.image_size(&src);
                }
                default_box = (0.0, 0.0);
                // `width`/`height` attributes already arrive as style; ratio from both attributes
                if natural.is_none()
                    && let (Some(w), Some(h)) = (self.attr(bx, "width").and_then(|v| v.parse::<f32>().ok()), self.attr(bx, "height").and_then(|v| v.parse::<f32>().ok()))
                    && w > 0.0
                    && h > 0.0
                {
                    natural = Some((w, h));
                }
            }
            "svg" => {
                let vb = self.attr(bx, "viewbox").map(|v| v.split(|c: char| c == ',' || c.is_whitespace()).filter(|x| !x.is_empty()).filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>());
                if let Some(v) = vb
                    && v.len() == 4
                    && v[2] > 0.0
                    && v[3] > 0.0
                {
                    natural = Some((v[2], v[3]));
                    default_box = (300.0, 150.0);
                    if style_w.is_none() && style_h.is_none() {
                        // no sizing: behave like a replaced element with the default 300px width
                        default_box = (300.0, 300.0 * v[3] / v[2]);
                    }
                }
            }
            "input" => {
                let ty = self.attr(bx, "type").unwrap_or("text").to_ascii_lowercase();
                let lh = self.used_line_height(s);
                let cw = self.char_width(s);
                if TEXT_INPUTS.contains(&ty.as_str()) {
                    let size = self.attr(bx, "size").and_then(|v| v.parse::<f32>().ok()).filter(|v| *v > 0.0).unwrap_or(20.0);
                    let extra = self.fonts.max_char_width(&Font::of(s)) - cw;
                    natural = Some((math::round32(size * cw + extra), lh));
                } else if matches!(ty.as_str(), "submit" | "button" | "reset") {
                    let t = self.control_text(bx);
                    natural = Some((self.text_w(&t, s), lh));
                } else if ty == "image" {
                    natural = self.image_src(bx).and_then(|src| self.fonts.image_size(&src));
                    default_box = (0.0, 0.0);
                } else if ty == "range" {
                    natural = Some((129.0, 16.0));
                } else if ty == "color" {
                    natural = Some((50.0, 27.0));
                } else if ty == "file" {
                    natural = Some((250.0, lh + 2.0));
                } else if ty == "checkbox" || ty == "radio" {
                    natural = Some((13.0, 13.0));
                }
                default_box = natural.unwrap_or((0.0, lh));
            }
            "select" => {
                let lh = self.used_line_height(s);
                let mut maxw = 0.0f32;
                let doc = self.doc;
                let mut stack: Vec<usize> = doc.nodes[bx.id].children.clone();
                let mut count = 0;
                while let Some(id) = stack.pop() {
                    if doc.nodes[id].tag() == Some("option") {
                        count += 1;
                        let t = doc.text_content(id).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                        maxw = maxw.max(self.text_w(&t, s));
                    } else {
                        stack.extend(doc.nodes[id].children.iter().copied());
                    }
                }
                let rows = self.attr(bx, "size").and_then(|v| v.parse::<f32>().ok()).filter(|v| *v > 1.0);
                let multiple = self.attr(bx, "multiple").is_some();
                let rows = rows.or(if multiple { Some(4.0) } else { None });
                let _ = count;
                natural = Some((maxw + 4.0 + 20.0 * (rows.is_none() as u8 as f32), lh * rows.unwrap_or(1.0) + 2.0 * (rows.is_none() as u8 as f32)));
                default_box = natural.unwrap_or((0.0, lh));
            }
            "textarea" => {
                let cols = self.attr(bx, "cols").and_then(|v| v.parse::<f32>().ok()).filter(|v| *v > 0.0).unwrap_or(20.0);
                let rows = self.attr(bx, "rows").and_then(|v| v.parse::<f32>().ok()).filter(|v| *v > 0.0).unwrap_or(2.0);
                let lh = self.used_line_height(s);
                natural = Some((cols * self.char_width(s) + 15.0, rows * lh));
                default_box = natural.unwrap_or((0.0, 0.0));
            }
            "progress" | "meter" => {
                natural = Some((160.0, 16.0));
            }
            "audio" => {
                natural = Some((300.0, 54.0));
            }
            _ => {}
        }
        let is_control = matches!(tag, "input" | "select" | "textarea" | "progress" | "meter")
            && !(tag == "input" && self.attr(bx, "type").is_some_and(|t| t.eq_ignore_ascii_case("image")));
        let ratio = if is_control { None } else { natural.filter(|(w, h)| *w > 0.0 && *h > 0.0).map(|(w, h)| w / h).or(s.aspect_ratio) };
        let (mut w, mut h) = match (style_w, style_h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => match (ratio, natural) {
                (Some(r), _) => (w, w / r),
                (None, Some((_, nh))) => (w, nh),
                _ => (w, default_box.1),
            },
            (None, Some(h)) => match (ratio, natural) {
                (Some(r), _) => (h * r, h),
                (None, Some((nw, _))) => (nw, h),
                _ => (default_box.0, h),
            },
            (None, None) => natural.unwrap_or(default_box),
        };
        // min/max constraints keep the aspect ratio when the other axis is auto
        let maxw = s.max_width.resolve(a.avail_w).map(|v| if s.border_box { (v - hpb).max(0.0) } else { v });
        let minw = s.min_width.resolve(a.avail_w).map(|v| if s.border_box { (v - hpb).max(0.0) } else { v });
        if let Some(m) = maxw
            && w > m
        {
            if style_h.is_none() && w > 0.0 {
                h = h * m / w;
            }
            w = m;
        }
        if let Some(m) = minw
            && w < m
        {
            if style_h.is_none() && w > 0.0 {
                h = h * m / w;
            }
            w = m;
        }
        let cbh = cb_h.unwrap_or(f32::INFINITY);
        let maxh = match &s.max_height {
            Dim::Len(_, pct) if *pct != 0.0 && cb_h.is_none() => None,
            d => d.resolve(if cbh.is_finite() { cbh } else { 0.0 }).map(|v| if s.border_box { (v - vpb).max(0.0) } else { v }),
        };
        if let Some(m) = maxh
            && h > m
        {
            if style_w.is_none() && h > 0.0 {
                w = w * m / h;
            }
            h = m;
        }
        (w.max(0.0), h.max(0.0))
    }

    /// Paint the content of a replaced element into a content-box-sized area.
    pub fn paint_replaced(&mut self, bx: Bx, s: &Style, cw: f32, ch: f32) -> Out {
        let mut out = Out::default();
        let rect = Rect { x: 0.0, y: 0.0, width: cw, height: ch };
        let tag = self.tag(bx).unwrap_or("");
        let link = self.link_of(bx);
        let doc = self.doc;
        let mut text = |this: &mut Self, paint: &mut Paint, x: f32, y: f32, t: String, color: Color| {
            let w = this.text_w(&t, s);
            paint.cmds.push(Command::Text {
                x,
                y,
                text: t,
                size: s.font_size,
                color,
                link: None,
                weight: s.font_weight,
                italic: s.font_italic,
                family: s.font_family,
                decoration: 0,
                width: w,
                letter_spacing: 0.0,
                blur: 0.0,
            });
        };
        match tag {
            "img" => {
                match self.image_src(bx) {
                    Some(src) if cw > 0.0 && ch > 0.0 => {
                        let natural = if s.object_fit != 0 { self.fonts.image_size(&src).filter(|n| n.0 > 0.0 && n.1 > 0.0) } else { None };
                        match natural {
                            Some((nw, nh)) => {
                                let contain = (cw / nw).min(ch / nh);
                                let scale = match s.object_fit {
                                    1 => contain,
                                    2 => (cw / nw).max(ch / nh),
                                    3 => 1.0,
                                    _ => contain.min(1.0),
                                };
                                let (w, h) = (nw * scale, nh * scale);
                                let x = s.object_position.0.resolve_offset(cw - w);
                                let y = s.object_position.1.resolve_offset(ch - h);
                                let draw = Rect { x, y, width: w, height: h };
                                let overflow = x < -0.01 || y < -0.01 || x + w > cw + 0.01 || y + h > ch + 0.01;
                                if overflow {
                                    out.paint.cmds.push(Command::PushClip { rect, radius: 0.0 });
                                }
                                out.paint.cmds.push(Command::Image { rect: draw, src, link: link.clone(), opacity: 1.0 });
                                if overflow {
                                    out.paint.cmds.push(Command::PopClip);
                                }
                            }
                            None => out.paint.cmds.push(Command::Image { rect, src, link: link.clone(), opacity: 1.0 }),
                        }
                    }
                    None => {
                        if let Some(alt) = self.attr(bx, "alt")
                            && !alt.is_empty()
                        {
                            text(self, &mut out.paint, 0.0, 0.0, alt.to_string(), s.color);
                        }
                    }
                    _ => {}
                }
            }
            "input" if matches!(self.attr(bx, "type").map(|t| t.to_ascii_lowercase()).as_deref(), Some("image")) => {
                if let Some(src) = self.image_src(bx) {
                    out.paint.cmds.push(Command::Image { rect, src, link: link.clone(), opacity: 1.0 });
                }
            }
            "svg" => {
                let mut markup = crate::html::serialize_outer(doc, bx.id);
                if !markup.contains("xmlns=") {
                    markup = markup.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1);
                }
                let hex = format!("#{:06x}", s.color & 0xffffff);
                markup = markup.replace("currentColor", &hex).replace("currentcolor", &hex);
                // make sure the size is explicit so the host scales it to the box
                if !markup.contains(" width=") {
                    markup = markup.replacen("<svg", &format!("<svg width=\"{cw}\" height=\"{ch}\""), 1);
                }
                let src = format!("data:image/svg+xml;utf8,{}", percent_encode_svg(&markup));
                out.paint.cmds.push(Command::Image { rect, src, link: link.clone(), opacity: 1.0 });
            }
            "input" => {
                let ty = self.attr(bx, "type").unwrap_or("text").to_ascii_lowercase();
                let lh = self.used_line_height(s);
                let ty = ty.as_str();
                if (ty == "checkbox" || ty == "radio") && s.appearance_none {
                } else if ty == "checkbox" || ty == "radio" {
                    out.baseline = Some(ch);
                    let checked = self.attr(bx, "checked").is_some();
                    let round = if ty == "radio" { cw / 2.0 } else { 2.0 };
                    let (fill, edge) = if checked { (0xff0075ffu32, 0xff0075ffu32) } else { (0xffffffff, 0xff767676) };
                    out.paint.cmds.push(Command::FillRect { rect, color: fill, radius: round });
                    out.paint.cmds.push(Command::Border { rect, widths: [1.0; 4], colors: [edge; 4], styles: [1; 4], radius: round });
                    if checked {
                        if ty == "radio" {
                            let r = Rect { x: cw * 0.3, y: ch * 0.3, width: cw * 0.4, height: ch * 0.4 };
                            out.paint.cmds.push(Command::FillRect { rect: r, color: 0xffffffff, radius: cw * 0.2 });
                        } else {
                            out.paint.cmds.push(Command::Text {
                                x: cw * 0.2,
                                y: (ch - lh * 0.8) / 2.0 - 1.0,
                                text: "\u{2713}".into(),
                                size: s.font_size * 0.9,
                                color: 0xffffffff,
                                link: None,
                                weight: 700,
                                italic: false,
                                family: s.font_family,
                                decoration: 0,
                                width: cw * 0.6,
                                letter_spacing: 0.0, blur: 0.0, });
                        }
                    }
                } else if ty == "range" {
                    out.paint.cmds.push(Command::FillRect { rect: Rect { x: 0.0, y: ch / 2.0 - 2.0, width: cw, height: 4.0 }, color: 0xffd0d0d0, radius: 2.0 });
                    let v = self.attr(bx, "value").and_then(|v| v.parse::<f32>().ok()).unwrap_or(50.0);
                    let (mn, mx) = (self.attr(bx, "min").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0), self.attr(bx, "max").and_then(|v| v.parse::<f32>().ok()).unwrap_or(100.0));
                    let t = if mx > mn { ((v - mn) / (mx - mn)).clamp(0.0, 1.0) } else { 0.5 };
                    out.paint.cmds.push(Command::FillRect { rect: Rect { x: t * (cw - 16.0), y: 0.0, width: 16.0, height: 16.0 }, color: 0xff0075ff, radius: 8.0 });
                } else if ty == "color" {
                    let c = self.attr(bx, "value").and_then(color::parse_color).unwrap_or(0xff000000);
                    out.paint.cmds.push(Command::FillRect { rect: Rect { x: 4.0, y: 4.0, width: (cw - 8.0).max(0.0), height: (ch - 8.0).max(0.0) }, color: c, radius: 0.0 });
                } else {
                    let value = self.control_text(bx);
                    let placeholder = self.attr(bx, "placeholder").unwrap_or("");
                    let button_like = matches!(ty, "submit" | "button" | "reset");
                    let (shown, c) = if value.is_empty() && !button_like { (placeholder.to_string(), 0xff757575u32) } else { (value, s.color) };
                    {
                        // text controls align on the baseline of their text
                        let asc = self.fonts.ascent(&Font::of(s));
                        out.baseline = Some(((ch - lh) / 2.0).max(0.0) + asc);
                    }
                    if !shown.is_empty() {
                        out.paint.cmds.push(Command::PushClip { rect, radius: 0.0 });
                        let w = self.text_w(&shown, s);
                        let x = if button_like { ((cw - w) / 2.0).max(0.0) } else { 0.0 };
                        text(self, &mut out.paint, x, ((ch - lh) / 2.0).max(0.0), shown, c);
                        out.paint.cmds.push(Command::PopClip);
                    }
                }
            }
            "textarea" => {
                let content = doc.text_content(bx.id).unwrap_or_default();
                let placeholder = self.attr(bx, "placeholder").unwrap_or("").to_string();
                let lh = self.used_line_height(s);
                out.paint.cmds.push(Command::PushClip { rect, radius: 0.0 });
                let (body, c) = if content.is_empty() { (placeholder, 0xff757575u32) } else { (content, s.color) };
                for (i, line) in body.trim_start_matches('\n').split('\n').enumerate() {
                    if !line.is_empty() {
                        text(self, &mut out.paint, 0.0, i as f32 * lh, line.trim_end_matches('\r').to_string(), c);
                    }
                }
                out.paint.cmds.push(Command::PopClip);
            }
            "select" => {
                let lh = self.used_line_height(s);
                let t = self.selected_option_text(bx);
                out.baseline = Some(((ch - lh) / 2.0).max(0.0) + self.fonts.ascent(&Font::of(s)));
                out.paint.cmds.push(Command::PushClip { rect, radius: 0.0 });
                if !t.is_empty() {
                    text(self, &mut out.paint, 2.0, ((ch - lh) / 2.0).max(0.0), t, s.color);
                }
                out.paint.cmds.push(Command::PopClip);
                if self.attr(bx, "multiple").is_none() && self.attr(bx, "size").is_none() && !s.appearance_none {
                    text(self, &mut out.paint, cw - 14.0, ((ch - lh) / 2.0).max(0.0), "\u{25be}".into(), s.color);
                }
            }
            "progress" | "meter" => {
                let v = self.attr(bx, "value").and_then(|v| v.parse::<f32>().ok());
                let max = self.attr(bx, "max").and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0).max(0.0001);
                if let Some(v) = v {
                    let frac = (v / max).clamp(0.0, 1.0);
                    out.paint.cmds.push(Command::FillRect { rect: Rect { x: 0.0, y: 0.0, width: cw * frac, height: ch }, color: if tag == "meter" { 0xff10a010 } else { 0xff0075ff }, radius: 0.0 });
                }
            }
            _ => {}
        }
        out
    }

    // -----------------------------------------------------------------------
    // List markers
    // -----------------------------------------------------------------------

    fn list_index(&self, bx: Bx) -> i32 {
        let doc = self.doc;
        let Some(parent) = doc.nodes[bx.id].parent else { return 1 };
        let p = &doc.nodes[parent];
        let reversed = p.attribute("reversed").is_some() && p.tag() == Some("ol");
        let items: Vec<usize> = p
            .children
            .iter()
            .copied()
            .filter(|&c| doc.nodes[c].tag().is_some() && self.st.nodes[c].display == Display::ListItem)
            .collect();
        let start = p.attribute("start").and_then(|v| v.trim().parse::<i32>().ok());
        let (mut counter, step) = if reversed { (start.unwrap_or(items.len() as i32), -1) } else { (start.unwrap_or(1), 1) };
        for id in items {
            if let Some(v) = doc.nodes[id].attribute("value").and_then(|v| v.trim().parse::<i32>().ok()) {
                counter = v;
            }
            if id == bx.id {
                return counter;
            }
            counter += step;
        }
        counter
    }

    pub fn marker_text(&self, bx: Bx, s: &Style) -> Option<String> {
        use ListStyleType as L;
        let n = || self.list_index(bx);
        Some(match s.list_style_type {
            L::None => return None,
            L::Disc => "\u{2022}".to_string(),
            L::Circle => "\u{25e6}".to_string(),
            L::Square => "\u{25aa}".to_string(),
            L::Decimal => format!("{}.", n()),
            L::DecimalLeadingZero => format!("{:02}.", n()),
            L::LowerAlpha => format!("{}.", to_alpha(n(), false)),
            L::UpperAlpha => format!("{}.", to_alpha(n(), true)),
            L::LowerRoman => format!("{}.", to_roman(n(), false)),
            L::UpperRoman => format!("{}.", to_roman(n(), true)),
            L::DisclosureClosed => "\u{25b6}".to_string(),
            L::DisclosureOpen => "\u{25bc}".to_string(),
            L::LowerGreek => {
                let i = (n() - 1).rem_euclid(24) as u32;
                format!("{}.", char::from_u32(0x3b1 + i + (i >= 17) as u32).unwrap_or('α'))
            }
        })
    }

    pub fn add_list_marker(&mut self, bx: Bx, s: &Style, paint: &mut Paint, first_baseline: Option<f32>) {
        if s.list_style_inside {
            return;
        }
        let Some(marker) = self.marker_text(bx, s) else { return };
        let font = Font::of(s);
        let w = self.text_w(&format!("{marker} "), s);
        let mw = self.text_w(&marker, s);
        let asc = self.fonts.ascent(&font);
        let (asc_m, desc_m, lh) = self.vmetrics(s);
        let ch = asc_m + desc_m;
        let y = match first_baseline {
            Some(b) => b - asc,
            None => (lh - ch) / 2.0,
        };
        let _ = w;
        // The marker box ends one space before the content edge.
        let space = self.text_w(" ", s);
        paint.cmds.push(Command::Text {
            x: -(mw + space),
            y,
            text: marker,
            size: s.font_size,
            color: s.color,
            link: None,
            weight: s.font_weight,
            italic: s.font_italic,
            family: s.font_family,
            decoration: 0,
            width: mw,
            letter_spacing: 0.0, blur: 0.0, });
    }

    #[allow(dead_code)]
    pub fn is_text_node(&self, bx: Bx) -> bool {
        matches!(self.doc.nodes[bx.id].kind, NodeKind::Text(_))
    }
}
