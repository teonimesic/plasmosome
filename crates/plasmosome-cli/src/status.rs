use std::io::Write;
use std::time::Instant;

use crate::cli::{Context, Invocation, client, refuse};
use crate::codes::{ClientCode, Exit};

pub(crate) fn run(
    invocation: Invocation,
    _context: &Context<'_>,
    _start: Instant,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Exit {
    let error = client(
        ClientCode::Internal,
        "not written yet".to_string(),
        Vec::new(),
    );
    refuse(
        invocation.command.command.to_string(),
        error,
        invocation.form,
        stdout,
        stderr,
    )
}
