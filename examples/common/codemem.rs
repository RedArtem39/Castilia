//! Executable memory for the JIT on desktop hosts (W^X: pages are written, then made read+execute).
use castilia::platform::CodeMemory;
use std::ptr::NonNull;

pub struct HostCodeMemory;

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const PAGE_READWRITE: u32 = 0x04;
    const PAGE_EXECUTE_READ: u32 = 0x20;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(addr: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }
    pub fn install(code: &[u8]) -> Option<*mut u8> {
        unsafe {
            let p = VirtualAlloc(std::ptr::null_mut(), code.len(), MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) as *mut u8;
            if p.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(code.as_ptr(), p, code.len());
            let mut old = 0;
            if VirtualProtect(p as *mut c_void, code.len(), PAGE_EXECUTE_READ, &mut old) == 0 {
                return None;
            }
            FlushInstructionCache(GetCurrentProcess(), p as *const c_void, code.len());
            Some(p)
        }
    }
}

#[cfg(unix)]
mod sys {
    use std::ffi::c_void;
    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;
    const MAP_PRIVATE: i32 = 2;
    #[cfg(target_os = "macos")]
    const MAP_ANON: i32 = 0x1000;
    #[cfg(not(target_os = "macos"))]
    const MAP_ANON: i32 = 0x20;
    unsafe extern "C" {
        fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
    }
    pub fn install(code: &[u8]) -> Option<*mut u8> {
        unsafe {
            let p = mmap(std::ptr::null_mut(), code.len(), PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
            if p as isize == -1 {
                return None;
            }
            std::ptr::copy_nonoverlapping(code.as_ptr(), p as *mut u8, code.len());
            if mprotect(p, code.len(), PROT_READ | PROT_EXEC) != 0 {
                return None;
            }
            Some(p as *mut u8)
        }
    }
}

impl CodeMemory for HostCodeMemory {
    fn install(&mut self, code: &[u8]) -> Option<NonNull<u8>> {
        NonNull::new(sys::install(code)?)
    }
}
