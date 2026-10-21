//! The `plasmosome` command line's client logic. A caller reads a daemon's reply with
//! [`reply::read_reply`], judges it with [`reply::judge_reply`], wraps it in an [`Envelope`] and
//! writes it with [`emit`].

#![doc(test(attr(forbid(unsafe_code))))]

mod codes;
mod envelope;
pub mod json;
mod refusal;
pub mod reply;

pub use codes::{ClientCode, Exit};
pub use envelope::{
    ClientError, EmitError, Envelope, ErrorBody, Form, Instance, KernelError, Stream, emit,
};
pub use refusal::{PathOrigin, map_refusal};
