use crate::clock::ClockReading;
use std::time::Instant;
use uuid::Uuid;

/// The shortest lease or idle limit, in milliseconds (spec 028 §1).
pub const MIN_LIMIT_MS: u64 = 1_000;
/// The longest lease or idle limit: 30 days, in milliseconds.
pub const MAX_LIMIT_MS: u64 = 2_592_000_000;
/// The shortest drain budget D, in milliseconds.
pub const MIN_DRAIN_MS: u64 = 1;
/// The longest drain budget D, in milliseconds.
pub const MAX_DRAIN_MS: u64 = 30_000;
/// D when a declaration names no `drain_ms`.
pub const DEFAULT_DRAIN_MS: u64 = 5_000;
/// The default ceiling for an attachment that holds a grant: a 15-minute lease.
pub const DEFAULT_LEASE_MS: u64 = 900_000;
/// The 2 s in H = B + 2 s + D. It is also how long after B the controller may take to start an
/// expiry's removal.
pub const REFERENCE_GRACE_MS: u64 = 2_000;

/// A UTC wall-clock instant, in milliseconds since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallMs(u64);

/// Why a text is not an RFC 3339 UTC instant this crate accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeTextError {
    /// Not `YYYY-MM-DDTHH:MM:SS`, then optionally `.` and one to three digits, then `Z`.
    Form,
    /// The named field is outside its range: `month`, `day` (for that month and year), `hour`,
    /// `minute` or `second`. A leap second, 60, is refused here.
    Field { name: &'static str },
    /// Earlier than 1970-01-01T00:00:00Z.
    BeforeEpoch,
}

impl WallMs {
    /// The instant `ms` milliseconds after 1970-01-01T00:00:00Z.
    pub fn from_unix_ms(ms: u64) -> WallMs {
        WallMs(ms)
    }

    /// Milliseconds since 1970-01-01T00:00:00Z.
    pub fn unix_ms(self) -> u64 {
        self.0
    }

    /// Reads `YYYY-MM-DDTHH:MM:SSZ`, with an optional fraction of one to three digits before
    /// the `Z`. Only uppercase `T` and `Z` are accepted. An offset, `+00:00` included, more
    /// than three fraction digits, surrounding space, a leap second, a year past 9999 and an
    /// instant before 1970 are refused.
    pub fn parse_rfc3339(text: &str) -> Result<WallMs, TimeTextError> {
        let _ = text;
        unimplemented!()
    }

    /// Writes `YYYY-MM-DDTHH:MM:SS.sssZ`, always with three fraction digits. A year past 9999
    /// is written with more digits, which [`WallMs::parse_rfc3339`] refuses.
    pub fn to_rfc3339(self) -> String {
        unimplemented!()
    }

    /// The instant `ms` later, or `None` past what a `u64` of milliseconds holds.
    pub fn checked_add_ms(self, ms: u64) -> Option<WallMs> {
        let _ = ms;
        unimplemented!()
    }
}

/// The host boot session a continuous reading belongs to: `kern.bootsessionuuid` on Darwin and
/// `/proc/sys/kernel/random/boot_id` on Linux (spec 023). `Display` writes it as a lowercase
/// hyphenated UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BootSession(Uuid);

impl BootSession {
    /// The boot session with this identity.
    pub fn from_uuid(uuid: Uuid) -> BootSession {
        BootSession(uuid)
    }

    /// Reads a hyphenated UUID of 36 characters, in either case. Any other text is `None`.
    pub fn parse(text: &str) -> Option<BootSession> {
        let _ = text;
        unimplemented!()
    }

    /// The identity.
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for BootSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        unimplemented!()
    }
}

/// A reading of the continuous clock, or a deadline on it: nanoseconds within one boot session.
/// The clock counts while the host sleeps, and setting the wall clock does not move it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Continuous {
    pub boot: BootSession,
    pub ns: u64,
}

/// The moment B a lifetime ends, as two readings (spec 028 §3). B has passed once either has. A
/// continuous deadline from another boot session than the current reading's is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub not_after: WallMs,
    pub continuous: Continuous,
}

impl Bound {
    /// The bound `ms` after `now`, with both readings taken from that one reading. `None` when
    /// either reading would overflow.
    pub fn after(now: &ClockReading, ms: u64) -> Option<Bound> {
        let _ = (now, ms);
        unimplemented!()
    }

    /// True once the wall reading reaches `not_after`, or the continuous reading of the same
    /// boot session reaches the continuous deadline. At B itself it has passed.
    pub fn passed(&self, now: &ClockReading) -> bool {
        let _ = now;
        unimplemented!()
    }

    /// Whole milliseconds left on the reading with less time left, and 0 once that one has
    /// passed. With another boot session it is the wall reading's.
    pub fn remaining_ms(&self, now: &ClockReading) -> u64 {
        let _ = now;
        unimplemented!()
    }

    /// The earlier of two bounds, each reading taken separately. When the two continuous
    /// deadlines are from different boot sessions they cannot be compared, so the result is
    /// `other` whole when its wall reading is earlier, and `self` otherwise.
    pub fn earlier(self, other: Bound) -> Bound {
        let _ = other;
        unimplemented!()
    }

    /// Where the bound plus `extra_ms` falls on the wait clock, from this one reading:
    /// `now.wait` plus [`Bound::remaining_ms`] plus `extra_ms`. The wait clock can stop while
    /// the host sleeps, so a caller derives this again at each reading and never stores it.
    /// The sum saturates at `u64::MAX` milliseconds, which an `Instant` on macOS and Linux
    /// holds; where one cannot, the deadline is `now.wait`.
    pub fn wait_deadline(&self, now: &ClockReading, extra_ms: u64) -> Instant {
        let _ = (now, extra_ms);
        unimplemented!()
    }
}

/// A lease, idle limit or drain budget outside its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeError {
    /// What was checked: `"a lease or idle limit"` or `"a drain budget"`.
    pub limit: &'static str,
    pub value: u64,
    pub min: u64,
    pub max: u64,
}

/// Returns `ms` when it is a lease or idle limit spec 028 allows: 1,000 ms to 30 days, inclusive.
pub fn check_limit_ms(ms: u64) -> Result<u64, RangeError> {
    let _ = ms;
    unimplemented!()
}

/// Returns `ms` when it is a drain budget spec 028 allows: 1 to 30,000 ms, inclusive.
pub fn check_drain_ms(ms: u64) -> Result<u64, RangeError> {
    let _ = ms;
    unimplemented!()
}

impl std::fmt::Display for TimeTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        unimplemented!()
    }
}

impl std::error::Error for TimeTextError {}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        unimplemented!()
    }
}

impl std::error::Error for RangeError {}
