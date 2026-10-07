use crate::{
    css::{self, StyleSheet, Viewport},
    dom::{Document, NodeId, NodeKind},
    embedded_css, html,
    js::Runtime,
    layout::{DisplayList, FontMetrics},
    render_document,
};
use alloc::{
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::RefCell;

const MAX_SCRIPT_BYTES: usize = 256 * 1024;
const MAX_SCRIPTS: usize = 32;

#[derive(Debug, Default)]
pub struct ScriptReport {
    pub executed: usize,
    pub external_loaded: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Document and JS state live together so DOM changes can be rendered again.
#[derive(Debug)]
pub struct Page {
    pub document: Document,
    external_css: String,
    link_css: BTreeMap<NodeId, String>,
    sheet_cache: RefCell<Option<(String, Rc<StyleSheet>)>>,
    runtime: Runtime,
    pub scripts: ScriptReport,
}

impl Page {
    pub fn new(html_source: &str, external_css: &str) -> Self {
        Self::from_document(html::parse(html_source), external_css)
    }

    pub fn from_document(document: Document, external_css: &str) -> Self {
        Self::from_document_with_loader(document, external_css, |_| Err(String::from("no external script loader")))
    }

    /// Provide the text of a `<link rel=stylesheet>` so it is applied in document order.
    pub fn set_link_stylesheet(&mut self, link: NodeId, css: String) {
        self.link_css.insert(link, css);
    }

    /// Supported classic scripts run in document order after HTML parsing.
    /// `load_script` receives the external `src` attribute as written.
    pub fn from_document_with_loader(document: Document, external_css: &str, mut load_script: impl FnMut(&str) -> Result<String, String>) -> Self {
        let mut page = Self {
            document,
            external_css: external_css.to_string(),
            link_css: BTreeMap::new(),
            sheet_cache: RefCell::new(None),
            runtime: Runtime::new(),
            scripts: ScriptReport::default(),
        };
        let scripts: Vec<usize> = page
            .document
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.tag() == Some("script")).then_some(id))
            .collect();
        page.scripts.skipped += scripts.len().saturating_sub(MAX_SCRIPTS);
        for id in scripts.into_iter().take(MAX_SCRIPTS) {
            if !page.document.is_connected(id) {
                continue;
            }
            let node = &page.document.nodes[id];
            if node.attribute("type").is_some_and(|kind| {
                !matches!(kind.trim().to_ascii_lowercase().as_str(), "" | "text/javascript" | "application/javascript")
            }) || node.attribute("async").is_some()
            {
                page.scripts.skipped += 1;
                continue;
            }
            let source = if let Some(src) = node.attribute("src") {
                match load_script(src) {
                    Ok(source) => {
                        page.scripts.external_loaded += 1;
                        source
                    }
                    Err(error) => {
                        page.scripts.errors.push(format!("script node {id} ({src}): {error}"));
                        continue;
                    }
                }
            } else {
                node.children
                    .first()
                    .and_then(|&child| match &page.document.nodes[child].kind {
                        NodeKind::Text(source) => Some(source.clone()),
                        _ => None,
                    })
                    .unwrap_or_default()
            };
            if source.len() > MAX_SCRIPT_BYTES {
                page.scripts.errors.push(format!("script node {id}: size limit exceeded"));
                continue;
            }
            if source.trim().is_empty() {
                continue;
            }
            match page.runtime.execute(&source, &mut page.document) {
                Ok(()) => page.scripts.executed += 1,
                Err(error) => page.scripts.errors.push(format!("script node {id}: {error}")),
            }
        }
        page
    }

    pub fn dispatch(&mut self, node: NodeId, event: &str) -> Result<usize, String> {
        if !self.document.is_connected(node) {
            return Err(String::from("event target is not connected"));
        }
        let mut current = Some(node);
        let mut path = Vec::new();
        while let Some(id) = current {
            path.push(id);
            current = self.document.nodes[id].parent;
        }
        self.runtime.begin_event();
        let mut count = 0;
        for id in path {
            count += self.runtime.dispatch_to(id, node, event, &mut self.document)?;
        }
        Ok(count)
    }

    pub fn click_at(&mut self, display: &DisplayList, x: f32, y: f32) -> Result<Option<NodeId>, String> {
        let Some(node) = display.hit_test(x, y) else {
            return Ok(None);
        };
        self.dispatch(node, "click")?;
        Ok(Some(node))
    }

    /// The `href` of the nearest enclosing link of `node`, if any.
    pub fn link_of(&self, node: NodeId) -> Option<String> {
        let mut cur = Some(node);
        while let Some(id) = cur {
            let n = &self.document.nodes[id];
            if n.tag() == Some("a")
                && let Some(h) = n.attribute("href")
            {
                return Some(h.to_string());
            }
            cur = n.parent;
        }
        None
    }

    fn stylesheet(&self) -> Rc<StyleSheet> {
        let css = embedded_css(&self.document, &self.link_css, &self.external_css);
        let mut cache = self.sheet_cache.borrow_mut();
        if let Some((text, sheet)) = cache.as_ref()
            && *text == css
        {
            return sheet.clone();
        }
        let sheet = Rc::new(css::parse(&css));
        *cache = Some((css, sheet.clone()));
        sheet
    }

    /// Render with the given viewport width and a 768px tall viewport.
    pub fn render(&self, viewport_width: f32, fonts: &impl FontMetrics) -> DisplayList {
        self.render_viewport(Viewport::new(viewport_width, 768.0), fonts)
    }

    pub fn render_viewport(&self, viewport: Viewport, fonts: &impl FontMetrics) -> DisplayList {
        let sheet = self.stylesheet();
        render_document(&self.document, &sheet, viewport, fonts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, tests::Mono};

    #[test]
    fn inline_script_and_click_reflow_page() {
        let mut page = Page::new(
            "<p id='message'>before</p><button id='go'>Go</button><script>const message = document.getElementById('message'); message.textContent = 'ready'; document.getElementById('go').addEventListener('click', () => { message.textContent = 'clicked'; });</script>",
            "",
        );
        assert_eq!(page.scripts.executed, 1);
        let before = page.render(300.0, &Mono);
        assert!(before.commands.iter().any(|c| matches!(c, Command::Text { text, .. } if text == "ready")));
        let go = page.document.get_element_by_id("go").unwrap();
        assert_eq!(page.dispatch(go, "click").unwrap(), 1);
        let after = page.render(300.0, &Mono);
        assert!(after.commands.iter().any(|c| matches!(c, Command::Text { text, .. } if text == "clicked")));
    }

    #[test]
    fn script_creates_element_and_layout_includes_it() {
        let page = Page::new(
            "<script>const note = document.createElement('p'); note.textContent = 'created'; document.body.appendChild(note);</script>",
            "",
        );
        assert_eq!(page.scripts.executed, 1);
        assert!(page.render(300.0, &Mono).commands.iter().any(|c| matches!(c, Command::Text { text, .. } if text == "created")));
    }

    #[test]
    fn click_coordinates_find_element_and_bubble_event() {
        let mut page = Page::new(
            "<div id='outer'><button id='go'>Go</button></div><p id='result'>old</p><script>document.getElementById('outer').addEventListener('click', (e) => { document.getElementById('result').textContent = e.target.id + ' ' + e.currentTarget.id; });</script>",
            "",
        );
        let list = page.render(300.0, &Mono);
        let (x, y) = list
            .commands
            .iter()
            .find_map(|c| match c {
                Command::Text { x, y, text, .. } if text == "Go" => Some((*x + 1.0, *y + 1.0)),
                _ => None,
            })
            .unwrap();
        let go = page.document.get_element_by_id("go").unwrap();
        assert_eq!(page.click_at(&list, x, y).unwrap(), Some(go));
        assert_eq!(
            page.document.text_content(page.document.get_element_by_id("result").unwrap()).as_deref(),
            Some("go outer")
        );
        assert_eq!(page.click_at(&list, -1.0, -1.0).unwrap(), None);
    }
}
