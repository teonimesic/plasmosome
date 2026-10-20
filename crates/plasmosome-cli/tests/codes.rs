use plasmosome_cli::{ClientCode, Exit};

const SPEC_025_CLIENT_CODES: [(&str, u8); 18] = [
    ("invalid_argument", 64),
    ("not_served", 2),
    ("invalid_home", 2),
    ("no_instance", 1),
    ("not_running", 1),
    ("unreachable", 1),
    ("deadline", 1),
    ("transport", 1),
    ("local_io", 1),
    ("no_reply", 1),
    ("outcome_unknown", 3),
    ("ambiguous_instance", 2),
    ("unsafe_socket", 2),
    ("peer_mismatch", 2),
    ("socket_path_too_long", 2),
    ("reply_too_large", 2),
    ("bad_reply", 70),
    ("internal", 70),
];

#[test]
fn every_client_code_has_its_exit() {
    let listed: Vec<(&str, u8)> = ClientCode::ALL
        .iter()
        .map(|code| (code.as_str(), code.exit().code()))
        .collect();
    assert_eq!(listed, SPEC_025_CLIENT_CODES);
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
