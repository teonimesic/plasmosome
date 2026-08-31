---
id: 001
title: The membrane supervises its cell's brokers
status: in_review
priority: 1
specs: [001]
intents: []
refs:
  [
    crates/plasmosome-membrane/AGENTS.md,
    crates/plasmosome-membrane/src/lib.rs,
    crates/plasmosome-membrane/src/vmm.rs,
    crates/plasmosome-membrane/src/readiness.rs,
    docs/specs/001-control-protocol.md,
    AGENTS.md,
  ]
done_when: >-
  membraned spawns each of a cell's brokers, answers membrane.status ready only
  once every broker answers its own control socket, reports a broker that stops
  answering rather than staying ready, and leaves no broker process behind when
  the supervisor is dropped — the last proven by a raw waitpid returning ECHILD.
pr: https://github.com/teonimesic/plasmosome/pull/13
evidence:
---

## Why

`plasmosome-membrane` owns the VM child today and nothing else. The brokers a cell needs have no
owner: nothing spawns them, nothing notices when one dies, and nothing reaps them. A broker that
keeps running after its cell is gone is exactly the failure this project exists to prevent.

Readiness is affected too. `membrane.status` is the one answer the controller trusts about a
cell, and today it can report ready while the brokers behind it are not serving.

`docs/specs/001-control-protocol.md` §4 already reserves broker spawn and supervision for the
next step of P1, so the shape of the work is bounded but the behavior is not yet specified.

## Plan

**Deliverable:** `crates/plasmosome-membrane/src/brokers.rs`, holding a supervisor that spawns a
cell's brokers, answers a readiness question about the set, and reaps every one on drop. Out of
scope: the controller daemon, the control protocol wire format, `main.rs` staying `fn main() {}`,
anything in `plasmosome-core`, and any real broker binary.

**Build on the two seams that exist. Do not invent a third.** `vmm::Launch` is already the fork
seam and `VmmChild` already kills and reaps on drop — one broker is one `VmmChild`, so the
no-orphan guarantee is inherited rather than rewritten. `readiness::probe(&Path, Duration)` is
already the answered-query probe, and its socket path is already injected.

**The one new seam.** The set must be testable without binding real sockets, so aggregation takes
the probe as a dependency rather than calling it directly:

```rust
pub trait Probe {
    fn probe(&self, socket: &Path, deadline: Duration) -> Readiness;
}
```

with a unit struct forwarding to `readiness::probe` as the production implementation. Two adapters
exist the moment the tests land, so this is a real seam and not a hypothetical one.

**The contract.**

| Item | Behavior |
| --- | --- |
| `BrokerSpec { name, control_socket }` | what to spawn and where to ask it |
| `BrokerSet::spawn(specs, launcher, prober)` | forks one child per spec; a fork failure kills and reaps everything already spawned before returning the error |
| `BrokerSet::status(deadline)` | `Ready` only when every broker answers ready; otherwise names **which** broker and why, carrying the `NotReady` it got |
| `Drop` | every child killed and reaped, inherited from `VmmChild` |

`status` must re-probe on every call. A broker that answered once and stopped answering reports
not ready on the next call — cached readiness is the failure this crate's own doc calls out.

**Test table.** Every test names the broker or the condition in its failure message.

| Test | Proves |
| --- | --- |
| `a_set_is_ready_only_when_every_broker_answers` | one not-ready broker makes the set not ready, and the report names it |
| `a_broker_that_stops_answering_flips_the_set_to_not_ready` | readiness is re-probed, not cached — the fake prober answers ready then unreachable |
| `a_set_reports_which_broker_is_not_ready` | the failure carries the broker's name and its `NotReady` reason |
| `dropping_a_set_reaps_every_broker` | raw `waitpid` returns `ECHILD` for every pid the set held |
| `a_fork_failure_reaps_what_was_already_spawned` | no orphan when spawn fails part way — proven by `waitpid`, not by asserting a code path ran |

**Prove the reaping test can fail.** `dropping_a_set_reaps_every_broker` must be run against a
`Drop` that skips reaping, observed failing, and that output recorded. Three tests in this repo
have passed against the bug they named; do not add a fourth.

**Definition of done:** every line of `done_when`, the five tests above, and the gate in root
`AGENTS.md`. `main.rs` is untouched.

STOP when done. Do not start the controller.

## Notes
