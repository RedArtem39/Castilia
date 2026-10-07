//! Global allocator that aborts the process past a fixed heap budget, so a runaway script in a
//! test run can never push the machine into swap. Limit: `CASTILIA_MEM_MB` (default 1536).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Limited;

static USED: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(1536 << 20);
static PEAK: AtomicUsize = AtomicUsize::new(0);

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
}

fn over(size: usize) -> bool {
    let now = USED.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
    now > LIMIT.load(Ordering::Relaxed)
}

fn out_of_memory() -> ! {
    // no allocation here: write directly and leave
    use std::io::Write;
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
