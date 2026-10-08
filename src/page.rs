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
    /// Base for resolving relative URLs in scripts (`<base href>`); defaults to `url`.
    pub document_base: Option<String>,
    /// Text of `<link rel=stylesheet>` elements, applied in document order.
    pub link_css: BTreeMap<NodeId, String>,
    /// Execution mode and limits.
    pub config: crate::config::EngineConfig,
    /// Executable memory for the native tier (only used in `ExecMode::Jit`).
    pub code_memory: Option<Box<dyn crate::platform::CodeMemory>>,
    /// Host stop signal for runaway scripts.
    pub interrupt: Option<Box<dyn crate::platform::Interrupt>>,
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
    image_sizes: BTreeMap<String, (f32, f32)>,
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
            runtime: Runtime::with_config(&options.config),
            scripts: ScriptReport::default(),
            layout_rects: RefCell::new(None),
            image_sizes: BTreeMap::new(),
        };
        if let Some(mem) = options.code_memory {
            page.runtime.set_code_memory(mem);
        }
        if let Some(i) = options.interrupt {
            page.runtime.set_interrupt(i);
        }
        let config = options.config.clone();
        if let Some(url) = &options.url {
            page.runtime.set_url(url);
        }
        if let Some(h) = options.host {
            page.runtime.set_host(h);
        }
        if let Some((w, h)) = options.viewport {
            page.runtime.set_viewport(w, h);
        }
        if let Some(b) = &options.document_base {
            page.runtime.set_document_base(b);
        }
        page.link_css = options.link_css;
        page.runtime.set_style_sources(&page.link_css, &page.external_css);
        let scripts: Vec<usize> = page
            .document
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.tag() == Some("script")).then_some(id))
            .collect();
        page.scripts.skipped += scripts.len().saturating_sub(config.max_scripts);
        // import maps apply to every module regardless of position
        for &id in &scripts {
            let n = &page.document.nodes[id];
            if n.attribute("type").is_some_and(|t| t.trim().eq_ignore_ascii_case("importmap")) {
                let json = page.document.text_content(id).unwrap_or_default();
                page.runtime.add_import_map(&json, &mut page.document);
            }
        }
        // deferred scripts run after parsing, in document order: modules and classic `defer` ones
        // (url is None for classic scripts)
        let mut modules: Vec<(usize, Option<String>, String)> = Vec::new();
        // the list holds bare node ids: keep those nodes from being collected and reused
        page.runtime.interp.pinned_nodes = scripts.clone();
        for id in scripts.into_iter().take(config.max_scripts) {
            // ids of collected nodes are reused: make sure this is still the script we listed
            if !page.document.is_connected(id) || page.document.nodes[id].tag() != Some("script") {
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
            if source.len() > config.max_script_bytes {
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
                modules.push((id, Some(url), source));
                continue;
            }
            let node = &page.document.nodes[id];
            let external = node.attribute("src").is_some();
            if external && node.attribute("defer").is_some() && node.attribute("async").is_none() {
                modules.push((id, None, source));
                continue;
            }
            match page.runtime.run_script(&source, Some(id), &mut page.document) {
                Ok(()) => page.scripts.executed += 1,
                Err(error) => page.scripts.errors.push(format!("script node {id}: {error}")),
            }
            // a parser-blocking download gives script-inserted scripts time to arrive
            if external {
                page.runtime.run_pending_scripts(&mut page.document);
            }
        }
        // module scripts are deferred: they run after parsing, in document order
        for (id, url, source) in modules {
            let r = match &url {
                Some(url) => page.runtime.run_module(url, Some(&source), &mut page.document),
                None => page.runtime.run_script(&source, Some(id), &mut page.document),
            };
            match r {
                Ok(()) => page.scripts.executed += 1,
                Err(error) => page.scripts.errors.push(format!("deferred script node {id} ({}): {error}", url.as_deref().unwrap_or("classic"))),
            }
            if url.is_none() {
                page.runtime.run_pending_scripts(&mut page.document);
            }
        }
        page.runtime.interp.pinned_nodes.clear();
        page.runtime.dom_content_loaded(&mut page.document);
        page.runtime.advance(&mut page.document, config.settle_ms, config.settle_fires);
        page.runtime.load(&mut page.document);
        page.runtime.advance(&mut page.document, 100.0, 500);
        page.absorb_sheets();
        page
    }

    /// Stop using the host given at load time (it may borrow resources that are about to go away).
    pub fn clear_host(&mut self) {
        self.runtime.clear_host();
    }

    /// Record intrinsic image sizes (keyed by `src` as written in the document) used by layout.
    pub fn set_image_sizes(&mut self, sizes: BTreeMap<String, (f32, f32)>) {
        self.image_sizes = sizes;
    }

    /// `src` values of images (and CSS background images) found in a display list.
    pub fn image_sources(display: &DisplayList) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &display.commands {
            if let crate::layout::Command::Image { src, .. } = c {
                if !out.contains(src) {
                    out.push(src.clone());
                }
            }
        }
        out
    }

    fn absorb_sheets(&mut self) {
        // collected <link> nodes may have had their ids reused
        let doc = &self.document;
        self.link_css.retain(|id, _| doc.nodes.get(*id).is_some_and(|n| n.tag() == Some("link")));
        let new = self.runtime.take_new_sheets();
        if !new.is_empty() {
            for (id, css) in new {
                self.link_css.insert(id, css);
            }
            self.runtime.set_style_sources(&self.link_css, &self.external_css);
        }
    }

    pub fn set_host(&mut self, host: Box<dyn Host>) {
        self.runtime.set_host(host);
    }

    /// The current document URL as seen by scripts (`location.href`).
    pub fn url(&self) -> &str {
        self.runtime.url()
    }

    /// Bytecode/native tier counters (compiled functions, AST fallbacks, native compilations).
    pub fn vm_stats(&self) -> crate::js::vm::VmStats {
        self.runtime.vm_stats()
    }

    /// Live objects by kind/constructor (diagnostics).
    pub fn heap_histogram(&mut self, top: usize) -> Vec<(String, usize)> {
        self.runtime.heap_histogram(&mut self.document, top)
    }

    /// Run the cycle collector now (returns freed objects + scopes).
    pub fn collect_garbage(&mut self) -> usize {
        self.runtime.collect_garbage()
    }

    /// Cycle collector counters.
    pub fn gc_stats(&self) -> crate::js::gc::GcStats {
        self.runtime.gc_stats()
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

    /// The executable memory and interrupt given at load time, for reuse by the next page.
    pub fn take_platform(&mut self) -> (Option<alloc::boxed::Box<dyn crate::platform::CodeMemory>>, Option<alloc::boxed::Box<dyn crate::platform::Interrupt>>) {
        self.runtime.take_platform()
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

    /// The post-layout step of a frame: resize observers see the new boxes, then due timers and
    /// animation frames run. Returns whether scripts changed the DOM (the page needs a new layout).
    pub fn after_layout(&mut self) -> bool {
        self.sync_layout();
        let _ = self.runtime.eval_in_page(&mut self.document, "typeof __afterLayout === 'function' && __afterLayout()");
        self.runtime.advance(&mut self.document, 50.0, 200);
        self.absorb_sheets();
        self.runtime.take_dirty()
    }

    /// Advance the virtual clock, running due timers/animation frames. Returns whether the DOM changed.
    pub fn advance_time(&mut self, ms: f64) -> bool {
        self.sync_layout();
        self.runtime.advance(&mut self.document, ms, 2000);
        self.absorb_sheets();
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
        let r = self.runtime.dispatch_event(&mut self.document, node, event, class, bubbles, cancelable, &[]);
        self.absorb_sheets();
        r
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

    fn stylesheet_with(&self, extra: &str) -> Rc<StyleSheet> {
        let mut css = embedded_css(&self.document, &self.link_css, &self.external_css);
        if let Some(text) = self.document.adopted_css.get(&self.document.root) {
            css.push_str(text);
            css.push('\n');
        }
        css.push_str(extra);
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

    /// `@font-face` rules of the current document styles.
    pub fn font_faces(&self) -> Vec<crate::css::FontFace> {
        self.stylesheet_with("").font_faces.clone()
    }

    /// (family, weight, italic) of every family name text in the document may be drawn with, and
    /// the characters that text contains (for `unicode-range` subsets).
    pub fn used_fonts(&self, viewport_width: f32) -> (alloc::collections::BTreeSet<(String, u16, bool)>, alloc::collections::BTreeSet<u32>) {
        let sheet = self.stylesheet_with("");
        let styles = crate::css::compute_styles(&self.document, &sheet, Viewport::new(viewport_width, 768.0));
        let mut out = alloc::collections::BTreeSet::new();
        let mut chars = alloc::collections::BTreeSet::new();
        for (id, node) in self.document.nodes.iter().enumerate() {
            let crate::dom::NodeKind::Text(t) = &node.kind else { continue };
            if t.trim().is_empty() {
                continue;
            }
            let s = styles.get(id);
            if s.display == crate::css::Display::None {
                continue;
            }
            if let Some(names) = &s.font_names {
                for n in names.split(',') {
                    out.insert((String::from(n), s.font_weight, s.font_italic));
                }
            }
            if chars.len() < 8000 {
                chars.extend(t.chars().filter(|c| !c.is_whitespace()).map(|c| c as u32));
            }
        }
        (out, chars)
    }

    /// Render with the given viewport width and a 768px tall viewport.
    pub fn render(&self, viewport_width: f32, fonts: &impl FontMetrics) -> DisplayList {
        self.render_viewport(Viewport::new(viewport_width, 768.0), fonts)
    }

    pub fn render_viewport(&self, viewport: Viewport, fonts: &impl FontMetrics) -> DisplayList {
        let composed = crate::shadow::compose(&self.document);
        let (doc, extra) = match &composed {
            Some(c) => (&c.doc, c.css.as_str()),
            None => (&self.document, ""),
        };
        let sheet = self.stylesheet_with(extra);
        let list = if self.image_sizes.is_empty() {
            render_document(doc, &sheet, viewport, fonts)
        } else {
            let wrapped = crate::layout::WithImageSizes { inner: fonts, sizes: &self.image_sizes };
            render_document(doc, &sheet, viewport, &wrapped)
        };
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

    #[test]
    fn shadow_dom_renders_slots_and_scoped_styles() {
        let mut page = Page::new(
            "<style>p { color: #00ff00 }</style><my-card><span slot='t'>Title</span>light</my-card><p>outside</p><script>const h = document.querySelector('my-card'); const r = h.attachShadow({mode:'open'}); r.innerHTML = '<style>p { color: #ff0000 } :host { display:block }</style><p><slot name=t></slot>|<slot></slot></p>';</script>",
            "",
        );
        let list = page.render(400.0, &Mono);
        let color_of = |t: &str| list.commands.iter().find_map(|c| match c { Command::Text { text, color, .. } if text.contains(t) => Some(*color), _ => None });
        assert_eq!(color_of("Title"), Some(0xffff0000));
        assert_eq!(color_of("outside"), Some(0xff00ff00));
        assert!(color_of("light").is_some());
        let _ = &mut page;
    }

    #[test]
    fn adopted_style_sheets_apply() {
        let page = Page::new(
            "<x-a></x-a><p>doc</p><script>const s = new CSSStyleSheet(); s.replaceSync('b { color: #ff0000 }'); const r = document.querySelector('x-a').attachShadow({mode:'open'}); r.adoptedStyleSheets = [s]; r.innerHTML = '<b>inner</b>'; const d = new CSSStyleSheet(); d.replaceSync('p { color: #0000ff }'); document.adoptedStyleSheets = [...document.adoptedStyleSheets, d]; s.insertRule('i { color: #00ff00 }'); r.innerHTML += '<i>later</i>';</script>",
            "",
        );
        let list = page.render(400.0, &Mono);
        let color_of = |t: &str| list.commands.iter().find_map(|c| match c { Command::Text { text, color, .. } if text.contains(t) => Some(*color), _ => None });
        assert_eq!(color_of("inner"), Some(0xffff0000));
        assert_eq!(color_of("doc"), Some(0xff0000ff));
        assert_eq!(color_of("later"), Some(0xff00ff00));
    }

    #[test]
    fn declarative_shadow_dom() {
        let page = Page::new("<div><template shadowrootmode='open'><b>from shadow</b></template></div>", "");
        let list = page.render(400.0, &Mono);
        assert!(list.commands.iter().any(|c| matches!(c, Command::Text { text, .. } if text.contains("from shadow"))));
    }

    #[test]
    fn custom_elements_with_shadow_dom() {
        let page = Page::new(
            "<x-greet name='Ann'></x-greet><script>class G extends HTMLElement { static get observedAttributes(){ return ['name']; } constructor(){ super(); this.attachShadow({mode:'open'}); } connectedCallback(){ this.draw(); } attributeChangedCallback(){ if (this.isConnected) this.draw(); } draw(){ this.shadowRoot.innerHTML = '<b>Hello ' + this.getAttribute('name') + '</b>'; } } customElements.define('x-greet', G); const e = document.createElement('x-greet'); e.setAttribute('name','Bob'); document.body.appendChild(e);</script>",
            "",
        );
        let list = page.render(400.0, &Mono);
        let texts: Vec<&str> = list.commands.iter().filter_map(|c| match c { Command::Text { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert!(texts.iter().any(|t| t.contains("Hello Ann")), "{texts:?} {:?}", page.console());
        assert!(texts.iter().any(|t| t.contains("Hello Bob")), "{texts:?}");
    }
}
