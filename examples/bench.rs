#[path = "common/net.rs"]
mod net;

use castilia::{Command, DisplayList, FontMetrics, network::load_page, render};
use std::{env, fmt::Write, fs, hint::black_box, time::Instant};

struct Mono;
impl FontMetrics for Mono {
    fn text_width(&self, text: &str, size: f32) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }
    fn line_height(&self, size: f32) -> f32 {
        size * 1.25
    }
}

struct Case {
    name: &'static str,
    html: &'static str,
    css: &'static str,
    supported: bool,
    check: fn(&DisplayList) -> bool,
}

fn has_text(list: &DisplayList, needle: &str) -> bool {
    list.commands
        .iter()
        .any(|command| matches!(command, Command::Text { text, .. } if text == needle))
}
fn text_y(list: &DisplayList, needle: &str) -> Option<f32> {
    list.commands.iter().find_map(|command| match command {
        Command::Text { text, y, .. } if text == needle => Some(*y),
        _ => None,
    })
}
fn checks() -> [Case; 12] {
    [
        Case {
            name: "Basic HTML text",
            html: "<h1>Hello</h1><p>world</p>",
            css: "",
            supported: true,
            check: |l| has_text(l, "Hello") && has_text(l, "world"),
        },
        Case {
            name: "Broken paragraph recovery",
            html: "<p>first<p>second",
            css: "",
            supported: true,
            check: |l| text_y(l, "second") > text_y(l, "first"),
        },
        Case {
            name: "CSS class cascade",
            html: "<p class='x'>color</p>",
            css: "p { color: red } .x { color: blue }",
            supported: true,
            check: |l| {
                l.commands.iter().any(
                    |c| matches!(c, Command::Text { text, color: 0x0000ff, .. } if text == "color"),
                )
            },
        },
        Case {
            name: "Text wrapping",
            html: "<p>alpha beta gamma</p>",
            css: "",
            supported: true,
            check: |l| text_y(l, "gamma") > text_y(l, "alpha"),
        },
        Case {
            name: "Link target",
            html: "<a href='/next'>next</a>",
            css: "",
            supported: true,
            check: |l| {
                l.commands.iter().any(
                    |c| matches!(c, Command::Text { link: Some(target), .. } if target == "/next"),
                )
            },
        },
        Case {
            name: "Image placement",
            html: "<img src='x.png' width='20' height='10'>",
            css: "",
            supported: true,
            check: |l| {
                l.commands.iter().any(|c| matches!(c, Command::Image { rect, .. } if rect.width == 20.0 && rect.height == 10.0))
            },
        },
        Case {
            name: "JavaScript DOM update",
            html: "<script>document.body.textContent='dynamic'</script>",
            css: "",
            supported: true,
            check: |l| has_text(l, "dynamic"),
        },
        Case {
            name: "JavaScript condition and DOM insertion",
            html: "<script>if (2 > 1) { const p = document.createElement('p'); p.textContent = 'inserted'; document.body.appendChild(p); }</script>",
            css: "",
            supported: true,
            check: |l| has_text(l, "inserted"),
        },
        Case {
            name: "Basic flex row",
            html: "<div style='display:flex'><div>left</div><div>right</div></div>",
            css: "",
            supported: true,
            check: |l| text_y(l, "left") == text_y(l, "right"),
        },
        Case {
            name: "Form control",
            html: "<input value='typed'>",
            css: "",
            supported: false,
            check: |l| has_text(l, "typed"),
        },
        Case {
            name: "Linear table rows",
            html: "<table><tr><td>one</td></tr><tr><td>two</td></tr></table>",
            css: "",
            supported: true,
            check: |l| text_y(l, "two") > text_y(l, "one"),
        },
        Case {
            name: "CSS media query",
            html: "<p>responsive</p>",
            css: "@media (max-width: 500px) { p { color: red } }",
            supported: true,
            check: |l| {
                l.commands.iter().any(|c| matches!(c, Command::Text { text, color: 0xff0000, .. } if text == "responsive"))
            },
        },
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let live = args.first().is_some_and(|arg| arg == "--live");
    let mut output = String::from(
        "# Castilia MVP benchmark\n\nThis is a small capability probe, not a web standards conformance score. PASS means one narrowly defined behavior worked. FAIL is an expected MVP limitation for the cases marked unsupported.\n\n## Local capability checks\n\n| Case | Scope | Result |\n|---|---|---|\n",
    );
    let mut supported_passes = 0;
    let mut supported_total = 0;
    for case in checks() {
        let list = render(case.html, case.css, 110.0, &Mono);
        let passed = (case.check)(&list);
        if case.supported {
            supported_total += 1;
        }
        if case.supported && passed {
            supported_passes += 1;
        }
        writeln!(
            output,
            "| {} | {} | {} |",
            case.name,
            if case.supported { "MVP" } else { "Future" },
            if passed { "PASS" } else { "FAIL" }
        )?;
    }
    writeln!(
        output,
        "\nSupported checks passed: {supported_passes}/{supported_total}.\n"
    )?;
    let mut live_failures = 0;

    let synthetic = format!(
        "<style>p {{ color: #234567; margin: 2px }}</style>{}",
        "<p>alpha beta gamma delta epsilon zeta eta theta</p>".repeat(100)
    );
    let _ = render(&synthetic, "", 800.0, &Mono);
    let mut samples = Vec::new();
    for _ in 0..40 {
        let start = Instant::now();
        black_box(render(black_box(&synthetic), "", 800.0, &Mono));
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    writeln!(
        output,
        "## Local rendering speed\n\n100 paragraphs, 40 warm runs, network excluded. Median: {:.2} ms; p95: {:.2} ms. These numbers depend on this machine and build mode.\n",
        samples[20], samples[37]
    )?;

    if live {
        let urls: Vec<&str> = if args.len() > 1 {
            args[1..].iter().map(String::as_str).collect()
        } else {
            vec![
                "https://httpbin.org/html",
                "https://en.wikipedia.org/wiki/World_Wide_Web",
                "https://www.rust-lang.org/",
                "https://www.w3.org/",
                "https://www.python.org/",
                "https://news.ycombinator.com/",
            ]
        };
        output.push_str("## Live pages\n\nContent checks are limited to named default sites. PASS means expected words appear in the drawing list, not that the layout or interactions match a browser. CSS counts are approximate.\n\n| URL | Fetch | Expected content | HTML | CSS loaded | DOM | Draw commands | JS run/errors | External JS loaded | JS skipped | Scripts | Forms | Tables | CSS ignored (selectors / declarations / at-rules) | Elapsed |\n|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
        let mut fetcher = net::HostFetcher::new();
        for url in urls {
            let start = Instant::now();
            match load_page(&mut fetcher, url, 800.0, &Mono) {
                Ok(page) => {
                    let r = &page.report;
                    let expected = match url {
                        "https://httpbin.org/html" => Some("Herman Melville"),
                        "https://en.wikipedia.org/wiki/World_Wide_Web" => Some("World Wide Web"),
                        "https://www.rust-lang.org/" => Some("Rust"),
                        "https://www.w3.org/" => Some("W3C"),
                        "https://www.python.org/" => Some("Python"),
                        "https://news.ycombinator.com/" => Some("Hacker News"),
                        _ => None,
                    };
                    let visible = page
                        .display
                        .commands
                        .iter()
                        .filter_map(|command| match command {
                            Command::Text { text, .. } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    let content_result = expected.map_or("n/a", |words| {
                        if visible.contains(words) {
                            "PASS"
                        } else {
                            "FAIL"
                        }
                    });
                    if content_result == "FAIL" {
                        live_failures += 1;
                    }
                    writeln!(
                        output,
                        "| {} | OK | {} | {} | {}/{} | {} | {} | {}/{} | {} | {} | {} | {} | {} | {} / {} / {} | {:.0} ms |",
                        page.url,
                        content_result,
                        r.html_bytes,
                        r.loaded_stylesheets,
                        r.linked_stylesheets,
                        r.dom_nodes,
                        r.commands,
                        r.scripts_executed,
                        r.script_errors,
                        r.external_scripts_loaded,
                        r.scripts_skipped,
                        r.scripts,
                        r.forms,
                        r.tables,
                        r.ignored_css_selectors,
                        r.ignored_css_declarations,
                        r.css_at_rules,
                        start.elapsed().as_secs_f64() * 1000.0
                    )?;
                    for warning in r.warnings.iter().take(8) {
                        writeln!(output, "\n- {}: {}", page.url, warning)?;
                    }
                    if r.warnings.len() > 8 {
                        writeln!(
                            output,
                            "\n- {}: {} more warnings",
                            page.url,
                            r.warnings.len() - 8
                        )?;
                    }
                }
                Err(error) => {
                    live_failures += 1;
                    writeln!(
                        output,
                        "| {} | {} | FAIL | — | — | — | — | — | — | — | — | — | — | — | — | {:.0} ms |",
                        url,
                        error.replace('|', "/"),
                        start.elapsed().as_secs_f64() * 1000.0
                    )?;
                }
            }
        }
    }
    fs::write("bench-report.md", &output)?;
    println!("wrote bench-report.md");
    if supported_passes != supported_total || live_failures != 0 {
        return Err(std::io::Error::other(format!(
            "benchmark failures: {} local, {live_failures} live",
            supported_total - supported_passes
        ))
        .into());
    }
    Ok(())
}
