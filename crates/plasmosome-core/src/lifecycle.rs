use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Inactive,
    Loading,
    Active,
    Draining,
    Unloading,
    Failed,
}

impl PluginState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PluginState::Inactive => "inactive",
            PluginState::Loading => "loading",
            PluginState::Active => "active",
            PluginState::Draining => "draining",
            PluginState::Unloading => "unloading",
            PluginState::Failed => "failed",
        }
    }

    pub fn accepts_invocations(&self) -> bool {
        matches!(self, PluginState::Active)
    }

    pub fn transition(&self, next: PluginState) -> Result<PluginState, StateError> {
        let allowed = matches!(
            (self, next),
            (PluginState::Inactive, PluginState::Loading)
                | (PluginState::Loading, PluginState::Active)
                | (PluginState::Loading, PluginState::Failed)
                | (PluginState::Active, PluginState::Draining)
                | (PluginState::Draining, PluginState::Unloading)
                | (PluginState::Draining, PluginState::Failed)
                | (PluginState::Unloading, PluginState::Inactive)
                | (PluginState::Failed, PluginState::Inactive)
        );
        if allowed {
            Ok(next)
        } else {
            Err(StateError {
                from: *self,
                to: next,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateError {
    pub from: PluginState,
    pub to: PluginState,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "illegal lifecycle transition {} -> {}",
            self.from.as_str(),
            self.to.as_str()
        )
    }
}

impl std::error::Error for StateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_lifecycle_walks_forward() {
        let mut state = PluginState::Inactive;
        for next in [
            PluginState::Loading,
            PluginState::Active,
            PluginState::Draining,
            PluginState::Unloading,
            PluginState::Inactive,
        ] {
            state = state.transition(next).expect("forward step must be legal");
        }
        assert_eq!(state, PluginState::Inactive);
    }

    #[test]
    fn loading_can_fail() {
        assert_eq!(
            PluginState::Loading.transition(PluginState::Failed),
            Ok(PluginState::Failed)
        );
        assert_eq!(
            PluginState::Failed.transition(PluginState::Inactive),
            Ok(PluginState::Inactive)
        );
    }

    #[test]
    fn only_active_plugins_accept_invocations() {
        for state in [
            PluginState::Inactive,
            PluginState::Loading,
            PluginState::Draining,
            PluginState::Unloading,
            PluginState::Failed,
        ] {
            assert!(!state.accepts_invocations());
        }
        assert!(PluginState::Active.accepts_invocations());
    }

    #[test]
    fn skipping_the_drain_window_is_illegal() {
        let err = PluginState::Active
            .transition(PluginState::Unloading)
            .unwrap_err();
        assert_eq!(err.from, PluginState::Active);
        assert_eq!(err.to, PluginState::Unloading);
    }

    #[test]
    fn reactivation_requires_a_full_cycle() {
        assert!(
            PluginState::Draining
                .transition(PluginState::Active)
                .is_err()
        );
        assert!(
            PluginState::Inactive
                .transition(PluginState::Active)
                .is_err()
        );
    }
}
