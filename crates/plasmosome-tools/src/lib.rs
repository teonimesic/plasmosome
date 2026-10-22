//! The repository's own CI and maintainer tools. Each tool module is one tool, and the binary of
//! the same name calls its `run`; `bounded` is shared by the tools.

#![doc(test(attr(forbid(unsafe_code))))]
#![deny(clippy::exit)]

pub mod bench_medians;
pub mod bounded;
pub mod quickstart;
pub mod record_facts;
