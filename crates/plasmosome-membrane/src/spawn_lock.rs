use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

static SPAWN_LOCK: RwLock<()> = RwLock::new(());

pub(crate) fn hold_for_fork() -> RwLockReadGuard<'static, ()> {
    todo!()
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "nothing outside tests creates inheritable descriptors yet; the helper pipes and listeners will"
    )
)]
pub(crate) fn hold_for_descriptors() -> RwLockWriteGuard<'static, ()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmm::{Launch, VmmChild, VmmState};
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

    #[derive(Debug, PartialEq, Eq)]
    enum Event {
        SpawnStarted,
        Released,
        SpawnReturned,
    }

    fn spawn_and_reap() {
        let mut child = VmmChild::spawn(ExitAtOnce).expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 })
        );
    }

    #[test]
    fn spawn_waits_while_another_thread_holds_the_descriptor_lock() {
        let (events, observed) = mpsc::channel();
        let held = hold_for_descriptors();
        let spawner = {
            let events = events.clone();
            std::thread::spawn(move || {
                events
                    .send(Event::SpawnStarted)
                    .expect("the test thread listens");
                let child = VmmChild::spawn(ExitAtOnce);
                events
                    .send(Event::SpawnReturned)
                    .expect("the test thread listens");
                child
            })
        };
        assert_eq!(observed.recv_timeout(PATIENCE), Ok(Event::SpawnStarted));
        assert_eq!(
            observed.recv_timeout(HELD),
            Err(RecvTimeoutError::Timeout),
            "spawn forked while another thread was creating descriptors"
        );
        events
            .send(Event::Released)
            .expect("the test thread listens");
        drop(held);
        assert_eq!(observed.recv_timeout(PATIENCE), Ok(Event::Released));
        assert_eq!(observed.recv_timeout(PATIENCE), Ok(Event::SpawnReturned));
        let mut child = spawner
            .join()
            .expect("the spawning thread finishes")
            .expect("the fork succeeds");
        assert_eq!(
            child.wait_terminal(PATIENCE),
            Ok(VmmState::Exited { code: 0 })
        );
    }

    #[test]
    fn a_panic_while_creating_descriptors_does_not_stop_later_spawns() {
        let poisoner = std::thread::spawn(|| {
            let _held = hold_for_descriptors();
            panic!("a descriptor setup that panics while holding the lock");
        });
        assert!(poisoner.join().is_err(), "the poisoning thread panicked");
        assert!(SPAWN_LOCK.is_poisoned(), "the lock is poisoned");
        drop(hold_for_descriptors());
        spawn_and_reap();
    }
}
