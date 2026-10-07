//! Turns a `DisplayList` into an SVG document (host-only helper used by the examples).
use castilia::{Command, DisplayList, Font, FontMetrics, Rect, css::FontFamily};
use std::fmt::Write;

/// Pixel-snap a box the way browsers do before painting.
fn snap(r: &Rect) -> Rect {
    let (x0, y0, x1, y1) = (r.x.round(), r.y.round(), (r.x + r.width).round(), (r.y + r.height).round());
    Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
}

fn escape(input: &str) -> String {
    input.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn rgb(c: u32) -> String {
    format!("#{:06x}", c & 0xffffff)
}

fn opacity(c: u32) -> f32 {
    (c >> 24) as f32 / 255.0
}

pub struct Resource<'a> {
    pub url: &'a str,
    pub mime: &'a str,
    pub bytes: &'a [u8],
}

pub fn to_svg(list: &DisplayList, fonts: &dyn FontMetrics, resources: &[Resource], viewport_h: f32) -> String {
    let height = list.height.max(viewport_h).max(1.0);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        list.width, height, list.width, height
    );
    let bg = if list.background >> 24 > 0 { rgb(list.background) } else { "white".into() };
    let _ = writeln!(out, "<rect width=\"100%\" height=\"100%\" fill=\"{bg}\"/>");
    let mut defs = 0usize;
    let mut clip_depth = 0usize;
    for command in &list.commands {
        match command {
            Command::FillRect { rect, color, radius } => {
                let rect = &snap(rect);
                let r = radius.min(rect.width / 2.0).min(rect.height / 2.0);
                let _ = writeln!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{r}\" fill=\"{}\" fill-opacity=\"{}\"/>",
                    rect.x, rect.y, rect.width, rect.height, rgb(*color), opacity(*color)
                );
            }
            Command::Border { rect, widths, colors, styles, radius } => border(&mut out, &snap(rect), &widths.map(|w| if w > 0.0 { w.round().max(1.0) } else { 0.0 }), colors, styles, *radius),
            Command::Text { x, y, text, size, color, link, weight, italic, family, decoration, width, letter_spacing, blur } => {
                let font = Font { size: *size, weight: *weight, italic: *italic, family: *family };
                let asc = fonts.ascent(&font);
                let fam = match family {
                    FontFamily::Serif => "Liberation Serif, serif",
                    FontFamily::Sans => "Liberation Sans, sans-serif",
                    FontFamily::System => "Inter, sans-serif",
                    FontFamily::Monospace => "DejaVu Sans Mono, monospace",
                };
                if let Some(l) = link {
                    let _ = write!(out, "<a href=\"{}\">", escape(l));
                }
                let mut deco = String::new();
                if decoration & 1 != 0 {
                    deco.push_str(" underline");
                }
                if decoration & 2 != 0 {
                    deco.push_str(" line-through");
                }
                if decoration & 4 != 0 {
                    deco.push_str(" overline");
                }
                let filter = if *blur > 0.0 {
                    defs += 1;
                    let _ = writeln!(out, "<filter id=\"b{defs}\" x=\"-20%\" y=\"-50%\" width=\"140%\" height=\"200%\"><feGaussianBlur stdDeviation=\"{}\"/></filter>", blur / 2.0);
                    format!(" filter=\"url(#b{defs})\"")
                } else {
                    String::new()
                };
                let _ = write!(
                    out,
                    "<text{filter} x=\"{x}\" y=\"{}\" letter-spacing=\"{letter_spacing}\" font-family=\"{fam}\" font-size=\"{size}\" font-weight=\"{weight}\" font-style=\"{}\" fill=\"{}\" fill-opacity=\"{}\" xml:space=\"preserve\"{}{}>{}</text>",
                    (y + asc).round(),
                    if *italic { "italic" } else { "normal" },
                    rgb(*color),
                    opacity(*color),
                    if deco.is_empty() { String::new() } else { format!(" text-decoration=\"{}\"", deco.trim()) },
                    if *width > 0.0 { String::new() } else { String::new() },
                    escape(text)
                );
                if link.is_some() {
                    out.push_str("</a>");
                }
                out.push('\n');
            }
            Command::Image { rect, src, link, opacity } => {
                let href = resources
                    .iter()
                    .find(|r| r.url == src)
                    .map(|r| format!("data:{};base64,{}", r.mime, base64_encode(r.bytes)))
                    .unwrap_or_else(|| src.clone());
                if let Some(l) = link {
                    let _ = write!(out, "<a href=\"{}\">", escape(l));
                }
                let _ = write!(
                    out,
                    "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" opacity=\"{opacity}\" href=\"{}\"/>",
                    rect.x, rect.y, rect.width, rect.height, escape(&href)
                );
                if link.is_some() {
                    out.push_str("</a>");
                }
                out.push('\n');
            }
            Command::Gradient { rect, angle, radial, stops, radius } => {
                defs += 1;
                let id = format!("g{defs}");
                let cx = rect.x + rect.width / 2.0;
                let cy = rect.y + rect.height / 2.0;
                if *radial {
                    let _ = write!(out, "<defs><radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{cx}\" cy=\"{cy}\" r=\"{}\">", rect.width.max(rect.height) / 2.0);
                } else {
                    let a = angle.to_radians();
                    let (sx, sy) = (a.sin(), -a.cos());
                    let half = (rect.width * sx.abs() + rect.height * sy.abs()) / 2.0;
                    let _ = write!(
                        out,
                        "<defs><linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">",
                        cx - sx * half, cy - sy * half, cx + sx * half, cy + sy * half
                    );
                }
                for (pos, c) in stops {
                    let _ = write!(out, "<stop offset=\"{pos}\" stop-color=\"{}\" stop-opacity=\"{}\"/>", rgb(*c), opacity(*c));
                }
                let _ = writeln!(out, "{}</defs>", if *radial { "</radialGradient>" } else { "</linearGradient>" });
                let _ = writeln!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\" fill=\"url(#{id})\"/>",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Command::Shadow { rect, radius, dx, dy, blur, spread, color, inset } => {
                if *inset {
                    continue;
                }
                defs += 1;
                let id = format!("s{defs}");
                let _ = writeln!(
                    out,
                    "<defs><filter id=\"{id}\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\"><feGaussianBlur stdDeviation=\"{}\"/></filter></defs><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\" fill=\"{}\" fill-opacity=\"{}\" filter=\"url(#{id})\"/>",
                    blur / 2.0,
                    rect.x + dx - spread,
                    rect.y + dy - spread,
                    rect.width + spread * 2.0,
                    rect.height + spread * 2.0,
                    rgb(*color),
                    opacity(*color)
                );
            }
            Command::PushClip { rect, radius } => {
                defs += 1;
                clip_depth += 1;
                let _ = writeln!(
                    out,
                    "<clipPath id=\"c{defs}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\"/></clipPath><g clip-path=\"url(#c{defs})\">",
                    rect.x, rect.y, rect.width, rect.height
                );
            }
            Command::PushTransform { m } => {
                clip_depth += 1;
                let _ = writeln!(out, "<g transform=\"matrix({} {} {} {} {} {})\">", m[0], m[1], m[2], m[3], m[4], m[5]);
            }
            Command::PopTransform | Command::PopClip => {
                if clip_depth > 0 {
                    clip_depth -= 1;
                    out.push_str("</g>\n");
                }
            }
        }
    }
    for _ in 0..clip_depth {
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}

fn border(out: &mut String, r: &Rect, w: &[f32; 4], colors: &[u32; 4], styles: &[u8; 4], radius: f32) {
    let uniform = w.iter().all(|x| *x == w[0]) && colors.iter().all(|c| *c == colors[0]) && styles.iter().all(|s| *s == styles[0]) && w[0] > 0.0;
    if uniform {
        let dash = match styles[0] {
            2 => format!(" stroke-dasharray=\"{} {}\"", w[0] * 3.0, w[0] * 3.0),
            3 => format!(" stroke-dasharray=\"{} {}\" stroke-linecap=\"round\"", 0.01, w[0] * 2.0),
            _ => String::new(),
        };
        let rr = (radius - w[0] / 2.0).max(0.0).min(r.width / 2.0).min(r.height / 2.0);
        let _ = writeln!(
            out,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{rr}\" fill=\"none\" stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"{}\"{dash}/>",
            r.x + w[0] / 2.0,
            r.y + w[0] / 2.0,
            (r.width - w[0]).max(0.0),
            (r.height - w[0]).max(0.0),
            rgb(colors[0]),
            opacity(colors[0]),
            w[0]
        );
        return;
    }
    // per-side trapezoids with mitered corners
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
    let side = |out: &mut String, i: usize, pts: [(f32, f32); 4]| {
        if w[i] > 0.0 && styles[i] != 0 {
            let p: Vec<String> = pts.iter().map(|(x, y)| format!("{x},{y}")).collect();
            let _ = writeln!(out, "<polygon points=\"{}\" fill=\"{}\" fill-opacity=\"{}\"/>", p.join(" "), rgb(colors[i]), opacity(colors[i]));
        }
    };
    side(out, 0, [(x0, y0), (x1, y0), (x1 - w[1], y0 + w[0]), (x0 + w[3], y0 + w[0])]);
    side(out, 1, [(x1, y0), (x1, y1), (x1 - w[1], y1 - w[2]), (x1 - w[1], y0 + w[0])]);
    side(out, 2, [(x0, y1), (x1, y1), (x1 - w[1], y1 - w[2]), (x0 + w[3], y1 - w[2])]);
    side(out, 3, [(x0, y0), (x0, y1), (x0 + w[3], y1 - w[2]), (x0 + w[3], y0 + w[0])]);
}

pub fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
