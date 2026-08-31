# plasmid-sdk

The contract for building a **plasmid** — a capability module that a cell can attach and detach
at runtime.

A plasmid is not a kernel patch. It is an independent crate that declares what it needs (hosts,
paths, credentials, tools), implements its logic against a stable interface, and ships with its
own fixtures. The kernel discovers it, grants exactly what its manifest declares, and can revoke
it mid-run without the plasmid's cooperation.

This crate is the stability boundary: build against it, and your plasmid works with any kernel
that supports the same major version. The kernel never needs to know your plasmid exists.

Only the contract ships here. A package has one dependency table, shared by its library and
any binary beside it, so this crate's table is exactly what your plasmid inherits — which is why
the `plasmid` command line is a [crate of its own](../plasmid) rather than a binary target here.

## Status

Reserved and deliberately unimplemented. The interface is being designed rather than accreted —
a contract that is frozen too early is worse than one that arrives late. The `plasmid new`
scaffold refuses to run rather than generating against a shape that will change.

## Intended shape

A plasmid crate contains its component, its manifest (capabilities, scopes, credential delivery),
its mock fixtures, and its tests. Nothing else.

Tests: `cargo test -p plasmid-sdk`
