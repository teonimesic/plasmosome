use crate::lifetime::{BootSession, Continuous, WallMs};
use std::sync::Mutex;
use std::time::Instant;

/// One reading of the three clocks a lifetime decision uses, taken together (spec 028 §3).
#[derive(Clone, Copy, Debug)]
pub struct ClockReading {
    /// UTC wall-clock time. An operator or a time service can step it either way.
    pub wall: WallMs,
    /// The continuous clock, with its boot session.
    pub continuous: Continuous,
    /// The clock waits and timeouts use. It can stop while the host sleeps, so a deadline on it
    /// is derived again at each reading.
    pub wait: Instant,
}

/// Where the controller and the supervisor read time. Tests pass a [`ManualClock`]; production
/// passes the system clock in `plasmosome-core`.
pub trait Clock: Send + Sync {
    /// The three readings, taken together.
    fn now(&self) -> ClockReading;
}

/// A clock for tests. Each of its three readings moves only when the test moves it, and
/// independently of the other two.
#[derive(Debug)]
pub struct ManualClock {
    reading: Mutex<ClockReading>,
}

impl ManualClock {
    /// Starts with the wall reading at `wall`, the continuous reading at `continuous_ns` in
    /// `boot`, and the wait reading at the real `Instant::now()`.
    pub fn new(wall: WallMs, boot: BootSession, continuous_ns: u64) -> ManualClock {
        let _ = (wall, boot, continuous_ns);
        unimplemented!()
    }

    /// Steps the wall reading by `ms`, back when negative. It stops at 1970-01-01T00:00:00Z.
    pub fn advance_wall_ms(&self, ms: i64) {
        let _ = ms;
        unimplemented!()
    }

    /// Moves the continuous reading forward by `ms`.
    pub fn advance_continuous_ms(&self, ms: u64) {
        let _ = ms;
        unimplemented!()
    }

    /// Moves the wait reading forward by `ms`. Panics past what an `Instant` holds.
    pub fn advance_wait_ms(&self, ms: u64) {
        let _ = ms;
        unimplemented!()
    }

    /// Moves all three readings forward by `ms`, as time passing on an awake host does.
    pub fn advance_all_ms(&self, ms: u64) {
        let _ = ms;
        unimplemented!()
    }

    /// Starts a new boot session, with the continuous reading at `continuous_ns`.
    pub fn set_boot(&self, boot: BootSession, continuous_ns: u64) {
        let _ = (boot, continuous_ns);
        unimplemented!()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> ClockReading {
        unimplemented!()
    }
}
