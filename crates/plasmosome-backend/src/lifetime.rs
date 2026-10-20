use crate::clock::ClockReading;
use std::time::{Duration, Instant};
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

const NS_PER_MS: u64 = 1_000_000;
const MS_PER_DAY: u64 = 86_400_000;
const EPOCH_YEAR: u64 = 1970;

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
        let bytes = text.as_bytes();
        let fraction = match bytes.len() {
            20 => &[][..],
            22..=24 if bytes[19] == b'.' => &bytes[20..bytes.len() - 1],
            _ => return Err(TimeTextError::Form),
        };
        let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
        if separators.iter().any(|&(at, byte)| bytes[at] != byte) || bytes[bytes.len() - 1] != b'Z'
        {
            return Err(TimeTextError::Form);
        }
        let number = |digits: &[u8]| {
            digits.iter().try_fold(0u64, |value, &digit| {
                digit
                    .is_ascii_digit()
                    .then(|| value * 10 + u64::from(digit - b'0'))
            })
        };
        let field = |from: usize, to: usize| number(&bytes[from..to]).ok_or(TimeTextError::Form);
        let year = field(0, 4)?;
        let month = field(5, 7)?;
        let day = field(8, 10)?;
        let hour = field(11, 13)?;
        let minute = field(14, 16)?;
        let second = field(17, 19)?;
        let mut millis = number(fraction).ok_or(TimeTextError::Form)?;
        for _ in fraction.len()..3 {
            millis *= 10;
        }
        let out_of_range = |name| Err(TimeTextError::Field { name });
        if !(1..=12).contains(&month) {
            return out_of_range("month");
        }
        if !(1..=days_in_month(year, month)).contains(&day) {
            return out_of_range("day");
        }
        if hour > 23 {
            return out_of_range("hour");
        }
        if minute > 59 {
            return out_of_range("minute");
        }
        if second > 59 {
            return out_of_range("second");
        }
        if year < EPOCH_YEAR {
            return Err(TimeTextError::BeforeEpoch);
        }
        let seconds =
            days_since_epoch(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
        Ok(WallMs(seconds * 1_000 + millis))
    }

    /// Writes `YYYY-MM-DDTHH:MM:SS.sssZ`, always with three fraction digits. A year past 9999
    /// is written with more digits, which [`WallMs::parse_rfc3339`] refuses.
    pub fn to_rfc3339(self) -> String {
        let (year, month, day) = civil_from_days(self.0 / MS_PER_DAY);
        let of_day = self.0 % MS_PER_DAY;
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
            of_day / 3_600_000,
            of_day / 60_000 % 60,
            of_day / 1_000 % 60,
            of_day % 1_000
        )
    }

    /// The instant `ms` later, or `None` past what a `u64` of milliseconds holds.
    pub fn checked_add_ms(self, ms: u64) -> Option<WallMs> {
        self.0.checked_add(ms).map(WallMs)
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
        if text.len() != 36 {
            return None;
        }
        Uuid::try_parse(text).ok().map(BootSession)
    }

    /// The identity.
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for BootSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

/// A reading of the continuous clock, or a deadline on it: nanoseconds within one boot session.
/// The clock counts while the host sleeps, and setting the wall clock does not move it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Continuous {
    pub boot: BootSession,
    pub ns: u64,
}

/// The moment B a lifetime ends, as two readings (spec 028 §3). B has passed at a reading when
/// either of the two has reached its deadline there. A continuous deadline from another boot
/// session than the reading's is ignored.
///
/// A bound keeps no state, and nothing records that B passed. [`Bound::passed`],
/// [`Bound::remaining_ms`] and [`Bound::wait_deadline`] each look only at the reading they are
/// given. So after the wall clock is set back, a bound that had passed only on its wall reading
/// reads as not passed, with time left, and its wait deadline moves later.
///
/// A caller that keeps a fence follows one rule. At the fence, it moves the continuous deadline
/// to where B fell on that reading: the reading's continuous value less the time overdue,
/// saturating at 0, in the reading's boot session. The time overdue is `now.wait` less
/// `wait_deadline(now, 0)`, both from that reading. The bound then reads as passed at every
/// later reading of that boot session. Unless the subtraction saturated, its wait deadline
/// stays where it was at the fence while the host is awake, and a sleep only moves it earlier.
/// `bound.earlier(Bound::after(&now, 0))` is not enough: it moves the continuous deadline to
/// the reading at the fence, not to B, so after a wall step back the wait deadline is later by
/// the time overdue at the fence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bound {
    pub not_after: WallMs,
    pub continuous: Continuous,
}

impl Bound {
    /// The bound `ms` after `now`, with both readings taken from that one reading. `None` when
    /// either reading would overflow.
    pub fn after(now: &ClockReading, ms: u64) -> Option<Bound> {
        let ns = now.continuous.ns.checked_add(ms.checked_mul(NS_PER_MS)?)?;
        Some(Bound {
            not_after: now.wall.checked_add_ms(ms)?,
            continuous: Continuous {
                boot: now.continuous.boot,
                ns,
            },
        })
    }

    /// True when the wall reading is at or past `not_after`, or the continuous reading of the
    /// same boot session is at or past the continuous deadline. At B itself it has passed. It
    /// reads only `now`, so after a wall step back a bound that had passed only on its wall
    /// reading is false again; [`Bound`] gives the rule for keeping a fence.
    pub fn passed(&self, now: &ClockReading) -> bool {
        now.wall >= self.not_after
            || (now.continuous.boot == self.continuous.boot
                && now.continuous.ns >= self.continuous.ns)
    }

    /// Milliseconds left on the reading with less time left, rounded up, so it is 0 exactly
    /// when [`Bound::passed`] is true. With another boot session it is the wall reading's. It
    /// reads only `now`, so after a wall step back a bound that had passed only on its wall
    /// reading has time left again; [`Bound`] gives the rule for keeping a fence.
    pub fn remaining_ms(&self, now: &ClockReading) -> u64 {
        let left = u128::try_from(self.remaining_ns(now)).unwrap_or(0);
        u64::try_from(left.div_ceil(u128::from(NS_PER_MS))).unwrap_or(u64::MAX)
    }

    /// The earlier of two bounds, each reading taken separately. This is exact only when both
    /// are from one boot session. Across sessions the continuous deadlines cannot be compared,
    /// so the result is `other` whole when its wall reading is earlier, and `self` otherwise. A
    /// reading from the session the result did not come from then ignores its continuous
    /// deadline, as the bound rule says, so only the wall reading can end it there.
    pub fn earlier(self, other: Bound) -> Bound {
        if self.continuous.boot != other.continuous.boot {
            return if other.not_after < self.not_after {
                other
            } else {
                self
            };
        }
        Bound {
            not_after: self.not_after.min(other.not_after),
            continuous: Continuous {
                boot: self.continuous.boot,
                ns: self.continuous.ns.min(other.continuous.ns),
            },
        }
    }

    /// Where B plus `extra_ms` falls on the wait clock, seen from this one reading: `now.wait`
    /// plus the time left on the reading with less time left, plus `extra_ms`. After B the
    /// time left is negative, so the time overdue is taken off. On an awake host with no wall
    /// step, the deadline is the same instant at every reading, before B and after it. On the
    /// system clock the wall reading has 1 ms resolution, so where the wall reading has less
    /// time left, that holds only to within 1 ms. When B plus `extra_ms` has passed, the
    /// deadline is at or before `now.wait`, already reached. The wait clock stops while the
    /// host sleeps and the other two run on, so the deadline from a reading after a sleep is
    /// earlier, by the time asleep, than the one from a reading before it. It reads only
    /// `now`, so after a wall step back a bound that had passed only on its wall reading gets
    /// a later deadline; [`Bound`] gives the rule for keeping a fence. Where an `Instant`
    /// cannot hold the result, which cannot happen on macOS or Linux, the deadline is
    /// `now.wait`, already reached.
    pub fn wait_deadline(&self, now: &ClockReading, extra_ms: u64) -> Instant {
        let ns = self.remaining_ns(now) + i128::from(extra_ms) * i128::from(NS_PER_MS);
        let shift = duration_from_ns(ns.unsigned_abs());
        let deadline = if ns >= 0 {
            now.wait.checked_add(shift)
        } else {
            now.wait.checked_sub(shift)
        };
        deadline.unwrap_or(now.wait)
    }

    fn remaining_ns(&self, now: &ClockReading) -> i128 {
        let ns_per_ms = i128::from(NS_PER_MS);
        let wall = (i128::from(self.not_after.0) - i128::from(now.wall.0)) * ns_per_ms;
        if now.continuous.boot != self.continuous.boot {
            return wall;
        }
        wall.min(i128::from(self.continuous.ns) - i128::from(now.continuous.ns))
    }
}

fn duration_from_ns(ns: u128) -> Duration {
    let per_second = 1_000_000_000;
    let seconds = u64::try_from(ns / per_second).unwrap_or(u64::MAX);
    let nanos = u32::try_from(ns % per_second).unwrap_or(0);
    Duration::new(seconds, nanos)
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
    check_range(ms, "a lease or idle limit", MIN_LIMIT_MS, MAX_LIMIT_MS)
}

/// Returns `ms` when it is a drain budget spec 028 allows: 1 to 30,000 ms, inclusive.
pub fn check_drain_ms(ms: u64) -> Result<u64, RangeError> {
    check_range(ms, "a drain budget", MIN_DRAIN_MS, MAX_DRAIN_MS)
}

fn check_range(value: u64, limit: &'static str, min: u64, max: u64) -> Result<u64, RangeError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(RangeError {
            limit,
            value,
            min,
            max,
        })
    }
}

fn is_leap(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year / 400;
    let year_of_era = year % 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let days = days + 719_468;
    let era = days / 146_097;
    let day_of_era = days % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = era * 400 + year_of_era + u64::from(month <= 2);
    (year, month, day)
}

impl std::fmt::Display for TimeTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimeTextError::Form => {
                f.write_str("not an RFC 3339 UTC instant of the form YYYY-MM-DDTHH:MM:SS[.sss]Z")
            }
            TimeTextError::Field { name } => write!(f, "the {name} is out of range"),
            TimeTextError::BeforeEpoch => f.write_str("before 1970-01-01T00:00:00Z"),
        }
    }
}

impl std::error::Error for TimeTextError {}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ms is not {}, which is {} to {} ms",
            self.value, self.limit, self.min, self.max
        )
    }
}

impl std::error::Error for RangeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, ManualClock};
    use proptest::prelude::*;
    use std::time::Duration;

    const MS: u64 = 1_000_000;
    const HOUR_MS: u64 = 3_600_000;
    const LATEST_TEXT_MS: u64 = 253_402_300_799_999;

    fn boot(n: u128) -> BootSession {
        BootSession::from_uuid(Uuid::from_u128(n))
    }

    fn reading(wall_ms: u64, boot: BootSession, continuous_ns: u64, wait: Instant) -> ClockReading {
        ClockReading {
            wall: WallMs::from_unix_ms(wall_ms),
            continuous: Continuous {
                boot,
                ns: continuous_ns,
            },
            wait,
        }
    }

    fn bound(not_after_ms: u64, boot: BootSession, continuous_ns: u64) -> Bound {
        Bound {
            not_after: WallMs::from_unix_ms(not_after_ms),
            continuous: Continuous {
                boot,
                ns: continuous_ns,
            },
        }
    }

    const B_WALL: u64 = 1_792_483_200_000;
    const B_CONTINUOUS: u64 = 900_000 * MS;

    fn at(wall_ms: u64, continuous_ns: u64) -> ClockReading {
        reading(wall_ms, boot(1), continuous_ns, Instant::now())
    }

    #[test]
    fn after_takes_both_readings_from_one_reading() {
        let now = reading(1_000_000, boot(1), 5_000 * MS, Instant::now());
        assert_eq!(
            Bound::after(&now, 900_000),
            Some(bound(1_900_000, boot(1), 905_000 * MS))
        );
    }

    #[test]
    fn passed_by_wall_alone() {
        let b = bound(B_WALL, boot(1), B_CONTINUOUS);
        assert!(b.passed(&at(B_WALL, B_CONTINUOUS - 1_000 * MS)));
        assert!(b.passed(&at(B_WALL + 1, B_CONTINUOUS - 1_000 * MS)));
    }

    #[test]
    fn passed_by_continuous_alone() {
        let b = bound(B_WALL, boot(1), B_CONTINUOUS);
        assert!(b.passed(&at(B_WALL - HOUR_MS, B_CONTINUOUS)));
        assert!(b.passed(&at(B_WALL - HOUR_MS, B_CONTINUOUS + 1)));
    }

    #[test]
    fn not_passed_one_millisecond_before_on_both() {
        let b = bound(B_WALL, boot(1), B_CONTINUOUS);
        assert!(!b.passed(&at(B_WALL - 1, B_CONTINUOUS - MS)));
        assert!(!b.passed(&at(B_WALL - 1, B_CONTINUOUS - 1)));
    }

    #[test]
    fn other_boot_session_ignores_continuous() {
        let b = bound(B_WALL, boot(1), B_CONTINUOUS);
        let far_past = u64::MAX / 2;
        let before = reading(B_WALL - 1, boot(2), far_past, Instant::now());
        assert!(!b.passed(&before));
        let at_b = reading(B_WALL, boot(2), 0, Instant::now());
        assert!(b.passed(&at_b));
    }

    #[test]
    fn remaining_is_the_smaller_and_never_negative() {
        let b = bound(B_WALL, boot(1), B_CONTINUOUS);
        assert_eq!(
            b.remaining_ms(&at(B_WALL - 10_000, B_CONTINUOUS - 4_000 * MS)),
            4_000
        );
        assert_eq!(
            b.remaining_ms(&at(B_WALL - 4_000, B_CONTINUOUS - 10_000 * MS)),
            4_000
        );
        assert_eq!(
            b.remaining_ms(&at(B_WALL - 10_000, B_CONTINUOUS - 1_500_000)),
            2
        );
        assert_eq!(b.remaining_ms(&at(B_WALL - 10_000, B_CONTINUOUS - 1)), 1);
        assert_eq!(
            b.remaining_ms(&at(B_WALL + 5, B_CONTINUOUS - 10_000 * MS)),
            0
        );
        assert_eq!(
            b.remaining_ms(&at(B_WALL - 10_000, B_CONTINUOUS + 5 * MS)),
            0
        );
        assert_eq!(b.remaining_ms(&at(B_WALL, B_CONTINUOUS)), 0);
        let other_boot = reading(B_WALL - 10_000, boot(2), B_CONTINUOUS - MS, Instant::now());
        assert_eq!(b.remaining_ms(&other_boot), 10_000);
        let other_boot_past = reading(B_WALL - 10_000, boot(2), u64::MAX, Instant::now());
        assert_eq!(b.remaining_ms(&other_boot_past), 10_000);
    }

    #[test]
    fn remaining_is_zero_exactly_when_passed() {
        let b = bound(B_WALL, boot(1), B_CONTINUOUS);
        let readings = [
            at(B_WALL - 1, B_CONTINUOUS - 1),
            at(B_WALL - 1, B_CONTINUOUS),
            at(B_WALL - 1, B_CONTINUOUS + 1),
            at(B_WALL, B_CONTINUOUS - 1),
            at(B_WALL + 1, B_CONTINUOUS - 1),
            at(B_WALL - HOUR_MS, B_CONTINUOUS - 999_999),
            reading(B_WALL - 1, boot(2), u64::MAX, Instant::now()),
            reading(B_WALL, boot(2), 0, Instant::now()),
        ];
        for now in readings {
            assert_eq!(b.remaining_ms(&now) == 0, b.passed(&now), "{now:?}");
        }
    }

    #[test]
    fn earlier_takes_each_reading() {
        let wall_first = bound(B_WALL - 1_000, boot(1), B_CONTINUOUS);
        let continuous_first = bound(B_WALL, boot(1), B_CONTINUOUS - 1_000 * MS);
        let both = bound(B_WALL - 1_000, boot(1), B_CONTINUOUS - 1_000 * MS);
        assert_eq!(wall_first.earlier(continuous_first), both);
        assert_eq!(continuous_first.earlier(wall_first), both);
        assert_eq!(both.earlier(both), both);
    }

    #[test]
    fn earlier_across_boot_sessions_takes_the_earlier_wall_whole() {
        let this_boot = bound(B_WALL, boot(1), B_CONTINUOUS);
        let other_earlier = bound(B_WALL - 1, boot(2), B_CONTINUOUS + 1_000 * MS);
        let other_later = bound(B_WALL + 1, boot(2), 1);
        let other_equal = bound(B_WALL, boot(2), 1);
        assert_eq!(this_boot.earlier(other_earlier), other_earlier);
        assert_eq!(other_earlier.earlier(this_boot), other_earlier);
        assert_eq!(this_boot.earlier(other_later), this_boot);
        assert_eq!(this_boot.earlier(other_equal), this_boot);
        assert_eq!(other_equal.earlier(this_boot), other_equal);
    }

    #[test]
    fn wait_deadline_rederives_from_remaining() {
        let base = Instant::now();
        let start = reading(B_WALL, boot(1), B_CONTINUOUS, base);
        let b = Bound::after(&start, 10_000).unwrap();
        let s = Duration::from_secs;
        assert_eq!(b.wait_deadline(&start, 2_000), base + s(12));

        let wait_moved = reading(B_WALL, boot(1), B_CONTINUOUS, base + s(1));
        assert_eq!(b.wait_deadline(&wait_moved, 2_000), base + s(13));

        let awake = reading(
            B_WALL + 4_000,
            boot(1),
            B_CONTINUOUS + 4_000 * MS,
            base + s(4),
        );
        assert_eq!(b.wait_deadline(&awake, 2_000), base + s(12));

        let slept = reading(B_WALL + 5_000, boot(1), B_CONTINUOUS + 5_000 * MS, base);
        assert_eq!(b.wait_deadline(&slept, 2_000), base + s(7));

        let past = reading(
            B_WALL + 10_000,
            boot(1),
            B_CONTINUOUS + 10_000 * MS,
            base + s(10),
        );
        assert_eq!(b.wait_deadline(&past, 2_000), base + s(12));
        assert_eq!(b.wait_deadline(&past, 0), base + s(10));
        let later = reading(
            B_WALL + 13_000,
            boot(1),
            B_CONTINUOUS + 13_000 * MS,
            base + s(13),
        );
        assert_eq!(b.wait_deadline(&later, 2_000), base + s(12));
    }

    #[test]
    fn wait_deadline_does_not_overflow() {
        let base = Instant::now();
        let b = bound(u64::MAX, boot(1), u64::MAX);
        let now = reading(0, boot(1), 0, base);
        let expected = base + Duration::from_nanos(u64::MAX) + Duration::from_millis(u64::MAX);
        assert_eq!(b.wait_deadline(&now, u64::MAX), expected);
    }

    #[test]
    fn work_admitted_a_second_before_b_is_cut_at_b_plus_d() {
        let clock = ManualClock::new(WallMs::from_unix_ms(B_WALL), boot(1), B_CONTINUOUS);
        let admitted = clock.now();
        let b = Bound::after(&admitted, 1_000).unwrap();
        let cap = admitted.wait + Duration::from_millis(1_000 + DEFAULT_DRAIN_MS);
        assert_eq!(b.wait_deadline(&admitted, DEFAULT_DRAIN_MS), cap);
        for step in 1..=16 {
            clock.advance_all_ms(500);
            let now = clock.now();
            assert_eq!(
                b.wait_deadline(&now, DEFAULT_DRAIN_MS),
                cap,
                "{} ms after admission",
                step * 500
            );
            assert_eq!(now.wait >= cap, step >= 12);
        }
    }

    #[test]
    fn a_deadline_derived_again_never_moves_later() {
        let clock = ManualClock::new(WallMs::from_unix_ms(B_WALL), boot(1), B_CONTINUOUS);
        let b = Bound::after(&clock.now(), 2_000).unwrap();
        let mut last = b.wait_deadline(&clock.now(), DEFAULT_DRAIN_MS);
        let steps: [(u64, u64); 8] = [
            (1_500, 1_500),
            (1_000, 1_000),
            (0, 700),
            (1_000, 1_000),
            (2_000, 2_000),
            (0, 3_000),
            (500, 500),
            (0, 60_000),
        ];
        for (awake_ms, slept_ms) in steps {
            clock.advance_all_ms(awake_ms);
            let asleep = slept_ms - awake_ms;
            clock.advance_wall_ms(i64::try_from(asleep).unwrap());
            clock.advance_continuous_ms(asleep);
            let now = clock.now();
            let deadline = b.wait_deadline(&now, DEFAULT_DRAIN_MS);
            assert!(deadline <= last, "moved later after {awake_ms} ms awake");
            if asleep == 0 {
                assert_eq!(deadline, last, "moved after {awake_ms} ms awake");
            }
            last = deadline;
        }
        assert!(last <= clock.now().wait);
    }

    #[test]
    fn a_sleep_past_b_plus_d_leaves_the_deadline_reached() {
        let clock = ManualClock::new(WallMs::from_unix_ms(B_WALL), boot(1), B_CONTINUOUS);
        let admitted = clock.now();
        let b = Bound::after(&admitted, 1_000).unwrap();
        clock.advance_wall_ms(60_000);
        clock.advance_continuous_ms(60_000);
        let woke = clock.now();
        assert_eq!(woke.wait, admitted.wait);
        assert!(b.wait_deadline(&woke, DEFAULT_DRAIN_MS) <= woke.wait);
        assert!(b.wait_deadline(&woke, DEFAULT_DRAIN_MS) < admitted.wait);
    }

    fn fenced(b: Bound, at_fence: &ClockReading) -> Bound {
        let overdue = at_fence.wait.duration_since(b.wait_deadline(at_fence, 0));
        Bound {
            not_after: b.not_after,
            continuous: Continuous {
                boot: at_fence.continuous.boot,
                ns: at_fence
                    .continuous
                    .ns
                    .saturating_sub(u64::try_from(overdue.as_nanos()).unwrap()),
            },
        }
    }

    #[test]
    fn a_bound_remembers_nothing_and_a_kept_fence_holds_after_a_wall_step_back() {
        let clock = ManualClock::new(WallMs::from_unix_ms(B_WALL), boot(1), B_CONTINUOUS);
        let admitted = clock.now();
        let b = Bound::after(&admitted, 60_000).unwrap();
        let s = Duration::from_secs;
        clock.advance_all_ms(59_000);
        clock.advance_wall_ms(2_000);
        let at_fence = clock.now();
        assert!(b.passed(&at_fence));
        assert_eq!(
            b.wait_deadline(&at_fence, DEFAULT_DRAIN_MS),
            admitted.wait + s(63)
        );

        clock.advance_wall_ms(-2_000);
        let stepped_back = clock.now();
        assert!(!b.passed(&stepped_back));
        assert_eq!(b.remaining_ms(&stepped_back), 1_000);
        assert_eq!(
            b.wait_deadline(&stepped_back, DEFAULT_DRAIN_MS),
            admitted.wait + s(65)
        );

        let kept = fenced(b, &at_fence);
        assert!(kept.passed(&stepped_back));
        assert_eq!(kept.remaining_ms(&stepped_back), 0);
        assert_eq!(
            kept.wait_deadline(&stepped_back, DEFAULT_DRAIN_MS),
            admitted.wait + s(63)
        );

        let not_enough = b.earlier(Bound::after(&at_fence, 0).unwrap());
        assert_eq!(
            not_enough.wait_deadline(&stepped_back, DEFAULT_DRAIN_MS),
            admitted.wait + s(64)
        );
    }

    #[test]
    fn a_kept_fence_stays_reached_after_an_hour_step_back() {
        let clock = ManualClock::new(WallMs::from_unix_ms(B_WALL), boot(1), B_CONTINUOUS);
        let admitted = clock.now();
        let b = Bound::after(&admitted, 60_000).unwrap();
        clock.advance_all_ms(30_000);
        clock.advance_wall_ms(i64::try_from(HOUR_MS).unwrap());
        let at_fence = clock.now();
        let kept = fenced(b, &at_fence);
        assert!(kept.wait_deadline(&at_fence, DEFAULT_DRAIN_MS) <= at_fence.wait);

        clock.advance_all_ms(1_000);
        clock.advance_wall_ms(-i64::try_from(HOUR_MS).unwrap());
        let stepped_back = clock.now();
        assert!(!b.passed(&stepped_back));
        assert_eq!(b.remaining_ms(&stepped_back), 29_000);
        assert_eq!(
            b.wait_deadline(&stepped_back, DEFAULT_DRAIN_MS),
            stepped_back.wait + Duration::from_secs(34)
        );
        assert!(kept.passed(&stepped_back));
        assert_eq!(kept.remaining_ms(&stepped_back), 0);
        assert!(kept.wait_deadline(&stepped_back, DEFAULT_DRAIN_MS) <= stepped_back.wait);

        let not_enough = b.earlier(Bound::after(&at_fence, 0).unwrap());
        assert_eq!(
            not_enough.wait_deadline(&stepped_back, DEFAULT_DRAIN_MS),
            stepped_back.wait + Duration::from_secs(4)
        );
    }

    #[test]
    fn wait_deadline_follows_the_reading_with_less_time_left() {
        let base = Instant::now();
        let start = reading(B_WALL, boot(1), B_CONTINUOUS, base);
        let b = Bound::after(&start, 600_000).unwrap();
        let s = Duration::from_secs;

        let wall_back = reading(B_WALL - HOUR_MS, boot(1), B_CONTINUOUS, base);
        assert_eq!(b.wait_deadline(&wall_back, 0), base + s(600));

        let wall_forward = reading(B_WALL + 3_000, boot(1), B_CONTINUOUS, base);
        assert_eq!(b.wait_deadline(&wall_forward, 0), base + s(597));

        let wall_past = reading(B_WALL + 601_000, boot(1), B_CONTINUOUS, base);
        assert_eq!(b.wait_deadline(&wall_past, 2_000), base + s(1));

        let continuous_ahead = reading(B_WALL, boot(1), B_CONTINUOUS + 4_000 * MS, base);
        assert_eq!(b.wait_deadline(&continuous_ahead, 0), base + s(596));

        let other_boot = reading(B_WALL + 5_000, boot(2), 0, base);
        assert_eq!(b.wait_deadline(&other_boot, 0), base + s(595));
        let other_boot_past = reading(B_WALL + 5_000, boot(2), u64::MAX, base);
        assert_eq!(b.wait_deadline(&other_boot_past, 0), base + s(595));
    }

    #[test]
    fn limit_range_edges() {
        assert_eq!(
            check_limit_ms(999),
            Err(RangeError {
                limit: "a lease or idle limit",
                value: 999,
                min: 1_000,
                max: 2_592_000_000,
            })
        );
        assert_eq!(check_limit_ms(1_000), Ok(1_000));
        assert_eq!(check_limit_ms(2_592_000_000), Ok(2_592_000_000));
        assert_eq!(
            check_limit_ms(2_592_000_001).map_err(|refusal| refusal.value),
            Err(2_592_000_001)
        );
        assert!(check_limit_ms(0).is_err());
        assert!(check_limit_ms(u64::MAX).is_err());
        assert_eq!(check_limit_ms(DEFAULT_LEASE_MS), Ok(900_000));
    }

    #[test]
    fn drain_range_edges() {
        assert_eq!(
            check_drain_ms(0),
            Err(RangeError {
                limit: "a drain budget",
                value: 0,
                min: 1,
                max: 30_000,
            })
        );
        assert_eq!(check_drain_ms(1), Ok(1));
        assert_eq!(check_drain_ms(30_000), Ok(30_000));
        assert_eq!(
            check_drain_ms(30_001).map_err(|refusal| refusal.value),
            Err(30_001)
        );
        assert_eq!(check_drain_ms(DEFAULT_DRAIN_MS), Ok(5_000));
    }

    #[test]
    fn a_range_refusal_names_the_limit_and_the_value() {
        let refusal = check_limit_ms(999).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "999 ms is not a lease or idle limit, which is 1000 to 2592000000 ms"
        );
        let refusal = check_drain_ms(30_001).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "30001 ms is not a drain budget, which is 1 to 30000 ms"
        );
    }

    #[test]
    fn the_constants_are_spec_028s() {
        assert_eq!(
            (MIN_LIMIT_MS, MAX_LIMIT_MS, MIN_DRAIN_MS, MAX_DRAIN_MS),
            (1_000, 30 * 24 * HOUR_MS, 1, 30_000)
        );
        assert_eq!(
            (DEFAULT_DRAIN_MS, DEFAULT_LEASE_MS, REFERENCE_GRACE_MS),
            (5_000, 15 * 60_000, 2_000)
        );
    }

    #[test]
    fn rfc3339_accepts_utc_z_with_up_to_three_fraction_digits() {
        let accepted = [
            ("2026-10-20T08:00:00Z", 1_792_483_200_000),
            ("2026-10-20T08:00:00.5Z", 1_792_483_200_500),
            ("2026-10-20T08:00:00.12Z", 1_792_483_200_120),
            ("2026-10-20T08:00:00.123Z", 1_792_483_200_123),
            ("2026-10-20T08:00:00.000Z", 1_792_483_200_000),
            ("1970-01-01T00:00:00Z", 0),
            ("1970-01-01T00:00:00.001Z", 1),
            ("2024-02-29T23:59:59Z", 1_709_251_199_000),
            ("2000-03-01T00:00:00Z", 951_868_800_000),
            ("2100-02-28T12:34:56Z", 4_107_501_296_000),
            ("1972-06-30T23:59:59Z", 78_796_799_000),
            ("9999-12-31T23:59:59.999Z", LATEST_TEXT_MS),
            ("2026-01-31T00:00:00Z", 1_769_817_600_000),
            ("2026-03-31T00:00:00Z", 1_774_915_200_000),
            ("2026-05-31T00:00:00Z", 1_780_185_600_000),
            ("2026-07-31T00:00:00Z", 1_785_456_000_000),
            ("2026-08-31T00:00:00Z", 1_788_134_400_000),
            ("2026-10-31T00:00:00Z", 1_793_404_800_000),
            ("2026-12-31T00:00:00Z", 1_798_675_200_000),
            ("2026-04-30T00:00:00Z", 1_777_507_200_000),
            ("2026-06-30T00:00:00Z", 1_782_777_600_000),
            ("2026-09-30T00:00:00Z", 1_790_726_400_000),
            ("2026-11-30T00:00:00Z", 1_795_996_800_000),
            ("2400-02-29T00:00:00Z", 13_574_563_200_000),
        ];
        for (text, ms) in accepted {
            assert_eq!(
                WallMs::parse_rfc3339(text),
                Ok(WallMs::from_unix_ms(ms)),
                "{text}"
            );
        }
    }

    #[test]
    fn rfc3339_refuses_everything_else() {
        let refused = [
            ("2026-10-20T08:00:00+00:00", TimeTextError::Form),
            ("2026-10-20T08:00:00-00:00", TimeTextError::Form),
            ("2026-10-20T10:00:00+02:00", TimeTextError::Form),
            ("2026-10-20T08:00:00.1234Z", TimeTextError::Form),
            ("2026-10-20T08:00:00.Z", TimeTextError::Form),
            ("2026-10-20T08:00:00z", TimeTextError::Form),
            ("2026-10-20t08:00:00Z", TimeTextError::Form),
            ("2026-10-20 08:00:00Z", TimeTextError::Form),
            ("2026-10-20T08:00Z", TimeTextError::Form),
            ("2026-10-20T08:00:00", TimeTextError::Form),
            ("2026-10-20", TimeTextError::Form),
            ("26-10-20T08:00:00Z", TimeTextError::Form),
            ("2026-1-20T08:00:00Z", TimeTextError::Form),
            ("2026-10-20T8:00:00Z", TimeTextError::Form),
            ("10000-01-01T00:00:00Z", TimeTextError::Form),
            ("+2026-10-20T08:00:00Z", TimeTextError::Form),
            (" 2026-10-20T08:00:00Z", TimeTextError::Form),
            ("2026-10-20T08:00:00Z ", TimeTextError::Form),
            ("2026-10-20T08:00:00Z\n", TimeTextError::Form),
            ("2026-10-2xT08:00:00Z", TimeTextError::Form),
            ("2026/10-20T08:00:00Z", TimeTextError::Form),
            ("2026-10/20T08:00:00Z", TimeTextError::Form),
            ("2026-10-20T08x00:00Z", TimeTextError::Form),
            ("2026-10-20T08:00x00Z", TimeTextError::Form),
            ("2026-10-20T08:00:00x123Z", TimeTextError::Form),
            ("2026-10-20T08:00:00.1x3Z", TimeTextError::Form),
            ("２026-10-20T08:00:00Z", TimeTextError::Form),
            ("", TimeTextError::Form),
            (
                "2026-10-20T23:59:60Z",
                TimeTextError::Field { name: "second" },
            ),
            (
                "2026-10-20T23:60:00Z",
                TimeTextError::Field { name: "minute" },
            ),
            (
                "2026-10-20T24:00:00Z",
                TimeTextError::Field { name: "hour" },
            ),
            (
                "2026-00-20T08:00:00Z",
                TimeTextError::Field { name: "month" },
            ),
            (
                "2026-13-20T08:00:00Z",
                TimeTextError::Field { name: "month" },
            ),
            ("2026-10-00T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2026-10-32T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2026-04-31T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2026-06-31T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2026-09-31T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2026-11-31T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2026-02-29T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2100-02-29T08:00:00Z", TimeTextError::Field { name: "day" }),
            ("2200-02-29T00:00:00Z", TimeTextError::Field { name: "day" }),
            ("2024-02-30T00:00:00Z", TimeTextError::Field { name: "day" }),
            ("1969-12-31T23:59:59Z", TimeTextError::BeforeEpoch),
            ("1969-12-31T23:59:59.999Z", TimeTextError::BeforeEpoch),
            ("0000-01-01T00:00:00Z", TimeTextError::BeforeEpoch),
        ];
        for (text, refusal) in refused {
            assert_eq!(WallMs::parse_rfc3339(text), Err(refusal), "{text:?}");
        }
        assert_eq!(
            WallMs::parse_rfc3339("2000-02-29T00:00:00Z"),
            Ok(WallMs::from_unix_ms(951_782_400_000))
        );
    }

    #[test]
    fn rfc3339_refusals_say_why() {
        assert_eq!(
            TimeTextError::Form.to_string(),
            "not an RFC 3339 UTC instant of the form YYYY-MM-DDTHH:MM:SS[.sss]Z"
        );
        assert_eq!(
            TimeTextError::Field { name: "second" }.to_string(),
            "the second is out of range"
        );
        assert_eq!(
            TimeTextError::BeforeEpoch.to_string(),
            "before 1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn rfc3339_writes_three_fraction_digits_and_z() {
        let written = [
            (0, "1970-01-01T00:00:00.000Z"),
            (1, "1970-01-01T00:00:00.001Z"),
            (1_792_483_200_500, "2026-10-20T08:00:00.500Z"),
            (1_709_251_199_123, "2024-02-29T23:59:59.123Z"),
            (951_868_800_000, "2000-03-01T00:00:00.000Z"),
            (LATEST_TEXT_MS, "9999-12-31T23:59:59.999Z"),
            (LATEST_TEXT_MS + 1, "10000-01-01T00:00:00.000Z"),
        ];
        for (ms, text) in written {
            assert_eq!(WallMs::from_unix_ms(ms).to_rfc3339(), text);
        }
        assert!(WallMs::from_unix_ms(u64::MAX).to_rfc3339().ends_with('Z'));
    }

    #[test]
    fn rfc3339_parsed_values_round_trip() {
        let texts = [
            "2026-10-20T08:00:00Z",
            "2026-10-20T08:00:00.5Z",
            "2026-10-20T08:00:00.05Z",
            "2026-10-20T08:00:00.123Z",
            "2024-02-29T23:59:59.999Z",
            "1970-01-01T00:00:00Z",
            "9999-12-31T23:59:59.999Z",
        ];
        for text in texts {
            let parsed = WallMs::parse_rfc3339(text).unwrap();
            assert_eq!(
                WallMs::parse_rfc3339(&parsed.to_rfc3339()),
                Ok(parsed),
                "{text}"
            );
        }
    }

    proptest! {
        #[test]
        fn every_writable_instant_parses_back(ms in 0..=LATEST_TEXT_MS) {
            let wall = WallMs::from_unix_ms(ms);
            prop_assert_eq!(WallMs::parse_rfc3339(&wall.to_rfc3339()), Ok(wall));
        }
    }

    #[test]
    fn checked_add_ms_refuses_overflow() {
        assert_eq!(
            WallMs::from_unix_ms(1).checked_add_ms(2),
            Some(WallMs::from_unix_ms(3))
        );
        assert_eq!(WallMs::from_unix_ms(u64::MAX).checked_add_ms(1), None);
        assert_eq!(WallMs::from_unix_ms(7).unix_ms(), 7);
    }

    #[test]
    fn after_returns_none_on_overflow() {
        let base = Instant::now();
        let wall_near_max = reading(u64::MAX - 10, boot(1), 0, base);
        assert_eq!(Bound::after(&wall_near_max, 11), None);
        assert_eq!(
            Bound::after(&wall_near_max, 10).map(|b| b.not_after),
            Some(WallMs::from_unix_ms(u64::MAX))
        );
        let continuous_near_max = reading(0, boot(1), u64::MAX - MS + 1, base);
        assert_eq!(Bound::after(&continuous_near_max, 1), None);
        let huge = reading(0, boot(1), 0, base);
        assert_eq!(Bound::after(&huge, u64::MAX / MS + 1), None);
        assert!(Bound::after(&huge, u64::MAX / MS).is_some());
    }

    #[test]
    fn boot_session_reads_only_hyphenated_uuids() {
        let uuid = Uuid::from_u128(0x5d1e_7c3a_0b2f_4e6d_9a8b_1c2d_3e4f_5a6b);
        let session = BootSession::from_uuid(uuid);
        assert_eq!(
            BootSession::parse("5D1E7C3A-0B2F-4E6D-9A8B-1C2D3E4F5A6B"),
            Some(session)
        );
        assert_eq!(
            BootSession::parse("5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b"),
            Some(session)
        );
        assert_eq!(session.uuid(), uuid);
        assert_eq!(session.to_string(), "5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b");
        for refused in [
            "",
            "5d1e7c3a0b2f4e6d9a8b1c2d3e4f5a6b",
            "{5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b}",
            "urn:uuid:5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b",
            "5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b\n",
            "5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6b\0",
            " 5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6",
            "5d1e7c3a-0b2f-4e6d-9a8b-1c2d3e4f5a6g",
            "5d1e7c3a-0b2f-4e6d-9a8b1-c2d3e4f5a6b",
        ] {
            assert_eq!(BootSession::parse(refused), None, "{refused:?}");
        }
    }
}
