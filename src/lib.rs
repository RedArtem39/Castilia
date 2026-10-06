#![no_std]

extern crate alloc;

pub mod css;
pub mod dom;
mod entities;
pub mod html;
pub mod js;
pub mod layout;
pub mod network;
pub mod page;

pub use dom::{Document, Node, NodeId, NodeKind};
pub use layout::{Command, DisplayList, FontMetrics, Rect};
pub use page::Page;

/// Parse a page and produce platform-independent drawing commands.
/// The caller owns resource loading, glyph measurement, painting and input.
pub fn render(
    html_source: &str,
    external_css: &str,
    viewport_width: f32,
    fonts: &impl FontMetrics,
) -> DisplayList {
    Page::new(html_source, external_css).render(viewport_width, fonts)
}

fn embedded_css(document: &Document, external_css: &str) -> alloc::string::String {
    let mut css_source = alloc::string::String::new();
    for (id, node) in document.nodes.iter().enumerate() {
        if document.is_connected(id) && node.tag() == Some("style") {
            for &child in &node.children {
                if let NodeKind::Text(text) = &document.nodes[child].kind {
                    css_source.push_str(text);
                    css_source.push('\n');
                }
            }
        }
    }
    css_source.push_str(external_css);
    css_source
}

fn render_document(
    document: &Document,
    css_source: &str,
    viewport_width: f32,
    fonts: &impl FontMetrics,
) -> DisplayList {
    let rules = css::parse_with_viewport(css_source, viewport_width);
    let styles = css::compute_styles(document, &rules);
    layout::layout(document, &styles, viewport_width, fonts)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mono;
    impl FontMetrics for Mono {
        fn text_width(&self, text: &str, size: f32) -> f32 {
            text.chars().count() as f32 * size * 0.5
        }
        fn line_height(&self, size: f32) -> f32 {
            size * 1.25
        }
    }

    #[test]
    fn embedded_css_reaches_the_display_list() {
        let list = render(
            "<style>.note { color: #123456 }</style><p class='note'>Привет</p>",
            "",
            320.0,
            &Mono,
        );
        assert!(list.commands.iter().any(|command| matches!(command, Command::Text { text, color: 0x123456, .. } if text == "Привет")));
    }
}
