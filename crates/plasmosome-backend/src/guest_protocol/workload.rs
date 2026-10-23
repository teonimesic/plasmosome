use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::BootToken;
use crate::wire::{ObjectOnly, validated_object_serde};

/// A canonical guest process identifier, written as `e-<positive u64>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecId(u64);

/// The text was not a canonical positive exec identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecIdFault;

impl ExecId {
    /// Builds an identifier from a positive counter; zero returns none.
    pub fn new(number: u64) -> Option<Self> {
        (number > 0).then_some(Self(number))
    }

    /// The positive counter, without the wire prefix.
    pub fn number(self) -> u64 {
        self.0
    }

    /// Accepts only `e-` followed by canonical, positive decimal within u64.
    pub fn parse(text: &str) -> Result<Self, ExecIdFault> {
        let digits = text.strip_prefix("e-").ok_or(ExecIdFault)?;
        if !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()) {
            digits.parse().ok().and_then(Self::new).ok_or(ExecIdFault)
        } else {
            Err(ExecIdFault)
        }
    }
}

impl Serialize for ExecId {
    fn serialize<S: Serializer>(&self, writer: S) -> Result<S::Ok, S::Error> {
        writer.serialize_str(&format!("e-{}", self.0))
    }
}

impl<'de> Deserialize<'de> for ExecId {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(reader)?).map_err(|_| D::Error::custom("invalid exec id"))
    }
}

/// A cell-wide retry key: 1 to 64 ASCII letters, digits, dots, underscores or hyphens.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RequestKey(String);

/// The key has an invalid length or character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestKeyFault;

impl RequestKey {
    /// Checks the exact key without trimming or rewriting it.
    pub fn parse(text: &str) -> Result<Self, RequestKeyFault> {
        if (1..=64).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            Ok(Self(text.to_owned()))
        } else {
            Err(RequestKeyFault)
        }
    }

    /// The checked key, with its original spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RequestKey {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(reader)?)
            .map_err(|_| D::Error::custom("invalid request key"))
    }
}

/// Why PID1 or a controller must refuse an argv; NUL names its argument index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgvFault {
    Empty,
    EmptyProgram,
    Nul { index: usize },
}

/// Validates an argv separately from wire decoding so PID1 can return `invalid_argv`.
pub fn check_argv(argv: &[String]) -> Result<(), ArgvFault> {
    let first = argv.first().ok_or(ArgvFault::Empty)?;
    if first.is_empty() {
        return Err(ArgvFault::EmptyProgram);
    }
    match argv.iter().position(|arg| arg.contains('\0')) {
        Some(index) => Err(ArgvFault::Nul { index }),
        None => Ok(()),
    }
}

/// A process state. JSON must be one of the three exact strings, never an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecState {
    Running,
    Exited,
    Signaled,
}

impl<'de> Deserialize<'de> for ExecState {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        match String::deserialize(reader)?.as_str() {
            "running" => Ok(Self::Running),
            "exited" => Ok(Self::Exited),
            "signaled" => Ok(Self::Signaled),
            _ => Err(D::Error::custom("unknown exec state")),
        }
    }
}

/// A Linux aarch64 signal number, from 1 through 64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Signal(u8);

impl Signal {
    /// Returns none outside the Linux aarch64 signal range.
    pub fn new(number: u8) -> Option<Self> {
        (1..=64).contains(&number).then_some(Self(number))
    }
    /// The Linux signal number.
    pub fn get(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Signal {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        Self::new(u8::deserialize(reader)?).ok_or_else(|| D::Error::custom("signal outside 1..64"))
    }
}

/// Exactly one process status. Terminal states record the time to reap in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExecStatus {
    Running {
        boot: BootToken,
        exec_id: ExecId,
    },
    Exited {
        boot: BootToken,
        exec_id: ExecId,
        exit_code: u8,
        duration_ms: u64,
    },
    Signaled {
        boot: BootToken,
        exec_id: ExecId,
        signal: Signal,
        duration_ms: u64,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusWire {
    boot: BootToken,
    exec_id: ExecId,
    state: ExecState,
    #[serde(default, deserialize_with = "present")]
    exit_code: Option<u8>,
    #[serde(default, deserialize_with = "present")]
    signal: Option<Signal>,
    #[serde(default, deserialize_with = "present")]
    duration_ms: Option<u64>,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    reader: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(reader).map(Some)
}

impl<'de> Deserialize<'de> for ExecStatus {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        let StatusWire {
            boot,
            exec_id,
            state,
            exit_code,
            signal,
            duration_ms,
        } = StatusWire::deserialize(ObjectOnly::new(reader))?;
        match (state, exit_code, signal, duration_ms) {
            (ExecState::Running, None, None, None) => Ok(Self::Running { boot, exec_id }),
            (ExecState::Exited, Some(exit_code), None, Some(duration_ms)) => Ok(Self::Exited {
                boot,
                exec_id,
                exit_code,
                duration_ms,
            }),
            (ExecState::Signaled, None, Some(signal), Some(duration_ms)) => Ok(Self::Signaled {
                boot,
                exec_id,
                signal,
                duration_ms,
            }),
            _ => Err(D::Error::custom("fields do not match exec state")),
        }
    }
}

impl ExecStatus {
    /// The boot that observed this process.
    pub fn boot(&self) -> BootToken {
        match self {
            Self::Running { boot, .. }
            | Self::Exited { boot, .. }
            | Self::Signaled { boot, .. } => *boot,
        }
    }

    /// The retained process this status describes.
    pub fn exec_id(&self) -> ExecId {
        match self {
            Self::Running { exec_id, .. }
            | Self::Exited { exec_id, .. }
            | Self::Signaled { exec_id, .. } => *exec_id,
        }
    }
}

/// The retained tail of one output stream; the counters are checked during construction and decoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct Stream {
    bytes: Vec<u8>,
    total: u64,
    truncated: bool,
    closed: bool,
}

/// An output tail exceeded its cap, lost retained bytes or contradicted its truncation flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFault {
    TooLong,
    ByteCount,
    Truncation,
}

impl std::fmt::Display for StreamFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid stream: {self:?}")
    }
}

impl std::error::Error for StreamFault {}

impl Stream {
    /// The maximum retained tail of either output stream.
    pub const MAX_BYTES: usize = 65_536;

    /// Checks that `bytes` keeps exactly the last min(total, MAX_BYTES) bytes; derives truncation.
    pub fn new(bytes: Vec<u8>, total: u64, closed: bool) -> Result<Self, StreamFault> {
        let stream = Self {
            truncated: total > bytes.len() as u64,
            bytes,
            total,
            closed,
        };
        stream.validate()?;
        Ok(stream)
    }

    fn validate(&self) -> Result<(), StreamFault> {
        if self.bytes.len() > Self::MAX_BYTES {
            return Err(StreamFault::TooLong);
        }
        if self.bytes.len() as u64 != self.total.min(Self::MAX_BYTES as u64) {
            return Err(StreamFault::ByteCount);
        }
        if self.truncated != (self.total > self.bytes.len() as u64) {
            return Err(StreamFault::Truncation);
        }
        Ok(())
    }

    /// The retained tail, in arrival order.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Every byte received, including bytes no longer retained.
    pub fn total(&self) -> u64 {
        self.total
    }
    /// Whether total exceeds the number of retained bytes.
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    /// Whether PID1 read EOF; process exit alone does not close a stream.
    pub fn closed(&self) -> bool {
        self.closed
    }
}

validated_object_serde!(Stream through Stream);

/// Selects which retained object is absent, exactly `exec` or `output` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotRetainedWhat {
    Exec,
    Output,
}

impl<'de> Deserialize<'de> for NotRetainedWhat {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        match String::deserialize(reader)?.as_str() {
            "exec" => Ok(Self::Exec),
            "output" => Ok(Self::Output),
            _ => Err(D::Error::custom("not_retained what must be exec or output")),
        }
    }
}
