# plasmosome-work-state

`plasmosome-work-state` provides a clone-local, verified Beads 1.1.2 shadow for reading the
repository's Markdown work records. Markdown at the selected commit remains authoritative.

One explicit bootstrap accepts caller-supplied pinned artifacts and installs an immutable shadow
generation under the clone's Git common directory:

```text
./tools/work-state bootstrap --source-ref REF --archive PATH --bd PATH
```

After that, every linked worktree in the clone can use the artifact-free local projections:

```text
./tools/work-state list [--json]
./tools/work-state show kind:NNN [--json]
./tools/work-state ready [--json]
./tools/work-state blocked [--json]
```

The launcher executes the installed wrapper rather than Cargo. Each query verifies the installed
runtime and reads a disposable copy, so Beads' read-side lock and journal activity cannot alter
the shared generation. Responses include the stored freshness envelope and are local projections;
they never authorize starting or claiming work.

This package does not synchronize or publish Beads state, manage leases, reconcile GitHub, or
perform a cutover. It completes the local-read and six-state freshness contracts only. Spec 014's
complete `offline-reads` acceptance still requires `heartbeat observe` and an operating-system
no-socket harness, which are deliberately outside this crate's scope.
