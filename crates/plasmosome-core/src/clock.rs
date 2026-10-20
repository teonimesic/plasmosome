use plasmosome_backend::{BootSession, Clock, ClockReading};
use std::fmt;
use std::time::SystemTime;

const CONTINUOUS_CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;

/// The production clock (spec 028 §3). The wall reading is the system's UTC time. The
/// continuous reading counts while the host sleeps and a wall-clock step does not move it:
/// `CLOCK_MONOTONIC_RAW` on macOS and `CLOCK_BOOTTIME` on Linux, never `Instant` or
/// `CLOCK_MONOTONIC`. It is tagged with the boot session read once, at [`SystemClock::new`]. The
/// wait reading is `Instant::now()`.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    boot: BootSession,
}

/// Why the system clock cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockError {
    /// The wall clock reads before 1970-01-01T00:00:00Z.
    BeforeEpoch,
    /// `clock_gettime` refused the continuous clock with this `errno`.
    Continuous { errno: i32 },
    /// The boot session could not be read, or what was read is not a UUID.
    BootSession { detail: String },
}

impl SystemClock {
    /// Reads the boot session, and checks that the wall clock is not before 1970 and that the
    /// continuous clock can be read. After this, [`Clock::now`] does not fail and does not
    /// panic: a wall clock later set before 1970 reads as 1970-01-01T00:00:00Z, and a
    /// continuous clock that refuses a read gives `u64::MAX`, so every bound of this boot
    /// session reads as passed.
    pub fn new() -> Result<SystemClock, ClockError> {
        unimplemented!()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> ClockReading {
        unimplemented!()
    }
}

/// The host boot session: `kern.bootsessionuuid` on macOS and
/// `/proc/sys/kernel/random/boot_id` on Linux, read now. It does not change while a process
/// runs.
pub fn boot_session() -> Result<BootSession, ClockError> {
    unimplemented!()
}

fn wall_ms_at(now: SystemTime) -> Result<u64, ClockError> {
    let _ = now;
    unimplemented!()
}

fn boot_from(value: &[u8], terminator: &[u8]) -> Result<BootSession, ClockError> {
    let _ = (value, terminator);
    unimplemented!()
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        unimplemented!()
    }
}

impl std::error::Error for ClockError {}
