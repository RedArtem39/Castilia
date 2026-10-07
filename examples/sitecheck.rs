//! Real-site stability check.
//!
//! `sitecheck [LIST] [--jobs N] [--timeout S] [--mode interp|jit]` loads every URL of LIST (default
//! `tools/sites.txt`) in its own child process (a crash or hang never takes the run down), then prints a
//! summary. `sitecheck --one URL` loads a single page and prints one `RESULT` line.
#[path = "common/deadline.rs"]
mod deadline;
#[path = "common/codemem.rs"]
mod codemem;
#[path = "common/memlimit.rs"]
mod memlimit;
#[global_allocator]
static ALLOC: memlimit::Limited = memlimit::Limited;

#[path = "common/font.rs"]
mod font;
#[path = "common/net.rs"]
mod net;

use std::{
    env, fs,
    io::Read,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn one(url: &str) {
    let url = url.to_string();
    let h = std::thread::Builder::new().stack_size(256 << 20).spawn(move || one_inner(&url)).unwrap();
    if h.join().is_err() {
        println!("RESULT panic");
    }
}

fn one_inner(url: &str) {
    let start = Instant::now();
    let mut fetcher = net::HostFetcher::new();
    let fonts = font::SystemFont::new().expect("system fonts");
    let mut config = castilia::EngineConfig::default();
    if let Some(m) = env::var("CASTILIA_JS_MODE").ok().and_then(|m| castilia::ExecMode::parse(&m)) {
        config.exec_mode = m;
    }
    config.max_native_depth = 1000;
    let options = castilia::network::LoadOptions { config, code_memory: Some(Box::new(codemem::HostCodeMemory)), interrupt: Some(Box::new(deadline::Deadline::after(std::time::Duration::from_secs(20)))) };
    match castilia::network::load_page_with(&mut fetcher, url, 1024.0, &fonts, options) {
        Ok(page) => {
            let r = &page.report;
            for w in r.warnings.iter().filter(|w| w.starts_with("script") || w.starts_with("module")) {
                let w: String = w.chars().take(300).collect();
                println!("ERR {w}");
            }
            if let Some(p) = &page.page {
                let st = p.vm_stats();
                println!("VM compiled={} fallbacks={} native={}", st.compiled, st.fallbacks, st.native_compiled);
            }
            let text_cmds = page.display.commands.iter().filter(|c| matches!(c, castilia::Command::Text { .. })).count();
            println!(
                "RESULT ok ms={} nodes={} cmds={} text={} height={} scripts={} executed={} errors={}",
                start.elapsed().as_millis(),
                r.dom_nodes,
                r.commands,
                text_cmds,
                page.display.height as i64,
                r.scripts,
                r.scripts_executed,
                r.script_errors
            );
        }
        Err(e) => println!("RESULT fetch-fail ms={} error={}", start.elapsed().as_millis(), e.replace('\n', " ")),
    }
}

struct Outcome {
    url: String,
    status: String,
    line: String,
    errors: Vec<String>,
    ms: u128,
}

fn run_child(exe: &str, url: &str, timeout: Duration, mode: &Option<String>) -> Outcome {
    let start = Instant::now();
    let mut cmd = Command::new(exe);
    cmd.arg("--one").arg(url).env("CASTILIA_MEM_MB", "1024").stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(m) = mode {
        cmd.env("CASTILIA_JS_MODE", m);
    }
    #[cfg(windows)]
    {
        // keep the desktop responsive while pages load
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_4000); // BELOW_NORMAL_PRIORITY_CLASS
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome { url: url.into(), status: "spawn-fail".into(), line: e.to_string(), errors: vec![], ms: 0 },
    };
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    let status_code = child.wait().ok();
    let out = out_t.join().unwrap_or_default();
    let err = err_t.join().unwrap_or_default();
    let ms = start.elapsed().as_millis();
    let errors: Vec<String> = out.lines().filter_map(|l| l.strip_prefix("ERR ")).map(String::from).collect();
    let result = out.lines().find_map(|l| l.strip_prefix("RESULT ")).map(String::from);
    let (status, line) = if timed_out {
        ("timeout".to_string(), String::new())
    } else if let Some(r) = result {
        let st = r.split_whitespace().next().unwrap_or("?").to_string();
        (st, r)
    } else {
        let panic = err.lines().find(|l| l.contains("panicked") || l.contains("overflow")).unwrap_or("").to_string();
        let panic = if panic.is_empty() { err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string() } else { panic };
        (format!("crash({})", status_code.and_then(|s| s.code()).unwrap_or(-1)), panic)
    };
    Outcome { url: url.into(), status, line, errors, ms }
}

fn field(line: &str, key: &str) -> Option<i64> {
    line.split_whitespace().find_map(|t| t.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

fn main() {
    memlimit::init();
    let args: Vec<String> = env::args().collect();
    if args.len() >= 3 && args[1] == "--one" {
        one(&args[2]);
        return;
    }
    let mut list = String::from("tools/sites.txt");
    let mut jobs = 3usize;
    let mut timeout = 90u64;
    let mut mode = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--jobs" => {
                jobs = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--timeout" => {
                timeout = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--mode" => {
                mode = Some(args[i + 1].clone());
                i += 1;
            }
            other => list = other.to_string(),
        }
        i += 1;
    }
    let urls: Vec<String> = fs::read_to_string(&list).expect("site list").lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
    let exe = env::current_exe().unwrap().to_string_lossy().to_string();
    let queue = Arc::new(Mutex::new(urls.clone().into_iter().enumerate().collect::<Vec<_>>()));
    let results: Arc<Mutex<Vec<(usize, Outcome)>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for _ in 0..jobs {
        let (queue, results, exe, mode) = (queue.clone(), results.clone(), exe.clone(), mode.clone());
        handles.push(std::thread::spawn(move || {
            loop {
                let Some((idx, url)) = queue.lock().unwrap().pop() else { break };
                let o = run_child(&exe, &url, Duration::from_secs(timeout), &mode);
                eprintln!("[{:>3}] {:<10} {:>6}ms {} {}", idx, o.status, o.ms, o.url, o.line);
                results.lock().unwrap().push((idx, o));
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let mut results = Arc::try_unwrap(results).ok().unwrap().into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let (mut reachable, mut stable, mut clean) = (0, 0, 0);
    let mut error_hist = std::collections::BTreeMap::<String, usize>::new();
    println!("| # | site | status | ms | text | height | scripts ok/err |");
    println!("|---|---|---|---|---|---|---|");
    for (idx, o) in &results {
        let key = if o.status.starts_with("crash") { "crash".to_string() } else { o.status.clone() };
        *counts.entry(key).or_default() += 1;
        if o.status != "fetch-fail" {
            reachable += 1;
        }
        let text = field(&o.line, "text").unwrap_or(0);
        let ok = o.status == "ok" && text > 0;
        if ok {
            stable += 1;
            if field(&o.line, "errors") == Some(0) {
                clean += 1;
            }
        }
        for e in &o.errors {
            // bucket by the message after the node prefix
            let msg = e.splitn(2, "): ").nth(1).or_else(|| e.splitn(2, ": ").nth(1)).unwrap_or(e);
            let msg: String = msg.chars().take(90).collect();
            *error_hist.entry(msg).or_default() += 1;
        }
        println!(
            "| {} | {} | {} | {} | {} | {} | {}/{} |",
            idx,
            o.url,
            o.status,
            o.ms,
            text,
            field(&o.line, "height").unwrap_or(0),
            field(&o.line, "executed").unwrap_or(0),
            field(&o.line, "errors").unwrap_or(0)
        );
    }
    println!();
    println!("statuses: {counts:?}");
    let pct = |n: usize| if reachable == 0 { 0.0 } else { n as f64 * 100.0 / reachable as f64 };
    println!("reachable: {reachable}/{}  stable (no crash/hang, renders text): {stable} ({:.1}%)  clean (also no script errors): {clean} ({:.1}%)", results.len(), pct(stable), pct(clean));
    let mut hist: Vec<_> = error_hist.into_iter().collect();
    hist.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\ntop script errors:");
    for (m, n) in hist.iter().take(40) {
        println!("{n:>4}  {m}");
    }
}
