//! Wall-clock budget per script task (an `Interrupt` implementation for desktop hosts).
//! Time spent waiting for the network is not charged to the script.
use castilia::platform::Interrupt;
use std::time::{Duration, Instant};

pub struct Deadline {
    per_task: Duration,
    end: Instant,
    io_since: Option<Instant>,
}

impl Deadline {
    pub fn per_task(d: Duration) -> Deadline {
        Deadline { per_task: d, end: Instant::now() + d, io_since: None }
    }
}

impl Interrupt for Deadline {
    fn should_stop(&mut self) -> bool {
        Instant::now() >= self.end
    }
    fn task_started(&mut self) {
        self.end = Instant::now() + self.per_task;
    }
    fn io_begin(&mut self) {
        self.io_since = Some(Instant::now());
    }
    fn io_end(&mut self) {
        if let Some(s) = self.io_since.take() {
            self.end += s.elapsed();
        }
    }
}
