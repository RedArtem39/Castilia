//! Native tier (baseline compiler). Filled in by the architecture backends.
use crate::platform::CodeMemory;
use alloc::boxed::Box;

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
}
