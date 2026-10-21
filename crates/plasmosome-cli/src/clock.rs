use std::time::{Duration, Instant};

/// Where the command line reads time. A command's deadline is counted on it from the command's
/// start. The binary passes [`SystemClock`]; a test passes a clock it controls, from any thread.
pub trait Clock: Sync {
    /// A monotonic reading.
    fn now(&self) -> Instant;
}

/// The production clock: `Instant::now()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The moment a command must be done by, on one clock.
pub struct Deadline<'a> {
    clock: &'a dyn Clock,
    at: Instant,
    ms: u32,
}

impl<'a> Deadline<'a> {
    /// The deadline `ms` milliseconds after `start`, a reading of `clock`.
    pub fn new(clock: &'a dyn Clock, start: Instant, ms: u32) -> Deadline<'a> {
        Deadline {
            clock,
            at: start + Duration::from_millis(u64::from(ms)),
            ms,
        }
    }

    /// The time left, read from the clock now. It is zero once the deadline has passed.
    pub fn left(&self) -> Duration {
        self.at.saturating_duration_since(self.clock.now())
    }

    /// The deadline's length in milliseconds, which a `deadline` refusal reports.
    pub fn ms(&self) -> u32 {
        self.ms
    }
}
