use crate::{
    css::{self, StyleSheet, Viewport},
    dom::{Document, NodeId, NodeKind},
    embedded_css, html,
    js::{Host, Prop, Runtime},
    layout::{DisplayList, FontMetrics},
    render_document,
};
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cell::RefCell;

const MAX_SCRIPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCRIPTS: usize = 256;
/// Virtual milliseconds of timers run after load so that `setTimeout`-driven UIs settle.
const SETTLE_MS: f64 = 1500.0;
const SETTLE_FIRES: usize = 3000;

#[derive(Debug, Default)]
pub struct ScriptReport {
    pub executed: usize,
    pub external_loaded: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Optional inputs for loading a page.
#[derive(Default)]
pub struct PageOptions {
    /// Document URL (`location.href`, base for relative URLs in scripts).
    pub url: Option<String>,
    /// Services for `fetch`, `XMLHttpRequest`, dynamic scripts, navigation and dialogs.
    pub host: Option<Box<dyn Host>>,
    /// Viewport used for `innerWidth`/`matchMedia` while scripts run.
    pub viewport: Option<(f32, f32)>,
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
    layout_rects: RefCell<Option<BTreeMap<usize, [f32; 4]>>>,
}

fn event_class(name: &str) -> (&'static str, bool, bool) {
    // (class, bubbles, cancelable)
    match name {
        "click" | "dblclick" | "mousedown" | "mouseup" | "contextmenu" | "auxclick" => ("MouseEvent", true, true),
        "mousemove" | "mouseover" | "mouseout" => ("MouseEvent", true, true),
        "mouseenter" | "mouseleave" => ("MouseEvent", false, false),
        "pointerdown" | "pointerup" | "pointermove" | "pointerover" | "pointerout" | "pointercancel" => ("PointerEvent", true, true),
        "keydown" | "keyup" | "keypress" => ("KeyboardEvent", true, true),
        "input" => ("InputEvent", true, false),
        "beforeinput" => ("InputEvent", true, true),
        "focus" | "blur" => ("FocusEvent", false, false),
        "focusin" | "focusout" => ("FocusEvent", true, false),
        "submit" => ("SubmitEvent", true, true),
        "wheel" => ("WheelEvent", true, true),
        "touchstart" | "touchend" | "touchmove" => ("TouchEvent", true, true),
        "load" | "error" | "scroll" | "toggle" | "unload" => ("Event", false, false),
        _ => ("Event", true, false),
    }
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
    pub fn from_document_with_loader(document: Document, external_css: &str, load_script: impl FnMut(&str) -> Result<String, String>) -> Self {
        Self::load(document, external_css, PageOptions::default(), load_script)
    }

    pub fn load(document: Document, external_css: &str, options: PageOptions, mut load_script: impl FnMut(&str) -> Result<String, String>) -> Self {
        let mut page = Self {
            document,
            external_css: external_css.to_string(),
            link_css: BTreeMap::new(),
            sheet_cache: RefCell::new(None),
            runtime: Runtime::new(),
            scripts: ScriptReport::default(),
            layout_rects: RefCell::new(None),
        };
        if let Some(url) = &options.url {
            page.runtime.set_url(url);
        }
        if let Some(h) = options.host {
            page.runtime.set_host(h);
        }
        if let Some((w, h)) = options.viewport {
            page.runtime.set_viewport(w, h);
        }
        let scripts: Vec<usize> = page
            .document
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.tag() == Some("script")).then_some(id))
            .collect();
        page.scripts.skipped += scripts.len().saturating_sub(MAX_SCRIPTS);
        // import maps apply to every module regardless of position
        for &id in &scripts {
            let n = &page.document.nodes[id];
            if n.attribute("type").is_some_and(|t| t.trim().eq_ignore_ascii_case("importmap")) {
                let json = page.document.text_content(id).unwrap_or_default();
                page.runtime.add_import_map(&json, &mut page.document);
            }
        }
        let mut modules: Vec<(usize, String, String)> = Vec::new();
        for id in scripts.into_iter().take(MAX_SCRIPTS) {
            if !page.document.is_connected(id) {
                continue;
            }
            let node = &page.document.nodes[id];
            let kind = node.attribute("type").map(|k| k.trim().to_ascii_lowercase()).unwrap_or_default();
            if !matches!(kind.as_str(), "" | "text/javascript" | "application/javascript" | "text/ecmascript" | "application/ecmascript" | "module") || node.attribute("nomodule").is_some() {
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
                page.document.text_content(id).unwrap_or_default()
            };
            if source.len() > MAX_SCRIPT_BYTES {
                page.scripts.errors.push(format!("script node {id}: size limit exceeded"));
                continue;
            }
            if source.trim().is_empty() {
                continue;
            }
            if kind == "module" {
                let base = page.runtime.url().to_string();
                let url = match page.document.nodes[id].attribute("src") {
                    Some(src) => crate::js::url::resolve(&base, src).unwrap_or_else(|| src.to_string()),
                    None => format!("{base}#inline-module-{id}"),
                };
                modules.push((id, url, source));
                continue;
            }
            match page.runtime.run_script(&source, Some(id), &mut page.document) {
                Ok(()) => page.scripts.executed += 1,
                Err(error) => page.scripts.errors.push(format!("script node {id}: {error}")),
            }
        }
        // module scripts are deferred: they run after parsing, in document order
        for (id, url, source) in modules {
            match page.runtime.run_module(&url, Some(&source), &mut page.document) {
                Ok(()) => page.scripts.executed += 1,
                Err(error) => page.scripts.errors.push(format!("module script node {id} ({url}): {error}")),
            }
        }
        page.runtime.dom_content_loaded(&mut page.document);
        page.runtime.advance(&mut page.document, SETTLE_MS, SETTLE_FIRES);
        page.runtime.load(&mut page.document);
        page.runtime.advance(&mut page.document, 100.0, 500);
        page
    }

    pub fn set_host(&mut self, host: Box<dyn Host>) {
        self.runtime.set_host(host);
    }

    /// The current document URL as seen by scripts (`location.href`).
    pub fn url(&self) -> &str {
        self.runtime.url()
    }

    /// Console output and uncaught errors collected so far.
    pub fn console(&self) -> Vec<String> {
        self.runtime.logs()
    }

    /// Evaluate a snippet in the page (debugging / automation).
    pub fn evaluate(&mut self, source: &str) -> Result<String, String> {
        self.sync_layout();
        self.runtime.eval_in_page(&mut self.document, source)
    }

    /// Navigation requested by scripts or by activating a link/form since the last call.
    pub fn take_navigation(&mut self) -> Option<String> {
        self.runtime.take_navigation()
    }

    /// True when scripts changed the DOM since the last call (a re-render is needed).
    pub fn take_dirty(&mut self) -> bool {
        self.runtime.take_dirty()
    }

    fn sync_layout(&mut self) {
        if let Some(r) = self.layout_rects.borrow().as_ref() {
            self.runtime.set_layout(r.clone());
        }
    }

    /// Advance the virtual clock, running due timers/animation frames. Returns whether the DOM changed.
    pub fn advance_time(&mut self, ms: f64) -> bool {
        self.sync_layout();
        self.runtime.advance(&mut self.document, ms, 2000);
        self.runtime.take_dirty()
    }

    pub fn has_pending_timers(&self) -> bool {
        self.runtime.has_pending_timers()
    }

    pub fn dispatch(&mut self, node: NodeId, event: &str) -> Result<usize, String> {
        if !self.document.is_connected(node) {
            return Err(String::from("event target is not connected"));
        }
        self.sync_layout();
        let (class, bubbles, cancelable) = event_class(event);
        self.runtime.dispatch_event(&mut self.document, node, event, class, bubbles, cancelable, &[])
    }

    /// Full pointer interaction at page coordinates: mousedown, mouseup, click, with focus handling.
    pub fn click_at(&mut self, display: &DisplayList, x: f32, y: f32) -> Result<Option<NodeId>, String> {
        let Some(node) = display.hit_test(x, y) else {
            return Ok(None);
        };
        self.sync_layout();
        let pos = [("clientX", Prop::Num(x as f64)), ("clientY", Prop::Num(y as f64)), ("pageX", Prop::Num(x as f64)), ("pageY", Prop::Num(y as f64)), ("screenX", Prop::Num(x as f64)), ("screenY", Prop::Num(y as f64)), ("button", Prop::Num(0.0)), ("detail", Prop::Num(1.0))];
        for (ty, class) in [("pointerdown", "PointerEvent"), ("mousedown", "MouseEvent")] {
            self.runtime.dispatch_event(&mut self.document, node, ty, class, true, true, &pos)?;
        }
        // focus the nearest focusable ancestor (or clear focus)
        let mut focus = None;
        let mut cur = Some(node);
        while let Some(id) = cur {
            let n = &self.document.nodes[id];
            let focusable = matches!(n.tag(), Some("input" | "textarea" | "select" | "button")) && n.attribute("disabled").is_none()
                || (n.tag() == Some("a") && n.attribute("href").is_some())
                || n.attribute("tabindex").is_some()
                || n.attribute("contenteditable").is_some_and(|v| v != "false");
            if focusable {
                focus = Some(id);
                break;
            }
            cur = n.parent;
        }
        self.runtime.focus(&mut self.document, focus);
        for (ty, class) in [("pointerup", "PointerEvent"), ("mouseup", "MouseEvent")] {
            self.runtime.dispatch_event(&mut self.document, node, ty, class, true, true, &pos)?;
        }
        self.runtime.dispatch_event(&mut self.document, node, "click", "MouseEvent", true, true, &pos)?;
        Ok(Some(node))
    }

    /// Currently focused element, if any.
    pub fn focused(&self) -> Option<NodeId> {
        self.runtime.focused()
    }

    pub fn focus(&mut self, node: Option<NodeId>) {
        self.runtime.focus(&mut self.document, node);
    }

    /// Type text into the focused element (keydown/keypress/input/keyup per character).
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        for ch in text.chars() {
            let target = self.runtime.focused().unwrap_or(self.document.body);
            let key = ch.to_string();
            let code = if ch.is_ascii_alphabetic() { format!("Key{}", ch.to_ascii_uppercase()) } else if ch.is_ascii_digit() { format!("Digit{ch}") } else if ch == ' ' { String::from("Space") } else { String::new() };
            let props = [("key", Prop::Str(&key)), ("code", Prop::Str(&code)), ("keyCode", Prop::Num(ch.to_ascii_uppercase() as u32 as f64)), ("which", Prop::Num(ch.to_ascii_uppercase() as u32 as f64)), ("charCode", Prop::Num(ch as u32 as f64))];
            let n_down = self.runtime.dispatch_event(&mut self.document, target, "keydown", "KeyboardEvent", true, true, &props)?;
            let _ = n_down;
            self.runtime.dispatch_event(&mut self.document, target, "keypress", "KeyboardEvent", true, true, &props)?;
            self.runtime.insert_text(&mut self.document, target, &key);
            self.runtime.dispatch_event(&mut self.document, target, "keyup", "KeyboardEvent", true, true, &props)?;
        }
        Ok(())
    }

    /// Press a named key ("Enter", "Backspace", "Tab", "Escape", "ArrowDown", ...) on the focused element.
    pub fn press_key(&mut self, key: &str) -> Result<(), String> {
        let target = self.runtime.focused().unwrap_or(self.document.body);
        let code = match key {
            "Enter" => 13.0,
            "Backspace" => 8.0,
            "Tab" => 9.0,
            "Escape" => 27.0,
            " " => 32.0,
            "ArrowLeft" => 37.0,
            "ArrowUp" => 38.0,
            "ArrowRight" => 39.0,
            "ArrowDown" => 40.0,
            "Delete" => 46.0,
            _ => 0.0,
        };
        let props = [("key", Prop::Str(key)), ("code", Prop::Str(key)), ("keyCode", Prop::Num(code)), ("which", Prop::Num(code))];
        self.sync_layout();
        self.runtime.dispatch_event(&mut self.document, target, "keydown", "KeyboardEvent", true, true, &props)?;
        let prevented = self.runtime.last_event_prevented();
        if !prevented {
            self.runtime.key_default(&mut self.document, target, key);
        }
        self.runtime.dispatch_event(&mut self.document, target, "keyup", "KeyboardEvent", true, true, &props)?;
        Ok(())
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
        let list = render_document(&self.document, &sheet, viewport, fonts);
        *self.layout_rects.borrow_mut() = Some(Runtime::rects_from_hits(&self.document, &list.hit_regions));
        list
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
