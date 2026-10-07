//! Castilia: a small, portable browser engine core.
//!
//! The crate is `#![no_std]` (plus `alloc`) and has no required dependencies. The host
//! supplies font metrics, networking and painting; the engine parses HTML, applies CSS,
//! runs scripts, lays the page out and emits a platform-independent [`DisplayList`].
#![no_std]

extern crate alloc;

pub mod color;
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
mod presentation;
pub mod selector;
pub mod shadow;

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
                for &child in &node.children {
                    if let NodeKind::Text(t) = &document.nodes[child].kind {
                        text.push_str(t);
                    }
                }
                let disabled = node.attribute("type").is_some_and(|t| !t.is_empty() && !t.eq_ignore_ascii_case("text/css"));
                if !disabled {
                    wrap(&mut css, &text, node.attribute("media"));
                }
            }
            Some("link") => {
                if let Some(text) = link_css.get(&id) {
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

    #[test]
    fn embedded_css_reaches_the_display_list() {
        let list = render("<style>.note { color: #123456 }</style><p class='note'>Привет</p>", "", 320.0, &Mono);
        assert!(list.commands.iter().any(|c| matches!(c, Command::Text { text, color: 0xff123456, .. } if text == "Привет")));
    }
}
