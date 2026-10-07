//! Services a port provides to the engine.
//!
//! The core is `#![no_std]` + `alloc` and never touches the OS. A port implements:
//!
//! | Service | Trait | Required |
//! |---|---|---|
//! | Text measurement, image sizes | [`FontMetrics`](crate::layout::FontMetrics) | yes |
//! | HTTP(S), URL resolution | [`Fetcher`](crate::network::Fetcher) | for networked pages |
//! | `fetch`/XHR, dialogs, navigation | [`Host`](crate::js::Host) | for interactive pages |
//! | Executable memory for the JIT | [`CodeMemory`] | only for [`ExecMode::Jit`](crate::config::ExecMode) |
//!
//! Painting consumes the platform-independent [`DisplayList`](crate::layout::DisplayList).
//! Time is virtual (the engine advances it when the host calls `Page::advance_time`), so no clock
//! is needed and runs are deterministic.

use core::ptr::NonNull;

/// Executable memory for native code produced by the JIT.
///
/// `install` must copy `code` into memory that is executable (and, on W^X systems, no longer
/// writable) and return its address. Returning `None` keeps the function interpreted. The memory
/// must stay valid and unchanged for as long as the runtime that installed it is alive.
pub trait CodeMemory {
    fn install(&mut self, code: &[u8]) -> Option<NonNull<u8>>;
}

/// The CPU architecture the native tier would generate code for, if any.
pub fn jit_target() -> Option<&'static str> {
    if cfg!(all(feature = "jit", target_arch = "x86_64")) { Some("x86_64") } else { None }
}
