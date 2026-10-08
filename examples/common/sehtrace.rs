//! Windows diagnostics: on a stack overflow print the innermost frames instead of only
//! "has overflowed its stack". Enabled by `CASTILIA_SEH=1`; build with line tables for file:line.
#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io::Write;

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *mut ExceptionRecord,
        address: *mut c_void,
    }
    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }
    unsafe extern "system" {
        fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
        fn SetThreadStackGuarantee(size: *mut u32) -> i32;
        fn RtlCaptureStackBackTrace(skip: u32, count: u32, frames: *mut *mut c_void, hash: *mut u32) -> u16;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }
    const STACK_OVERFLOW: u32 = 0xC000_00FD;

    unsafe extern "system" fn handler(info: *mut ExceptionPointers) -> i32 {
        let code = unsafe { (*(*info).record).code };
        if code != STACK_OVERFLOW {
            return 0; // continue search
        }
        let mut frames = [std::ptr::null_mut::<c_void>(); 48];
        let n = unsafe { RtlCaptureStackBackTrace(0, 48, frames.as_mut_ptr(), std::ptr::null_mut()) } as usize;
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        let mut err = std::io::stderr();
        let _ = write!(err, "stack overflow; frame offsets (resolve with `browse --resolve ...`):");
        for f in &frames[..n] {
            let _ = write!(err, " {:x}", (*f as usize).wrapping_sub(base));
        }
        let _ = writeln!(err);
        std::process::exit(4)
    }

    /// Install the handler and reserve stack for it on the calling thread.
    pub fn install_on_this_thread() {
        if std::env::var_os("CASTILIA_SEH").is_none() {
            return;
        }
        unsafe {
            let mut size: u32 = 512 * 1024;
            SetThreadStackGuarantee(&mut size);
            AddVectoredExceptionHandler(1, handler);
        }
    }
}

#[cfg(windows)]
pub use imp::install_on_this_thread;

/// Print function and line for module-relative code offsets (hex) printed by the handler.
#[cfg(windows)]
pub fn resolve(offsets: &[String]) {
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
    }
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    for o in offsets {
        let Ok(off) = usize::from_str_radix(o, 16) else { continue };
        let mut line = format!("{o}:");
        backtrace::resolve((base + off) as *mut std::ffi::c_void, |sym| {
            let name = sym.name().map(|n| n.to_string()).unwrap_or_default();
            let file = sym.filename().and_then(|f| f.file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            line.push_str(&format!(" {name} ({file}:{})", sym.lineno().unwrap_or(0)));
        });
        println!("{line}");
    }
}

#[cfg(not(windows))]
pub fn resolve(_: &[String]) {}

#[cfg(not(windows))]
pub fn install_on_this_thread() {}
