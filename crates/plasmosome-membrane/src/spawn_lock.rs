use crate::vmm::SpawnError;
use std::cell::Cell;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
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
/// `with_descriptors_held` makes one, so a function that takes it can run only
/// inside descriptor creation.
pub(crate) struct DescriptorsHeld(());

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
pub(crate) fn with_descriptors_held<T: AsFd + Into<OwnedFd>>(
    create: impl FnOnce(&DescriptorsHeld) -> std::io::Result<T>,
) -> std::io::Result<T> {
    write_held(&SPAWN_LOCK, || {
        create(&DescriptorsHeld(())).and_then(close_on_exec_or_closed)
    })
}

fn close_on_exec_or_closed<T: AsFd>(created: T) -> std::io::Result<T> {
    let flags = unsafe { libc::fcntl(created.as_fd().as_raw_fd(), libc::F_GETFD) };
    if flags != -1 && flags & libc::FD_CLOEXEC != 0 {
        Ok(created)
    } else {
        Err(std::io::Error::other(
            "a descriptor created under the spawn lock was not close-on-exec, so it was closed",
        ))
    }
}

/// Forks while holding the read side of the spawn lock.
///
/// The fork waits while another thread is inside `with_descriptors_held`, and
/// the parent releases the lock once `fork` returns. The child never touches
/// its copy of the lock, so it stays async-signal-safe. From inside
/// `with_descriptors_held` on this thread it forks nothing and returns
/// `SpawnError::DescriptorLockHeld`. A failed fork returns
/// `SpawnError::ForkFailed` with its errno.
pub(crate) fn fork() -> Result<Forked, SpawnError> {
    fork_holding(&SPAWN_LOCK, || unsafe { libc::fork() })
}

fn write_held<T>(lock: &RwLock<()>, create: impl FnOnce() -> T) -> T {
    if CREATING_DESCRIPTORS.get() {
        return create();
    }
    let _held = lock.write().unwrap_or_else(PoisonError::into_inner);
    let _creating = Creating::mark();
    create()
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
        fn launch(self) -> ! {
            unsafe { libc::_exit(0) }
        }
    }

    struct ReportsInheritedReadSide;

    impl Launch for ReportsInheritedReadSide {
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

    fn inheritable_null() -> OwnedFd {
        let raw = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(raw >= 0, "/dev/null opens");
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

    struct ForkDuringCreation {
        reached_the_lock_while_held: bool,
        forked_while_held: bool,
        child_status: libc::c_int,
    }

    fn fork_during_creation(lock: &'static RwLock<()>) -> ForkDuringCreation {
        let released = Arc::new(AtomicBool::new(false));
        let (starting, started) = mpsc::channel();
        let (forker, watched) = write_held(lock, || {
            let watched = inheritable_null();
            let fd = watched.as_raw_fd();
            let seen = Arc::clone(&released);
            let forker = std::thread::spawn(move || {
                let _ = starting.send(());
                let reached_the_lock_while_held = !seen.load(Ordering::SeqCst);
                let mut forked_while_held = false;
                let forked = fork_holding(lock, || {
                    forked_while_held = !seen.load(Ordering::SeqCst);
                    unsafe { libc::fork() }
                });
                match forked {
                    Ok(Forked::Child) => {
                        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
                        let code = if flags == -1 {
                            4
                        } else if flags & libc::FD_CLOEXEC != 0 {
                            0
                        } else {
                            3
                        };
                        unsafe { libc::_exit(code) }
                    }
                    Ok(Forked::Parent(pid)) => {
                        Ok((reached_the_lock_while_held, forked_while_held, pid))
                    }
                    Err(error) => Err(error),
                }
            });
            started
                .recv_timeout(PATIENCE)
                .expect("the forking thread starts");
            std::thread::sleep(HOLD);
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0,
                "the descriptor is marked close-on-exec"
            );
            released.store(true, Ordering::SeqCst);
            (forker, watched)
        });
        let (reached_the_lock_while_held, forked_while_held, pid) = forker
            .join()
            .expect("the forking thread finishes")
            .expect("the fork succeeds");
        let mut child_status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut child_status, 0) }, pid);
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
    fn a_failed_fork_keeps_its_errno_and_releases_the_lock() {
        let lock = fresh_lock();
        let forked = fork_holding(lock, || {
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
