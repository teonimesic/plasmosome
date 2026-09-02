use plasmosome_work_state::contract::parse_contract_request;

#[test]
fn all_and_transport_accept_no_remote_or_credential_arguments() {
    for case in ["all", "transport"] {
        let request =
            parse_contract_request(["contract-test", case, "--archive", "archive", "--bd", "bd"])
                .expect("offline contract command parses");
        assert_eq!(request.case, case);
    }
    for forbidden in ["--github-remote", "--confirm-disposable"] {
        assert_eq!(
            parse_contract_request([
                "contract-test",
                "all",
                "--archive",
                "archive",
                "--bd",
                "bd",
                forbidden,
                "value"
            ])
            .unwrap_err(),
            "invalid_command"
        );
    }
}

#[test]
fn individual_new_cases_require_source_ref_and_all_defaults_to_origin_main() {
    for case in ["document-mapping", "shadow-parity"] {
        let request = parse_contract_request([
            "contract-test",
            case,
            "--source-ref",
            "origin/main",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ])
        .expect("new document case parses with an explicit source ref");
        assert_eq!(request.source_ref.as_deref(), Some("origin/main"));

        assert_eq!(
            parse_contract_request(["contract-test", case, "--archive", "archive", "--bd", "bd",])
                .unwrap_err(),
            "invalid_command"
        );
        assert_eq!(
            parse_contract_request([
                "contract-test",
                case,
                "--source-ref",
                "origin/main",
                "--source-ref",
                "HEAD",
                "--archive",
                "archive",
                "--bd",
                "bd",
            ])
            .unwrap_err(),
            "invalid_command"
        );
    }

    let aggregate =
        parse_contract_request(["contract-test", "all", "--archive", "archive", "--bd", "bd"])
            .expect("the existing aggregate form remains valid");
    assert_eq!(aggregate.source_ref.as_deref(), Some("origin/main"));

    assert_eq!(
        parse_contract_request([
            "contract-test",
            "transport",
            "--source-ref",
            "origin/main",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ])
        .unwrap_err(),
        "invalid_command"
    );
}

#[test]
fn source_flags_are_unambiguous_and_legacy_forms_stay_unchanged() {
    let aggregate = parse_contract_request([
        "contract-test",
        "all",
        "--source-ref",
        "13c0f68c13743f4db2fb123fef560f3fa12734d1",
        "--archive",
        "archive",
        "--bd",
        "bd",
    ])
    .expect("all accepts one explicit source ref");
    assert_eq!(
        aggregate.source_ref.as_deref(),
        Some("13c0f68c13743f4db2fb123fef560f3fa12734d1")
    );

    for values in [
        vec![
            "contract-test",
            "all",
            "--source-ref",
            "",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "document-mapping",
            "--source-ref",
            "   ",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--source-ref",
            "origin/main",
            "--source-ref",
            "HEAD",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--source-ref",
            "--archive",
            "archive",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--archive",
            "first",
            "--archive",
            "second",
            "--bd",
            "bd",
        ],
        vec![
            "contract-test",
            "all",
            "--archive",
            "archive",
            "--bd",
            "first",
            "--bd",
            "second",
        ],
    ] {
        assert_eq!(
            parse_contract_request(values).unwrap_err(),
            "invalid_command"
        );
    }
}
