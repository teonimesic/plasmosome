# plasmosome-work-state

This crate verifies the pinned Beads release before it opens disposable contract-test state. It
does not migrate or operate Plasmosome work state.

Keep process execution behind `CommandRunner`. Callers must supply artifacts and, for remote
probes, an explicitly disposable public GitHub fixture.
