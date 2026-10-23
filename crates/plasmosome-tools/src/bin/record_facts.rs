#![deny(clippy::exit)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ExitCode::from(plasmosome_tools::record_facts::run(
        &args,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}
