use plasmosome_core::{
    Controller, ControllerState, InstanceName, MAX_LINE_BYTES, Response, serve_connection,
};

const AFTER: &[u8] = b"{\"id\":\"after\",\"method\":\"plasmosome.status\",\"params\":{}}\n";

/// Serves `data`, then a status request with id `"after"`, on one connection to the real
/// `Controller`, and asserts one well-formed `Response` per line, the last a success for
/// `"after"`.
pub fn check(data: &[u8]) {
    if data.len() > MAX_LINE_BYTES {
        return;
    }
    let mut script = data.to_vec();
    script.push(b'\n');
    script.extend_from_slice(AFTER);
    let lines = data.iter().filter(|byte| **byte == b'\n').count() + 2;

    let mut controller = Controller::new(
        InstanceName::parse("fuzz").expect("fuzz is an instance name"),
        ControllerState::default(),
        0,
    );
    let mut out: Vec<u8> = Vec::new();
    let served = serve_connection(script.as_slice(), &mut out, &mut controller);
    assert!(served.is_ok(), "the connection failed: {served:?}");

    assert!(
        out.ends_with(b"\n"),
        "the last reply is not ended by a newline"
    );
    let replies: Vec<&[u8]> = out[..out.len() - 1].split(|byte| *byte == b'\n').collect();
    assert_eq!(
        replies.len(),
        lines,
        "the connection wrote {} replies for {lines} lines",
        replies.len()
    );

    let mut last = None;
    for reply in replies {
        let parsed = serde_json::from_slice::<Response>(reply);
        assert!(
            parsed.is_ok(),
            "the reply {:?} is not a control response: {parsed:?}",
            String::from_utf8_lossy(reply)
        );
        last = parsed.ok();
    }
    assert!(
        matches!(&last, Some(Response::Success { id, .. }) if id.get() == "\"after\""),
        "the request after the input was not answered with a success: {last:?}"
    );
}
