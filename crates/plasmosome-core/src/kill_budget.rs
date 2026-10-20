use plasmosome_backend::DrainSpec;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

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
        let _ = (arrival, total, now, clock);
        unimplemented!()
    }

    /// What is left of D now. Zero once D has passed.
    pub fn remaining(&self) -> Duration {
        unimplemented!()
    }

    /// The reserve kept for the lock wait: a tenth of D, from 1 ms to 3 s. Fixed for the
    /// request.
    pub fn lock_reserve(&self) -> Duration {
        unimplemented!()
    }

    /// Step 1's share. `NoBudget` with `now`, when the caller skips the step without listing
    /// it, and when less than 1 ms is left for it, when the caller lists `workload_stop`.
    pub fn workload_stop(&self) -> Share {
        unimplemented!()
    }

    /// Step 2's share, the publication exchange included. `NoBudget` when any of its three
    /// parts would be under 1 ms; the caller answers that as a drain timeout, 106.
    pub fn withdrawal(&self) -> Share<Withdrawal> {
        unimplemented!()
    }

    /// Step 3's deadlines. They are never under 1 ms, even after D has passed. With `now` the
    /// drain is 1 ms with `RevokePolicy::Force`; otherwise it is `RevokePolicy::Graceful`.
    pub fn guest_stop(&self) -> GuestStop {
        unimplemented!()
    }

    /// When step 4's lock wait ends: arrival plus D, or the latest instant this platform can
    /// represent before it. The caller tries the lock at least once, even when this has passed.
    pub fn lock_wait_until(&self) -> Instant {
        unimplemented!()
    }
}

/// The deadline `membraned` sends with the 4090 `shutdown` for this drain, strictly below the
/// drain's own deadline. `None` with `RevokePolicy::Force`, and when under 1 ms would be left:
/// then no `shutdown` is sent, and the helper is killed at the drain deadline.
pub fn shutdown_deadline(drain: &DrainSpec) -> Option<Duration> {
    let _ = drain;
    unimplemented!()
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
        let _ = (shortfall, now);
        unimplemented!()
    }

    /// Whether the reply is a retirement: exactly when `teardown` is absent.
    pub fn is_retirement(&self) -> bool {
        unimplemented!()
    }

    /// Whether no step fell short, so the reply carries no `incomplete`.
    pub fn is_empty(&self) -> bool {
        unimplemented!()
    }
}

impl Serialize for Incomplete {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let _ = serializer;
        unimplemented!()
    }
}

impl<'de> Deserialize<'de> for Incomplete {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _ = deserializer;
        unimplemented!()
    }
}
