use crate::{
    css,
    dom::{Document, NodeKind},
    embedded_css, html,
    layout::{Command, DisplayList, FontMetrics},
    page::Page,
    render_document,
};
use alloc::{string::String, vec::Vec};

const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_CSS_BYTES: usize = 2 * 1024 * 1024;
const MAX_STYLESHEETS: usize = 16;
const MAX_JS_BYTES: usize = 256 * 1024;

pub struct Response {
    pub final_url: String,
    pub body: Vec<u8>,
}

/// A platform adapter implements HTTP(S), redirects, URL resolution and TLS.
/// It must not return more than the requested byte limit.
pub trait Fetcher {
    fn get(&mut self, url: &str, max_bytes: usize) -> Result<Response, String>;
    fn resolve(&self, base: &str, reference: &str) -> Result<String, String>;
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
    load_page_mode(fetcher, url, width, fonts, false)
}

/// A simplified reading view for pages whose layout exceeds the MVP CSS engine.
pub fn load_reader_page(
    fetcher: &mut impl Fetcher,
    url: &str,
    width: f32,
    fonts: &impl FontMetrics,
) -> Result<LoadedPage, String> {
    load_page_mode(fetcher, url, width, fonts, true)
}

fn load_page_mode(
    fetcher: &mut impl Fetcher,
    url: &str,
    width: f32,
    fonts: &impl FontMetrics,
    reader: bool,
) -> Result<LoadedPage, String> {
    let response = fetcher.get(url, MAX_HTML_BYTES)?;
    if response.body.len() > MAX_HTML_BYTES {
        return Err(String::from("HTML exceeds size limit"));
    }
    let html_bytes = response.body.len();
    let html_source = String::from_utf8_lossy(&response.body);
    let mut warnings = Vec::new();
    if matches!(html_source, alloc::borrow::Cow::Owned(_)) {
        warnings.push(String::from(
            "HTML contains invalid UTF-8; replacement characters were used",
        ));
    }
    let document = html::parse(&html_source);
    let base = document
        .nodes
        .iter()
        .find(|node| node.tag() == Some("base"))
        .and_then(|node| node.attribute("href"))
        .and_then(|href| fetcher.resolve(&response.final_url, href).ok())
        .unwrap_or_else(|| response.final_url.clone());
    let mut external_css = String::new();
    let mut css_bytes = 0;
    let mut linked_stylesheets = 0;
    let mut loaded_stylesheets = 0;
    for node in &document.nodes {
        if node.tag() == Some("style") {
            for &child in &node.children {
                if let NodeKind::Text(text) = &document.nodes[child].kind {
                    css_bytes += text.len();
                }
            }
        }
        if node.tag() == Some("link")
            && node.attribute("rel").is_some_and(|rel| {
                rel.split_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            })
        {
            linked_stylesheets += 1;
            if linked_stylesheets > MAX_STYLESHEETS {
                warnings.push(String::from("stylesheet limit reached"));
                continue;
            }
            let Some(href) = node.attribute("href") else {
                continue;
            };
            let Ok(css_url) = fetcher.resolve(&base, href) else {
                warnings.push(alloc::format!("invalid stylesheet URL: {href}"));
                continue;
            };
            match fetcher.get(&css_url, MAX_CSS_BYTES) {
                Ok(sheet) if sheet.body.len() <= MAX_CSS_BYTES => {
                    let text = String::from_utf8_lossy(&sheet.body);
                    if matches!(text, alloc::borrow::Cow::Owned(_)) {
                        warnings.push(alloc::format!("stylesheet is not valid UTF-8: {css_url}"));
                    }
                    css_bytes += sheet.body.len();
                    external_css.push_str(&text);
                    external_css.push('\n');
                    loaded_stylesheets += 1;
                }
                Ok(_) => warnings.push(alloc::format!("stylesheet too large: {css_url}")),
                Err(error) => {
                    warnings.push(alloc::format!("stylesheet failed: {css_url}: {error}"))
                }
            }
        }
    }
    let css_audit = css::audit(&embedded_css(&document, &external_css));
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
        (render_document(&reading_document, "", width, fonts), None)
    } else {
        let page = Page::from_document_with_loader(document, &external_css, |src| {
            let url = fetcher.resolve(&base, src)?;
            let response = fetcher.get(&url, MAX_JS_BYTES)?;
            if response.body.len() > MAX_JS_BYTES {
                return Err(String::from("script exceeds size limit"));
            }
            String::from_utf8(response.body).map_err(|_| String::from("script is not valid UTF-8"))
        });
        let display = page.render(width, fonts);
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
            Ok(Response {
                final_url: String::from(url),
                body,
            })
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
                color: 0xff0000,
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
                .any(|command| matches!(command, Command::Text { text, .. } if text == "from"))
        );
    }
}
