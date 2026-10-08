//! Engine configuration: execution mode and resource limits.
//!
//! Everything a port may want to tune lives here, so a host never has to patch the engine:
//! pick [`ExecMode::Interpreter`] on platforms that forbid writable+executable memory (iOS,
//! consoles, W^X kernels, WebAssembly) and [`ExecMode::Jit`] where the host can provide
//! [`CodeMemory`](crate::platform::CodeMemory). Limits scale the engine down for small devices.

/// How JavaScript functions are executed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecMode {
    /// Portable bytecode interpreter only. Needs no executable memory and no architecture backend.
    Interpreter,
    /// Bytecode interpreter plus a baseline native compiler for hot functions. Falls back to
    /// [`ExecMode::Interpreter`] when the crate is built without the `jit` feature, the CPU
    /// architecture has no backend, or the host supplies no [`CodeMemory`](crate::platform::CodeMemory).
    Jit,
    /// Reference AST walker (no bytecode, no real generator suspension). Differential testing only.
    AstWalker,
}

impl ExecMode {
    /// Parse `interp`/`interpreter`, `jit`, `ast` (for command-line and environment switches).
    pub fn parse(s: &str) -> Option<ExecMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "interp" | "interpreter" | "bytecode" | "vm" => Some(ExecMode::Interpreter),
            "jit" | "native" => Some(ExecMode::Jit),
            "ast" | "tree" | "astwalker" => Some(ExecMode::AstWalker),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ExecMode::Interpreter => "interpreter",
            ExecMode::Jit => "jit",
            ExecMode::AstWalker => "ast",
        }
    }

    /// Whether this build can ever produce native code.
    pub const fn jit_compiled_in() -> bool {
        cfg!(feature = "jit")
    }
}

/// Tunable limits and execution policy for one page/runtime.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub exec_mode: ExecMode,
    /// Function entries + loop back-edges before a function is compiled to native code.
    pub jit_threshold: u32,
    /// Work budget (statements, calls, loop iterations) per script, event or timer task.
    /// A task that exceeds it throws an uncatchable-in-practice `Error` and the page goes on.
    pub task_budget: u64,
    /// Nesting of native ↔ JS re-entry (callbacks from built-ins, getters, AST fallback).
    /// Each level costs host stack; raise it only when the engine runs on a large stack.
    pub max_native_depth: usize,
    /// JS → JS calls handled by the bytecode loop (no host stack involved).
    pub max_frames: usize,
    /// Virtual milliseconds of timers run after `load` so `setTimeout`-driven pages settle.
    pub settle_ms: f64,
    /// Upper bound on timer callbacks fired while settling.
    pub settle_fires: usize,
    /// Objects a single task may allocate before it is stopped (runaway scripts).
    pub max_task_allocs: usize,
    /// Longest string scripts may build, in bytes.
    pub max_string_len: usize,
    /// Allocations between cycle collections (0 disables the collector).
    pub gc_threshold: usize,
    /// Log the functions each uncaught exception unwinds through (debugging).
    pub trace_errors: bool,
    /// Maximum number of `<script>` elements executed per page.
    pub max_scripts: usize,
    /// Scripts larger than this (bytes) are skipped.
    pub max_script_bytes: usize,
    /// Host stack the engine may use below the point where a task starts (bytes); deeper
    /// recursion throws `RangeError` instead of overflowing. Keep it under the thread's stack size.
    pub max_stack_bytes: usize,
}

impl EngineConfig {
    /// Desktop defaults: JIT when available.
    pub fn desktop() -> EngineConfig {
        EngineConfig {
            exec_mode: if cfg!(feature = "jit") { ExecMode::Jit } else { ExecMode::Interpreter },
            jit_threshold: 200,
            task_budget: 150_000_000,
            max_native_depth: 180,
            max_frames: 10_000,
            settle_ms: 1500.0,
            settle_fires: 3000,
            gc_threshold: 200_000,
            max_task_allocs: 8_000_000,
            max_string_len: 256 << 20,
            trace_errors: false,
            max_scripts: 256,
            max_script_bytes: 24 * 1024 * 1024,
            max_stack_bytes: 1 << 20,
        }
    }

    /// Interpreter-only configuration (no executable memory needed).
    pub fn interpreter() -> EngineConfig {
        EngineConfig { exec_mode: ExecMode::Interpreter, ..EngineConfig::desktop() }
    }

    /// Native tier enabled (silently degrades to the interpreter where unsupported).
    pub fn jit() -> EngineConfig {
        EngineConfig { exec_mode: ExecMode::Jit, ..EngineConfig::desktop() }
    }

    /// Small devices: interpreter, tighter budgets, shallow host stack.
    pub fn small_device() -> EngineConfig {
        EngineConfig {
            exec_mode: ExecMode::Interpreter,
            task_budget: 20_000_000,
            max_native_depth: 64,
            max_frames: 2_000,
            settle_ms: 500.0,
            settle_fires: 500,
            gc_threshold: 50_000,
            max_task_allocs: 1_000_000,
            max_string_len: 32 << 20,
            max_scripts: 64,
            max_script_bytes: 4 * 1024 * 1024,
            ..EngineConfig::desktop()
        }
    }

    /// The tier the runtime actually uses for this configuration.
    pub fn tier(&self) -> crate::js::vm::Tier {
        use crate::js::vm::Tier;
        match self.exec_mode {
            ExecMode::AstWalker => Tier::Ast,
            ExecMode::Interpreter => Tier::Bytecode,
            ExecMode::Jit if cfg!(feature = "jit") => Tier::Native,
            ExecMode::Jit => Tier::Bytecode,
        }
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig::desktop()
    }
}
