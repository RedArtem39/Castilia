#[path = "common/font.rs"]
mod font;
#[path = "common/net.rs"]
mod net;

use base64::Engine;
use castilia::{
    Command, DisplayList,
    network::{ImageResource, fetch_images, load_page, load_reader_page},
};
use std::{env, fmt::Write, fs};

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn svg(list: &DisplayList, images: &[ImageResource]) -> String {
    let mut output = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n<rect width=\"100%\" height=\"100%\" fill=\"white\"/>\n",
        list.width,
        list.height.max(1.0),
        list.width,
        list.height.max(1.0)
    );
    for command in &list.commands {
        match command {
            Command::FillRect { rect, color } => {
                writeln!(
                    output,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#{color:06x}\"/>",
                    rect.x, rect.y, rect.width, rect.height
                )
                .unwrap();
            }
            Command::Text {
                x,
                y,
                text,
                size,
                color,
                link,
            } => {
                if let Some(link) = link {
                    write!(output, "<a href=\"{}\">", escape(link)).unwrap();
                }
                write!(output, "<text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"{}\" fill=\"#{color:06x}\">{}</text>", x, y + size * 0.9, size, escape(text)).unwrap();
                if link.is_some() {
                    output.push_str("</a>");
                }
                output.push('\n');
            }
            Command::Image { rect, src, link } => {
                if let Some(link) = link {
                    write!(output, "<a href=\"{}\">", escape(link)).unwrap();
                }
                let data = images.iter().find(|image| image.url == *src).map(|image| {
                    format!(
                        "data:{};base64,{}",
                        image.mime,
                        base64::engine::general_purpose::STANDARD.encode(&image.bytes)
                    )
                });
                let href = data.as_deref().unwrap_or(src);
                write!(
                    output,
                    "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\"/>",
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    escape(href)
                )
                .unwrap();
                if link.is_some() {
                    output.push_str("</a>");
                }
                output.push('\n');
            }
        }
    }
    output.push_str("</svg>\n");
    output
}

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
    fs::write(&args[output_index], svg(&page.display, &images.resources))?;
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
