//! Global allocator that aborts the process past a fixed heap budget, so a runaway script in a
//! test run can never push the machine into swap. Limit: `CASTILIA_MEM_MB` (default 1536).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Limited;

static USED: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(1536 << 20);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// `CASTILIA_OOM_TRACE` read once at startup (the allocator itself must not allocate).
static OOM_TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Highest heap usage seen so far (bytes).
#[allow(dead_code)]
pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

/// Current heap usage (bytes).
#[allow(dead_code)]
pub fn used() -> usize {
    USED.load(Ordering::Relaxed)
}

pub fn init() {
    if let Some(mb) = std::env::var("CASTILIA_MEM_MB").ok().and_then(|v| v.parse::<usize>().ok()) {
        LIMIT.store(mb << 20, Ordering::Relaxed);
    }
    OOM_TRACE.store(std::env::var_os("CASTILIA_OOM_TRACE").is_some(), Ordering::Relaxed);
}

fn over(size: usize) -> bool {
    let now = USED.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
    now > LIMIT.load(Ordering::Relaxed)
}

fn out_of_memory() -> ! {
    use std::io::Write;
    // CASTILIA_OOM_TRACE: lift the cap and print where the budget ran out (diagnostics only)
    static TRACING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if OOM_TRACE.load(Ordering::Relaxed) && !TRACING.swap(true, Ordering::SeqCst) {
        LIMIT.store(usize::MAX, Ordering::Relaxed);
        let _ = writeln!(std::io::stderr(), "memory limit exceeded at (innermost frames):");
        let mut n = 0;
        backtrace::trace(|frame| {
            backtrace::resolve_frame(frame, |sym| {
                let name = sym.name().map(|n| n.to_string()).unwrap_or_default();
                let line = sym.lineno().unwrap_or(0);
                let file = sym.filename().and_then(|f| f.file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                let _ = writeln!(std::io::stderr(), "  {name} ({file}:{line})");
            });
            n += 1;
            n < 80
        });
        std::process::exit(3)
    }
    // no allocation here: write directly and leave
    let _ = std::io::stderr().write_all(b"memory limit exceeded\n");
    std::process::exit(3)
}

unsafe impl GlobalAlloc for Limited {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if over(l.size()) {
            out_of_memory();
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if over(l.size()) {
            out_of_memory();
        }
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        USED.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if new > l.size() && over(new - l.size()) {
            out_of_memory();
        } else if new < l.size() {
            USED.fetch_sub(l.size() - new, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, l, new) }
    }
}
