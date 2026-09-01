# plasmosome-work-state

This crate verifies the pinned Beads release before it opens disposable contract-test state. It
does not migrate or operate Plasmosome work state.

Keep process execution behind `CommandRunner`. Callers supply artifacts; transport outcomes are
scripted at that seam and never require a hosted fixture or credential.
