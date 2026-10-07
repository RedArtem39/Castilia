#[path = "common/font.rs"]
mod font;
#[path = "common/net.rs"]
mod net;

#[path = "common/svg.rs"]
mod svg;

use castilia::network::{fetch_images, load_page, load_reader_page};
use std::{env, fs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    let reader = args.len() == 4 && args[1] == "--reader";
    if !(args.len() == 3 || reader) {
        eprintln!(
            "usage: cargo run --features host-net --example browse -- [--reader] URL OUTPUT.svg"
        );
        std::process::exit(2);
    }
    let url_index = if reader { 2 } else { 1 };
    let output_index = url_index + 1;
    let mut fetcher = net::HostFetcher::new();
    let fonts = font::SystemFont::new()?;
    let page = if reader {
        load_reader_page(&mut fetcher, &args[url_index], 800.0, &fonts)?
    } else {
        load_page(&mut fetcher, &args[url_index], 800.0, &fonts)?
    };
    let images = fetch_images(&mut fetcher, &page.display, 16);
    let resources: Vec<svg::Resource> = images.resources.iter().map(|r| svg::Resource { url: &r.url, mime: r.mime, bytes: &r.bytes }).collect();
    fs::write(&args[output_index], svg::to_svg(&page.display, &fonts, &resources, 768.0))?;
    println!("URL: {}", page.url);
    println!(
        "HTML: {} bytes, CSS: {} bytes ({} of {} linked sheets loaded)",
        page.report.html_bytes,
        page.report.css_bytes,
        page.report.loaded_stylesheets,
        page.report.linked_stylesheets
    );
    println!(
        "DOM nodes: {}, drawing commands: {}, page height: {} px",
        page.report.dom_nodes, page.report.commands, page.display.height
    );
    println!(
        "Scripts: {}, images: {}, forms: {}, tables: {}",
        page.report.scripts, page.report.images, page.report.forms, page.report.tables
    );
    println!(
        "JS executed: {}, external loaded: {}, skipped: {}, errors: {}",
        page.report.scripts_executed,
        page.report.external_scripts_loaded,
        page.report.scripts_skipped,
        page.report.script_errors
    );
    println!("Image resources loaded: {}", images.resources.len());
    println!(
        "CSS features ignored (approx.): {} selectors, {} declarations, {} at-rules",
        page.report.ignored_css_selectors,
        page.report.ignored_css_declarations,
        page.report.css_at_rules
    );
    for warning in &page.report.warnings {
        println!("Warning: {warning}");
    }
    for warning in &images.warnings {
        println!("Warning: {warning}");
    }
    println!("SVG: {}", args[output_index]);
    Ok(())
}
