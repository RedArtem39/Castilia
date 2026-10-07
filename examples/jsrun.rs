//! Run scripts against an HTML document and print console output.
//! Usage: jsrun [--html file.html] script.js ... [-e 'expression']
#[path = "common/memlimit.rs"]
mod memlimit;
#[global_allocator]
static ALLOC: memlimit::Limited = memlimit::Limited;

use castilia::js::{Host, HostResponse};
use castilia::page::{Page, PageOptions};
use std::fs;

struct FileHost(String);

impl Host for FileHost {
    fn fetch(&mut self, _m: &str, url: &str, _h: &[(String, String)], _b: Option<&str>) -> Option<HostResponse> {
        let path = url.strip_prefix("http://localhost/")?.split(['?', '#']).next()?;
        let body = fs::read_to_string(format!("{}/{}", self.0, path)).ok();
        Some(match body {
            Some(body) => HostResponse { status: 200, status_text: "OK".into(), url: url.into(), headers: vec![], body },
            None => HostResponse { status: 404, status_text: "Not Found".into(), url: url.into(), headers: vec![], body: String::new() },
        })
    }
}

fn main() {
    memlimit::init();
    // run on a large stack so deep native re-entry is bounded by the engine's limits, not the OS
    let h = std::thread::Builder::new().stack_size(256 << 20).spawn(real_main).unwrap();
    h.join().unwrap();
}

fn real_main() {
    let mut config = castilia::EngineConfig::default();
    if let Some(m) = std::env::var("CASTILIA_JS_MODE").ok().and_then(|m| castilia::ExecMode::parse(&m)) {
        config.exec_mode = m;
    }
    config.max_native_depth = 1000;
    let mut html = String::from("<!doctype html><html><head></head><body></body></html>");
    let mut scripts = Vec::new();
    let mut expr = None;
    let mut root = String::from(".");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--html" => html = fs::read_to_string(args.next().unwrap()).unwrap(),
            "-e" => expr = args.next(),
            "--root" => root = args.next().unwrap(),
            "--mode" => config.exec_mode = castilia::ExecMode::parse(&args.next().unwrap()).expect("mode: interp|jit|ast"),
            path => scripts.push(path.to_string()),
        }
    }
    let mut doc = castilia::html::parse(&html);
    // append scripts as inline <script> nodes at the end of body so they run in order
    for path in &scripts {
        let src = fs::read_to_string(path).unwrap();
        let s = doc.create_element("script");
        doc.set_text_content(s, &src);
        doc.append_child(doc.body, s);
        let _ = path;
    }
    let started = std::time::Instant::now();
    let mut page = Page::load(doc, "", PageOptions { url: Some("http://localhost/".into()), host: Some(Box::new(FileHost(root))), config, ..Default::default() }, |_| Err("no loader".into()));
    eprintln!("scripts: executed={} errors={:?} ({:?})", page.scripts.executed, page.scripts.errors, started.elapsed());
    eprintln!("vm: {:?}", page.vm_stats());
    if let Some(e) = expr {
        match page.evaluate(&e) {
            Ok(v) => println!("=> {v}"),
            Err(e) => println!("!! {e}"),
        }
    }
    for l in page.console() {
        println!("{l}");
    }
}
