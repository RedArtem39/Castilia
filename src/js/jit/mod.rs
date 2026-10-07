//! Native tier: a baseline compiler from bytecode to machine code.
//!
//! The design follows "Sparkplug"-style baseline JITs: native code keeps the interpreter's frame
//! layout (operand stack, registers, pc), handles simple instructions inline (numbers, booleans,
//! register moves, comparisons and branches) and calls back into the interpreter's own handler
//! for everything else. Leaving native code at any instruction is therefore free, and correctness
//! never depends on the native tier: each fast path has the interpreter as its slow path.
//!
//! Executable memory comes from the host through [`CodeMemory`]; without it, on CPUs without a
//! backend, or when the crate is built without the `jit` feature, functions stay interpreted.
use super::{
    interp::Interp,
    value::*,
    vm::{exec::Step, Code, Frame, Op},
};
use crate::platform::CodeMemory;
use alloc::{boxed::Box, vec::Vec};
use core::ptr::NonNull;

#[cfg(target_arch = "x86_64")]
mod x64;

/// Per-runtime native tier state.
pub struct JitState {
    pub threshold: u32,
    pub memory: Option<Box<dyn CodeMemory>>,
}

impl Default for JitState {
    fn default() -> Self {
        JitState { threshold: 200, memory: None }
    }
}

/// Native code attached to a compiled function.
#[derive(Default)]
pub enum NativeSlot {
    #[default]
    Untried,
    Failed,
    Ready(Native),
}

pub struct Native {
    pub entry: NonNull<u8>,
    /// Offset of each bytecode instruction's native code from `entry`.
    pub table: Vec<u32>,
}

/// Shared state between native code and the slow-path helpers. Field offsets are part of the
/// native calling contract (see the backends).
#[repr(C)]
pub(crate) struct JitCtx {
    pub stack_ptr: *mut Value, // 0
    pub sp: usize,             // 8
    pub cap: usize,            // 16
    pub regs: *mut Value,      // 24
    pub pc: usize,             // 32
    pub interp: *mut Interp,   // 40
    pub frame: *mut Frame,     // 48
    pub code: *const Code,     // 56
    pub budget: i64,           // 64
    pub table: *const u32,     // 72
    pub base: *const u8,       // 80
    pub out: *mut Option<Result<Step, Ctl>>, // 88
}

/// Loop back-edges between work-budget checks in native code.
pub const TICK_CHUNK: i64 = 4096;

/// Status returned by native code and helpers.
pub const ST_NEXT: u32 = 0;
pub const ST_JUMP: u32 = 1;
pub const ST_EXIT: u32 = 2;

type Entry = unsafe extern "C" fn(*mut JitCtx) -> u32;

impl JitCtx {
    fn sync_from(&mut self, f: &mut Frame) {
        if f.stack.capacity() - f.stack.len() < 8 {
            f.stack.reserve(64);
        }
        self.stack_ptr = f.stack.as_mut_ptr();
        self.sp = f.stack.len();
        self.cap = f.stack.capacity();
        self.regs = f.regs.as_mut_ptr();
    }
}

/// Slow path: run instruction `pc` with the interpreter.
///
/// # Safety
/// Called only from native code generated for `ctx.code`, with a valid context.
pub(crate) unsafe extern "C" fn helper_generic(ctx: *mut JitCtx, pc: u32) -> u32 {
    // SAFETY: the context and everything it points to are live for the duration of the native call
    // (see `Interp::jit_enter`); no other Rust reference to the frame exists meanwhile.
    let ctx = unsafe { &mut *ctx };
    let it = unsafe { &mut *ctx.interp };
    let f = unsafe { &mut *ctx.frame };
    let code = unsafe { &*ctx.code };
    // SAFETY: native code only ever leaves initialised values below `sp`.
    unsafe { f.stack.set_len(ctx.sp) };
    let pc = pc as usize;
    let op = code.ops[pc];
    if let Op::Jump(t) = op {
        if (t as usize) <= pc {
            // budget chunk used up by native back-edges
            it.ticks += TICK_CHUNK as u64;
            ctx.budget = TICK_CHUNK;
        }
    }
    f.pc = pc + 1;
    let r = it.exec_op(f, code, op);
    ctx.sync_from(f);
    match r {
        Ok(None) => {
            if f.pc == pc + 1 {
                ST_NEXT
            } else {
                ctx.pc = f.pc;
                ST_JUMP
            }
        }
        Ok(Some(step)) => {
            unsafe { *ctx.out = Some(Ok(step)) };
            ST_EXIT
        }
        Err(e) => {
            unsafe { *ctx.out = Some(Err(e)) };
            ST_EXIT
        }
    }
}

impl Interp {
    /// Run `f` in native code if its function is (or just became) compiled. `None` keeps the
    /// interpreter in charge.
    pub(crate) fn jit_enter(&mut self, f: &mut Frame) -> Option<Result<Step, Ctl>> {
        let code = f.code.clone();
        let entry = {
            let slot = code.native.borrow();
            match &*slot {
                NativeSlot::Ready(n) => Some((n.entry, n.table.as_ptr())),
                NativeSlot::Failed => return None,
                NativeSlot::Untried => None,
            }
        };
        let (entry, table) = match entry {
            Some(e) => e,
            None => {
                let hot = code.hot.get();
                if hot < self.jit.threshold {
                    code.hot.set(hot + 1);
                    return None;
                }
                self.jit_compile(&code)?
            }
        };
        let mut out: Option<Result<Step, Ctl>> = None;
        let mut ctx = JitCtx {
            stack_ptr: core::ptr::null_mut(),
            sp: 0,
            cap: 0,
            regs: core::ptr::null_mut(),
            pc: f.pc,
            interp: self as *mut Interp,
            frame: f as *mut Frame,
            code: &*code as *const Code,
            budget: TICK_CHUNK,
            table,
            base: entry.as_ptr(),
            out: &mut out,
        };
        ctx.sync_from(f);
        // SAFETY: `entry` was produced by our backend for this code object and installed in
        // executable memory by the host; the context outlives the call.
        let entry: Entry = unsafe { core::mem::transmute::<*const u8, Entry>(entry.as_ptr()) };
        let status = unsafe { entry(&mut ctx) };
        // SAFETY: native code stores its stack depth in `ctx.sp` before returning.
        unsafe { f.stack.set_len(ctx.sp) };
        match status {
            ST_EXIT => out,
            _ => {
                f.pc = ctx.pc;
                None
            }
        }
    }

    fn jit_compile(&mut self, code: &Code) -> Option<(NonNull<u8>, *const u32)> {
        let compiled = self.jit_build(code);
        let mut slot = code.native.borrow_mut();
        match compiled {
            Some(n) => {
                let r = (n.entry, n.table.as_ptr());
                *slot = NativeSlot::Ready(n);
                self.vm_stats.native_compiled += 1;
                Some(r)
            }
            None => {
                *slot = NativeSlot::Failed;
                self.vm_stats.native_failed += 1;
                None
            }
        }
    }

    #[allow(unused_variables)]
    fn jit_build(&mut self, code: &Code) -> Option<Native> {
        #[cfg(target_arch = "x86_64")]
        {
            let (bytes, table) = x64::compile(code)?;
            let Some(mem) = self.jit.memory.as_mut() else {
                self.vm_stats.native_no_memory += 1;
                return None;
            };
            let entry = mem.install(&bytes)?;
            return Some(Native { entry, table });
        }
        #[allow(unreachable_code)]
        None
    }
}
