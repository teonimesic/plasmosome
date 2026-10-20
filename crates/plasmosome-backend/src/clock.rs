use crate::lifetime::{BootSession, Continuous, WallMs};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

const NS_PER_MS: u64 = 1_000_000;

/// One reading of the three clocks a lifetime decision uses, taken together (spec 028 §3).
#[derive(Clone, Copy, Debug)]
pub struct ClockReading {
    /// UTC wall-clock time. An operator or a time service can step it either way.
    pub wall: WallMs,
    /// The continuous clock, with its boot session. `SystemClock` in `plasmosome-core` gives
    /// `u64::MAX` for a read it refuses. Never keep such a reading as a start time, such as an
    /// idle limit's last use.
    pub continuous: Continuous,
    /// The clock waits and timeouts use. It can stop while the host sleeps, so a deadline on it
    /// taken before a sleep comes late after it, by the time asleep.
    pub wait: Instant,
}

/// Where the controller and the supervisor read time. Tests pass a [`ManualClock`].
/// `SystemClock` in `plasmosome-core::clock` is the production clock, for production code to
/// pass once a caller exists.
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
        ManualClock {
            reading: Mutex::new(ClockReading {
                wall,
                continuous: Continuous {
                    boot,
                    ns: continuous_ns,
                },
                wait: Instant::now(),
            }),
        }
    }

    /// Steps the wall reading by `ms`, back when negative. It stops at 1970-01-01T00:00:00Z.
    pub fn advance_wall_ms(&self, ms: i64) {
        step_wall(&mut self.reading(), ms);
    }

    /// Moves the continuous reading forward by `ms`. It stops at `u64::MAX` nanoseconds.
    pub fn advance_continuous_ms(&self, ms: u64) {
        step_continuous(&mut self.reading(), ms);
    }

    /// Moves the wait reading forward by `ms`. Panics past what an `Instant` holds.
    pub fn advance_wait_ms(&self, ms: u64) {
        step_wait(&mut self.reading(), ms);
    }

    /// Moves all three readings forward by `ms` at once, as time passing on an awake host does.
    pub fn advance_all_ms(&self, ms: u64) {
        let mut reading = self.reading();
        reading.wall = WallMs::from_unix_ms(reading.wall.unix_ms().saturating_add(ms));
        step_continuous(&mut reading, ms);
        step_wait(&mut reading, ms);
    }

    /// Starts a new boot session, with the continuous reading at `continuous_ns`.
    pub fn set_boot(&self, boot: BootSession, continuous_ns: u64) {
        self.reading().continuous = Continuous {
            boot,
            ns: continuous_ns,
        };
    }

    fn reading(&self) -> MutexGuard<'_, ClockReading> {
        self.reading.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn step_wall(reading: &mut ClockReading, ms: i64) {
    reading.wall = WallMs::from_unix_ms(reading.wall.unix_ms().saturating_add_signed(ms));
}

fn step_continuous(reading: &mut ClockReading, ms: u64) {
    reading.continuous.ns = reading
        .continuous
        .ns
        .saturating_add(ms.saturating_mul(NS_PER_MS));
}

fn step_wait(reading: &mut ClockReading, ms: u64) {
    reading.wait += Duration::from_millis(ms);
}

impl Clock for ManualClock {
    fn now(&self) -> ClockReading {
        *self.reading()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    const MS: u64 = 1_000_000;
    const WALL: u64 = 1_792_483_200_000;

    fn boot(n: u128) -> BootSession {
        BootSession::from_uuid(Uuid::from_u128(n))
    }

    fn snapshot(clock: &dyn Clock) -> (u64, BootSession, u64, Instant) {
        let reading = clock.now();
        (
            reading.wall.unix_ms(),
            reading.continuous.boot,
            reading.continuous.ns,
            reading.wait,
        )
    }

    #[test]
    fn starts_where_it_is_told_and_stays_there() {
        let before = Instant::now();
        let clock = ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), 5 * MS);
        let after = Instant::now();
        let (wall, session, ns, wait) = snapshot(&clock);
        assert_eq!((wall, session, ns), (WALL, boot(1), 5 * MS));
        assert!(before <= wait && wait <= after);
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(snapshot(&clock), (wall, session, ns, wait));
    }

    #[test]
    fn each_reading_moves_alone() {
        let clock = ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), 5 * MS);
        let (wall, session, ns, wait) = snapshot(&clock);

        clock.advance_wall_ms(7_000);
        assert_eq!(snapshot(&clock), (wall + 7_000, session, ns, wait));

        clock.advance_continuous_ms(3_000);
        assert_eq!(
            snapshot(&clock),
            (wall + 7_000, session, ns + 3_000 * MS, wait)
        );

        clock.advance_wait_ms(2_000);
        assert_eq!(
            snapshot(&clock),
            (
                wall + 7_000,
                session,
                ns + 3_000 * MS,
                wait + Duration::from_secs(2)
            )
        );
    }

    #[test]
    fn wall_can_move_back() {
        let clock = ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), 5 * MS);
        let (wall, session, ns, wait) = snapshot(&clock);
        clock.advance_wall_ms(-3_600_000);
        assert_eq!(snapshot(&clock), (wall - 3_600_000, session, ns, wait));
        clock.advance_wall_ms(-(WALL as i64));
        assert_eq!(snapshot(&clock).0, 0);
        clock.advance_wall_ms(-1);
        assert_eq!(snapshot(&clock).0, 0);
    }

    #[test]
    fn advance_all_moves_all_three() {
        let clock = ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), 5 * MS);
        let (wall, session, ns, wait) = snapshot(&clock);
        clock.advance_all_ms(1_500);
        assert_eq!(
            snapshot(&clock),
            (
                wall + 1_500,
                session,
                ns + 1_500 * MS,
                wait + Duration::from_millis(1_500)
            )
        );
    }

    #[test]
    fn continuous_saturates_instead_of_wrapping() {
        let clock = ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), u64::MAX - MS);
        clock.advance_continuous_ms(2);
        assert_eq!(snapshot(&clock).2, u64::MAX);
        clock.advance_continuous_ms(u64::MAX);
        assert_eq!(snapshot(&clock).2, u64::MAX);
    }

    #[test]
    fn a_new_boot_session_resets_only_the_continuous_reading() {
        let clock = ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), 5 * MS);
        let (wall, _, _, wait) = snapshot(&clock);
        clock.set_boot(boot(2), 7);
        assert_eq!(snapshot(&clock), (wall, boot(2), 7, wait));
    }

    #[test]
    fn a_manual_clock_is_shared_across_threads() {
        let clock: Arc<dyn Clock> =
            Arc::new(ManualClock::new(WallMs::from_unix_ms(WALL), boot(1), 0));
        let reader = Arc::clone(&clock);
        let read = std::thread::spawn(move || reader.now().wall)
            .join()
            .unwrap();
        assert_eq!(read, WallMs::from_unix_ms(WALL));
    }
}
