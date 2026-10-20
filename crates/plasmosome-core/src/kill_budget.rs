use plasmosome_backend::{DrainSpec, RevokePolicy};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const MS: Duration = Duration::from_millis(1);
const MOST_LOCK_RESERVE: Duration = Duration::from_secs(3);
const MOST_WORKLOAD_STOP: Duration = Duration::from_secs(5);

/// The deadline shares of one `cell.kill` (spec 023 §7). Every share is measured from what is
/// left of the kill's deadline D at the moment it is asked for, counted from the request's
/// arrival, so a kill that queued behind other mutations gets only what remains.
///
/// `arrival` and `clock` must read the same clock: `clock` returns the current reading, and
/// `arrival` is the reading taken when the request's line was read. Every share is a whole
/// number of milliseconds, so it reaches the wire unchanged.
pub struct KillBudget<C: Fn() -> Instant> {
    arrival: Instant,
    total: Duration,
    now: bool,
    clock: C,
}

/// A step's budget: at least 1 ms, or none. A step with no budget takes its escalation and
/// sends nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Share<T = Duration> {
    Run(T),
    NoBudget,
}

/// Step 2's share and how it divides: the removal's drain deadline, its effect allowance and
/// its publication allowance. Each part is at least 1 ms, and the three together fit inside
/// `share`, so the publication exchange ends within D.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Withdrawal {
    pub share: Duration,
    pub drain: Duration,
    pub effect: Duration,
    pub publication: Duration,
}

/// Step 3: the `DrainSpec` sent with `membrane.cell.kill`, and how long the controller waits
/// for its answer. The drain deadline is at most the answer wait, so `membraned` keeps the rest
/// for the group kill, the reap and the teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestStop {
    pub drain: DrainSpec,
    pub answer_wait: Duration,
}

impl<C: Fn() -> Instant> KillBudget<C> {
    /// A budget for a kill whose request arrived at `arrival` with deadline `total`, D. `now`
    /// is the request's `now`.
    pub fn new(arrival: Instant, total: Duration, now: bool, clock: C) -> Self {
        KillBudget {
            arrival,
            total,
            now,
            clock,
        }
    }

    /// What is left of D now. Zero once D has passed.
    pub fn remaining(&self) -> Duration {
        let elapsed = (self.clock)().saturating_duration_since(self.arrival);
        self.total.saturating_sub(elapsed)
    }

    /// The reserve kept for the lock wait: a tenth of D, from 1 ms to 3 s. Fixed for the
    /// request.
    pub fn lock_reserve(&self) -> Duration {
        (self.total / 10).clamp(MS, MOST_LOCK_RESERVE)
    }

    /// Step 1's share. With `now` it is `NoBudget`, and the caller skips the step without
    /// listing it. Without `now`, `NoBudget` means under 1 ms was left, and the caller lists
    /// `workload_stop`.
    pub fn workload_stop(&self) -> Share {
        if self.now {
            return Share::NoBudget;
        }
        let cap = (self.total / 10).clamp(MS, MOST_WORKLOAD_STOP);
        at_least_one_ms(cap.min(self.spendable() / 3))
    }

    /// Step 2's share, the publication exchange included. `NoBudget` when any of its three
    /// parts would be under 1 ms; the caller answers that as a drain timeout, 106.
    pub fn withdrawal(&self) -> Share<Withdrawal> {
        let share = self.spendable() / 2;
        let drain = share / 2;
        let effect = share / 4;
        match (
            at_least_one_ms(drain),
            at_least_one_ms(effect),
            at_least_one_ms(share - drain - effect),
        ) {
            (Share::Run(drain), Share::Run(effect), Share::Run(publication)) => {
                Share::Run(Withdrawal {
                    share: whole_ms(share),
                    drain,
                    effect,
                    publication,
                })
            }
            _ => Share::NoBudget,
        }
    }

    /// Step 3's deadlines. They are never under 1 ms, even after D has passed. With `now` the
    /// drain is 1 ms with `RevokePolicy::Force`; otherwise it is `RevokePolicy::Graceful`.
    pub fn guest_stop(&self) -> GuestStop {
        let answer_wait = whole_ms(self.spendable().max(MS));
        let drain = if self.now {
            DrainSpec {
                deadline: MS,
                policy: RevokePolicy::Force,
            }
        } else {
            let teardown = MS.max(answer_wait / 4);
            DrainSpec::graceful(whole_ms(answer_wait.saturating_sub(teardown).max(MS)))
        };
        GuestStop { drain, answer_wait }
    }

    /// When step 4's lock wait ends: arrival plus D, or the latest instant this platform can
    /// represent before it. The caller tries the lock at least once, even when this has passed.
    pub fn lock_wait_until(&self) -> Instant {
        let mut span = self.total;
        loop {
            if let Some(until) = self.arrival.checked_add(span) {
                return until;
            }
            span /= 2;
        }
    }

    fn spendable(&self) -> Duration {
        self.remaining().saturating_sub(self.lock_reserve())
    }
}

/// The deadline `membraned` sends with the 4090 `shutdown` for this drain, strictly below the
/// drain's own deadline. `None` with `RevokePolicy::Force`, and when under 1 ms would be left:
/// then no `shutdown` is sent, and the helper is killed at the drain deadline.
pub fn shutdown_deadline(drain: &DrainSpec) -> Option<Duration> {
    match drain.policy {
        RevokePolicy::Force => None,
        RevokePolicy::Graceful => {
            let kill_margin = MS.max(drain.deadline / 10);
            match at_least_one_ms(drain.deadline.saturating_sub(kill_margin)) {
                Share::Run(deadline) => Some(deadline),
                Share::NoBudget => None,
            }
        }
    }
}

fn whole_ms(duration: Duration) -> Duration {
    Duration::new(duration.as_secs(), duration.subsec_millis() * 1_000_000)
}

fn at_least_one_ms(duration: Duration) -> Share {
    let whole = whole_ms(duration);
    if whole < MS {
        Share::NoBudget
    } else {
        Share::Run(whole)
    }
}

/// A step of `cell.kill` that fell short, as its reply's `incomplete` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shortfall {
    /// The workload stop failed, was refused, timed out or had no budget.
    WorkloadStop,
    /// The guest got no `shutdown` for lack of budget, or ignored it, and its helper was
    /// killed. Never listed with `now`.
    Shutdown,
    /// The teardown was not clean, or the retirement check failed, so the cell was not retired.
    Teardown,
}

/// The reply's `incomplete`: a set over the closed [`Shortfall`] list. On the wire it is a JSON
/// array in the order `workload_stop`, `shutdown`, `teardown`, and any other string is refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Incomplete(BTreeSet<Shortfall>);

impl Incomplete {
    /// Records a shortfall. `Shutdown` is dropped when `now` is set.
    pub fn add(&mut self, shortfall: Shortfall, now: bool) {
        if !(now && shortfall == Shortfall::Shutdown) {
            self.0.insert(shortfall);
        }
    }

    /// Whether the reply is a retirement: exactly when `teardown` is absent.
    pub fn is_retirement(&self) -> bool {
        !self.0.contains(&Shortfall::Teardown)
    }

    /// Whether no step fell short, so the reply carries no `incomplete`.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for Incomplete {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Incomplete {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BTreeSet::deserialize(deserializer).map(Incomplete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plasmosome_backend::RevokePolicy;
    use proptest::prelude::*;
    use std::cell::Cell;

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    struct TestClock(Cell<Instant>);

    impl TestClock {
        fn at(start: Instant) -> TestClock {
            TestClock(Cell::new(start))
        }

        fn read(&self) -> Instant {
            self.0.get()
        }

        fn advance(&self, by: Duration) {
            self.0.set(self.0.get() + by);
        }

        fn set_to(&self, reading: Instant) {
            self.0.set(reading);
        }
    }

    fn is_whole_ms(duration: Duration) -> bool {
        duration.subsec_nanos().is_multiple_of(1_000_000)
    }

    #[test]
    fn shares_for_thirty_seconds() {
        let arrival = Instant::now();
        let clock = TestClock::at(arrival);
        let budget = KillBudget::new(arrival, ms(30_000), false, || clock.read());

        assert_eq!(budget.remaining(), ms(30_000));
        assert_eq!(budget.lock_reserve(), ms(3_000));
        assert_eq!(budget.workload_stop(), Share::Run(ms(3_000)));
        clock.advance(ms(3_000));
        assert_eq!(
            budget.withdrawal(),
            Share::Run(Withdrawal {
                share: ms(12_000),
                drain: ms(6_000),
                effect: ms(3_000),
                publication: ms(3_000),
            })
        );
        clock.advance(ms(12_000));
        let stop = budget.guest_stop();
        assert_eq!(
            stop,
            GuestStop {
                drain: DrainSpec::graceful(ms(9_000)),
                answer_wait: ms(12_000),
            }
        );
        assert_eq!(shutdown_deadline(&stop.drain), Some(ms(8_100)));
        assert_eq!(
            ms(3_000) + ms(12_000) + stop.answer_wait + budget.lock_reserve(),
            ms(30_000)
        );
        assert_eq!(budget.lock_wait_until(), arrival + ms(30_000));
    }

    #[test]
    fn now_gets_one_millisecond() {
        let arrival = Instant::now();
        let clock = TestClock::at(arrival);
        let budget = KillBudget::new(arrival, ms(30_000), true, || clock.read());

        assert_eq!(budget.workload_stop(), Share::NoBudget);
        assert_eq!(
            budget.withdrawal(),
            Share::Run(Withdrawal {
                share: ms(13_500),
                drain: ms(6_750),
                effect: ms(3_375),
                publication: ms(3_375),
            })
        );
        let stop = budget.guest_stop();
        assert_eq!(
            stop,
            GuestStop {
                drain: DrainSpec {
                    deadline: MS,
                    policy: RevokePolicy::Force,
                },
                answer_wait: ms(27_000),
            }
        );
        assert_eq!(shutdown_deadline(&stop.drain), None);
    }

    #[test]
    fn queued_until_spent() {
        for now in [false, true] {
            let arrival = Instant::now();
            let clock = TestClock::at(arrival);
            let budget = KillBudget::new(arrival, MS, now, || clock.read());
            clock.advance(ms(5));

            assert_eq!(budget.remaining(), Duration::ZERO);
            assert_eq!(budget.lock_reserve(), MS);
            assert_eq!(budget.workload_stop(), Share::NoBudget);
            assert_eq!(budget.withdrawal(), Share::NoBudget);
            let policy = if now {
                RevokePolicy::Force
            } else {
                RevokePolicy::Graceful
            };
            assert_eq!(
                budget.guest_stop(),
                GuestStop {
                    drain: DrainSpec {
                        deadline: MS,
                        policy,
                    },
                    answer_wait: MS,
                }
            );
            assert_eq!(shutdown_deadline(&budget.guest_stop().drain), None);
            assert_eq!(budget.lock_wait_until(), arrival + MS);
            assert!(budget.lock_wait_until() < clock.read());
        }
    }

    #[test]
    fn a_step_short_of_its_floor_has_no_budget() {
        let arrival = Instant::now();
        let clock = TestClock::at(arrival);
        let budget = KillBudget::new(arrival, ms(20), false, || clock.read());

        assert_eq!(budget.lock_reserve(), ms(2));
        clock.advance(ms(16));
        assert_eq!(budget.remaining(), ms(4));
        assert_eq!(budget.workload_stop(), Share::NoBudget);
        assert_eq!(budget.withdrawal(), Share::NoBudget);
        clock.advance(Duration::from_micros(500));
        assert_eq!(
            budget.guest_stop(),
            GuestStop {
                drain: DrainSpec::graceful(MS),
                answer_wait: MS,
            }
        );

        let budget = KillBudget::new(arrival, ms(30), false, || clock.read());
        clock.set_to(arrival + ms(19));
        assert_eq!(budget.remaining(), ms(11));
        assert_eq!(budget.workload_stop(), Share::Run(ms(2)));
        assert_eq!(
            budget.withdrawal(),
            Share::Run(Withdrawal {
                share: ms(4),
                drain: ms(2),
                effect: MS,
                publication: MS,
            })
        );
        clock.set_to(arrival + ms(20));
        assert_eq!(budget.withdrawal(), Share::NoBudget);
    }

    #[test]
    fn small_deadlines_get_exact_shares() {
        let arrival = Instant::now();
        let clock = TestClock::at(arrival);
        let budget = KillBudget::new(arrival, ms(5), false, || clock.read());
        assert_eq!(budget.lock_reserve(), MS);
        assert_eq!(budget.workload_stop(), Share::Run(MS));
        let stop = budget.guest_stop();
        assert_eq!(
            stop,
            GuestStop {
                drain: DrainSpec::graceful(ms(3)),
                answer_wait: ms(4),
            }
        );
        assert_eq!(shutdown_deadline(&stop.drain), Some(ms(2)));
        clock.advance(MS);
        assert_eq!(budget.withdrawal(), Share::NoBudget);
        let stop = budget.guest_stop();
        assert_eq!(
            stop,
            GuestStop {
                drain: DrainSpec::graceful(ms(2)),
                answer_wait: ms(3),
            }
        );
        assert_eq!(shutdown_deadline(&stop.drain), Some(MS));

        let clock = TestClock::at(arrival);
        let budget = KillBudget::new(arrival, ms(9), false, || clock.read());
        assert_eq!(budget.workload_stop(), Share::Run(MS));

        let clock = TestClock::at(arrival);
        let budget = KillBudget::new(arrival, ms(15), false, || clock.read());
        assert_eq!(budget.lock_reserve(), Duration::from_micros(1_500));
        assert_eq!(budget.workload_stop(), Share::Run(MS));
        clock.advance(MS);
        assert_eq!(
            budget.withdrawal(),
            Share::Run(Withdrawal {
                share: ms(6),
                drain: ms(3),
                effect: MS,
                publication: MS,
            })
        );
    }

    #[test]
    fn a_budget_built_after_its_request_queued_counts_from_arrival() {
        let arrival = Instant::now();
        let clock = TestClock::at(arrival + ms(4_000));
        let budget = KillBudget::new(arrival, ms(30_000), false, || clock.read());

        assert_eq!(budget.remaining(), ms(26_000));
        assert_eq!(budget.workload_stop(), Share::Run(ms(3_000)));
        assert_eq!(
            budget.withdrawal(),
            Share::Run(Withdrawal {
                share: ms(11_500),
                drain: ms(5_750),
                effect: ms(2_875),
                publication: ms(2_875),
            })
        );
        assert_eq!(budget.lock_wait_until(), arrival + ms(30_000));
    }

    #[test]
    fn shutdown_stays_below_the_drain_deadline() {
        assert_eq!(shutdown_deadline(&DrainSpec::graceful(MS)), None);
        assert_eq!(
            shutdown_deadline(&DrainSpec::graceful(Duration::from_micros(1_999))),
            None
        );
        assert_eq!(shutdown_deadline(&DrainSpec::graceful(ms(2))), Some(MS));
        assert_eq!(shutdown_deadline(&DrainSpec::graceful(ms(10))), Some(ms(9)));
        assert_eq!(
            shutdown_deadline(&DrainSpec::graceful(ms(20))),
            Some(ms(18))
        );
        assert_eq!(
            shutdown_deadline(&DrainSpec::graceful(Duration::from_micros(10_700))),
            Some(ms(9))
        );
        assert_eq!(
            shutdown_deadline(&DrainSpec {
                deadline: ms(30_000),
                policy: RevokePolicy::Force,
            }),
            None
        );
    }

    #[test]
    fn a_deadline_no_instant_holds_still_answers() {
        let arrival = Instant::now();
        let clock = TestClock::at(arrival);
        for total in [ms(u64::MAX), Duration::MAX] {
            let budget = KillBudget::new(arrival, total, false, || clock.read());
            assert_eq!(budget.remaining(), total);
            assert_eq!(budget.lock_reserve(), ms(3_000));
            assert_eq!(budget.workload_stop(), Share::Run(ms(5_000)));
            assert!(matches!(budget.withdrawal(), Share::Run(_)));
            let stop = budget.guest_stop();
            assert!(stop.drain.deadline < stop.answer_wait);
            assert!(budget.lock_wait_until() > arrival + ms(1_000_000_000));
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum Took {
        Share { per_mille: u32 },
        Any(Duration),
    }

    impl Took {
        fn of(self, share: Duration) -> Duration {
            match self {
                Took::Share { per_mille } => share * per_mille / 1_000,
                Took::Any(elapsed) => elapsed,
            }
        }
    }

    fn twice_in_nanos(total: Duration) -> u64 {
        u64::try_from(total.as_nanos() * 2).unwrap_or(u64::MAX)
    }

    fn took(total: Duration) -> impl Strategy<Value = Took> {
        prop_oneof![
            Just(Took::Share { per_mille: 1_000 }),
            (0u32..=1_000).prop_map(|per_mille| Took::Share { per_mille }),
            (0..=twice_in_nanos(total)).prop_map(|nanos| Took::Any(Duration::from_nanos(nanos))),
        ]
    }

    fn kill() -> impl Strategy<Value = (Duration, bool, Duration, Took, Took)> {
        (prop_oneof![1u64..=50, 1u64..=120_000], any::<bool>()).prop_flat_map(|(total_ms, now)| {
            let total = ms(total_ms);
            (
                Just(total),
                Just(now),
                prop_oneof![
                    Just(Duration::ZERO),
                    (0..=twice_in_nanos(total)).prop_map(Duration::from_nanos),
                ],
                took(total),
                took(total),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn every_sent_deadline_is_positive_and_nested(
            (total, now, queued, first, second) in kill(),
        ) {
            let arrival = Instant::now();
            let clock = TestClock::at(arrival);
            let budget = KillBudget::new(arrival, total, now, || clock.read());
            let reserve = budget.lock_reserve();
            clock.advance(queued);

            let workload = match budget.workload_stop() {
                Share::Run(share) => {
                    prop_assert!(!now);
                    prop_assert!(share >= MS && is_whole_ms(share));
                    share
                }
                Share::NoBudget => Duration::ZERO,
            };
            let first = first.of(workload);
            clock.advance(first);

            let withdrawal = match budget.withdrawal() {
                Share::Run(parts) => {
                    for part in [parts.share, parts.drain, parts.effect, parts.publication] {
                        prop_assert!(part >= MS && is_whole_ms(part));
                    }
                    prop_assert!(parts.drain + parts.effect + parts.publication <= parts.share);
                    parts.share
                }
                Share::NoBudget => Duration::ZERO,
            };
            let second = second.of(withdrawal);
            clock.advance(second);

            let stop = budget.guest_stop();
            prop_assert!(stop.drain.deadline >= MS && is_whole_ms(stop.drain.deadline));
            prop_assert!(stop.answer_wait >= MS && is_whole_ms(stop.answer_wait));
            prop_assert!(stop.drain.deadline <= stop.answer_wait);
            if stop.answer_wait >= ms(2) {
                prop_assert!(stop.answer_wait - stop.drain.deadline >= MS.max(stop.answer_wait / 4));
            }
            if now {
                prop_assert_eq!(stop.drain, DrainSpec { deadline: MS, policy: RevokePolicy::Force });
            } else {
                prop_assert_eq!(stop.drain.policy, RevokePolicy::Graceful);
            }
            if let Some(shutdown) = shutdown_deadline(&stop.drain) {
                prop_assert!(!now);
                prop_assert!(shutdown >= MS && is_whole_ms(shutdown));
                prop_assert!(shutdown < stop.drain.deadline);
            }

            let overran = first > workload || second > withdrawal;
            if total >= ms(20) && queued.is_zero() && !overran {
                prop_assert!(first + second + stop.answer_wait + reserve <= total);
            }
        }

        #[test]
        fn shares_never_exceed_what_is_left(
            (total, now, queued, first, second) in kill(),
        ) {
            let arrival = Instant::now();
            let clock = TestClock::at(arrival);
            let budget = KillBudget::new(arrival, total, now, || clock.read());
            let reserve = budget.lock_reserve();
            prop_assert_eq!(reserve, (total / 10).clamp(MS, ms(3_000)));
            clock.advance(queued);
            let mut elapsed = queued;

            let left = budget.remaining();
            prop_assert_eq!(left, total.saturating_sub(elapsed));
            let workload = match budget.workload_stop() {
                Share::Run(share) => {
                    prop_assert!(share <= left);
                    prop_assert!(share + reserve <= left);
                    share
                }
                Share::NoBudget => Duration::ZERO,
            };
            let first = first.of(workload);
            clock.advance(first);
            elapsed += first;

            let left = budget.remaining();
            prop_assert_eq!(left, total.saturating_sub(elapsed));
            let withdrawal = match budget.withdrawal() {
                Share::Run(parts) => {
                    prop_assert!(parts.share <= left);
                    prop_assert!(parts.share + reserve <= left);
                    parts.share
                }
                Share::NoBudget => Duration::ZERO,
            };
            clock.advance(second.of(withdrawal));
            elapsed += second.of(withdrawal);

            let left = budget.remaining();
            prop_assert_eq!(left, total.saturating_sub(elapsed));
            let stop = budget.guest_stop();
            prop_assert!(stop.answer_wait <= left.saturating_sub(reserve).max(MS));
            prop_assert_eq!(budget.lock_wait_until(), arrival + total);
        }
    }

    const EVERY: [Shortfall; 3] = [
        Shortfall::WorkloadStop,
        Shortfall::Shutdown,
        Shortfall::Teardown,
    ];

    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];

    fn wire_name(shortfall: Shortfall) -> &'static str {
        match shortfall {
            Shortfall::WorkloadStop => "workload_stop",
            Shortfall::Shutdown => "shutdown",
            Shortfall::Teardown => "teardown",
        }
    }

    #[test]
    fn incomplete_is_closed_and_ordered() {
        for mask in 0u8..8 {
            let chosen: Vec<Shortfall> = (0..3)
                .filter(|index| mask & (1 << index) != 0)
                .map(|index| EVERY[index])
                .collect();
            let expected = serde_json::Value::from(
                chosen
                    .iter()
                    .map(|shortfall| wire_name(*shortfall))
                    .collect::<Vec<_>>(),
            );
            for order in ORDERS {
                let mut incomplete = Incomplete::default();
                for index in order {
                    if chosen.contains(&EVERY[index]) {
                        incomplete.add(EVERY[index], false);
                    }
                }
                assert_eq!(serde_json::to_value(&incomplete).unwrap(), expected);
                assert_eq!(
                    incomplete.is_retirement(),
                    !chosen.contains(&Shortfall::Teardown),
                    "{chosen:?}"
                );
                assert_eq!(incomplete.is_empty(), chosen.is_empty());
                let read: Incomplete = serde_json::from_value(expected.clone()).unwrap();
                assert_eq!(read, incomplete);
            }
        }
    }

    #[test]
    fn now_lists_neither_the_workload_stop_nor_shutdown() {
        let mut incomplete = Incomplete::default();
        incomplete.add(Shortfall::WorkloadStop, true);
        incomplete.add(Shortfall::Shutdown, true);
        assert!(incomplete.is_empty());
        assert!(incomplete.is_retirement());
        assert_eq!(serde_json::to_string(&incomplete).unwrap(), "[]");

        incomplete.add(Shortfall::Teardown, true);
        incomplete.add(Shortfall::Shutdown, true);
        incomplete.add(Shortfall::WorkloadStop, true);
        assert_eq!(
            serde_json::to_string(&incomplete).unwrap(),
            r#"["teardown"]"#
        );
        assert!(!incomplete.is_retirement());
    }

    #[test]
    fn incomplete_refuses_any_other_string() {
        let read: Incomplete =
            serde_json::from_str(r#"["teardown","workload_stop","shutdown"]"#).unwrap();
        assert_eq!(
            serde_json::to_string(&read).unwrap(),
            r#"["workload_stop","shutdown","teardown"]"#
        );
        for refused in [
            r#"["safe"]"#,
            r#"["Shutdown"]"#,
            r#"["workload-stop"]"#,
            r#"["teardown",""]"#,
            r#"["teardown","teardown"]"#,
            r#"["shutdown","workload_stop","shutdown"]"#,
            r#"[1]"#,
            r#""teardown""#,
            r#"{"teardown":true}"#,
            "null",
        ] {
            assert!(
                serde_json::from_str::<Incomplete>(refused).is_err(),
                "{refused}"
            );
        }
    }
}
