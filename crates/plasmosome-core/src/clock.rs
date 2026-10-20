use plasmosome_backend::{BootSession, Clock, ClockReading, Continuous, WallMs};
use std::fmt;
use std::io;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
const CONTINUOUS_CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC_RAW;
#[cfg(target_os = "linux")]
const CONTINUOUS_CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
#[cfg(target_os = "linux")]
const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

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
    /// session reads as passed. The fence is permanent, so one refused read ends every lease
    /// of the boot session for good. The next good reading is lower than `u64::MAX`, so a
    /// caller that subtracts two continuous readings must saturate.
    pub fn new() -> Result<SystemClock, ClockError> {
        wall_ms_at(SystemTime::now())?;
        continuous_ns()?;
        Ok(SystemClock {
            boot: boot_session()?,
        })
    }
}

impl Clock for SystemClock {
    fn now(&self) -> ClockReading {
        reading_from(
            self.boot,
            wall_ms_at(SystemTime::now()),
            continuous_ns(),
            Instant::now(),
        )
    }
}

fn reading_from(
    boot: BootSession,
    wall_ms: Result<u64, ClockError>,
    continuous_ns: Result<u64, ClockError>,
    wait: Instant,
) -> ClockReading {
    ClockReading {
        wall: WallMs::from_unix_ms(wall_ms.unwrap_or(0)),
        continuous: Continuous {
            boot,
            ns: continuous_ns.unwrap_or(u64::MAX),
        },
        wait,
    }
}

/// The host boot session: `kern.bootsessionuuid` on macOS and
/// `/proc/sys/kernel/random/boot_id` on Linux, read now. It does not change while a process
/// runs.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn boot_session() -> Result<BootSession, ClockError> {
    let mut text = [0u8; 64];
    let mut length = text.len();
    // SAFETY: the name is a NUL-terminated literal. `text` and `length` are live locals that
    // outlive the call, and `length` holds the size of `text`, so the kernel writes at most
    // that many bytes. The new-value pointer is null with length 0, so nothing is set.
    let read = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            text.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        return Err(ClockError::BootSession {
            detail: format!(
                "sysctl kern.bootsessionuuid: {}",
                io::Error::last_os_error()
            ),
        });
    }
    boot_from(&text[..length.min(text.len())], b"\0")
}

/// The host boot session: `kern.bootsessionuuid` on macOS and
/// `/proc/sys/kernel/random/boot_id` on Linux, read now. It does not change while a process
/// runs.
#[cfg(target_os = "linux")]
pub fn boot_session() -> Result<BootSession, ClockError> {
    let text = std::fs::read(BOOT_ID_PATH).map_err(|error| ClockError::BootSession {
        detail: format!("{BOOT_ID_PATH}: {error}"),
    })?;
    boot_from(&text, b"\n")
}

fn wall_ms_at(now: SystemTime) -> Result<u64, ClockError> {
    let since = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ClockError::BeforeEpoch)?;
    Ok(u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

#[allow(unsafe_code)]
fn continuous_ns() -> Result<u64, ClockError> {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a live, writable `timespec` that outlives the call, and
    // `CONTINUOUS_CLOCK` is a clock this target defines.
    if unsafe { libc::clock_gettime(CONTINUOUS_CLOCK, &mut now) } != 0 {
        return Err(ClockError::Continuous {
            errno: io::Error::last_os_error().raw_os_error().unwrap_or(0),
        });
    }
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(now.tv_nsec).unwrap_or(0);
    Ok(seconds.saturating_mul(1_000_000_000).saturating_add(nanos))
}

fn boot_from(value: &[u8], terminator: &[u8]) -> Result<BootSession, ClockError> {
    let line = value.strip_suffix(terminator).unwrap_or(value);
    std::str::from_utf8(line)
        .ok()
        .and_then(BootSession::parse)
        .ok_or_else(|| ClockError::BootSession {
            detail: format!("{:?} is not a UUID", String::from_utf8_lossy(value)),
        })
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClockError::BeforeEpoch => {
                f.write_str("the wall clock reads before 1970-01-01T00:00:00Z")
            }
            ClockError::Continuous { errno } => {
                write!(f, "the continuous clock could not be read: errno {errno}")
            }
            ClockError::BootSession { detail } => {
                write!(f, "the boot session could not be read: {detail}")
            }
        }
    }
}

impl std::error::Error for ClockError {}

#[cfg(test)]
mod tests {
    use super::*;
    use plasmosome_backend::{Bound, DEFAULT_LEASE_MS};
    use std::time::{Duration, Instant, UNIX_EPOCH};

    const UUID_TEXT: &str = "5D1E7C3A-0B2F-4E6D-9A8B-1C2D3E4F5A6B";

    #[test]
    fn system_boot_session_is_stable() {
        let first = boot_session().unwrap();
        let second = boot_session().unwrap();
        assert_eq!(first, second);
        assert_eq!(BootSession::parse(&first.to_string()), Some(first));
        let clock = SystemClock::new().unwrap();
        assert_eq!(clock.now().continuous.boot, first);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_boot_session_is_kern_bootsessionuuid() {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            BootSession::parse(text.trim_end_matches('\n')),
            Some(boot_session().unwrap())
        );
    }

    #[test]
    fn continuous_advances() {
        let clock = SystemClock::new().unwrap();
        let first = clock.now();
        std::thread::sleep(Duration::from_millis(50));
        let second = clock.now();
        assert!(second.wait.duration_since(first.wait) >= Duration::from_millis(50));
        let advanced = second.continuous.ns - first.continuous.ns;
        assert!(advanced >= 45_000_000, "advanced {advanced} ns");
        assert!(advanced < 5_000_000_000, "advanced {advanced} ns");
        assert_eq!(second.continuous.boot, first.continuous.boot);
    }

    #[test]
    fn the_wall_reading_is_unix_milliseconds() {
        let clock = SystemClock::new().unwrap();
        let unix_ms = || {
            u64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap()
        };
        let before = unix_ms();
        let wall = clock.now().wall.unix_ms();
        let after = unix_ms();
        assert!(before <= wall && wall <= after, "{before} {wall} {after}");
    }

    #[test]
    fn the_wait_reading_is_instant_now() {
        let clock = SystemClock::new().unwrap();
        let before = Instant::now();
        let wait = clock.now().wait;
        let after = Instant::now();
        assert!(before <= wait && wait <= after);
    }

    #[test]
    fn continuous_clock_is_the_specified_one() {
        #[cfg(target_os = "macos")]
        assert_eq!(CONTINUOUS_CLOCK, libc::CLOCK_MONOTONIC_RAW);
        #[cfg(target_os = "linux")]
        assert_eq!(CONTINUOUS_CLOCK, libc::CLOCK_BOOTTIME);
    }

    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    unsafe extern "C" {
        /// Takes nothing and returns the tick count since boot, which advances while the host
        /// sleeps. It has no failure.
        fn mach_continuous_time() -> u64;
        /// Writes the ratio of nanoseconds to ticks through `info`, which must point to a
        /// writable `Timebase`. Returns 0 on success.
        fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn mach_continuous_ns() -> u64 {
        let mut timebase = Timebase { numer: 0, denom: 0 };
        // SAFETY: `timebase` is a live, writable `Timebase` with the C layout of
        // `mach_timebase_info_data_t`, and it outlives the call.
        assert_eq!(unsafe { mach_timebase_info(&mut timebase) }, 0);
        // SAFETY: `mach_continuous_time` takes no arguments and touches no caller memory.
        let ticks = unsafe { mach_continuous_time() };
        let ns = u128::from(ticks) * u128::from(timebase.numer) / u128::from(timebase.denom);
        u64::try_from(ns).unwrap()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn continuous_agrees_with_mach_continuous_time() {
        let clock = SystemClock::new().unwrap();
        let before = mach_continuous_ns();
        let reading = clock.now().continuous.ns;
        let after = mach_continuous_ns();
        assert!(
            before.saturating_sub(1_000_000) <= reading && reading <= after + 1_000_000,
            "{before} {reading} {after}"
        );
    }

    #[test]
    fn a_refused_continuous_read_fences_every_bound_of_the_boot_session() {
        let boot = BootSession::parse(UUID_TEXT).unwrap();
        let wait = Instant::now();
        let good = reading_from(boot, Ok(1_792_483_200_000), Ok(5_000_000_000), wait);
        assert_eq!(good.wall.unix_ms(), 1_792_483_200_000);
        assert_eq!(good.continuous.boot, boot);
        assert_eq!(good.continuous.ns, 5_000_000_000);
        assert_eq!(good.wait, wait);
        let lease = Bound::after(&good, DEFAULT_LEASE_MS).unwrap();
        assert!(!lease.passed(&good));

        let refused = Err(ClockError::Continuous { errno: 22 });
        let failed = reading_from(boot, Ok(1_792_483_200_000), refused, wait);
        assert_eq!(failed.continuous.ns, u64::MAX);
        assert!(lease.passed(&failed));
        assert_eq!(lease.remaining_ms(&failed), 0);
    }

    #[test]
    fn a_wall_clock_set_before_1970_reads_as_1970() {
        let boot = BootSession::parse(UUID_TEXT).unwrap();
        let wait = Instant::now();
        let reading = reading_from(boot, Err(ClockError::BeforeEpoch), Ok(7), wait);
        assert_eq!(reading.wall.unix_ms(), 0);
        assert_eq!(reading.continuous.ns, 7);
    }

    #[test]
    fn a_wall_clock_before_1970_is_refused() {
        let before = UNIX_EPOCH - Duration::from_millis(1);
        assert_eq!(wall_ms_at(before), Err(ClockError::BeforeEpoch));
        assert_eq!(wall_ms_at(UNIX_EPOCH), Ok(0));
        assert_eq!(
            wall_ms_at(UNIX_EPOCH + Duration::from_micros(1_792_483_200_000_999)),
            Ok(1_792_483_200_000)
        );
    }

    #[test]
    fn boot_text_is_one_uuid_and_its_terminator() {
        let session = BootSession::parse(UUID_TEXT).unwrap();
        let sysctl = format!("{UUID_TEXT}\0");
        assert_eq!(boot_from(sysctl.as_bytes(), b"\0"), Ok(session));
        let proc_file = format!("{}\n", UUID_TEXT.to_lowercase());
        assert_eq!(boot_from(proc_file.as_bytes(), b"\n"), Ok(session));
        assert_eq!(boot_from(UUID_TEXT.as_bytes(), b"\n"), Ok(session));
        for refused in [
            &b""[..],
            b"\n",
            b"not a uuid\n",
            b"5D1E7C3A-0B2F-4E6D-9A8B-1C2D3E4F5A6B\n\n",
            b"5D1E7C3A-0B2F-4E6D-9A8B-1C2D3E4F5A6B\0\0",
            b"\xff\xfe\n",
        ] {
            match boot_from(refused, b"\n") {
                Err(ClockError::BootSession { detail }) => assert!(!detail.is_empty()),
                other => panic!("{refused:?} gave {other:?}"),
            }
        }
    }

    #[test]
    fn clock_errors_say_what_failed() {
        assert_eq!(
            ClockError::BeforeEpoch.to_string(),
            "the wall clock reads before 1970-01-01T00:00:00Z"
        );
        assert_eq!(
            ClockError::Continuous { errno: 22 }.to_string(),
            "the continuous clock could not be read: errno 22"
        );
        assert_eq!(
            ClockError::BootSession {
                detail: "\"x\" is not a UUID".into()
            }
            .to_string(),
            "the boot session could not be read: \"x\" is not a UUID"
        );
    }
}
