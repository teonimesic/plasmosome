use std::fs;

use plasmosome_cli::{ClientCode, Exit};

const SPEC_025: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/025-plasmosome-command-line.md"
);

const CODE_TABLE_HEADER: &str = "| Code | Exit | When | Structured fields |";

fn spec_025_client_codes() -> Vec<(String, u8)> {
    let spec = fs::read_to_string(SPEC_025).expect("spec 025 is readable beside the crate");
    spec.lines()
        .skip_while(|line| *line != CODE_TABLE_HEADER)
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|row| {
            let cells: Vec<&str> = row.split('|').map(str::trim).collect();
            let exit = cells[2]
                .parse()
                .unwrap_or_else(|_| panic!("{row:?} gives an exit that is not a number"));
            (cells[1].trim_matches('`').to_string(), exit)
        })
        .collect()
}

#[test]
fn every_client_code_has_its_exit() {
    let table = spec_025_client_codes();
    assert!(
        !table.is_empty(),
        "spec 025's client code table was not found"
    );
    let listed: Vec<(String, u8)> = ClientCode::ALL
        .iter()
        .map(|code| (code.as_str().to_string(), code.exit().code()))
        .collect();
    assert_eq!(listed, table);
}

#[test]
fn each_exit_has_its_status() {
    let statuses: Vec<u8> = [
        Exit::Done,
        Exit::Unavailable,
        Exit::Refused,
        Exit::OutcomeUnknown,
        Exit::Usage,
        Exit::Internal,
    ]
    .into_iter()
    .map(Exit::code)
    .collect();
    assert_eq!(statuses, [0, 1, 2, 3, 64, 70]);
}
