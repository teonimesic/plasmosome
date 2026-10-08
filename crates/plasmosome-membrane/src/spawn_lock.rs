use crate::vmm::SpawnError;
use std::cell::Cell;
use std::sync::{PoisonError, RwLock};

static SPAWN_LOCK: RwLock<()> = RwLock::new(());

thread_local! {
    static CREATING_DESCRIPTORS: Cell<bool> = const { Cell::new(false) };
}

pub(crate) enum Forked {
    Child,
    Parent(libc::pid_t),
}

/// Runs `create` while no `VmmChild::spawn` in this process can fork.
///
/// Use it only to create descriptors and mark them close-on-exec, where the
/// platform needs two calls for that. Never hold it across a blocking call
/// such as `accept`: every `VmmChild::spawn` in the process waits until
/// `create` returns. A spawn on this thread from inside `create` forks nothing
/// and returns `SpawnError::DescriptorLockHeld`, and a nested call runs under
/// the lock already held. Only descriptors created inside `create` are
/// covered, and forks that bypass this lock, such as `std::process::Command`,
/// are not held back.
pub(crate) fn with_descriptors_held<T>(create: impl FnOnce() -> T) -> T {
    write_held(&SPAWN_LOCK, create)
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

#[cfg(all(test, not(target_os = "linux")))]
pub(crate) fn while_forking<T>(during: impl FnOnce() -> T) -> T {
    let _forking = SPAWN_LOCK.read().unwrap_or_else(PoisonError::into_inner);
    during()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmm::{Launch, SpawnError, VmmChild, VmmState};
    use std::sync::TryLockError;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    const PATIENCE: Duration = Duration::from_secs(2);
    const HELD: Duration = Duration::from_millis(250);

    struct ExitAtOnce;

    impl Launch for ExitAtOnce {
        fn launch(self) -> ! {
            unsafe { libc::_exit(0) }
        }
    }

    struct ReportsCloseOnExec(libc::c_int);

    impl Launch for ReportsCloseOnExec {
        fn launch(self) -> ! {
            let flags = unsafe { libc::fcntl(self.0, libc::F_GETFD) };
            let code = if flags == -1 {
                4
            } else if flags & libc::FD_CLOEXEC != 0 {
                0
            } else {
                3
            };
            unsafe { libc::_exit(code) }
        }
    }

    fn fresh_lock() -> &'static RwLock<()> {
        Box::leak(Box::new(RwLock::new(())))
    }

    fn read_side_held(lock: &RwLock<()>) -> bool {
        matches!(lock.try_write(), Err(TryLockError::WouldBlock))
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
    fn a_child_never_inherits_a_descriptor_before_it_is_close_on_exec() {
        let (spawner, ends) = with_descriptors_held(|| {
            let mut ends = [0; 2];
            assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0, "a pipe opens");
            let watched = ends[1];
            let spawner = std::thread::spawn(move || VmmChild::spawn(ReportsCloseOnExec(watched)));
            std::thread::sleep(HELD);
            for end in ends {
                assert_eq!(
                    unsafe { libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC) },
                    0,
                    "the pipe end is marked close-on-exec"
                );
            }
            (spawner, ends)
        });
        let mut child = spawner
            .join()
            .expect("the spawning thread finishes")
            .expect("the fork succeeds");
        let state = child.wait_terminal(PATIENCE);
        for end in ends {
            unsafe { libc::close(end) };
        }
        assert_eq!(
            state,
            Ok(VmmState::Exited { code: 0 }),
            "exit 3 means the child inherited the pipe end before it was close-on-exec"
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
            let spawned = with_descriptors_held(|| VmmChild::spawn(ExitAtOnce));
            let _ = sent.send(spawned.map(|mut child| child.wait_terminal(PATIENCE)));
        });
        match received.recv_timeout(PATIENCE) {
            Ok(Err(SpawnError::DescriptorLockHeld)) => {}
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
