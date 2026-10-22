//! The repository's own CI and maintainer tools. Each public module is one tool, and the binary
//! of the same name calls its `run`.

#![doc(test(attr(forbid(unsafe_code))))]
#![deny(clippy::exit)]

pub mod bench_medians;
