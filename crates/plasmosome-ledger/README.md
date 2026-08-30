# plasmosome-ledger

Typed reversibility. Every effect a plasmid causes is recorded with its inverse, so detaching it
can be *verified* rather than trusted.

Attaching a capability creates real things: sockets, mounts, routes, files, processes. Detaching
must remove exactly those and nothing else. This crate records each effect as it happens, replays
the inverses in reverse order on detach, and classifies what can be undone.

Not everything can. An effect that already left the system — a pushed commit, a sent email —
has no inverse, and the ledger says so rather than pretending. Those are typed differently and
require an explicit force to discard, which the record then notes.

## What's inside

| Concept | Meaning |
| --- | --- |
| `Exact` | The kernel can run a precise inverse (unmount, close, unlink) |
| `Compensating` | No exact inverse, but a registered action restores an acceptable state |
| `Delayed` | Held at the boundary until commit; discarding it is enough |
| `External` | Already visible outside; no inverse exists — revocation is policy, not cleanup |

A detach over the first three is a safe operation. Over `External`, it is an assertion the
operator makes and the record keeps.

Tests: `cargo test -p plasmosome-ledger`
