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

