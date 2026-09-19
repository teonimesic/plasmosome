use crate::readiness::{Cancelled, NotReady, ProbeBudget, ProbeStopped, Readiness};
use crate::vmm::{SpawnError, VmmChild};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// Asks one broker's control socket whether it is serving.
pub trait Probe {
    /// Returns what the socket answered inside the borrowed `budget`, or
    /// `Err(Cancelled)` when the daemon is shutting down. An implementation
    /// must ask again on every call: an answer kept from last time cannot
    /// report a broker that has since stopped serving. The budget is the set's
    /// whole remaining allowance; spending it is reported as
    /// [`NotReady::TimedOut`] by the implementation, never renewed.
    fn probe(&self, socket: &Path, budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled>;
}

/// One broker to spawn, and the control socket it answers on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerSpec {
    pub name: String,
    pub control_socket: PathBuf,
}

/// What a set of brokers last answered. `NotReady` names the broker that held
/// the set back and carries the answer it gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetStatus {
    Ready,
    NotReady {
        broker: String,
        reason: NotReady,
    },
    /// The call's budget ran out before every broker had been asked.
    /// `unreached` is the first broker that was never asked; `asked` names the
    /// brokers that spent the budget, in the order they were probed, so its
    /// last entry is the one that ran the clock out.
    DeadlineSpent {
        unreached: String,
        asked: Vec<String>,
    },
    /// A set with no brokers. Never `Ready`: a cell whose brokers are all
    /// absent has nothing enforcing, and answering ready there would let a
    /// caller treat an unenforced cell as a working one.
    Empty,
}

impl SetStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, SetStatus::Ready)
    }
}

/// Why a set refused or failed to spawn a broker.
#[derive(Debug)]
pub enum SetSpawnError {
    /// The fork itself failed.
    Forked(SpawnError),
    /// Two specs named the same control socket. One socket answering for two
    /// brokers makes a dead broker read as ready.
    DuplicateControlSocket { socket: PathBuf, held_by: String },
}

impl std::fmt::Display for SetSpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetSpawnError::Forked(error) => write!(f, "{error}"),
            SetSpawnError::DuplicateControlSocket { socket, held_by } => write!(
                f,
                "control socket {} is already answered by broker `{held_by}`",
                socket.display()
            ),
        }
    }
}

impl std::error::Error for SetSpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SetSpawnError::Forked(error) => Some(error),
            SetSpawnError::DuplicateControlSocket { .. } => None,
        }
    }
}

/// The broker a set could not spawn, and why.
#[derive(Debug)]
pub struct SpawnFailed {
    pub broker: String,
    pub reason: SetSpawnError,
}

impl std::fmt::Display for SpawnFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "broker `{}` could not be spawned: {}",
            self.broker, self.reason
        )
    }
}

impl std::error::Error for SpawnFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.reason)
    }
}

struct Broker {
    name: String,
    control_socket: PathBuf,
    child: VmmChild,
}

/// A cell's brokers, each one an owned direct child and managed process group.
/// Dropping the set attempts group cleanup and direct-child reap for every broker. Authority
/// loss or an operating-system error is reported by [`VmmChild`] and can leave residue.
pub struct BrokerSet<P> {
    brokers: Vec<Broker>,
    prober: P,
}

impl<P: Probe> BrokerSet<P> {
    /// Spawns one child per spec, in order, through `launcher`. When a spawn
    /// fails, dropping the brokers that already started applies the same managed-group cleanup.
    /// Cleanup failure is reported as a Drop diagnostic because this error reports the spawn.
    ///
    /// Two specs may not share a control socket. One socket answering for two
    /// brokers makes a dead broker read as ready, which is the false positive
    /// the answered-query rule exists to prevent.
    pub fn spawn(
        specs: Vec<BrokerSpec>,
        mut launcher: impl FnMut(&BrokerSpec) -> Result<VmmChild, SpawnError>,
        prober: P,
    ) -> Result<BrokerSet<P>, SpawnFailed> {
        let mut brokers: Vec<Broker> = Vec::with_capacity(specs.len());
        for spec in specs {
            if let Some(other) = brokers
                .iter()
                .find(|held| held.control_socket == spec.control_socket)
            {
                return Err(SpawnFailed {
                    broker: spec.name,
                    reason: SetSpawnError::DuplicateControlSocket {
                        socket: spec.control_socket,
                        held_by: other.name.clone(),
                    },
                });
            }
            match launcher(&spec) {
                Ok(child) => brokers.push(Broker {
                    name: spec.name,
                    control_socket: spec.control_socket,
                    child,
                }),
                Err(reason) => {
                    return Err(SpawnFailed {
                        broker: spec.name,
                        reason: SetSpawnError::Forked(reason),
                    });
                }
            }
        }
        Ok(BrokerSet { brokers, prober })
    }

    /// Asks every broker whether it is serving and returns `Ready` only when
    /// all of them answered ready. Every call asks again. `deadline` is one
    /// fixed budget for the whole call, not for one broker: the probes share
    /// the same clock, so a broker cannot extend the call by trickling its
    /// reply, and a probe that comes back after the budget gets no vote —
    /// that broker reports timed out. A broker the budget never reached is
    /// `DeadlineSpent`, and an empty set is `Empty` — neither is `Ready`.
    ///
    /// `Err(Cancelled)` is not a broker verdict: the daemon is shutting down,
    /// the query has no answer, and no wire state is fabricated for it.
    pub fn status(
        &self,
        deadline: Duration,
        shutdown: &AtomicBool,
    ) -> Result<SetStatus, Cancelled> {
        self.status_with(deadline, shutdown, Instant::now)
    }

    /// The same walk on an injected clock, so tests can place expiry and
    /// cancellation at exact points without sleeping.
    fn status_with(
        &self,
        deadline: Duration,
        shutdown: &AtomicBool,
        now: impl Fn() -> Instant,
    ) -> Result<SetStatus, Cancelled> {
        let budget = ProbeBudget::new_at(deadline, shutdown, now());
        if self.brokers.is_empty() {
            return match budget.remaining_at(now()) {
                Err(ProbeStopped::Cancelled) => Err(Cancelled),
                Ok(_) | Err(ProbeStopped::TimedOut) => Ok(SetStatus::Empty),
            };
        }
        let mut asked = Vec::with_capacity(self.brokers.len());
        for broker in &self.brokers {
            match budget.remaining_at(now()) {
                Err(ProbeStopped::Cancelled) => return Err(Cancelled),
                Err(ProbeStopped::TimedOut) => {
                    return Ok(SetStatus::DeadlineSpent {
                        unreached: broker.name.clone(),
                        asked,
                    });
                }
                Ok(_) => {}
            }
            let answer = self.prober.probe(&broker.control_socket, &budget);
            match budget.remaining_at(now()) {
                Err(ProbeStopped::Cancelled) => return Err(Cancelled),
                Err(ProbeStopped::TimedOut) => {
                    return Ok(SetStatus::NotReady {
                        broker: broker.name.clone(),
                        reason: NotReady::TimedOut,
                    });
                }
                Ok(_) => {}
            }
            match answer {
                Err(Cancelled) => return Err(Cancelled),
                Ok(Readiness::NotReady(reason)) => {
                    return Ok(SetStatus::NotReady {
                        broker: broker.name.clone(),
                        reason,
                    });
                }
                Ok(Readiness::Ready { .. }) => asked.push(broker.name.clone()),
            }
        }
        Ok(SetStatus::Ready)
    }

    /// The process ids of the set's brokers, in spawn order. Once the set has
    /// been dropped these pids may belong to unrelated processes, so a caller
    /// must not signal them.
    pub fn pids(&self) -> Vec<i32> {
        self.brokers
            .iter()
            .map(|broker| broker.child.pid())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmm::Launch;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    const DEADLINE: Duration = Duration::from_millis(500);

    struct SleepForever;

    impl Launch for SleepForever {
        fn launch(self) -> ! {
            loop {
                unsafe { libc::pause() };
            }
        }
    }

    struct ScriptedProbe {
        scripts: Mutex<HashMap<PathBuf, Vec<Readiness>>>,
    }

    impl ScriptedProbe {
        fn new() -> ScriptedProbe {
            ScriptedProbe {
                scripts: Mutex::new(HashMap::new()),
            }
        }

        fn answering(self, socket: &Path, answers: Vec<Readiness>) -> ScriptedProbe {
            self.scripts
                .lock()
                .expect("the script of probe answers is uncontended")
                .insert(socket.to_path_buf(), answers);
            self
        }
    }

    impl Probe for ScriptedProbe {
        fn probe(&self, socket: &Path, _budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
            let mut scripts = self
                .scripts
                .lock()
                .expect("the script of probe answers is uncontended");
            let script = scripts.get_mut(socket).unwrap_or_else(|| {
                panic!("no scripted answer for the broker at {}", socket.display())
            });
            if script.len() > 1 {
                Ok(script.remove(0))
            } else {
                Ok(script[0].clone())
            }
        }
    }

    struct CostlyProbe {
        cost: Duration,
        budgets: Mutex<Vec<Option<Duration>>>,
    }

    impl CostlyProbe {
        fn costing(cost: Duration) -> CostlyProbe {
            CostlyProbe {
                cost,
                budgets: Mutex::new(Vec::new()),
            }
        }

        fn budgets_handed_out(&self) -> Vec<Option<Duration>> {
            self.budgets
                .lock()
                .expect("the record of probe budgets is uncontended")
                .clone()
        }
    }

    impl Probe for CostlyProbe {
        fn probe(&self, _socket: &Path, budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
            let left = budget.remaining().ok();
            self.budgets
                .lock()
                .expect("the record of probe budgets is uncontended")
                .push(left);
            std::thread::sleep(self.cost.min(left.unwrap_or_default()));
            if left.is_none_or(|left| self.cost > left) {
                return Ok(Readiness::NotReady(NotReady::TimedOut));
            }
            Ok(ready())
        }
    }

    impl Probe for Arc<CostlyProbe> {
        fn probe(&self, socket: &Path, budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
            CostlyProbe::probe(self, socket, budget)
        }
    }

    struct OvershootingProbe {
        cost: Duration,
    }

    impl Probe for OvershootingProbe {
        fn probe(&self, _socket: &Path, _budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
            std::thread::sleep(self.cost);
            Ok(ready())
        }
    }

    struct TimedProbe {
        clock: Clock,
        script: Mutex<Vec<(u64, Readiness)>>,
    }

    impl TimedProbe {
        fn scripted(clock: Clock, script: Vec<(u64, Readiness)>) -> TimedProbe {
            TimedProbe {
                clock,
                script: Mutex::new(script),
            }
        }
    }

    impl Probe for TimedProbe {
        fn probe(&self, _socket: &Path, _budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
            let mut script = self.script.lock().expect("the probe script is uncontended");
            let (at, answer) = if script.len() > 1 {
                script.remove(0)
            } else {
                script[0].clone()
            };
            self.clock.advance_to(at);
            Ok(answer)
        }
    }

    #[derive(Clone)]
    struct Clock {
        base: Instant,
        ms: Rc<Cell<u64>>,
    }

    impl Clock {
        fn new() -> Clock {
            Clock {
                base: Instant::now(),
                ms: Rc::new(Cell::new(0)),
            }
        }

        fn now(&self) -> impl Fn() -> Instant + '_ {
            let ms = Rc::clone(&self.ms);
            let base = self.base;
            move || {
                let next = ms.get() + 1;
                ms.set(next);
                base.checked_add(Duration::from_millis(next))
                    .expect("the test clock stays in range")
            }
        }

        fn advance_to(&self, ms: u64) {
            if ms > self.ms.get() {
                self.ms.set(ms);
            }
        }
    }

    fn ready() -> Readiness {
        Readiness::Ready {
            state: "serving".to_string(),
        }
    }

    fn starting() -> Readiness {
        Readiness::NotReady(NotReady::Reported {
            state: "starting".to_string(),
        })
    }

    fn gone(socket: &Path) -> Readiness {
        Readiness::NotReady(NotReady::Unreachable {
            path: socket.to_path_buf(),
        })
    }

    fn spec(dir: &Path, name: &str) -> BrokerSpec {
        BrokerSpec {
            name: name.to_string(),
            control_socket: dir.join(format!("{name}.control")),
        }
    }

    fn forking() -> impl FnMut(&BrokerSpec) -> Result<VmmChild, SpawnError> {
        |_spec: &BrokerSpec| VmmChild::spawn(SleepForever)
    }

    fn assert_reaped(pid: i32, broker: &str) {
        let mut status: libc::c_int = 0;
        let observed = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        let errno = std::io::Error::last_os_error().raw_os_error();
        assert!(
            observed == -1 && errno == Some(libc::ECHILD),
            "broker `{broker}` at pid {pid} was still a child of this process, so it outlived the set"
        );
    }

    #[test]
    fn a_set_is_ready_only_when_every_broker_answers() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let specs = vec![spec(dir.path(), "egressd"), spec(dir.path(), "dnsd")];

        let all_answering = ScriptedProbe::new()
            .answering(&specs[0].control_socket, vec![ready()])
            .answering(&specs[1].control_socket, vec![ready()]);
        let serving =
            BrokerSet::spawn(specs.clone(), forking(), all_answering).expect("every broker forks");
        assert_eq!(
            serving.status(DEADLINE, &flag),
            Ok(SetStatus::Ready),
            "a set whose brokers all answer ready must be ready"
        );

        let one_short = ScriptedProbe::new()
            .answering(&specs[0].control_socket, vec![ready()])
            .answering(&specs[1].control_socket, vec![starting()]);
        let held_back =
            BrokerSet::spawn(specs.clone(), forking(), one_short).expect("every broker forks");
        assert_eq!(
            held_back.status(DEADLINE, &flag),
            Ok(SetStatus::NotReady {
                broker: "dnsd".to_string(),
                reason: NotReady::Reported {
                    state: "starting".to_string()
                }
            }),
            "`dnsd` is not serving, so the set must not report ready"
        );
    }

    #[test]
    fn a_broker_that_stops_answering_flips_the_set_to_not_ready() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let only = spec(dir.path(), "egressd");
        let socket = only.control_socket.clone();
        let prober = ScriptedProbe::new().answering(&socket, vec![ready(), gone(&socket)]);
        let set = BrokerSet::spawn(vec![only], forking(), prober).expect("the broker forks");

        assert_eq!(
            set.status(DEADLINE, &flag),
            Ok(SetStatus::Ready),
            "`egressd` answered ready, so the set is ready"
        );
        assert_eq!(
            set.status(DEADLINE, &flag),
            Ok(SetStatus::NotReady {
                broker: "egressd".to_string(),
                reason: NotReady::Unreachable { path: socket }
            }),
            "`egressd` stopped answering, so the set must ask again and report it rather than repeat the ready it got before"
        );
    }

    #[test]
    fn a_set_reports_which_broker_is_not_ready() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let specs = vec![
            spec(dir.path(), "egressd"),
            spec(dir.path(), "dnsd"),
            spec(dir.path(), "credentiald"),
        ];
        let prober = ScriptedProbe::new()
            .answering(&specs[0].control_socket, vec![ready()])
            .answering(&specs[1].control_socket, vec![starting()])
            .answering(&specs[2].control_socket, vec![ready()]);
        let set = BrokerSet::spawn(specs, forking(), prober).expect("every broker forks");

        match set.status(DEADLINE, &flag) {
            Ok(SetStatus::NotReady { broker, reason }) => {
                assert_eq!(
                    broker, "dnsd",
                    "the set must name the broker holding it back"
                );
                assert_eq!(
                    reason,
                    NotReady::Reported {
                        state: "starting".to_string()
                    },
                    "the set must carry the answer `dnsd` gave, not only that it was not ready"
                );
            }
            other => panic!("`dnsd` reported not ready, so the set cannot be {other:?}"),
        }
    }

    #[test]
    fn dropping_a_set_reaps_every_broker() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let names = ["egressd", "dnsd", "credentiald"];
        let specs = names.iter().map(|name| spec(dir.path(), name)).collect();
        let set =
            BrokerSet::spawn(specs, forking(), ScriptedProbe::new()).expect("every broker forks");

        let pids = set.pids();
        assert_eq!(
            pids.len(),
            names.len(),
            "every spec must have become a child"
        );
        drop(set);

        for (pid, name) in pids.into_iter().zip(names) {
            assert_reaped(pid, name);
        }
    }

    #[test]
    fn one_deadline_covers_the_whole_set_however_many_brokers_it_has() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let names = [
            "egressd",
            "dnsd",
            "credentiald",
            "filed",
            "clockd",
            "logd",
            "keyd",
            "netd",
        ];
        let specs = names.iter().map(|name| spec(dir.path(), name)).collect();
        let prober = Arc::new(CostlyProbe::costing(Duration::from_millis(90)));
        let set =
            BrokerSet::spawn(specs, forking(), Arc::clone(&prober)).expect("every broker forks");
        let budget = Duration::from_millis(200);

        let started = Instant::now();
        let status = set.status(budget, &flag);
        let elapsed = started.elapsed();

        let handed_out = prober.budgets_handed_out();
        assert!(
            handed_out.windows(2).all(|pair| pair[1] < pair[0]),
            "each probe must see what is left of the set's budget, but the budgets handed out were {handed_out:?}"
        );
        assert!(
            elapsed < budget * 2,
            "a set of {} brokers took {elapsed:?} to answer, far past the single {budget:?} budget its caller allowed",
            names.len()
        );
        assert!(
            !status.as_ref().is_ok_and(SetStatus::is_ready),
            "the budget ran out before every broker had answered, so the set cannot be ready, got {status:?}"
        );
    }

    #[test]
    fn a_probe_that_overshoots_the_budget_gets_no_vote() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let specs = vec![spec(dir.path(), "egressd"), spec(dir.path(), "dnsd")];
        let prober = OvershootingProbe {
            cost: Duration::from_millis(150),
        };
        let set = BrokerSet::spawn(specs, forking(), prober).expect("every broker forks");

        let status = set.status(Duration::from_millis(100), &flag);

        assert!(
            !status.as_ref().is_ok_and(SetStatus::is_ready),
            "a set with an overshooting broker cannot be ready"
        );
        assert_eq!(
            status,
            Ok(SetStatus::NotReady {
                broker: "egressd".to_string(),
                reason: NotReady::TimedOut,
            }),
            "`egressd` spent the whole budget and answered late, so its ready is refused and it reports timed out"
        );
    }

    #[test]
    fn a_broker_the_budget_never_reached_is_named_with_the_ones_that_spent_it() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let specs = vec![
            spec(dir.path(), "egressd"),
            spec(dir.path(), "dnsd"),
            spec(dir.path(), "credentiald"),
        ];
        let clock = Clock::new();
        let prober = TimedProbe::scripted(
            clock.clone(),
            vec![(99, ready()), (0, ready()), (0, ready())],
        );
        let set = BrokerSet::spawn(specs, forking(), prober).expect("every broker forks");

        let status = set.status_with(Duration::from_millis(100), &flag, clock.now());

        assert!(
            !status.as_ref().is_ok_and(SetStatus::is_ready),
            "a set with a broker that was never asked cannot be ready"
        );
        assert_eq!(
            status,
            Ok(SetStatus::DeadlineSpent {
                unreached: "dnsd".to_string(),
                asked: vec!["egressd".to_string()],
            }),
            "expiry between completed probes names the broker the budget never reached"
        );
    }

    #[test]
    fn a_last_broker_that_answers_at_expiry_reports_timed_out() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let specs = vec![spec(dir.path(), "egressd"), spec(dir.path(), "dnsd")];
        let clock = Clock::new();
        let prober = TimedProbe::scripted(clock.clone(), vec![(0, ready()), (150, ready())]);
        let set = BrokerSet::spawn(specs, forking(), prober).expect("every broker forks");

        let status = set.status_with(Duration::from_millis(100), &flag, clock.now());

        assert_eq!(
            status,
            Ok(SetStatus::NotReady {
                broker: "dnsd".to_string(),
                reason: NotReady::TimedOut,
            }),
            "a ready that landed on the budget's last moment is a late answer, and the set refuses it"
        );
    }

    #[test]
    fn a_set_with_no_brokers_is_never_ready() {
        let flag = AtomicBool::new(false);
        let set = match BrokerSet::spawn(Vec::new(), forking(), ScriptedProbe::new()) {
            Ok(set) => set,
            Err(failure) => panic!("an empty set spawns nothing and cannot fail: {failure}"),
        };

        assert_eq!(
            set.status(Duration::from_millis(50), &flag),
            Ok(SetStatus::Empty),
            "a cell with no brokers has nothing enforcing and must not answer ready"
        );
        assert!(
            !set.status(Duration::from_millis(50), &flag)
                .as_ref()
                .is_ok_and(SetStatus::is_ready),
            "an empty set must not read as ready"
        );
    }

    #[test]
    fn a_set_with_no_brokers_still_reports_cancellation() {
        let flag = AtomicBool::new(true);
        let set = BrokerSet::spawn(Vec::new(), forking(), ScriptedProbe::new())
            .expect("an empty set spawns nothing");
        assert_eq!(
            set.status(Duration::from_secs(10), &flag),
            Err(Cancelled),
            "shutdown is checked even when there is nothing to ask"
        );
    }

    #[test]
    fn a_spent_budget_names_the_first_broker_unreached_with_nothing_asked() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = AtomicBool::new(false);
        let specs = vec![spec(dir.path(), "egressd"), spec(dir.path(), "dnsd")];
        let prober = ScriptedProbe::new().answering(&specs[0].control_socket, vec![ready()]);
        let set = BrokerSet::spawn(specs, forking(), prober).expect("every broker forks");

        let status = set.status(Duration::ZERO, &flag);

        assert_eq!(
            status,
            Ok(SetStatus::DeadlineSpent {
                unreached: "egressd".to_string(),
                asked: Vec::new(),
            }),
            "a zero budget enters no broker at all"
        );
    }

    #[test]
    fn a_shutdown_during_a_probe_cancels_the_whole_status() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let flag = Arc::new(AtomicBool::new(false));
        let specs = vec![spec(dir.path(), "egressd"), spec(dir.path(), "dnsd")];
        let cancelling = CancellingProbe {
            flag: Arc::clone(&flag),
        };
        let set = BrokerSet::spawn(specs, forking(), cancelling).expect("every broker forks");

        assert_eq!(
            set.status(DEADLINE, &flag),
            Err(Cancelled),
            "a shutdown seen inside a probe cancels the query: no verdict, no next broker"
        );
    }

    struct CancellingProbe {
        flag: Arc<AtomicBool>,
    }

    impl Probe for CancellingProbe {
        fn probe(&self, _socket: &Path, _budget: &ProbeBudget<'_>) -> Result<Readiness, Cancelled> {
            self.flag.store(true, Ordering::Relaxed);
            Err(Cancelled)
        }
    }

    #[test]
    fn two_brokers_may_not_share_a_control_socket() {
        let home = tempfile::tempdir().expect("a temporary directory");
        let shared = home.path().join("shared.uds");
        let specs = vec![
            BrokerSpec {
                name: "egressd".to_string(),
                control_socket: shared.clone(),
            },
            BrokerSpec {
                name: "dnsd".to_string(),
                control_socket: shared.clone(),
            },
        ];

        let failure = match BrokerSet::spawn(specs, forking(), ScriptedProbe::new()) {
            Ok(_) => panic!("a shared control socket must be refused"),
            Err(failure) => failure,
        };

        assert_eq!(failure.broker, "dnsd");
        match failure.reason {
            SetSpawnError::DuplicateControlSocket { held_by, .. } => {
                assert_eq!(
                    held_by, "egressd",
                    "the refusal names the broker that holds it"
                )
            }
            other => panic!("expected a duplicate control socket, got {other}"),
        }
    }

    #[test]
    fn a_fork_failure_reaps_what_was_already_spawned() {
        let dir = tempfile::tempdir().expect("a temporary directory for the control sockets");
        let specs = vec![
            spec(dir.path(), "egressd"),
            spec(dir.path(), "dnsd"),
            spec(dir.path(), "credentiald"),
        ];
        let mut spawned = Vec::new();

        let attempt = BrokerSet::spawn(
            specs,
            |spec: &BrokerSpec| {
                if spec.name == "credentiald" {
                    return Err(SpawnError::ForkFailed(std::io::Error::from_raw_os_error(
                        libc::EAGAIN,
                    )));
                }
                let child = VmmChild::spawn(SleepForever)?;
                spawned.push((child.pid(), spec.name.clone()));
                Ok(child)
            },
            ScriptedProbe::new(),
        );
        let failure = match attempt {
            Err(failure) => failure,
            Ok(_) => panic!("`credentiald` cannot fork, so the set cannot be spawned"),
        };

        assert_eq!(
            failure.broker, "credentiald",
            "the error must name the broker that could not be spawned"
        );
        assert_eq!(
            spawned.len(),
            2,
            "`egressd` and `dnsd` must have been spawned before `credentiald` failed"
        );
        for (pid, name) in spawned {
            assert_reaped(pid, &name);
        }
    }
}
