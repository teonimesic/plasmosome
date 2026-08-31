---
id: 002
title: A test system and CI that covers every layer, performance included
status: approved
date: 2026-08-30
originator: Stefano
---

This project needs a great test system, not a passing one. Testing should reach every layer:
unit tests inside a crate, integration tests across crates, end-to-end tests of a whole cell, and
performance tests. Performance matters here more than in most projects — a kernel that takes
seconds to attach a capability is a different product from one that takes milliseconds — so it
has to be measured continuously, not checked once and assumed.

The shape I want is hexagonal: the core logic testable without the outside world, and the outside
world reachable through adapters that a test can replace. That is what makes fast unit tests and
honest end-to-end tests possible at the same time, instead of trading one for the other.

CI should cover the same ground, and further: that every crate still compiles on its own, that
the crates work together, and eventually that they can be published. This is too large for one
piece of work. It should become several specs, and some of them will have to wait — anything that
needs a real Linux host to run brings a lot of infrastructure with it, and should not block the
parts that can be built on a Mac today.

One benchmark I want specifically: how fast does work run **inside a cell** compared to the same
work on the **host** and in a **plain Docker image**? Put Rust in all three, run something that
parallelises — a test suite is the obvious candidate — and measure both how long it took and how
much of the host's resources it managed to use when no limit was set. The result I am hoping for
is that a cell with unlimited resource access performs close to the host. If it does not, I want
to know the gap and where it goes.

## Outcome

(filled in later)
