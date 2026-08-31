---
id: 002
title: A test system and CI that covers every layer, performance included
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

## Outcome

(filled in later)
