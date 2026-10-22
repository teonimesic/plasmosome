use plasmosome_membrane::readiness::{NotReady, Readiness, classify};

const DIAGNOSTIC_BYTES: usize = 1024;

/// Classifies the first line of `data` as a broker's readiness reply, and asserts that the
/// verdict is one a reply line can give and that a malformed line keeps at most 1,024 bytes of
/// it, counting each U+FFFD as the one byte it may stand for.
pub fn check(data: &[u8]) {
    let frame = match data.iter().position(|byte| *byte == b'\n') {
        Some(end) => &data[..end],
        None => data,
    };
    match classify(frame) {
        Readiness::Ready { .. } | Readiness::NotReady(NotReady::Reported { .. }) => {}
        Readiness::NotReady(NotReady::Malformed { line }) => {
            let kept: usize = line
                .chars()
                .map(|kept| match kept {
                    char::REPLACEMENT_CHARACTER => 1,
                    kept => kept.len_utf8(),
                })
                .sum();
            assert!(
                kept <= DIAGNOSTIC_BYTES,
                "a malformed reply keeps {kept} bytes of the broker's line, past {DIAGNOSTIC_BYTES}"
            );
        }
        verdict @ Readiness::NotReady(NotReady::TimedOut | NotReady::Unreachable { .. }) => {
            panic!("a reply line was judged {verdict:?}, which only the socket side gives")
        }
    }
}
