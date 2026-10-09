use super::digest::{Digest, DigestError};

/// One of the two connections the guest shim opens to the host's 4090
/// control port. The shim opens the normal lane first and the withdrawal lane
/// after the normal lane's hello.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlLane {
    /// The lane that carries every 4090 verb.
    Normal,
    /// The prioritized lane that admits only hello, observe and remove.
    Withdrawal,
}

/// The hello request line the host writes on `lane`, ending in one newline.
///
/// It carries exactly `id`, `method` and `params: {lane}`. The guest shim
/// exits on any other field, so send these bytes unchanged.
pub fn hello_request(lane: ControlLane) -> &'static [u8] {
    todo!("{lane:?}")
}

/// The request id that `hello_request(lane)` carries and its reply must
/// echo: 1 on the normal lane, 2 on the withdrawal lane.
pub fn request_id(lane: ControlLane) -> u64 {
    todo!("{lane:?}")
}

/// The guest's boot token: 32 random bytes the shim draws once per boot and
/// returns on both lanes as 64 lowercase hexadecimal digits.
///
/// `Debug` prints those 64 digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BootToken(Digest);

impl BootToken {
    /// Reads exactly 64 lowercase hexadecimal digits. Uppercase digits, any
    /// other byte and any other length are refused with the first fault.
    pub fn parse_hex(text: &str) -> Result<BootToken, DigestError> {
        todo!("{text}")
    }

    /// The 64 lowercase hexadecimal digits.
    pub fn hex(&self) -> String {
        todo!()
    }
}

impl std::fmt::Debug for BootToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("{f:p}")
    }
}

/// A hello reply that passed every check in [`judge_reply`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HelloAnswer {
    /// The lane the reply arrived on.
    pub lane: ControlLane,
    /// The guest's boot token.
    pub boot: BootToken,
    /// The SHA-256 of the guest policy the shim verified; equal to the
    /// expected one.
    pub policy: Digest,
}

/// Why [`judge_reply`] refused a hello reply.
///
/// `field` and `path` are JSON pointers into the reply: `""` is the whole
/// reply and `"/result/boot"` its boot token. Text the guest chose is cut to
/// 1024 bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HelloRefusal {
    /// The frame is not UTF-8.
    NotUtf8,
    /// The frame is not exactly one JSON value.
    NotJson { detail: String },
    /// An object in the frame repeats the key at `path`.
    DuplicateKey { path: String },
    /// The value at `field` is not a JSON object.
    NotAnObject { field: String },
    /// The record holds a field hello does not define.
    UnknownField { field: String },
    /// The record lacks a field hello requires.
    MissingField { field: String },
    /// The value at `field` is not a string.
    WrongType { field: String },
    /// The reply answers `found`, written as JSON, instead of the lane's
    /// integer request id.
    WrongId { expected: u64, found: String },
    /// The guest answered with an error object, under any id. `code` is
    /// `None` when the error has no integer `code`; `message` is empty when
    /// it has no string `message`.
    GuestError { code: Option<i64>, message: String },
    /// `boot` is not 64 lowercase hexadecimal digits.
    BadBoot { fault: DigestError },
    /// `policy` is not 64 lowercase hexadecimal digits.
    BadPolicy { fault: DigestError },
    /// The guest verified a policy other than the expected one.
    PolicyMismatch { expected: Digest, reported: Digest },
    /// This lane's boot token differs from the one an earlier hello returned.
    BootChanged { first: BootToken, second: BootToken },
}

impl std::fmt::Display for HelloRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("{f:p}")
    }
}

impl std::error::Error for HelloRefusal {}

/// Judges one hello reply read from `lane`.
///
/// `frame` is one line without its newline; the caller's reader bounds its
/// size. The reply must be exactly `{"id", "result": {"boot", "policy"}}`
/// with the lane's request id, a 64-digit lowercase hexadecimal boot token
/// and a policy equal to `expected_policy`. Pass the boot token of the
/// normal lane's answer as `earlier` when judging the withdrawal lane, and
/// of the original association when judging a reconnection; the boot must
/// then equal it. An error object refuses as [`HelloRefusal::GuestError`]
/// whatever its id, because protocol failures answer under a `null` id.
pub fn judge_reply(
    lane: ControlLane,
    frame: &[u8],
    expected_policy: &Digest,
    earlier: Option<&BootToken>,
) -> Result<HelloAnswer, HelloRefusal> {
    todo!("{lane:?} {frame:?} {expected_policy:?} {earlier:?}")
}
