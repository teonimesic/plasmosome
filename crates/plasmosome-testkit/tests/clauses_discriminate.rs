use std::collections::BTreeMap;

use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantKind, Handle, LedgerEntry,
    OsObject, OsState, PluginId, UniverseClass, UniverseOp, UniverseRemoval,
};
use plasmosome_testkit::conformance;

/// The one way a `DefectiveBackend` departs from the `EnforcementBackend`
/// contract. `None` departs in no way at all and must pass every clause, so a
/// clause that panics against any other variant panicked because of that
/// variant and not because the backend around it is sloppy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Defect {
    None,
    /// Revoke withdraws the right capability and hands back an entry that
    /// describes a different one.
    RevokeReturnsAStranger,
    /// Revoke of a handle no grant ever issued reports success.
    UnknownHandleReportsSuccess,
    /// Revoke forgets the handle and leaves the object it materialized.
    RevokeKeepsTheObject,
    /// Revoke empties the universe instead of removing its own object.
    RevokeTidiesTheWholeUniverse,
    /// Every grant materializes a second object nobody asked for.
    GrantMaterializesAShadow,
    /// Every grant is issued the same handle number.
    OneHandleForEveryLiveGrant,
    /// `apply_removal` reports success and removes nothing.
    RemovalIsANoOp,
    /// A handle stays revocable after it was revoked.
    ARevokedHandleRevokesAgain,
}

/// A backend that keeps a ledger by handle and a universe of the objects its
/// grants materialized, carrying exactly one defect. Construct it with
/// `carrying`; hand `conformance` a closure that constructs a fresh one.
struct DefectiveBackend {
    defect: Defect,
    state: OsState,
    ledger: BTreeMap<u64, LedgerEntry>,
    spent: BTreeMap<u64, LedgerEntry>,
    next_handle: u64,
}

impl DefectiveBackend {
    fn carrying(defect: Defect) -> DefectiveBackend {
        DefectiveBackend {
            defect,
            state: OsState::new(),
            ledger: BTreeMap::new(),
            spent: BTreeMap::new(),
            next_handle: 0,
        }
    }

    fn withdraw(&mut self, entry: &LedgerEntry) -> Result<(), BackendError> {
        match self.defect {
            Defect::RevokeKeepsTheObject => Ok(()),
            Defect::RevokeTidiesTheWholeUniverse => {
                self.state = OsState::new();
                Ok(())
            }
            _ => self.apply_removal(removal_of(&entry.capability)),
        }
    }
}

impl EnforcementBackend for DefectiveBackend {
    fn grant(&mut self, grant: Grant) -> LedgerEntry {
        self.next_handle += 1;
        let entry = LedgerEntry {
            handle: Handle(self.next_handle),
            plugin: grant.plugin,
            capability: grant.capability,
            kind: grant.kind,
        };
        self.state.insert(object_of(&entry));
        if self.defect == Defect::GrantMaterializesAShadow {
            self.state.insert(shadow_of(&entry));
        }
        self.ledger.insert(entry.handle.raw(), entry.clone());
        if self.defect == Defect::OneHandleForEveryLiveGrant {
            return LedgerEntry {
                handle: Handle(1),
                ..entry
            };
        }
        entry
    }

    fn revoke(&mut self, handle: Handle, _drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        let Some(entry) = self.ledger.remove(&handle.raw()) else {
            return match self.defect {
                Defect::UnknownHandleReportsSuccess => Ok(a_stranger(handle)),
                Defect::ARevokedHandleRevokesAgain => self
                    .spent
                    .get(&handle.raw())
                    .cloned()
                    .ok_or(BackendError::UnknownHandle { handle }),
                _ => Err(BackendError::UnknownHandle { handle }),
            };
        };
        self.withdraw(&entry)?;
        self.spent.insert(handle.raw(), entry.clone());
        if self.defect == Defect::RevokeReturnsAStranger {
            return Ok(a_stranger(handle));
        }
        Ok(entry)
    }

    fn snapshot_os_state(&self) -> OsState {
        self.state.clone()
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        self.state.insert(op.object());
        Ok(())
    }

    fn apply_removal(&mut self, removal: UniverseRemoval) -> Result<(), BackendError> {
        if self.defect == Defect::RemovalIsANoOp {
            return Ok(());
        }
        let (class, key) = (removal.class(), removal.key());
        self.state
            .remove(class, &key)
            .map(|_| ())
            .ok_or(BackendError::UnknownObject {
                class: class.as_str(),
                key,
            })
    }

    fn plant(&mut self, object: OsObject) {
        self.state.insert(object);
    }
}

fn object_of(entry: &LedgerEntry) -> OsObject {
    let owner = entry.plugin.clone();
    let op = match &entry.capability {
        Capability::SessionFile { path } => UniverseOp::WriteSessionFile {
            path: path.clone(),
            owner,
        },
        Capability::UdsSocket { path } => UniverseOp::BindUds {
            path: path.clone(),
            owner,
        },
        Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
            host: host.clone(),
            route: route.clone(),
            owner,
        },
        Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
            pid: *pid,
            name: name.clone(),
            owner,
        },
        Capability::Mount { source, target } => UniverseOp::AddMount {
            source: source.clone(),
            target: target.clone(),
            owner,
        },
    };
    op.object()
}

fn removal_of(capability: &Capability) -> UniverseRemoval {
    match capability {
        Capability::SessionFile { path } => {
            UniverseRemoval::RemoveSessionFile { path: path.clone() }
        }
        Capability::UdsSocket { path } => UniverseRemoval::UnbindUds { path: path.clone() },
        Capability::ProxyMap { host, .. } => UniverseRemoval::RemoveProxyMap { host: host.clone() },
        Capability::Broker { pid, .. } => UniverseRemoval::KillBroker { pid: *pid },
        Capability::Mount { target, .. } => UniverseRemoval::RemoveMount {
            target: target.clone(),
        },
    }
}

fn shadow_of(entry: &LedgerEntry) -> OsObject {
    OsObject {
        class: UniverseClass::SessionFile,
        key: format!("session/shadow/{}", entry.handle.raw()),
        owner: entry.plugin.clone(),
    }
}

fn a_stranger(handle: Handle) -> LedgerEntry {
    LedgerEntry {
        handle,
        plugin: PluginId::from("stranger"),
        capability: Capability::SessionFile {
            path: "stranger.md".to_string(),
        },
        kind: GrantKind::Hot,
    }
}

fn carrying(defect: Defect) -> impl Fn() -> DefectiveBackend {
    move || DefectiveBackend::carrying(defect)
}

#[test]
fn a_backend_with_no_defect_passes_every_clause() {
    conformance::grant_is_replayable(carrying(Defect::None));
    conformance::revoke_unknown_handle_is_error(carrying(Defect::None));
    conformance::drained_revoke_removes_object(carrying(Defect::None));
    conformance::planted_residue_survives_unrelated_revoke(carrying(Defect::None));
    conformance::snapshot_never_invents_objects(carrying(Defect::None));
    conformance::live_grants_hold_distinct_handles(carrying(Defect::None));
    conformance::apply_and_removal_reach_the_universe(carrying(Defect::None));
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::None));
}

#[test]
#[should_panic(expected = "revoking a handle must return the entry the grant issued")]
fn grant_is_replayable_catches_a_revoke_that_returns_a_stranger() {
    conformance::grant_is_replayable(carrying(Defect::RevokeReturnsAStranger));
}

#[test]
#[should_panic(expected = "revoking the never-granted handle")]
fn revoke_unknown_handle_is_error_catches_a_success_report() {
    conformance::revoke_unknown_handle_is_error(carrying(Defect::UnknownHandleReportsSuccess));
}

#[test]
#[should_panic(expected = "a drained revoke left")]
fn drained_revoke_removes_object_catches_a_revoke_that_keeps_the_object() {
    conformance::drained_revoke_removes_object(carrying(Defect::RevokeKeepsTheObject));
}

#[test]
#[should_panic(expected = "removed the unrelated")]
fn planted_residue_survives_unrelated_revoke_catches_a_revoke_that_tidies_the_universe() {
    conformance::planted_residue_survives_unrelated_revoke(carrying(
        Defect::RevokeTidiesTheWholeUniverse,
    ));
}

#[test]
#[should_panic(expected = "which was never granted or planted")]
fn snapshot_never_invents_objects_catches_a_grant_that_materializes_a_shadow() {
    conformance::snapshot_never_invents_objects(carrying(Defect::GrantMaterializesAShadow));
}

#[test]
#[should_panic(expected = "is already holding")]
fn live_grants_hold_distinct_handles_catches_one_handle_issued_twice() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::OneHandleForEveryLiveGrant));
}

#[test]
#[should_panic(expected = "an applied removal left")]
fn apply_and_removal_reach_the_universe_catches_a_removal_that_removes_nothing() {
    conformance::apply_and_removal_reach_the_universe(carrying(Defect::RemovalIsANoOp));
}

#[test]
#[should_panic(expected = "revoking the already-revoked handle")]
fn revoke_of_a_revoked_handle_is_error_catches_a_handle_that_revokes_twice() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::ARevokedHandleRevokesAgain));
}
