//! RESERVED CRATE — do not build on this yet.
//!
//! `plasmid-sdk` is the keystone crate P1 will publish as the stability
//! boundary: the frozen WIT worlds, the host bindings, the test harness, and
//! the `plasmid new` scaffold. Per 91-D3 the P1 goal is the kernel and
//! plasmid architecture, and per the P1 freeze plan the WIT world is NOT
//! designed here yet — `wit/plasmid.wit` is a placeholder world holding the
//! package and world names only so nothing else claims them. The SDK's actual
//! surface (host imports, tool exports, fixture harness) is a deferred,
//! decision-required design; it is intentionally absent from this commit.
//!
//! Plasmids become their own crates that build against this SDK once frozen
//! (plasmid-github-pr, plasmid-workspace, plasmid-model-provider,
//! plasmid-mock-github per the RENAME-NOTES decomposition). The kernel never
//! knows which exist.
