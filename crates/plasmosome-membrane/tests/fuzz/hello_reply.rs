use plasmosome_membrane::runtime::digest::Digest;
use plasmosome_membrane::runtime::hello::{
    BootToken, HelloRefusal, judge_normal, judge_withdrawal,
};

const GUEST_TEXT_BYTES: usize = 1024;

/// Judges the first line of `data` as the hello reply of both lanes, and asserts that every
/// text a refusal keeps from the guest is at most 1,024 bytes and that no pointer it keeps ends
/// inside a `~` escape.
pub fn check(data: &[u8]) {
    let frame = match data.iter().position(|byte| *byte == b'\n') {
        Some(end) => &data[..end],
        None => data,
    };
    let policy = Digest::parse_hex(&"11".repeat(32)).expect("the policy is 64 hexadecimal digits");
    let boot = BootToken::parse_hex(&"22".repeat(32)).expect("the boot is 64 hexadecimal digits");
    if let Err(refusal) = judge_normal(frame, &policy) {
        bounded(&refusal);
    }
    if let Err(refusal) = judge_withdrawal(frame, &policy, &boot) {
        bounded(&refusal);
    }
}

fn bounded(refusal: &HelloRefusal) {
    let _ = refusal.to_string();
    match refusal {
        HelloRefusal::DuplicateKey { path } => pointer("DuplicateKey.path", path),
        HelloRefusal::UnknownField { field } => pointer("UnknownField.field", field),
        HelloRefusal::WrongId { found, .. } => guest_text("WrongId.found", found),
        HelloRefusal::GuestError {
            message, detail, ..
        } => {
            guest_text("GuestError.message", message);
            if let Some(detail) = detail {
                guest_text("GuestError.detail", detail);
            }
        }
        HelloRefusal::NotUtf8
        | HelloRefusal::NotJson { .. }
        | HelloRefusal::NotAnObject { .. }
        | HelloRefusal::MissingField { .. }
        | HelloRefusal::WrongType { .. }
        | HelloRefusal::BadBoot { .. }
        | HelloRefusal::BadPolicy { .. }
        | HelloRefusal::PolicyMismatch { .. }
        | HelloRefusal::BootChanged { .. } => {}
    }
}

fn guest_text(name: &str, text: &str) {
    assert!(
        text.len() <= GUEST_TEXT_BYTES,
        "{name} keeps {} bytes of the guest's text, past {GUEST_TEXT_BYTES}",
        text.len()
    );
}

fn pointer(name: &str, pointer: &str) {
    guest_text(name, pointer);
    assert!(
        !pointer.ends_with('~'),
        "{name} ends inside a ~ escape: {:?}",
        &pointer[pointer.floor_char_boundary(pointer.len().saturating_sub(8))..]
    );
}
