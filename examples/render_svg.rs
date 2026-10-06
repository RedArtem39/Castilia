use castilia::{Command, render};
use std::{env, fmt::Write, fs};

#[path = "common/font.rs"]
mod font;

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: cargo run --example render_svg -- INPUT.html OUTPUT.svg");
        std::process::exit(2);
    }
    let html = fs::read_to_string(&args[1])?;
    let fonts = font::SystemFont::new()?;
    let list = render(&html, "", 800.0, &fonts);
    let mut svg = format!(
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
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#{color:06x}\"/>",
                    rect.x, rect.y, rect.width, rect.height
                )?;
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
                    write!(svg, "<a href=\"{}\">", escape(link))?;
                }
                write!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"{}\" fill=\"#{color:06x}\">{}</text>",
                    x,
                    y + size * 0.9,
                    size,
                    escape(text)
                )?;
                if link.is_some() {
                    svg.push_str("</a>");
                }
                svg.push('\n');
            }
            Command::Image { rect, src, link } => {
                if let Some(link) = link {
                    write!(svg, "<a href=\"{}\">", escape(link))?;
                }
                write!(
                    svg,
                    "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\"/>",
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    escape(src)
                )?;
                if link.is_some() {
                    svg.push_str("</a>");
                }
                svg.push('\n');
            }
        }
    }
    svg.push_str("</svg>\n");
    fs::write(&args[2], svg)?;
    println!(
        "wrote {} drawing commands to {}",
        list.commands.len(),
        args[2]
    );
    Ok(())
}
