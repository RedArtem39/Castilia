//! Castilia: a small, portable browser engine core.
//!
//! The crate is `#![no_std]` (plus `alloc`) and has no required dependencies. The host
//! supplies font metrics, networking and painting; the engine parses HTML, applies CSS,
//! runs scripts, lays the page out and emits a platform-independent [`DisplayList`].
#![no_std]

extern crate alloc;

pub mod color;
pub mod config;
pub mod css;
pub mod cssload;
mod css_values;
pub mod dom;
pub mod encoding;
mod entities;
pub mod html;
pub mod image;
pub mod js;
pub mod layout;
mod layout_flex;
mod layout_grid;
mod layout_inline;
mod layout_replaced;
mod layout_table;
pub mod math;
pub mod network;
pub mod page;
pub mod platform;
mod presentation;
pub mod selector;
pub mod shadow;

pub use config::{EngineConfig, ExecMode};
pub use css::Viewport;
pub use dom::{Document, Node, NodeId, NodeKind};
pub use layout::{Command, DisplayList, Font, FontMetrics, HitRegion, Rect};
pub use page::Page;

/// Parse a page and produce platform-independent drawing commands.
/// The caller owns resource loading, glyph measurement, painting and input.
pub fn render(html_source: &str, external_css: &str, viewport_width: f32, fonts: &impl FontMetrics) -> DisplayList {
    Page::new(html_source, external_css).render(viewport_width, fonts)
}

/// Collect `<style>` contents (and loaded `<link>` sheets) in document order, then the
/// legacy `external` stylesheet text.
pub(crate) fn embedded_css(document: &Document, link_css: &alloc::collections::BTreeMap<NodeId, alloc::string::String>, external_css: &str) -> alloc::string::String {
    use alloc::string::String;
    let mut css = String::new();
    let mut stack = alloc::vec![document.root];
    while let Some(id) = stack.pop() {
        let node = &document.nodes[id];
        let wrap = |css: &mut String, text: &str, media: Option<&str>| match media.map(str::trim) {
            Some(m) if !m.is_empty() && !m.eq_ignore_ascii_case("all") => {
                css.push_str("@media ");
                css.push_str(m);
                css.push_str(" {");
                css.push_str(text);
                css.push_str("\n}\n");
            }
            _ => {
                css.push_str(text);
                css.push('\n');
            }
        };
        match node.tag() {
            Some("style") => {
                let mut text = String::new();
                if let Some(o) = document.sheet_override.get(&id) {
                    text.push_str(o);
                } else {
                    for &child in &node.children {
                        if let NodeKind::Text(t) = &document.nodes[child].kind {
                            text.push_str(t);
                        }
                    }
                }
                let disabled = node.attribute("type").is_some_and(|t| !t.is_empty() && !t.eq_ignore_ascii_case("text/css"));
                if !disabled {
                    wrap(&mut css, &text, node.attribute("media"));
                }
            }
            Some("link") => {
                if let Some(text) = document.sheet_override.get(&id).or_else(|| link_css.get(&id)) {
                    wrap(&mut css, text, node.attribute("media"));
                }
            }
            _ => {}
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    css.push_str(external_css);
    css
}

pub(crate) fn render_document(document: &Document, sheet: &css::StyleSheet, viewport: Viewport, fonts: &dyn FontMetrics) -> DisplayList {
    let styles = css::compute_styles(document, sheet, viewport);
    let first = layout::layout(document, &styles, viewport, fonts);
    if !sheet.has_container {
        return first;
    }
    // Container queries depend on laid-out sizes: restyle with the sizes from the first pass.
    let sizes: alloc::collections::BTreeMap<NodeId, (f32, f32)> = first.container_sizes.iter().map(|&(id, w, h)| (id, (w, h))).collect();
    let styles = css::compute_styles_with(document, sheet, viewport, &sizes);
    let second = layout::layout(document, &styles, viewport, fonts);
    // a third pass settles nested containers whose size depends on outer query results
    let sizes: alloc::collections::BTreeMap<NodeId, (f32, f32)> = second.container_sizes.iter().map(|&(id, w, h)| (id, (w, h))).collect();
    let styles = css::compute_styles_with(document, sheet, viewport, &sizes);
    layout::layout(document, &styles, viewport, fonts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Command;

    pub(crate) struct Mono;
    impl FontMetrics for Mono {
        fn text_width(&self, text: &str, size: f32) -> f32 {
            text.chars().count() as f32 * size * 0.5
        }
        fn line_height(&self, size: f32) -> f32 {
            size * 1.25
        }
        fn text_width_font(&self, text: &str, font: &Font) -> f32 {
            self.text_width(text, font.size)
        }
    }

    fn texts(list: &DisplayList) -> alloc::vec::Vec<(alloc::string::String, f32, f32, u32)> {
        list.commands.iter().filter_map(|c| match c {
            Command::Text { text, x, y, color, .. } => Some((text.clone(), *x, *y, *color)),
            _ => None,
        }).collect()
    }

    #[test]
    fn nested_fixed_media_or_and_placeholder() {
        let html = "<style>.w{position:fixed;left:0;right:0}@media (min-width:600px) or (max-width:10px){b{color:#ff0000}}.x::placeholder{opacity:0}</style>
            <div class=w><span style='position:fixed;top:50px'>INNER</span></div><b>OR</b><input class=x placeholder=PH>";
        let t = texts(&render(html, "", 800.0, &Mono));
        assert!(t.iter().any(|(s, _, y, _)| s == "INNER" && (*y - 50.0).abs() < 20.0), "{t:?}");
        assert!(t.iter().any(|(s, _, _, c)| s == "OR" && *c == 0xffff0000), "{t:?}");
        assert!(t.iter().any(|(s, _, _, c)| s == "PH" && *c >> 24 == 0), "{t:?}");
    }

    #[test]
    fn grid_track_sizing_matches_browsers() {
        let x_of = |html: &str, word: &str| texts(&render(html, "", 1000.0, &Mono)).into_iter().find(|t| t.0 == word).map(|t| t.1).unwrap();
        // minmax(0, 300px) grows to its limit; justify-content: center then places it
        let html = "<style>body{margin:0}.g{display:grid;grid-template-columns:minmax(0,300px) minmax(0,100px);justify-content:center}</style><div class=g><div>AA</div><div>BB</div></div>";
        assert_eq!(x_of(html, "AA"), 300.0);
        assert_eq!(x_of(html, "BB"), 600.0);
        // overflowing tracks with justify-content: center spill out on both sides (unsafe alignment)
        let html = "<style>body{margin:0}.g{display:grid;grid-template-columns:200px 1000px 200px;justify-content:center}</style><div class=g><div>L</div><div>MID</div><div>R</div></div>";
        assert_eq!(x_of(html, "MID"), 0.0);
        // a shrink-to-fit grid of minmax(0,1fr) tracks is as wide as the content needs
        let html = "<style>body{margin:0}.f{display:flex;flex-direction:column;align-items:center}.g{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));column-gap:10px}.s{grid-column:span 4}</style><div class=f><div class=g><div class=s>ABCDEFGHIJ</div></div></div>";
        // 10 chars * 8px = 80px wide, centred in 1000px
        assert_eq!(x_of(html, "ABCDEFGHIJ"), 460.0);
    }

    #[test]
    fn text_wrap_balance_evens_lines() {
        let html = "<style>body{margin:0}h1{width:400px;font-size:16px;text-wrap:balance}</style><h1>aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk</h1>";
        let t = texts(&render(html, "", 1000.0, &Mono));
        let lines: alloc::vec::Vec<_> = t.iter().map(|t| t.0.chars().count()).collect();
        assert_eq!(lines.len(), 2, "{t:?}");
        assert!(lines[0].abs_diff(lines[1]) <= 5, "{t:?}");
    }

    #[test]
    fn embedded_css_reaches_the_display_list() {
        let list = render("<style>.note { color: #123456 }</style><p class='note'>Привет</p>", "", 320.0, &Mono);
        assert!(list.commands.iter().any(|c| matches!(c, Command::Text { text, color: 0xff123456, .. } if text == "Привет")));
    }
}

