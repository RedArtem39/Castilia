//! Wall-clock budget per script task (an `Interrupt` implementation for desktop hosts).
use castilia::platform::Interrupt;
use std::time::{Duration, Instant};

pub struct Deadline {
    per_task: Duration,
    end: Instant,
}

impl Deadline {
    pub fn per_task(d: Duration) -> Deadline {
        Deadline { per_task: d, end: Instant::now() + d }
    }
}

impl Interrupt for Deadline {
    fn should_stop(&mut self) -> bool {
        Instant::now() >= self.end
    }
    fn task_started(&mut self) {
        self.end = Instant::now() + self.per_task;
    }
}
