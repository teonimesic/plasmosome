use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("lane-probe: refused: runs only in a Linux guest");
    ExitCode::from(2)
}
