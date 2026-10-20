use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("lane-vmm: refused: runs only on macOS");
    ExitCode::from(2)
}
