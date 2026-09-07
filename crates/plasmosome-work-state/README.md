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

Explicit online observation is available after bootstrapping the current wrapper:

```text
./tools/work-state sync [--json]
```

Sync uses the project binding compiled from `tools/work-state-project.toml` into the installed
wrapper, not a mutable checkout configuration. It observes `refs/dolt/data` at
`https://github.com/teonimesic/plasmosome.git`, initializes a fresh private Beads candidate using
the canonical `git+https` URL, then observes the ref again. Only a stable ref and exact parity
with the active Markdown shadow can activate a complete generation. This includes source,
authority mode, document content and links, lifecycle, owner, dependencies and operational
digests; sync does not reimport Markdown or replace those facts with different remote values.

An absent ref returns `remote_uninitialized` without cloning or creating the remote ref. Transport
loss, a moving ref, pending operations or incompatible remote data refuse without exposing the
candidate. A justified observation may atomically update freshness metadata; refusal output
reports whether this happened as `state_changed`. Historical successful-sync time and pending
operation ids are preserved. Success says `synchronized as of`, never that an offline view is
current. Bootstrap and sync contend on one nonblocking activation lock; reads remain lock-free.

The real pinned local-store contract can be exercised without hosted fixtures or credentials:

```text
./tools/work-state contract-test online-sync --source-ref REF --archive PATH --bd PATH
```

Its remote observations and clone outcomes are recorded at the command seam; local snapshots,
parity and generation activation use real pinned Beads. `contract-test all` includes this case.

This package never publishes Beads state, creates the remote data ref, manages leases, mutates
lifecycle or claims, reconciles GitHub, backs up state, or performs a cutover. Spec 014's complete
`offline-reads` acceptance still requires `heartbeat observe` and an operating-system no-socket
harness, which are deliberately outside this task.
