/// One of the six exit statuses of spec 025 section 4. A caller branches on it before reading
/// any output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// 0: stdout holds the result.
    Done,
    /// 1: nothing can have changed, and repeating the command is safe.
    Unavailable,
    /// 2: the kernel, the registry or the client refused the request.
    Refused,
    /// 3: a request that changes state may have taken effect; observe before acting again.
    OutcomeUnknown,
    /// 64: the command line is wrong. `EX_USAGE` in `sysexits.h`.
    Usage,
    /// 70: the client or the daemon broke the protocol or failed. `EX_SOFTWARE`.
    Internal,
}

impl Exit {
    /// Returns the process exit code.
    pub fn code(self) -> u8 {
        todo!()
    }
}

/// The closed list of the client's own refusal codes, under `domain: "client"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientCode {
    InvalidArgument,
    NotServed,
    InvalidHome,
    NoInstance,
    NotRunning,
    Unreachable,
    Deadline,
    Transport,
    LocalIo,
    NoReply,
    OutcomeUnknown,
    AmbiguousInstance,
    UnsafeSocket,
    PeerMismatch,
    SocketPathTooLong,
    ReplyTooLarge,
    BadReply,
    Internal,
}

impl ClientCode {
    /// Every code, in the order of spec 025's table.
    pub const ALL: [ClientCode; 18] = [
        ClientCode::InvalidArgument,
        ClientCode::NotServed,
        ClientCode::InvalidHome,
        ClientCode::NoInstance,
        ClientCode::NotRunning,
        ClientCode::Unreachable,
        ClientCode::Deadline,
        ClientCode::Transport,
        ClientCode::LocalIo,
        ClientCode::NoReply,
        ClientCode::OutcomeUnknown,
        ClientCode::AmbiguousInstance,
        ClientCode::UnsafeSocket,
        ClientCode::PeerMismatch,
        ClientCode::SocketPathTooLong,
        ClientCode::ReplyTooLarge,
        ClientCode::BadReply,
        ClientCode::Internal,
    ];

    /// Returns the code as the error object's `code` string.
    pub fn as_str(self) -> &'static str {
        todo!()
    }

    /// Returns the exit status the command ends with when it refuses with this code.
    pub fn exit(self) -> Exit {
        todo!()
    }
}
