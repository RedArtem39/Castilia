use crate::{
    css::{Display, Style},
    dom::{Document, NodeId, NodeKind},
};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    FillRect {
        rect: Rect,
        color: u32,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        color: u32,
        link: Option<String>,
    },
    Image {
        rect: Rect,
        src: String,
        link: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
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
        self.hit_regions
            .iter()
            .rev()
            .find(|region| {
                x >= region.rect.x
                    && y >= region.rect.y
                    && x < region.rect.x + region.rect.width
                    && y < region.rect.y + region.rect.height
            })
            .map(|region| region.node)
    }
}

/// All coordinates use CSS pixels. `Text.y` is the top of the line box.
pub trait FontMetrics {
    fn text_width(&self, text: &str, size: f32) -> f32;
    fn line_height(&self, size: f32) -> f32;
}

enum InlineItem {
    Text(String, f32, u32, Option<String>, NodeId),
    Image(String, f32, f32, Option<String>, NodeId),
    Break,
    BlockBreak,
}

pub fn layout(
    doc: &Document,
    styles: &[Style],
    viewport_width: f32,
    fonts: &impl FontMetrics,
) -> DisplayList {
    let width = viewport_width.max(0.0);
    let mut commands = Vec::new();
    let mut hit_regions = Vec::new();
    let height = layout_block(
        doc,
        styles,
        doc.body,
        0.0,
        0.0,
        width,
        fonts,
        &mut commands,
        &mut hit_regions,
    );
    DisplayList {
        width,
        height,
        commands,
        hit_regions,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_block(
    doc: &Document,
    styles: &[Style],
    id: NodeId,
    x: f32,
    y: f32,
    available: f32,
    fonts: &impl FontMetrics,
    commands: &mut Vec<Command>,
    hit_regions: &mut Vec<HitRegion>,
) -> f32 {
    let style = &styles[id];
    if style.display == Display::None {
        return 0.0;
    }
    let box_x = x + style.margin.left;
    let box_y = y + style.margin.top;
    let region_index = hit_regions.len();
    hit_regions.push(HitRegion {
        rect: Rect {
            x: box_x,
            y: box_y,
            width: 0.0,
            height: 0.0,
        },
        node: id,
    });
    let outer_width = style
        .width
        .map(|width| width.resolve(available) + style.padding.left + style.padding.right)
        .unwrap_or((available - style.margin.left - style.margin.right).max(0.0));
    let content_x = box_x + style.padding.left;
    let content_width = (outer_width - style.padding.left - style.padding.right).max(0.0);
    let mut cursor_y = box_y + style.padding.top;
    let background_index = commands.len();
    if style.display == Display::Flex {
        let items: Vec<_> = doc.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&child| matches!(styles[child].display, Display::Block | Display::Flex))
            .collect();
        let gaps = style.gap * items.len().saturating_sub(1) as f32;
        let fixed: f32 = items
            .iter()
            .filter_map(|&child| {
                let child_style = &styles[child];
                child_style.width.map(|width| {
                    width.resolve(content_width)
                        + child_style.padding.left
                        + child_style.padding.right
                        + child_style.margin.left
                        + child_style.margin.right
                })
            })
            .sum();
        let auto_count = items
            .iter()
            .filter(|&&child| styles[child].width.is_none())
            .count();
        let auto_width = if auto_count > 0 {
            ((content_width - fixed - gaps) / auto_count as f32).max(0.0)
        } else {
            0.0
        };
        let mut cursor_x = content_x;
        let mut row_height: f32 = 0.0;
        for child in items {
            let child_style = &styles[child];
            let slot = child_style
                .width
                .map(|width| {
                    width.resolve(content_width)
                        + child_style.padding.left
                        + child_style.padding.right
                        + child_style.margin.left
                        + child_style.margin.right
                })
                .unwrap_or(auto_width);
            let available = if child_style.width.is_some() {
                content_width
            } else {
                slot
            };
            row_height = row_height.max(layout_block(
                doc,
                styles,
                child,
                cursor_x,
                cursor_y,
                available,
                fonts,
                commands,
                hit_regions,
            ));
            cursor_x += slot + style.gap;
        }
        cursor_y += row_height;
        let mut inline = Vec::new();
        for &child in &doc.nodes[id].children {
            if styles[child].display == Display::Inline {
                collect_inline(doc, styles, child, None, &mut inline);
            }
        }
        cursor_y += layout_inline(
            &inline,
            content_x,
            cursor_y,
            content_width,
            fonts,
            commands,
            hit_regions,
        );
    } else {
        let mut inline = Vec::new();
        let mut previous_bottom_margin: f32 = 0.0;
        for &child in &doc.nodes[id].children {
            if styles[child].display == Display::None {
                continue;
            }
            if matches!(styles[child].display, Display::Block | Display::Flex) {
                let inline_height = layout_inline(
                    &inline,
                    content_x,
                    cursor_y,
                    content_width,
                    fonts,
                    commands,
                    hit_regions,
                );
                cursor_y += inline_height;
                inline.clear();
                if inline_height > 0.0 {
                    previous_bottom_margin = 0.0;
                }
                cursor_y -= previous_bottom_margin
                    .min(styles[child].margin.top)
                    .max(0.0);
                cursor_y += layout_block(
                    doc,
                    styles,
                    child,
                    content_x,
                    cursor_y,
                    content_width,
                    fonts,
                    commands,
                    hit_regions,
                );
                previous_bottom_margin = styles[child].margin.bottom;
            } else {
                collect_inline(doc, styles, child, None, &mut inline);
            }
        }
        cursor_y += layout_inline(
            &inline,
            content_x,
            cursor_y,
            content_width,
            fonts,
            commands,
            hit_regions,
        );
    }
    let content_height = (cursor_y - box_y - style.padding.top).max(
        style
            .height
            .map(|height| height.resolve(available))
            .unwrap_or(0.0),
    );
    let box_height = style.padding.top + content_height + style.padding.bottom;
    hit_regions[region_index].rect = Rect {
        x: box_x,
        y: box_y,
        width: outer_width,
        height: box_height,
    };
    if let Some(color) = style.background {
        commands.insert(
            background_index,
            Command::FillRect {
                rect: Rect {
                    x: box_x,
                    y: box_y,
                    width: outer_width,
                    height: box_height,
                },
                color,
            },
        );
    }
    style.margin.top + box_height + style.margin.bottom
}

fn collect_inline(
    doc: &Document,
    styles: &[Style],
    id: NodeId,
    link: Option<&str>,
    output: &mut Vec<InlineItem>,
) {
    let node = &doc.nodes[id];
    let style = &styles[id];
    if style.display == Display::None {
        return;
    }
    let link = if node.tag() == Some("a") {
        node.attribute("href").or(link)
    } else {
        link
    };
    let hit_node = if node.tag().is_some() {
        id
    } else {
        node.parent.unwrap_or(id)
    };
    let nested_block = matches!(style.display, Display::Block | Display::Flex);
    if nested_block {
        output.push(InlineItem::BlockBreak);
    }
    match &node.kind {
        NodeKind::Text(text) => output.push(InlineItem::Text(
            text.clone(),
            style.font_size,
            style.color,
            link.map(str::to_string),
            hit_node,
        )),
        NodeKind::Element { tag, .. } if tag == "br" => output.push(InlineItem::Break),
        NodeKind::Element { tag, .. } if tag == "img" => {
            if let Some(src) = node.attribute("src") {
                let width = style
                    .width
                    .map(|n| n.resolve(300.0))
                    .or_else(|| node.attribute("width").and_then(|n| n.parse().ok()))
                    .unwrap_or(300.0);
                let height = style
                    .height
                    .map(|n| n.resolve(150.0))
                    .or_else(|| node.attribute("height").and_then(|n| n.parse().ok()))
                    .unwrap_or(150.0);
                output.push(InlineItem::Image(
                    src.to_string(),
                    width.max(0.0),
                    height.max(0.0),
                    link.map(str::to_string),
                    id,
                ));
            }
        }
        NodeKind::Element { tag, .. } if tag == "td" || tag == "th" => {
            for &child in &node.children {
                collect_inline(doc, styles, child, link, output);
            }
            output.push(InlineItem::Text(
                String::from(" "),
                style.font_size,
                style.color,
                link.map(str::to_string),
                id,
            ));
        }
        _ => {
            for &child in &node.children {
                collect_inline(doc, styles, child, link, output);
            }
        }
    }
    if nested_block {
        output.push(InlineItem::BlockBreak);
    }
}

fn layout_inline(
    items: &[InlineItem],
    x: f32,
    y: f32,
    width: f32,
    fonts: &impl FontMetrics,
    commands: &mut Vec<Command>,
    hit_regions: &mut Vec<HitRegion>,
) -> f32 {
    let mut cursor_x = x;
    let mut cursor_y = y;
    let mut line_height: f32 = 0.0;
    let mut pending_space = false;
    let mut has_content = false;
    for item in items {
        match item {
            InlineItem::BlockBreak => {
                if has_content {
                    cursor_y += line_height;
                    cursor_x = x;
                    line_height = 0.0;
                    pending_space = false;
                    has_content = false;
                }
            }
            InlineItem::Break => {
                cursor_y += if has_content {
                    line_height
                } else {
                    fonts.line_height(16.0)
                };
                cursor_x = x;
                line_height = 0.0;
                pending_space = false;
                has_content = false;
            }
            InlineItem::Image(src, image_width, image_height, link, node) => {
                if has_content && cursor_x + image_width > x + width {
                    cursor_y += line_height;
                    cursor_x = x;
                    line_height = 0.0;
                }
                commands.push(Command::Image {
                    rect: Rect {
                        x: cursor_x,
                        y: cursor_y,
                        width: *image_width,
                        height: *image_height,
                    },
                    src: src.clone(),
                    link: link.clone(),
                });
                hit_regions.push(HitRegion {
                    rect: Rect {
                        x: cursor_x,
                        y: cursor_y,
                        width: *image_width,
                        height: *image_height,
                    },
                    node: *node,
                });
                cursor_x += image_width;
                line_height = line_height.max(*image_height);
                has_content = true;
                pending_space = false;
            }
            InlineItem::Text(text, size, color, link, node) => {
                let mut offset = 0;
                while offset < text.len() {
                    let character = text[offset..].chars().next().unwrap();
                    if character.is_whitespace() {
                        pending_space = has_content;
                        offset += character.len_utf8();
                        continue;
                    }
                    let start = offset;
                    while offset < text.len() {
                        let character = text[offset..].chars().next().unwrap();
                        if character.is_whitespace() {
                            break;
                        }
                        offset += character.len_utf8();
                    }
                    let word = &text[start..offset];
                    let space_width = if pending_space && has_content {
                        fonts.text_width(" ", *size)
                    } else {
                        0.0
                    };
                    let word_width = fonts.text_width(word, *size);
                    if has_content && cursor_x + space_width + word_width > x + width {
                        cursor_y += line_height;
                        cursor_x = x;
                        line_height = 0.0;
                        has_content = false;
                    }
                    if pending_space && has_content {
                        cursor_x += space_width;
                    }
                    commands.push(Command::Text {
                        x: cursor_x,
                        y: cursor_y,
                        text: word.to_string(),
                        size: *size,
                        color: *color,
                        link: link.clone(),
                    });
                    hit_regions.push(HitRegion {
                        rect: Rect {
                            x: cursor_x,
                            y: cursor_y,
                            width: word_width,
                            height: fonts.line_height(*size),
                        },
                        node: *node,
                    });
                    cursor_x += word_width;
                    line_height = line_height.max(fonts.line_height(*size));
                    has_content = true;
                    pending_space = false;
                }
            }
        }
    }
    cursor_y - y + if has_content { line_height } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{css, html};
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
    fn wraps_text_and_carries_link_target() {
        let doc = html::parse("<p><a href='/next'>one two three</a></p>");
        let styles = css::compute_styles(&doc, &[]);
        let list = layout(&doc, &styles, 45.0, &Mono);
        let words: Vec<_> = list
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::Text { text, y, link, .. } => Some((text.as_str(), *y, link.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(words.len(), 3);
        assert!(words[1].1 > words[0].1);
        assert_eq!(words[0].2, Some("/next"));
    }

    #[test]
    fn adjacent_inline_nodes_do_not_gain_spaces() {
        let doc = html::parse("<p>ab<b>cd</b> ef</p>");
        let styles = css::compute_styles(&doc, &[]);
        let list = layout(&doc, &styles, 200.0, &Mono);
        let text: Vec<_> = list
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::Text { x, text, .. } => Some((*x, text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(text, [(8.0, "ab"), (24.0, "cd"), (48.0, "ef")]);
    }

    #[test]
    fn table_rows_are_readable_as_separate_lines() {
        let doc =
            html::parse("<table><tr><td>one</td><td>two</td></tr><tr><td>three</td></tr></table>");
        let styles = css::compute_styles(&doc, &[]);
        let list = layout(&doc, &styles, 300.0, &Mono);
        let y = |word| {
            list.commands
                .iter()
                .find_map(|command| match command {
                    Command::Text { text, y, .. } if text == word => Some(*y),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(y("one"), y("two"));
        assert!(y("three") > y("one"));
    }

    #[test]
    fn nested_table_rows_keep_line_breaks() {
        let doc = html::parse(
            "<table><tr><td><table><tr><td>first</td></tr><tr><td>second</td></tr></table></td></tr></table>",
        );
        let styles = css::compute_styles(&doc, &[]);
        let list = layout(&doc, &styles, 300.0, &Mono);
        let y = |word| {
            list.commands
                .iter()
                .find_map(|command| match command {
                    Command::Text { text, y, .. } if text == word => Some(*y),
                    _ => None,
                })
                .unwrap()
        };
        assert!(y("second") > y("first"));
    }
}
