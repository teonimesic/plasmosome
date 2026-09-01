# plasmosome-testkit — working notes

## What this crate is

Shared test support: builders for the noisy kernel types, the backend conformance suite every
`EnforcementBackend` implementation must pass, and the cross-crate scenarios. It is where the
rules that span crates live; each crate's own `AGENTS.md` stays authoritative for that crate.

Nothing here ships. The crate is `publish = false`, and the guard `testkit_is_dev_only` in
`plasmosome-guards` fails the build if another workspace crate names it outside
`[dev-dependencies]`.

## The four layers

| Layer | Lives in | Command | Exists today |
| --- | --- | --- | --- |
| Unit | each crate: `#[cfg(test)]` modules and its `tests/` | `cargo test -p <crate>` | yes |
| Integration | `crates/plasmosome-testkit/tests/` | `cargo test -p plasmosome-testkit` | yes |
| End-to-end | `crates/plasmosome-testkit/tests/e2e_*.rs`, `#[ignore]` until a cell boots | `cargo test -p plasmosome-testkit -- --ignored` | no — defined only |
| Performance | `benches/` per crate and in testkit | `cargo bench` | no — specs 005 and 006 |

A unit test exercises one crate through its own API. An integration test exercises two or more
crates together through public APIs only, with the outside world replaced at the seams below. An
end-to-end test drives the real binaries as black boxes over their control sockets, with nothing
replaced.

## The seam inventory

Every place the kernel touches the world outside the process, and the seam that guards it:

| Outside world | Seam | Fake |
| --- | --- | --- |
| OS enforcement (grants, revocation, residue) | `EnforcementBackend` (`plasmosome-backend`) | `FakeBackend`, `CompositeBackend` over fake leaves |
| Process spawning (the VMM child) | `Launch` (`plasmosome-membrane::vmm`) | test launchers already in the crate's tests |
| Filesystem (session log, state) | paths injected as arguments | `tempfile::TempDir` |
| Sockets (readiness probes) | socket path injected as argument | a test-owned socket in a `TempDir` |

**The seam rule.** Both forms in that table are seams. Which one a contact gets is decided by
what varies between the real thing and the test. When the *behavior* varies — enforcement that
really grants and revokes versus one that records what it was asked to do — the seam is a trait
with a fake that models the contract. When only the *resource* varies — the same filesystem calls
against a different directory, the same socket calls against a different path — the seam is the
path, injected as an argument. A trait over a file path would be ceremony: it would add an
indirection without letting a test observe anything a `TempDir` does not already give it.

For new code: the first time a change touches something not in that table — a network call, a
clock read that affects behavior, a new daemon — it adds the seam in the crate that owns the
contact, in whichever of the two forms fits, and a row here. It never calls the OS inline and
mocks it in the test.

## Hard rules

- **No mocking framework, ever.** Expectation-style mocks couple tests to call sequences; a fake
  that models the contract couples tests to behavior. Hand-built fakes only.
- **A fake that disagrees with a real adapter is the thing that is wrong.** A test passing
  against the fake and failing against the real backend means the fake gets fixed, never the
  test.
- **A conformance function's name is a contract.** Backends cite these names; renaming one is a
  change to every backend that does.
- **Conformance functions take a factory, not an instance** — `make: impl Fn() -> B` — so each
  one owns its backend and a clause cannot inherit another clause's state.
- **There is no clock seam, on purpose.** Durations are passed in as arguments and nothing reads
  wall time to decide anything. If a flaky time-dependent test ever appears, that is the moment
  to add a clock trait — not before.
- **Builders carry only what a test in this repository uses.** A helper written for a caller that
  does not exist is a guess about the next test.
- **A clause that withdraws a set walks both orders, and each order needs its own witness.**
  `plasmosome-ledger`'s `replay` withdraws effects in reverse push order — revokes and
  `apply_removal`s alike — so a suite that only ever walks grant order certifies a backend that
  refuses the first handle a detach reaches for. `live_grants_hold_distinct_handles` runs its
  revoke phase twice, reverse first, and every message in that phase names which pass it came
  from. Each pass is pinned by a backend that fails on that pass and no other:
  `RevokesOnlyInGrantOrder` and `RevokesOnlyInReversePushOrder`. Without both, one pass can be
  deleted and nothing goes red — which is how the grant-order pass stood until an independent
  reviewer deleted it and watched the suite stay green. A new clause holding a set does the same.
  No clause holds a set of *applied* objects yet, so removal order is unwitnessed; the first one
  that does owes both orders too.
- **The fake does not model path containment.** `OsState` is a flat set of objects in five
  classes. A mount at `/workspace` and a socket at `/workspace/run/egressd.uds` are unrelated
  entries; nothing in the fake knows one sits inside the other. So in
  `tests/attach_detach_residue.rs`, LIFO replay order is proved by the explicit ordered-string
  assertion on `report.replayed` and by nothing else — replaying FIFO leaves the same empty
  state and fails only that one assertion. Do not delete or relax it into an order-blind
  comparison; it is the whole proof.

## Testing

`cargo test -p plasmosome-testkit`. That runs the conformance suite against `FakeBackend` and
`CompositeBackend`, plus the cross-crate scenarios. A new backend proves itself by calling the
same `conformance::` functions with its own constructor, in its own `tests/` file.
