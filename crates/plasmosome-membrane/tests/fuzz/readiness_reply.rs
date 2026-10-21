use plasmosome_membrane::readiness::{NotReady, Readiness, classify};

const DIAGNOSTIC_CHARS: usize = 1024;

/// Classifies the first line of `data` as a broker's readiness reply, and asserts that the
/// verdict is one a reply line can give and that a malformed line keeps at most 1,024
/// characters of it.
pub fn check(data: &[u8]) {
    let frame = match data.iter().position(|byte| *byte == b'\n') {
        Some(end) => &data[..end],
        None => data,
    };
    match classify(frame) {
        Readiness::Ready { .. } | Readiness::NotReady(NotReady::Reported { .. }) => {}
        Readiness::NotReady(NotReady::Malformed { line }) => assert!(
            line.chars().count() <= DIAGNOSTIC_CHARS,
            "a malformed reply keeps {} characters of the broker's line, past {DIAGNOSTIC_CHARS}",
            line.chars().count()
        ),
        verdict @ Readiness::NotReady(NotReady::TimedOut | NotReady::Unreachable { .. }) => {
            panic!("a reply line was judged {verdict:?}, which only the socket side gives")
        }
    }
}
