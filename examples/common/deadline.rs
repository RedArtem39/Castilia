//! Wall-clock budget for all scripts of a page (an `Interrupt` implementation for desktop hosts).
use castilia::platform::Interrupt;
use std::time::{Duration, Instant};

pub struct Deadline {
    end: Instant,
}

impl Deadline {
    pub fn after(d: Duration) -> Deadline {
        Deadline { end: Instant::now() + d }
    }
}

impl Interrupt for Deadline {
    fn should_stop(&mut self) -> bool {
        Instant::now() >= self.end
    }
}
