use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("lane-init: refused: runs only as a Linux guest's process 1");
    ExitCode::from(2)
}
