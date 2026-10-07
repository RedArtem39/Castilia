use crate::{
    css::{self, Viewport},
    dom::{Document, NodeKind},
    embedded_css, html,
    layout::{Command, DisplayList, FontMetrics},
    page::Page,
    render_document,
};
use alloc::{boxed::Box, string::String, vec::Vec};

const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_CSS_BYTES: usize = 2 * 1024 * 1024;
const MAX_STYLESHEETS: usize = 64;
const MAX_JS_BYTES: usize = 2 * 1024 * 1024;
const MAX_FETCH_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_PROBE_BYTES: usize = 512 * 1024;
const MAX_IMAGE_PROBES: usize = 64;

pub struct Response {
    pub final_url: String,
    pub body: Vec<u8>,
    /// `Content-Type` header value, used for charset detection.
    pub content_type: Option<String>,
    /// HTTP status (defaults to 200 for adapters that only report success).
    pub status: u16,
    /// Response headers (lowercase names) when the adapter exposes them.
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn new(final_url: String, body: Vec<u8>) -> Response {
        Response { final_url, body, content_type: None, status: 200, headers: Vec::new() }
    }
}

/// A platform adapter implements HTTP(S), redirects, URL resolution and TLS.
/// It should truncate bodies at the requested byte limit rather than fail.
pub trait Fetcher {
    fn get(&mut self, url: &str, max_bytes: usize) -> Result<Response, String>;
    fn resolve(&self, base: &str, reference: &str) -> Result<String, String>;
    /// Arbitrary requests for `fetch()`/`XMLHttpRequest` made by scripts. Adapters that only
    /// implement GET can leave the default.
    fn request(&mut self, method: &str, url: &str, _headers: &[(String, String)], _body: Option<&str>, max_bytes: usize) -> Result<Response, String> {
        if method.eq_ignore_ascii_case("GET") {
            self.get(url, max_bytes)
        } else {
            Err(String::from("only GET requests are supported"))
        }
    }
}

/// Bridges a [`Fetcher`] to the script runtime's [`Host`](crate::js::Host).
///
/// The fetcher is borrowed through a raw pointer so that the (`'static`) host can live inside the
/// page; it is only valid while the caller keeps the fetcher alive and must be cleared with
/// `Page::clear_host` before the fetcher goes away.
struct BorrowedHost {
    fetcher: *mut (dyn Fetcher + 'static),
}

impl crate::js::Host for BorrowedHost {
    fn fetch(&mut self, method: &str, url: &str, headers: &[(String, String)], body: Option<&str>) -> Option<crate::js::HostResponse> {
        // SAFETY: the pointer is created from a live `&mut F` in `load_page_mode`, used only while that
        // function is running (scripts execute synchronously inside it), and cleared before it returns.
        let fetcher = unsafe { &mut *self.fetcher };
        let r = fetcher.request(method, url, headers, body, MAX_FETCH_BYTES).ok()?;
        let cs = r.content_type.as_deref().and_then(crate::encoding::charset_from_content_type);
        Some(crate::js::HostResponse {
            status: r.status,
            status_text: String::new(),
            url: r.final_url,
            headers: {
                let mut h = r.headers;
                if let Some(ct) = r.content_type {
                    if !h.iter().any(|(k, _)| k == "content-type") {
                        h.push((String::from("content-type"), ct));
                    }
                }
                h
            },
            body: crate::encoding::decode_resource(&r.body, cs),
        })
    }

    fn navigate(&mut self, _url: &str) {}
}

/// A [`Host`](crate::js::Host) that forwards to a shared fetcher; use it with `Page::set_host` to keep
/// scripts connected after loading.
pub struct SharedFetcherHost<F: Fetcher> {
    pub fetcher: alloc::rc::Rc<core::cell::RefCell<F>>,
}

impl<F: Fetcher> crate::js::Host for SharedFetcherHost<F> {
    fn fetch(&mut self, method: &str, url: &str, headers: &[(String, String)], body: Option<&str>) -> Option<crate::js::HostResponse> {
        let r = self.fetcher.borrow_mut().request(method, url, headers, body, MAX_FETCH_BYTES).ok()?;
        let cs = r.content_type.as_deref().and_then(crate::encoding::charset_from_content_type);
        Some(crate::js::HostResponse { status: r.status, status_text: String::new(), url: r.final_url, headers: r.headers, body: crate::encoding::decode_resource(&r.body, cs) })
    }
}

#[derive(Debug)]
pub struct PageReport {
    pub html_bytes: usize,
    pub css_bytes: usize,
    pub dom_nodes: usize,
    pub commands: usize,
    pub scripts: usize,
    pub scripts_executed: usize,
    pub external_scripts_loaded: usize,
    pub scripts_skipped: usize,
    pub script_errors: usize,
    pub images: usize,
    pub forms: usize,
    pub tables: usize,
    pub linked_stylesheets: usize,
    pub loaded_stylesheets: usize,
    pub ignored_css_selectors: usize,
    pub ignored_css_declarations: usize,
    pub css_at_rules: usize,
    pub warnings: Vec<String>,
}

pub struct LoadedPage {
    pub url: String,
    pub display: DisplayList,
    pub report: PageReport,
    /// Retained document and JS state for further events and re-rendering.
    pub page: Option<Page>,
}

pub struct ImageResource {
    pub url: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

pub struct ImageLoad {
    pub resources: Vec<ImageResource>,
    pub warnings: Vec<String>,
}

/// Fetch referenced images. The platform adapter decodes and paints them.
pub fn fetch_images(
    fetcher: &mut impl Fetcher,
    list: &DisplayList,
    max_images: usize,
) -> ImageLoad {
    let mut resources = Vec::new();
    let mut warnings = Vec::new();
    for command in &list.commands {
        let Command::Image { src, .. } = command else {
            continue;
        };
        if resources
            .iter()
            .any(|item: &ImageResource| item.url == *src)
        {
            continue;
        }
        if resources.len() >= max_images {
            warnings.push(String::from("image count limit reached"));
            break;
        }
        match fetcher.get(src, 4 * 1024 * 1024) {
            Ok(response) => {
                if let Some(mime) = image_mime(&response.body) {
                    resources.push(ImageResource {
                        url: src.clone(),
                        mime,
                        bytes: response.body,
                    });
                } else {
                    warnings.push(alloc::format!("unsupported image format: {src}"));
                }
            }
            Err(error) => warnings.push(alloc::format!("image failed: {src}: {error}")),
        }
    }
    ImageLoad {
        resources,
        warnings,
    }
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
        return Some("image/svg+xml");
    }
    None
}

pub fn load_page(
    fetcher: &mut impl Fetcher,
    url: &str,
    width: f32,
    fonts: &impl FontMetrics,
) -> Result<LoadedPage, String> {
    load_page_mode(fetcher, url, width, fonts, false, LoadOptions::default())
}

/// Engine settings for [`load_page_with`].
#[derive(Default)]
pub struct LoadOptions {
    pub config: crate::config::EngineConfig,
    /// Executable memory for `ExecMode::Jit`.
    pub code_memory: Option<Box<dyn crate::platform::CodeMemory>>,
}

/// [`load_page`] with an explicit execution mode and limits.
pub fn load_page_with(
    fetcher: &mut impl Fetcher,
    url: &str,
    width: f32,
    fonts: &impl FontMetrics,
    options: LoadOptions,
) -> Result<LoadedPage, String> {
    load_page_mode(fetcher, url, width, fonts, false, options)
}

/// A simplified reading view for pages whose layout exceeds the MVP CSS engine.
pub fn load_reader_page(
    fetcher: &mut impl Fetcher,
    url: &str,
    width: f32,
    fonts: &impl FontMetrics,
) -> Result<LoadedPage, String> {
    load_page_mode(fetcher, url, width, fonts, true, LoadOptions::default())
}

fn fetch_css_text<F: Fetcher>(fetcher: &mut F, url: &str) -> Option<String> {
    let r = fetcher.get(url, MAX_CSS_BYTES).ok()?;
    let cs = r.content_type.as_deref().and_then(crate::encoding::charset_from_content_type);
    Some(crate::encoding::decode_resource(&r.body, cs))
}

/// Fetch images and read their pixel dimensions from the file headers.
fn probe_images<F: Fetcher>(fetcher: &mut F, base: &str, sources: &[String]) -> alloc::collections::BTreeMap<String, (f32, f32)> {
    let mut sizes = alloc::collections::BTreeMap::new();
    let mut fetched = 0;
    for src in sources {
        if src.starts_with("data:") {
            if let Some((w, h)) = crate::image::decode_data_url(src).and_then(|b| crate::image::probe_size(&b)) {
                sizes.insert(src.clone(), (w as f32, h as f32));
            }
            continue;
        }
        if fetched >= MAX_IMAGE_PROBES {
            break;
        }
        let Ok(url) = fetcher.resolve(base, src) else { continue };
        fetched += 1;
        if let Ok(r) = fetcher.get(&url, MAX_IMAGE_PROBE_BYTES) {
            if let Some((w, h)) = crate::image::probe_size(&r.body) {
                sizes.insert(src.clone(), (w as f32, h as f32));
            }
        }
    }
    sizes
}

fn load_page_mode<F: Fetcher>(
    fetcher: &mut F,
    url: &str,
    width: f32,
    fonts: &impl FontMetrics,
    reader: bool,
    load_options: LoadOptions,
) -> Result<LoadedPage, String> {
    let response = fetcher.get(url, MAX_HTML_BYTES)?;
    if response.body.len() > MAX_HTML_BYTES {
        return Err(String::from("HTML exceeds size limit"));
    }
    let html_bytes = response.body.len();
    let transport = response.content_type.as_deref().and_then(crate::encoding::charset_from_content_type);
    let html_source = crate::encoding::decode(&response.body, transport);
    let mut warnings = Vec::new();
    let mut document = html::parse(&html_source);
    let base = document
        .nodes
        .iter()
        .find(|node| node.tag() == Some("base"))
        .and_then(|node| node.attribute("href"))
        .and_then(|href| fetcher.resolve(&response.final_url, href).ok())
        .unwrap_or_else(|| response.final_url.clone());
    let external_css = String::new();
    let mut link_css: alloc::collections::BTreeMap<usize, String> = alloc::collections::BTreeMap::new();
    let mut css_bytes = 0;
    let mut linked_stylesheets = 0;
    let mut loaded_stylesheets = 0;
    let node_count = document.nodes.len();
    for node_id in 0..node_count {
        if document.nodes[node_id].tag() == Some("style") {
            let text = document.text_content(node_id).unwrap_or_default();
            css_bytes += text.len();
            if text.contains("@import") || text.contains("url(") {
                let processed = {
                    let mut fetch = |u: &str| fetch_css_text(fetcher, u);
                    crate::cssload::process(&text, &base, &mut fetch)
                };
                document.set_text_content(node_id, &processed);
            }
        }
        let node = &document.nodes[node_id];
        if node.tag() == Some("link")
            && node.attribute("rel").is_some_and(|rel| {
                rel.split_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            })
        {
            let Some(href) = node.attribute("href") else {
                continue;
            };
            if node.attribute("disabled").is_some() {
                continue;
            }
            linked_stylesheets += 1;
            if linked_stylesheets > MAX_STYLESHEETS {
                if linked_stylesheets == MAX_STYLESHEETS + 1 {
                    warnings.push(String::from("stylesheet limit reached"));
                }
                continue;
            }
            let Ok(css_url) = fetcher.resolve(&base, href) else {
                warnings.push(alloc::format!("invalid stylesheet URL: {href}"));
                continue;
            };
            match fetcher.get(&css_url, MAX_CSS_BYTES) {
                Ok(sheet) if sheet.body.len() <= MAX_CSS_BYTES => {
                    let cs = sheet.content_type.as_deref().and_then(crate::encoding::charset_from_content_type);
                    let text = crate::encoding::decode_resource(&sheet.body, cs);
                    css_bytes += sheet.body.len();
                    let processed = {
                        let mut fetch = |u: &str| fetch_css_text(fetcher, u);
                        crate::cssload::process(&text, &sheet.final_url, &mut fetch)
                    };
                    link_css.insert(node_id, processed);
                    loaded_stylesheets += 1;
                }
                Ok(_) => warnings.push(alloc::format!("stylesheet too large: {css_url}")),
                Err(error) => {
                    warnings.push(alloc::format!("stylesheet failed: {css_url}: {error}"))
                }
            }
        }
    }
    let css_audit = css::audit(&embedded_css(&document, &link_css, &external_css));
    let count = |tag| {
        document
            .nodes
            .iter()
            .filter(|node| node.tag() == Some(tag))
            .count()
    };
    let dom_nodes = document.nodes.len();
    let scripts = count("script");
    let images = count("img");
    let forms = count("form");
    let tables = count("table");
    let (mut display, page) = if reader {
        let reading_document = reader_document(&document);
        (render_document(&reading_document, &css::parse(""), Viewport::new(width, 768.0), fonts), None)
    } else {
        let fetcher_ptr: *mut F = fetcher;
        // SAFETY: erases the borrow's lifetime; the pointer is only dereferenced while this function
        // runs and the host holding it is dropped (`clear_host`) before returning.
        let erased: *mut (dyn Fetcher + 'static) = unsafe { core::mem::transmute::<*mut (dyn Fetcher + '_), *mut (dyn Fetcher + 'static)>(fetcher_ptr as *mut (dyn Fetcher + '_)) };
        let options = crate::page::PageOptions {
            url: Some(response.final_url.clone()),
            host: Some(Box::new(BorrowedHost { fetcher: erased })),
            viewport: Some((width, 768.0)),
            document_base: Some(base.clone()),
            link_css: link_css.clone(),
            config: load_options.config,
            code_memory: load_options.code_memory,
        };
        let base_for_scripts = base.clone();
        let mut page = Page::load(document, &external_css, options, |src| {
            // SAFETY: see `BorrowedHost`; scripts run synchronously inside `Page::load`.
            let fetcher = unsafe { &mut *fetcher_ptr };
            let url = fetcher.resolve(&base_for_scripts, src)?;
            let response = fetcher.get(&url, MAX_JS_BYTES)?;
            let cs = response.content_type.as_deref().and_then(crate::encoding::charset_from_content_type);
            Ok(crate::encoding::decode_resource(&response.body, cs))
        });
        page.clear_host();
        let mut display = page.render(width, fonts);
        // Probe image sizes so layout can use intrinsic dimensions, then lay out again.
        let sizes = probe_images(fetcher, &base, &Page::image_sources(&display));
        if !sizes.is_empty() {
            page.set_image_sizes(sizes);
            display = page.render(width, fonts);
        }
        (display, Some(page))
    };
    let scripts_executed = page.as_ref().map_or(0, |p| p.scripts.executed);
    let external_scripts_loaded = page.as_ref().map_or(0, |p| p.scripts.external_loaded);
    let scripts_skipped = page.as_ref().map_or(0, |p| p.scripts.skipped);
    let script_errors = page.as_ref().map_or(0, |p| p.scripts.errors.len());
    if let Some(page) = &page {
        warnings.extend(page.scripts.errors.iter().cloned());
    }
    for command in &mut display.commands {
        match command {
            Command::Image { src, link, .. } => {
                if let Ok(url) = fetcher.resolve(&base, src) {
                    *src = url;
                }
                resolve_link(fetcher, &base, link);
            }
            Command::Text { link, .. } => resolve_link(fetcher, &base, link),
            _ => {}
        }
    }
    let report = PageReport {
        html_bytes,
        css_bytes,
        dom_nodes,
        commands: display.commands.len(),
        scripts,
        scripts_executed,
        external_scripts_loaded,
        scripts_skipped,
        script_errors,
        images,
        forms,
        tables,
        linked_stylesheets,
        loaded_stylesheets,
        ignored_css_selectors: css_audit.ignored_selectors,
        ignored_css_declarations: css_audit.ignored_declarations,
        css_at_rules: css_audit.at_rules,
        warnings,
    };
    Ok(LoadedPage {
        url: response.final_url,
        display,
        report,
        page,
    })
}

fn reader_document(original: &Document) -> Document {
    let content = ["mw-content-text", "bodyContent", "content"]
        .iter()
        .find_map(|wanted| {
            original
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.attribute("id") == Some(*wanted))
                .map(|(id, _)| id)
        })
        .or_else(|| {
            original
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.tag() == Some("article"))
                .map(|(id, _)| id)
        })
        .or_else(|| {
            original
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.tag() == Some("main"))
                .map(|(id, _)| id)
        });
    let Some(content) = content else {
        return original.clone();
    };
    let title = original
        .nodes
        .iter()
        .enumerate()
        .find(|(_, node)| node.attribute("id") == Some("firstHeading"))
        .map(|(id, _)| id)
        .or_else(|| {
            original
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.tag() == Some("h1"))
                .map(|(id, _)| id)
        });
    let mut document = original.clone();
    document.nodes[document.body].children.clear();
    if let Some(title) = title
        && title != content
        && !is_descendant(&document, title, content)
    {
        document.nodes[title].parent = Some(document.body);
        document.nodes[document.body].children.push(title);
    }
    document.nodes[content].parent = Some(document.body);
    document.nodes[document.body].children.push(content);
    document
}

fn is_descendant(document: &Document, node: usize, ancestor: usize) -> bool {
    let mut parent = document.nodes[node].parent;
    while let Some(id) = parent {
        if id == ancestor {
            return true;
        }
        parent = document.nodes[id].parent;
    }
    false
}

fn resolve_link(fetcher: &impl Fetcher, base: &str, link: &mut Option<String>) {
    if let Some(value) = link
        && let Ok(url) = fetcher.resolve(base, value)
    {
        *value = url;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mock;
    impl Fetcher for Mock {
        fn get(&mut self, url: &str, _: usize) -> Result<Response, String> {
            let body = if url == "https://test.local/page" {
                b"<link rel='stylesheet' href='/site.css'><p class='x'><a href='/next'>hello</a></p><img src='/image.png' width='10' height='10'>".to_vec()
            } else if url == "https://test.local/site.css" {
                b".x a { color: red }".to_vec()
            } else if url == "https://test.local/reader" {
                b"<nav>menu</nav><h1 id='firstHeading'>Title</h1><main id='content'><p>article</p></main>".to_vec()
            } else if url == "https://test.local/script" {
                b"<p id='message'>before</p><script>document.getElementById('message').textContent = 'after'</script>".to_vec()
            } else if url == "https://test.local/external" {
                b"<p id='message'>before</p><script src='/app.js'></script><script>document.getElementById('message').textContent = window.message;</script>".to_vec()
            } else if url == "https://test.local/app.js" {
                b"window.message = 'from external';".to_vec()
            } else {
                return Err(String::from("not found"));
            };
            Ok(Response::new(String::from(url), body))
        }
        fn resolve(&self, base: &str, reference: &str) -> Result<String, String> {
            if reference.starts_with('/') {
                Ok(alloc::format!("https://test.local{reference}"))
            } else {
                Ok(alloc::format!("{base}/{reference}"))
            }
        }
    }
    struct Mono;
    impl FontMetrics for Mono {
        fn text_width(&self, text: &str, size: f32) -> f32 {
            text.len() as f32 * size * 0.5
        }
        fn line_height(&self, size: f32) -> f32 {
            size * 1.25
        }
    }
    #[test]
    fn downloads_page_and_linked_css() {
        let page = load_page(&mut Mock, "https://test.local/page", 300.0, &Mono).unwrap();
        assert_eq!(page.report.loaded_stylesheets, 1);
        assert!(page.display.commands.iter().any(|command| matches!(
            command,
            Command::Text {
                color: 0xffff0000,
                ..
            }
        )));
        assert!(page.display.commands.iter().any(|command| matches!(command, Command::Text { link: Some(url), .. } if url == "https://test.local/next")));
        assert!(page.display.commands.iter().any(|command| matches!(command, Command::Image { src, .. } if src == "https://test.local/image.png")));
    }

    #[test]
    fn reader_mode_keeps_article_and_title() {
        let page = load_reader_page(&mut Mock, "https://test.local/reader", 300.0, &Mono).unwrap();
        let words: Vec<_> = page
            .display
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(words.contains(&"Title"));
        assert!(words.contains(&"article"));
        assert!(!words.contains(&"menu"));
    }

    #[test]
    fn loaded_page_executes_inline_script() {
        let page = load_page(&mut Mock, "https://test.local/script", 300.0, &Mono).unwrap();
        assert_eq!(page.report.scripts_executed, 1);
        assert!(
            page.display
                .commands
                .iter()
                .any(|command| matches!(command, Command::Text { text, .. } if text == "after"))
        );
        assert!(page.page.is_some());
    }

    #[test]
    fn external_script_runs_before_following_inline_script() {
        let page = load_page(&mut Mock, "https://test.local/external", 300.0, &Mono).unwrap();
        assert_eq!(page.report.external_scripts_loaded, 1);
        assert_eq!(page.report.scripts_executed, 2);
        assert_eq!(page.report.script_errors, 0);
        assert!(
            page.display
                .commands
                .iter()
                .any(|command| matches!(command, Command::Text { text, .. } if text == "from external"))
        );
    }
}
