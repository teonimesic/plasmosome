use std::path::PathBuf;

use plasmosome_work_state::{run_contract, ContractRequest};

fn main() {
    let mut values = std::env::args().skip(1);
    if values.next().as_deref() != Some("contract-test") { fail("invalid_command", 2); }
    let Some(case) = values.next() else { fail("invalid_command", 2); };
    let mut archive = None;
    let mut binary = None;
    let mut remote = None;
    let mut confirmation = None;
    while let Some(flag) = values.next() {
        let Some(value) = values.next() else { fail("invalid_command", 2); };
        match flag.as_str() { "--archive" => archive = Some(PathBuf::from(value)), "--bd" => binary = Some(PathBuf::from(value)), "--github-remote" => remote = Some(value), "--confirm-disposable" => confirmation = Some(value), _ => fail("invalid_command", 2) }
    }
    let (Some(archive), Some(binary)) = (archive, binary) else { fail("invalid_command", 2); };
    let request = ContractRequest { case, archive, binary, github_remote: remote, confirmation };
    match run_contract(&request) {
        Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
        Err(result) => { println!("{}", serde_json::to_string(&result).unwrap()); fail(&result.code, if result.code == "cutover_blocked" { 1 } else { 2 }); }
    }
}

fn fail(message: &str, code: i32) -> ! { eprintln!("{message}"); std::process::exit(code) }
