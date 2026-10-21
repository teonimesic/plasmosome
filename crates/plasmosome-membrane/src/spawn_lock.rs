use crate::vmm::SpawnError;
use plasmosome_core::private_socket::PrivateListener;
use std::cell::Cell;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{PoisonError, RwLock};

static SPAWN_LOCK: RwLock<()> = RwLock::new(());

thread_local! {
    static CREATING_DESCRIPTORS: Cell<bool> = const { Cell::new(false) };
}

pub(crate) enum Forked {
    Child,
    Parent(libc::pid_t),
}

/// Proof that the write side of the spawn lock is held. Only
/// `with_descriptors_held` and `with_each_descriptor_held` make one, so a
/// function that takes it can run only inside descriptor creation.
pub(crate) struct DescriptorsHeld(());

/// A value that owns one descriptor, which `with_descriptors_held` checks for
/// close-on-exec.
///
/// Implement it only for a type that owns its descriptor, so that dropping
/// the value closes it: `with_descriptors_held` closes an inheritable
/// descriptor before the lock is released only by dropping the value. The
/// descriptor is lent for that check alone. Never close it, accept on it,
/// change its flags or keep a copy of it. It is lent as a `RawFd`, not a
/// `BorrowedFd`, so making a copy needs `unsafe`.
pub(crate) trait OwnsDescriptor {
    /// The descriptor this value owns.
    fn descriptor(&self) -> RawFd;
}

impl OwnsDescriptor for OwnedFd {
    fn descriptor(&self) -> RawFd {
        todo!()
    }
}

impl OwnsDescriptor for UnixStream {
    fn descriptor(&self) -> RawFd {
        todo!()
    }
}

impl OwnsDescriptor for PrivateListener {
    fn descriptor(&self) -> RawFd {
        todo!()
    }
}

/// Runs `create` while no `VmmChild::spawn` in this process can fork.
///
/// Use it only to create a descriptor and mark it close-on-exec, where the
/// platform needs two calls for that. `create` receives the `DescriptorsHeld`
/// proof, so a function that takes one cannot be called outside. `create`
/// returns the value that owns the descriptor, and only that descriptor is
/// checked: if it is not close-on-exec when `create` returns, it is closed
/// before the lock is released, and an error is returned instead. A
/// descriptor created inside `create` and kept anywhere else is not checked,
/// so the caller must make it close-on-exec before `create` returns.
///
/// Never hold it across a blocking call such as `accept`: every
/// `VmmChild::spawn` in the process waits until `create` returns. A spawn on
/// this thread from inside `create` forks nothing and returns
/// `SpawnError::DescriptorLockHeld`, and a nested call runs under the lock
/// already held. Only descriptors created inside `create` are held back from
/// a fork, and forks that bypass this lock, such as `std::process::Command`,
/// are not held back.
pub(crate) fn with_descriptors_held<T: AsFd>(
    create: impl FnOnce(&DescriptorsHeld) -> std::io::Result<T>,
) -> std::io::Result<T> {
    write_held(&SPAWN_LOCK, || {
        create(&DescriptorsHeld(())).and_then(close_on_exec_or_closed)
    })
}

/// Runs `create` as `with_descriptors_held` does, for a creation that makes
/// several descriptors, such as the two ends of a pipe.
///
/// `create` returns every descriptor it made, and each one is checked: if any
/// is not close-on-exec when `create` returns, all of them are closed before
/// the lock is released, and an error is returned instead. What
/// `with_descriptors_held` says about blocking calls, spawns, nesting and
/// descriptors kept elsewhere holds here too.
pub(crate) fn with_each_descriptor_held<const N: usize>(
    create: impl FnOnce(&DescriptorsHeld) -> std::io::Result<[OwnedFd; N]>,
) -> std::io::Result<[OwnedFd; N]> {
    write_held(&SPAWN_LOCK, || {
        create(&DescriptorsHeld(())).and_then(|created| {
            if created.iter().all(|fd| is_close_on_exec(fd.as_fd())) {
                Ok(created)
            } else {
                Err(not_close_on_exec())
            }
        })
    })
}

#[allow(unsafe_code)]
fn is_close_on_exec(fd: std::os::fd::BorrowedFd<'_>) -> bool {
    // SAFETY: fcntl with F_GETFD takes no pointer, and the borrow keeps the descriptor open for the
    // call.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
    flags != -1 && flags & libc::FD_CLOEXEC != 0
}

fn not_close_on_exec() -> std::io::Error {
    std::io::Error::other(
        "a descriptor created under the spawn lock was not close-on-exec, so it was closed",
    )
}

#[cfg(test)]
pub(crate) fn hold_fork_side() -> std::sync::RwLockReadGuard<'static, ()> {
    SPAWN_LOCK.read().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) fn waits_for_the_write_side(thread: std::thread::ThreadId) -> bool {
    WAITING_FOR_THE_WRITE_SIDE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&thread)
}

fn close_on_exec_or_closed<T: AsFd>(created: T) -> std::io::Result<T> {
    if is_close_on_exec(created.as_fd()) {
        Ok(created)
    } else {
        Err(not_close_on_exec())
    }
}

/// Forks while holding the read side of the spawn lock. In `Forked::Child`, do
/// only async-signal-safe work, and never return or unwind.
#[allow(unsafe_code)]
pub(crate) fn fork() -> Result<Forked, SpawnError> {
    // SAFETY: This function's doc binds every caller to do only async-signal-safe work in
    // Forked::Child and never return or unwind there. Its one caller, VmmChild::spawn, calls only
    // setsid and _exit itself, and its ExitOnUnwind guard exits the child if `launch` panics, after
    // the panic hook has run. That `launch` does only async-signal-safe work rests on Launch's
    // documented contract, which safe code can break, as the test launcher PanicOnLaunch does.
    // fork_holding holds the read side of the spawn lock, so no descriptor made inside
    // with_descriptors_held or with_each_descriptor_held is mid-creation. The lock does not cover a
    // descriptor made in two calls anywhere else, such as a socket or connection std makes on
    // macOS, which has no SOCK_CLOEXEC or accept4. daemon::run's doc gives the order that keeps the
    // daemon's own sockets out of a fork.
    fork_holding(&SPAWN_LOCK, || unsafe { libc::fork() })
}

fn write_held<T>(lock: &RwLock<()>, create: impl FnOnce() -> T) -> T {
    if CREATING_DESCRIPTORS.get() {
        return create();
    }
    let _held = write_side(lock);
    let _creating = Creating::mark();
    create()
}

fn write_side(lock: &RwLock<()>) -> std::sync::RwLockWriteGuard<'_, ()> {
    #[cfg(test)]
    let _waiting = WaitsForTheWriteSide::mark();
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
static WAITING_FOR_THE_WRITE_SIDE: std::sync::Mutex<Vec<std::thread::ThreadId>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
struct WaitsForTheWriteSide(std::thread::ThreadId);

#[cfg(test)]
impl WaitsForTheWriteSide {
    fn mark() -> WaitsForTheWriteSide {
        let waiting = std::thread::current().id();
        WAITING_FOR_THE_WRITE_SIDE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(waiting);
        WaitsForTheWriteSide(waiting)
    }
}

#[cfg(test)]
impl Drop for WaitsForTheWriteSide {
    fn drop(&mut self) {
        let mut waiting = WAITING_FOR_THE_WRITE_SIDE
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = waiting.iter().position(|thread| *thread == self.0) {
            waiting.swap_remove(at);
        }
    }
}

struct Creating;

impl Creating {
    fn mark() -> Creating {
        CREATING_DESCRIPTORS.set(true);
        Creating
    }
}

impl Drop for Creating {
    fn drop(&mut self) {
        CREATING_DESCRIPTORS.set(false);
    }
}

fn fork_holding(
    lock: &RwLock<()>,
    fork: impl FnOnce() -> libc::pid_t,
) -> Result<Forked, SpawnError> {
    if CREATING_DESCRIPTORS.get() {
        return Err(SpawnError::DescriptorLockHeld);
    }
    let held = lock.read().unwrap_or_else(PoisonError::into_inner);
    match fork() {
        0 => {
            std::mem::forget(held);
            Ok(Forked::Child)
        }
        pid if pid < 0 => Err(SpawnError::ForkFailed(std::io::Error::last_os_error())),
        pid => Ok(Forked::Parent(pid)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmm::{Launch, SpawnError, VmmChild, VmmState};
    use std::cell::Cell;
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, TryLockError};
    use std::time::{Duration, Instant};

    const PATIENCE: Duration = Duration::from_secs(10);
    const ABORT_AFTER: Duration = Duration::from_secs(60);
    const HOLD: Duration = Duration::from_millis(250);
    const ARRANGE_WITHIN: Duration = Duration::from_secs(30);

    struct ExitAtOnce;

    impl Launch for ExitAtOnce {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            // SAFETY: _exit takes no pointer and is async-signal-safe, which is all this forked
            // child may call.
            unsafe { libc::_exit(0) }
        }
    }

    struct ReportsInheritedReadSide;

    impl Launch for ReportsInheritedReadSide {
        #[allow(unsafe_code)]
        fn launch(self) -> ! {
            let code = match SPAWN_LOCK.try_write() {
                Err(TryLockError::WouldBlock) => 0,
                Err(TryLockError::Poisoned(acquired)) => {
                    std::mem::forget(acquired.into_inner());
                    3
                }
                Ok(acquired) => {
                    std::mem::forget(acquired);
                    3
                }
            };
            // SAFETY: _exit takes no pointer and is async-signal-safe, which is all this forked
            // child may call.
            unsafe { libc::_exit(code) }
        }
    }

    struct RecordsWhereItCloses<'a> {
        fd: Option<OwnedFd>,
        closed_under_lock: &'a Cell<Option<bool>>,
    }

    impl AsFd for RecordsWhereItCloses<'_> {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.fd
                .as_ref()
                .expect("the descriptor is still held")
                .as_fd()
        }
    }

    impl From<RecordsWhereItCloses<'_>> for OwnedFd {
        fn from(mut held: RecordsWhereItCloses<'_>) -> OwnedFd {
            held.fd.take().expect("the descriptor is still held")
        }
    }

    impl Drop for RecordsWhereItCloses<'_> {
        fn drop(&mut self) {
            if self.fd.is_some() {
                self.closed_under_lock.set(Some(matches!(
                    SPAWN_LOCK.try_read(),
                    Err(TryLockError::WouldBlock)
                )));
            }
        }
    }

    fn fresh_lock() -> &'static RwLock<()> {
        Box::leak(Box::new(RwLock::new(())))
    }

    fn read_side_held(lock: &RwLock<()>) -> bool {
        matches!(lock.try_write(), Err(TryLockError::WouldBlock))
    }

    #[allow(unsafe_code)]
    fn inheritable_null() -> OwnedFd {
        // SAFETY: The path is a static NUL-terminated string, and open keeps no pointer to it.
        let raw = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(raw >= 0, "/dev/null opens");
        // SAFETY: The assert above shows `raw` is a non-negative descriptor, and nothing else owns
        // it.
        unsafe { OwnedFd::from_raw_fd(raw) }
    }

    fn close_on_exec_null() -> std::io::Result<OwnedFd> {
        std::fs::File::open("/dev/null").map(OwnedFd::from)
    }

    fn on_a_thread<T: Send + 'static>(
        run: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, RecvTimeoutError> {
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(run());
        });
        received.recv_timeout(PATIENCE)
    }

    #[test]
    fn a_spawned_child_was_forked_holding_the_read_side_of_the_spawn_lock() {
        let mut child = VmmChild::spawn(ReportsInheritedReadSide).expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 }),
            "exit 3 means the child's copy of the spawn lock had no read side held"
        );
    }

    #[test]
    fn descriptor_creation_runs_holding_the_write_side_of_the_spawn_lock() {
        let mut write_side_held = false;
        let created = with_descriptors_held(|_held| {
            write_side_held = matches!(SPAWN_LOCK.try_read(), Err(TryLockError::WouldBlock));
            close_on_exec_null()
        });
        assert!(created.is_ok(), "a close-on-exec descriptor is returned");
        assert!(
            write_side_held,
            "creation ran holding the write side of the spawn lock"
        );
    }

    #[test]
    fn a_descriptor_still_inheritable_when_creation_returns_is_closed_under_the_lock() {
        let closed_under_lock = Cell::new(None);
        let created = with_descriptors_held(|_held| {
            Ok(RecordsWhereItCloses {
                fd: Some(inheritable_null()),
                closed_under_lock: &closed_under_lock,
            })
        });
        let refusal = created.err().map(|error| error.to_string());
        assert_eq!(
            refusal.as_deref(),
            Some(
                "a descriptor created under the spawn lock was not close-on-exec, so it was closed"
            )
        );
        assert_eq!(
            closed_under_lock.get(),
            Some(true),
            "the descriptor was closed before the write side was released"
        );
    }

    #[test]
    fn creating_several_descriptors_runs_holding_the_write_side_and_returns_them_all() {
        let mut write_side_held = false;
        let created = with_each_descriptor_held(|_held| {
            write_side_held = matches!(SPAWN_LOCK.try_read(), Err(TryLockError::WouldBlock));
            Ok([close_on_exec_null()?, close_on_exec_null()?])
        });
        let created = created.expect("two close-on-exec descriptors are returned");
        assert!(
            created.iter().all(|fd| fd.as_raw_fd() >= 0),
            "both descriptors came back"
        );
        assert!(
            write_side_held,
            "creation ran holding the write side of the spawn lock"
        );
    }

    #[test]
    fn several_descriptors_are_refused_when_any_one_is_still_inheritable() {
        for inheritable_at in [0, 1] {
            let created = with_each_descriptor_held(|_held| {
                let mut pair = [close_on_exec_null()?, close_on_exec_null()?];
                pair[inheritable_at] = inheritable_null();
                Ok(pair)
            });
            let refusal = created.err().map(|error| error.to_string());
            assert_eq!(
                refusal.as_deref(),
                Some(
                    "a descriptor created under the spawn lock was not close-on-exec, so it was closed"
                ),
                "an inheritable descriptor at index {inheritable_at} refuses the creation"
            );
        }
    }

    struct ForkDuringCreation {
        reached_the_lock_while_held: bool,
        forked_while_held: bool,
        child_status: libc::c_int,
    }

    #[allow(unsafe_code)]
    fn reaped_within(pid: libc::pid_t, patience: Duration) -> libc::c_int {
        let deadline = Instant::now() + patience;
        let mut status = 0;
        loop {
            // SAFETY: `status` is a local the call may write, and `pid` is a child this test forked
            // and has not reaped.
            match unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } {
                reaped if reaped == pid => return status,
                0 if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
                0 => {
                    // SAFETY: kill takes no pointer. waitpid just returned 0, so `pid` is an
                    // unreaped child of this process and still names it.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    // SAFETY: `status` is a local the call may write, and `pid` is the unreaped
                    // child just killed.
                    unsafe { libc::waitpid(pid, &mut status, 0) };
                    panic!("the child did not exit within {patience:?}, so it was killed");
                }
                _ if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {}
                other => panic!(
                    "waitpid returned {other}: {}",
                    std::io::Error::last_os_error()
                ),
            }
        }
    }

    #[allow(unsafe_code)]
    fn fork_during_creation(lock: &'static RwLock<()>) -> ForkDuringCreation {
        let released = Arc::new(AtomicBool::new(false));
        let (starting, started) = mpsc::channel();
        let (answer, answered) = mpsc::channel();
        let watched = write_held(lock, || {
            let watched = inheritable_null();
            let fd = watched.as_raw_fd();
            let seen = Arc::clone(&released);
            std::thread::spawn(move || {
                let _ = starting.send(());
                let reached_the_lock_while_held = !seen.load(Ordering::SeqCst);
                let mut forked_while_held = false;
                let forked = fork_holding(lock, || {
                    forked_while_held = !seen.load(Ordering::SeqCst);
                    // SAFETY: The forked child only reads one descriptor's flags with fcntl and
                    // calls _exit, both async-signal-safe.
                    unsafe { libc::fork() }
                });
                let outcome = match forked {
                    Ok(Forked::Child) => {
                        // SAFETY: fcntl with F_GETFD takes no pointer and is async-signal-safe, and
                        // the child only queries `fd`.
                        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
                        let code = if flags == -1 {
                            4
                        } else if flags & libc::FD_CLOEXEC != 0 {
                            0
                        } else {
                            3
                        };
                        // SAFETY: _exit takes no pointer and is async-signal-safe, which is all
                        // this forked child may call.
                        unsafe { libc::_exit(code) }
                    }
                    Ok(Forked::Parent(pid)) => {
                        Ok((reached_the_lock_while_held, forked_while_held, pid))
                    }
                    Err(error) => Err(error),
                };
                let _ = answer.send(outcome);
            });
            started
                .recv_timeout(PATIENCE)
                .expect("the forking thread starts");
            std::thread::sleep(HOLD);
            assert_eq!(
                // SAFETY: fcntl with F_SETFD takes an integer flag, not a pointer, and `watched`
                // keeps `fd` open.
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0,
                "the descriptor is marked close-on-exec"
            );
            released.store(true, Ordering::SeqCst);
            watched
        });
        let (reached_the_lock_while_held, forked_while_held, pid) = answered
            .recv_timeout(PATIENCE)
            .expect("the forking thread forks once creation ends; a timeout means it never got the lock")
            .expect("the fork succeeds");
        let child_status = reaped_within(pid, PATIENCE);
        drop(watched);
        ForkDuringCreation {
            reached_the_lock_while_held,
            forked_while_held,
            child_status,
        }
    }

    #[test]
    fn a_forked_child_never_inherits_a_descriptor_before_it_is_close_on_exec() {
        let lock = fresh_lock();
        let deadline = Instant::now() + ARRANGE_WITHIN;
        let attempt = loop {
            let attempt = fork_during_creation(lock);
            if attempt.reached_the_lock_while_held {
                break attempt;
            }
            assert!(
                Instant::now() < deadline,
                "the forking thread never reached the lock while creation was in progress"
            );
        };
        assert!(
            !attempt.forked_while_held,
            "the fork ran while another thread was creating a descriptor"
        );
        assert!(libc::WIFEXITED(attempt.child_status), "the child exits");
        assert_eq!(
            libc::WEXITSTATUS(attempt.child_status),
            0,
            "exit 3 means the child inherited the descriptor before it was close-on-exec"
        );
    }

    #[test]
    fn the_fork_runs_while_the_read_side_is_held() {
        let lock = fresh_lock();
        let mut held_during_fork = false;
        let forked = fork_holding(lock, || {
            held_during_fork = read_side_held(lock);
            4242
        });
        assert!(matches!(forked, Ok(Forked::Parent(4242))));
        assert!(
            held_during_fork,
            "the fork ran while the read side was held"
        );
        assert!(
            !read_side_held(lock),
            "the parent releases the read side after the fork"
        );
    }

    #[test]
    fn the_child_leaves_the_inherited_read_side_untouched() {
        let lock = fresh_lock();
        assert!(matches!(fork_holding(lock, || 0), Ok(Forked::Child)));
        assert!(
            read_side_held(lock),
            "the child neither unlocks nor drops the guard it inherited"
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn a_failed_fork_keeps_its_errno_and_releases_the_lock() {
        let lock = fresh_lock();
        let forked = fork_holding(lock, || {
            // SAFETY: close(-1) closes nothing: it fails with EBADF, the errno this test needs.
            unsafe { libc::close(-1) };
            -1
        });
        match forked {
            Err(SpawnError::ForkFailed(error)) => {
                assert_eq!(error.raw_os_error(), Some(libc::EBADF))
            }
            Err(other) => panic!("a failed fork is ForkFailed, got {other:?}"),
            Ok(_) => panic!("a fork that returned -1 is a failure"),
        }
        assert!(
            !read_side_held(lock),
            "a failed fork releases the read side"
        );
    }

    #[test]
    fn a_fork_from_inside_descriptor_creation_refuses_instead_of_waiting() {
        let lock = fresh_lock();
        let refused = on_a_thread(move || write_held(lock, || fork_holding(lock, || 4242)));
        assert!(
            matches!(refused, Ok(Err(SpawnError::DescriptorLockHeld))),
            "a fork on the thread holding the write side refuses; a timeout means it deadlocked"
        );
    }

    #[test]
    fn nested_descriptor_creation_runs_under_the_lock_already_held() {
        let lock = fresh_lock();
        let nested = on_a_thread(move || {
            write_held(lock, || {
                write_held(lock, || {
                    matches!(lock.try_read(), Err(TryLockError::WouldBlock))
                })
            })
        });
        assert_eq!(
            nested,
            Ok(true),
            "a nested call runs under the write side; a timeout means it deadlocked"
        );
    }

    #[test]
    fn a_fork_after_a_nested_creation_returns_still_refuses() {
        let lock = fresh_lock();
        let refused = on_a_thread(move || {
            write_held(lock, || {
                write_held(lock, || ());
                fork_holding(lock, || 4242)
            })
        });
        assert!(
            matches!(refused, Ok(Err(SpawnError::DescriptorLockHeld))),
            "the outer creation still holds the write side; a timeout means it deadlocked"
        );
    }

    #[test]
    fn a_panic_during_creation_leaves_the_lock_and_the_thread_usable() {
        let lock = fresh_lock();
        let unwound =
            std::panic::catch_unwind(|| write_held(lock, || panic!("a descriptor setup panics")));
        assert!(unwound.is_err(), "the panic reached the caller");
        assert!(
            lock.is_poisoned(),
            "the write side was held when it panicked"
        );
        assert!(
            matches!(fork_holding(lock, || 4242), Ok(Forked::Parent(4242))),
            "this thread is no longer marked as creating descriptors"
        );
        assert_eq!(write_held(lock, || 7), 7, "the write side recovers too");
    }

    #[test]
    fn spawn_from_inside_descriptor_creation_refuses_instead_of_deadlocking() {
        let (sent, received) = mpsc::channel();
        std::thread::spawn(move || {
            let mut spawned = None;
            let created = with_descriptors_held(|_held| {
                spawned = Some(VmmChild::spawn(ExitAtOnce));
                close_on_exec_null()
            });
            let spawned =
                spawned.map(|spawned| spawned.map(|mut child| child.wait_terminal(PATIENCE)));
            let _ = sent.send((spawned, created.is_ok()));
        });
        match received.recv_timeout(ABORT_AFTER) {
            Ok((Some(Err(SpawnError::DescriptorLockHeld)), true)) => {}
            Ok(other) => panic!("spawn inside descriptor creation must refuse, got {other:?}"),
            Err(_) => {
                let _ = std::io::Write::write_all(
                    &mut std::io::stderr(),
                    b"VmmChild::spawn deadlocked on a thread holding the descriptor lock; \
                      that thread holds the lock forever, so the process aborts\n",
                );
                std::process::abort()
            }
        }
    }
}
