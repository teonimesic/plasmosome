#![deny(clippy::exit)]

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::process::ExitCode::from(plasmosome_tools::fuzz_run::run(
        &args,
        &mut std::process::Command::status,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    ))
}
