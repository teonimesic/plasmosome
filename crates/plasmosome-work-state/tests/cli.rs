use plasmosome_work_state::contract::parse_contract_request;

#[test]
fn all_and_transport_accept_no_remote_or_credential_arguments() {
    for case in ["all", "transport"] {
        let request = parse_contract_request(["contract-test", case, "--archive", "archive", "--bd", "bd"])
            .expect("offline contract command parses");
        assert_eq!(request.case, case);
    }
    for forbidden in ["--github-remote", "--confirm-disposable"] {
        assert!(parse_contract_request(["contract-test", "all", "--archive", "archive", "--bd", "bd", forbidden, "value"]).is_err());
    }
}
