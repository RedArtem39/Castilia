use castilia::{Page, Viewport, css, html};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[path = "common/font.rs"]
mod font;
#[path = "common/svg.rs"]
mod svg;

/// Read a stylesheet from disk, inlining `@import`ed files in front of it.
fn load_css(path: &Path, depth: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else { return String::new() };
    let mut out = String::new();
    if depth < 4 {
        for (url, _) in css::parse(&text).imports {
            let target: PathBuf = path.parent().unwrap_or(Path::new(".")).join(url);
            out.push_str(&load_css(&target, depth + 1));
        }
    }
    out.push_str(&text);
    out
}

/// usage: render_svg INPUT.html OUTPUT.svg [WIDTH] [HEIGHT]
///
/// Local files referenced by `<link rel=stylesheet>` and `<script src>` are loaded from disk.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: cargo run --example render_svg -- INPUT.html OUTPUT.svg [WIDTH] [HEIGHT]");
        std::process::exit(2);
    }
    let width: f32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(800.0);
    let height: f32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(300.0);
    let source = fs::read_to_string(&args[1])?;
    let dir = Path::new(&args[1]).parent().unwrap_or(Path::new(".")).to_path_buf();
    let fonts = font::SystemFont::new()?;
    let document = html::parse(&source);
    let mut page = Page::from_document_with_loader(document, "", |src| fs::read_to_string(dir.join(src)).map_err(|e| e.to_string()));
    let links: Vec<(usize, String)> = page
        .document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.tag() == Some("link") && n.attribute("rel").is_some_and(|r| r.split_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet"))))
        .filter_map(|(id, n)| n.attribute("href").map(|h| (id, h.to_string())))
        .collect();
    for (id, href) in links {
        page.set_link_stylesheet(id, load_css(&dir.join(href), 0));
    }
    for error in &page.scripts.errors {
        eprintln!("script error: {error}");
    }
    let list = page.render_viewport(Viewport::new(width, height), &fonts);
    fs::write(&args[2], svg::to_svg(&list, &fonts, &[], height))?;
    println!("wrote {} drawing commands to {}", list.commands.len(), args[2]);
    Ok(())
}
