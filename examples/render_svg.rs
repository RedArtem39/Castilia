use castilia::{Page, Viewport};
use std::{env, fs};

#[path = "common/font.rs"]
mod font;
#[path = "common/svg.rs"]
mod svg;

/// usage: render_svg INPUT.html OUTPUT.svg [WIDTH] [HEIGHT]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: cargo run --example render_svg -- INPUT.html OUTPUT.svg [WIDTH] [HEIGHT]");
        std::process::exit(2);
    }
    let width: f32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(800.0);
    let height: f32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(300.0);
    let html = fs::read_to_string(&args[1])?;
    let fonts = font::SystemFont::new()?;
    let page = Page::new(&html, "");
    for error in &page.scripts.errors {
        eprintln!("script error: {error}");
    }
    let list = page.render_viewport(Viewport::new(width, height), &fonts);
    fs::write(&args[2], svg::to_svg(&list, &fonts, &[], height))?;
    println!("wrote {} drawing commands to {}", list.commands.len(), args[2]);
    Ok(())
}
