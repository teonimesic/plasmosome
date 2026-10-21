use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(plasmosome_cli::main_with(
        std::env::args_os().skip(1),
        env!("CARGO_PKG_VERSION"),
    ))
}
