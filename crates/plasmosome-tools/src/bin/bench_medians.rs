#![deny(clippy::exit)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let summary = std::env::var_os("GITHUB_STEP_SUMMARY").map(PathBuf::from);
    ExitCode::from(plasmosome_tools::bench_medians::run(
        &args,
        summary.as_deref(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}
