//! The `plasmosome` command line's client logic. The binary's `main` is [`main_with`]. A caller
//! reads a daemon's reply with [`reply::read_reply`], judges it with [`reply::judge_reply`], wraps
//! it in an [`Envelope`] and writes it with [`emit`].

#![doc(test(attr(forbid(unsafe_code))))]

mod args;
mod cli;
mod clock;
mod codes;
mod commands;
mod env;
mod envelope;
mod exchange;
mod help;
pub mod json;
mod refusal;
pub mod reply;
mod select;
mod verb;

pub use args::{
    ArgSource, CommandArgs, Flag, FlagKind, InstanceParam, Param, VerbCall, VerbDeadline,
};
pub use cli::{Context, Invocation, Reading, handle, main_with, read};
pub use clock::{Clock, Deadline, SystemClock};
pub use codes::{ClientCode, Exit};
pub use commands::{Build, COMMANDS, Command};
pub use env::Env;
pub use envelope::{
    ClientError, EmitError, Envelope, ErrorBody, Form, Instance, KernelError, Stream, emit,
};
pub use exchange::{Unreplied, exchange};
pub use refusal::{PathOrigin, map_refusal};
