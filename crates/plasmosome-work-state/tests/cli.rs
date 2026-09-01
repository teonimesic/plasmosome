use std::path::PathBuf;

use plasmosome_work_state::{run_contract, ContractRequest};

#[test]
fn github_and_all_refuse_a_missing_fixture() {
    for case in ["github", "all"] {
        let result = run_contract(&ContractRequest { case: case.to_owned(), archive: PathBuf::from("missing"), binary: PathBuf::from("missing"), github_remote: None, confirmation: None }).unwrap_err();
        assert_eq!(result.code, "github_fixture_required");
    }
}
