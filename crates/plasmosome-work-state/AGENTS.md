# plasmosome-work-state

This crate verifies the pinned Beads release before it opens disposable contract-test state. It can
build temporary Markdown-shadow migrations while Markdown remains authoritative; it does not
operate production Plasmosome work state or install Beads.

Keep process execution behind `CommandRunner`. Callers supply artifacts; transport outcomes are
scripted at that seam and never require a hosted fixture or credential.
