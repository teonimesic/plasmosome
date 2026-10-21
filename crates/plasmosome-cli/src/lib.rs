//! The `plasmosome` command line's client logic. A caller reads a daemon's reply with
//! [`json::scan`], wraps it in an [`Envelope`] and writes it with [`emit`].

#![doc(test(attr(forbid(unsafe_code))))]

mod cli;
mod codes;
mod commands;
mod envelope;
mod help;
pub mod json;

pub use cli::{Invocation, Reading, handle, main_with, read};
pub use codes::{ClientCode, Exit};
pub use commands::{COMMANDS, Command};
pub use envelope::{
    ClientError, EmitError, Envelope, ErrorBody, Form, Instance, KernelError, Stream, emit,
};
