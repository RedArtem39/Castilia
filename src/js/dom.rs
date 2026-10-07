//! DOM/BOM bindings (filled in incrementally).
use alloc::{boxed::Box, string::String};

#[derive(Default)]
pub struct HostSlot {
    pub host: Option<Box<dyn Host>>,
}

/// Services the embedder may provide to scripts.
pub trait Host {
    fn fetch(&mut self, _url: &str) -> Result<String, String> {
        Err(String::from("network unavailable"))
    }
}

pub struct DomState {
    pub base_url: String,
    /// Wall-clock origin (ms since the Unix epoch) corresponding to virtual time 0.
    pub epoch_ms: f64,
}

impl Default for DomState {
    fn default() -> Self {
        DomState { base_url: String::new(), epoch_ms: 1_767_225_600_000.0 }
    }
}
