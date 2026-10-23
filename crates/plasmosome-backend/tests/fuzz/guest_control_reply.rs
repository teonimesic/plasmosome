use plasmosome_backend::guest_protocol::*;
use plasmosome_backend::paging::MAX_FRAME_BYTES;
use serde_json::json;

/// Decodes a reader-bounded frame for every control method, checking typed successes
/// and every fault's guest text. The caller supplies a frame without its newline.
pub fn check(data: &[u8]) {
    if data.len() > MAX_FRAME_BYTES {
        return;
    }
    let boot = BootToken::from_bytes([37; 32]);
    let grant = "00112233-4455-4677-8899-aabbccddee25";
    let owner = json!({"cell":"cell-1","plugin":"plugin"});
    let operation = json!({"WriteSessionFile":{"id":grant,"owner":owner,"path":"/session",
        "recipe":{"contents":[3,17],"mode":"read_only","guest_path":"/run/session"}}});
    let inverse = json!({"id":grant,"capability":{"SessionFile":{"path":"/session",
        "recipe":{"contents":[3,17],"mode":"read_only","guest_path":"/run/session"}}}});
    for (method, params) in [
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
    ] {
        let request = encode_request(17, method, &params).unwrap();
        let call = decode_control_request(LaneState::Assigned(ControlLane::Normal), &request)
            .unwrap()
            .call;
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
        },
        ReplyFault::NotUtf8 | ReplyFault::BootChanged { .. } | ReplyFault::GrantMismatch { .. } => {
        }
    }
}
