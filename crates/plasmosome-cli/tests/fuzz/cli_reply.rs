use plasmosome_cli::json::{self, MAX_REPLY_BYTES};
use plasmosome_cli::reply::{ReadFailure, judge_reply, read_reply};

/// Reads the first reply line of `data`, judges it, and writes what it scans to.
///
/// It asserts that `read_reply` consumes exactly the first line and its newline, or returns
/// `Closed` when `data` holds none, and that every scanned value's compact form holds no byte
/// below 0x20 and scans back to an equal value.
pub fn check(data: &[u8]) {
    if data.len() > MAX_REPLY_BYTES {
        return;
    }
    let mut rest: &[u8] = data;
    let read = read_reply(&mut rest);
    let Some(end) = data.iter().position(|byte| *byte == b'\n') else {
        assert_eq!(
            read,
            Err(ReadFailure::Closed),
            "an input with no newline is a closed connection"
        );
        return;
    };
    let Ok(line) = read else {
        panic!("read_reply refused a line of {end} bytes: {read:?}");
    };
    assert!(
        line == data[..end],
        "read_reply returned {} bytes, not the {end} before the first newline",
        line.len()
    );
    assert!(
        rest == &data[end + 1..],
        "read_reply left {} bytes unread, not the {} after the first newline",
        rest.len(),
        data.len() - end - 1
    );

    let _ = judge_reply(&line);
    let value = match json::scan(&line) {
        Ok(value) => value,
        Err(error) => {
            let _ = error.to_string();
            return;
        }
    };
    let compact = json::write_compact(&value);
    assert!(
        compact.bytes().all(|byte| byte >= 0x20),
        "the compact form holds a byte below 0x20: {compact:?}"
    );
    let again = json::scan(compact.as_bytes());
    assert!(
        matches!(&again, Ok(again) if *again == value),
        "the compact form does not scan back to the value it was written from: {again:?}"
    );
    let _ = json::write_human(&value);
}
