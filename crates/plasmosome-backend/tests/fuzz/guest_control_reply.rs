use plasmosome_backend::guest_protocol::*;
use plasmosome_backend::paging::MAX_FRAME_BYTES;
use serde_json::json;

const BOOT_BYTES: [u8; 32] = [37; 32];

/// Decodes a reader-bounded frame for every control method, checking typed successes
/// and every fault's guest text. The caller supplies a frame without its newline.
pub fn check(data: &[u8]) {
    if data.len() > MAX_FRAME_BYTES {
        return;
    }
    let boot = BootToken::from_bytes(BOOT_BYTES);
    for call in calls() {
        match decode_control_reply(&call, 17, Some(&boot), data) {
            Ok(result) => {
                let encoded = encode_control_result(&RequestId::number(17), &result).unwrap();
                assert_eq!(
                    decode_control_reply(&call, 17, Some(&boot), &encoded),
                    Ok(result)
                );
            }
            Err(fault) => bounded(&fault),
        }
        if let Err(fault) = decode_control_reply(&call, 17, None, data) {
            bounded(&fault);
        }
    }
}

fn calls() -> impl Iterator<Item = ControlCall> {
    let grant = "00112233-4455-4677-8899-aabbccddee25";
    let owner = json!({"cell":"cell-1","plugin":"plugin"});
    let operation = json!({"WriteSessionFile":{"id":grant,"owner":owner,"path":"/session",
        "recipe":{"contents":[3,17],"mode":"read_only","guest_path":"/run/session"}}});
    let inverse = json!({"id":grant,"capability":{"SessionFile":{"path":"/session",
        "recipe":{"contents":[3,17],"mode":"read_only","guest_path":"/run/session"}}}});
    [
        ("hello", json!({"lane":"normal"})),
        (
            "observe",
            json!({"deadline_ms":17,"snapshot":17,"offset":3}),
        ),
        (
            "install",
            json!({"operation":operation,"address":null,"deadline_ms":17}),
        ),
        ("activate", json!({"grant":grant,"deadline_ms":17})),
        ("drain", json!({"grant":grant,"deadline_ms":17})),
        (
            "remove",
            json!({"inverse":inverse,"owner":owner,"force":true,"deadline_ms":17}),
        ),
        ("shutdown", json!({"deadline_ms":17})),
        (
            "exec",
            json!({"argv":["true"],"request_key":null,"deadline_ms":17}),
        ),
        (
            "exec_status",
            json!({"exec_id":"e-11","wait":true,"deadline_ms":17}),
        ),
        ("exec_output", json!({"exec_id":"e-11","deadline_ms":17})),
        ("workload_stop", json!({"deadline_ms":17})),
    ]
    .into_iter()
    .map(|(method, params)| {
        let request = encode_request(17, method, &params).unwrap();
        decode_control_request(LaneState::Assigned(ControlLane::Normal), &request)
            .unwrap()
            .call
    })
}

fn bounded(fault: &ReplyFault) {
    let text = |text: &str| assert!(text.len() <= 1024, "fault keeps {} guest bytes", text.len());
    match fault {
        ReplyFault::NotJson { detail }
        | ReplyFault::Envelope { detail }
        | ReplyFault::Result { detail } => text(detail),
        ReplyFault::WrongId { found, .. } => text(found),
        ReplyFault::Unattributed(error) | ReplyFault::Refused(error) => match error {
            BridgeError::Protocol { message, .. } => text(message),
            BridgeError::Refused {
                message,
                detail: RefusalDetail::Rejected { reason },
            } => {
                text(message);
                text(reason);
            }
            BridgeError::Refused { message, .. } => text(message),
        },
        ReplyFault::NotUtf8
        | ReplyFault::BootChanged { .. }
        | ReplyFault::GrantMismatch { .. }
        | ReplyFault::ExecMismatch { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn outcome(result: Result<ControlResult, ReplyFault>) -> &'static str {
        match result {
            Ok(_) => "Success",
            Err(ReplyFault::NotUtf8) => "NotUtf8",
            Err(ReplyFault::NotJson { .. }) => "NotJson",
            Err(ReplyFault::Envelope { .. }) => "Envelope",
            Err(ReplyFault::WrongId { .. }) => "WrongId",
            Err(ReplyFault::Unattributed(_)) => "Unattributed",
            Err(ReplyFault::Refused(_)) => "Refused",
            Err(ReplyFault::Result { .. }) => "Result",
            Err(ReplyFault::BootChanged { .. }) => "BootChanged",
            Err(ReplyFault::GrantMismatch { .. }) => "GrantMismatch",
            Err(ReplyFault::ExecMismatch { .. }) => "ExecMismatch",
        }
    }

    #[test]
    fn every_seed_has_its_expected_reply_outcomes() {
        let rows = [
            (
                "seed-active",
                [
                    "Result", "Result", "Result", "Success", "Result", "Result", "Result",
                ],
            ),
            (
                "seed-boot-changed",
                [
                    "BootChanged",
                    "Result",
                    "Result",
                    "Result",
                    "Result",
                    "Result",
                    "Result",
                ],
            ),
            ("seed-deep", ["Result"; 7]),
            (
                "seed-drained",
                [
                    "Result", "Result", "Result", "Result", "Success", "Result", "Result",
                ],
            ),
            ("seed-duplicate", ["Envelope"; 7]),
            ("seed-false", ["Result"; 7]),
            (
                "seed-grant-mismatch",
                [
                    "Result",
                    "Result",
                    "GrantMismatch",
                    "Result",
                    "Result",
                    "Result",
                    "Result",
                ],
            ),
            (
                "seed-hello",
                [
                    "Success", "Result", "Result", "Result", "Result", "Result", "Result",
                ],
            ),
            (
                "seed-installed",
                [
                    "Result", "Result", "Success", "Result", "Result", "Result", "Result",
                ],
            ),
            ("seed-long-text", ["Refused"; 7]),
            ("seed-malformed", ["NotJson"; 7]),
            ("seed-not-utf8", ["NotUtf8"; 7]),
            (
                "seed-observe",
                [
                    "Result", "Success", "Result", "Result", "Result", "Result", "Result",
                ],
            ),
            ("seed-protocol", ["Unattributed"; 7]),
            ("seed-refused", ["Refused"; 7]),
            (
                "seed-removed",
                [
                    "Result", "Result", "Result", "Result", "Result", "Success", "Result",
                ],
            ),
            (
                "seed-shutdown",
                [
                    "Result", "Result", "Result", "Result", "Result", "Result", "Success",
                ],
            ),
            ("seed-too-deep", ["NotJson"; 7]),
            ("seed-wrong-id", ["WrongId"; 7]),
        ];
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fuzz/corpus/guest_control_reply"
        ));
        let seeds: Vec<_> = replay::read_corpus(dir)
            .into_iter()
            .filter(|(name, _)| !name.starts_with("workload-"))
            .collect();
        assert_eq!(
            seeds
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            rows.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        );
        let boot = BootToken::from_bytes(BOOT_BYTES);
        let mut reached = BTreeSet::new();
        for ((name, bytes), (_, expected)) in seeds.iter().zip(rows) {
            check(bytes);
            let calls: Vec<_> = calls().take(7).collect();
            assert_eq!(calls.len(), expected.len());
            for (call, expected) in calls.iter().zip(expected) {
                let found = outcome(decode_control_reply(call, 17, Some(&boot), bytes));
                assert_eq!(found, expected, "{name} for {:?}", call.method());
                reached.insert(found);
                let without_boot = if expected == "BootChanged" {
                    "Success"
                } else {
                    expected
                };
                assert_eq!(
                    outcome(decode_control_reply(call, 17, None, bytes)),
                    without_boot,
                    "{name} for {:?} without a boot",
                    call.method()
                );
            }
        }
        assert_eq!(
            reached,
            BTreeSet::from([
                "Success",
                "NotUtf8",
                "NotJson",
                "Envelope",
                "WrongId",
                "Unattributed",
                "Refused",
                "Result",
                "BootChanged",
                "GrantMismatch"
            ])
        );
    }

    #[test]
    fn workload_seeds_reach_each_typed_result_and_correlation_fault() {
        let rows = [
            (
                "workload-boot",
                ["BootChanged", "BootChanged", "Result", "Result"],
            ),
            ("workload-exec", ["Success", "Success", "Result", "Result"]),
            ("workload-exited", ["Result", "Success", "Result", "Result"]),
            (
                "workload-mismatch",
                ["Success", "ExecMismatch", "Result", "Result"],
            ),
            ("workload-not-retained", ["Refused"; 4]),
            ("workload-output", ["Result", "Result", "Success", "Result"]),
            (
                "workload-signaled",
                ["Result", "Success", "Result", "Result"],
            ),
            ("workload-stop", ["Result", "Result", "Result", "Success"]),
        ];
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fuzz/corpus/guest_control_reply"
        ));
        let seeds: Vec<_> = replay::read_corpus(dir)
            .into_iter()
            .filter(|(name, _)| name.starts_with("workload-"))
            .collect();
        assert_eq!(
            seeds
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            rows.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        );
        let boot = BootToken::from_bytes(BOOT_BYTES);
        for ((name, bytes), (_, expected)) in seeds.iter().zip(rows) {
            check(bytes);
            let calls: Vec<_> = calls().skip(7).collect();
            assert_eq!(calls.len(), expected.len());
            for (call, expected) in calls.iter().zip(expected) {
                assert_eq!(
                    outcome(decode_control_reply(call, 17, Some(&boot), bytes)),
                    expected,
                    "{name} for {:?}",
                    call.method()
                );
            }
        }
    }
}
