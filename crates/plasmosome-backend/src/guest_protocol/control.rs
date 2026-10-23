use std::{fmt, net::Ipv4Addr, num::NonZeroU64};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{
    BootToken, BridgeError, ControlLane, EncodeError, ExecId, ExecState, ExecStatus,
    InboundRequest, ProtocolCode, ReplyFault, RequestId, RequestKey, RequestRefusal, Stream, Yes,
    cut, encode_request, encode_success, split_reply, split_request,
};
use crate::universe::{CellOwner, GrantId, UniverseOp, UniverseRemoval};
use crate::wire::{ObjectOnly, object_serde};
use crate::{digest::Digest, paging::ObservationPage};

/// A canonical IPv4 address in the guest proxy pool, inclusive of both endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProxyAddress(Ipv4Addr);

/// Why an address cannot name a guest proxy binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressFault {
    OutOfRange { address: Ipv4Addr },
    NotCanonical { text: String },
}

impl ProxyAddress {
    pub const FIRST: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
    pub const LAST: Ipv4Addr = Ipv4Addr::new(198, 19, 255, 254);

    /// Accepts only addresses from `FIRST` through `LAST`; allocates no binding.
    pub fn new(address: Ipv4Addr) -> Result<Self, AddressFault> {
        if !(Self::FIRST..=Self::LAST).contains(&address) {
            return Err(AddressFault::OutOfRange { address });
        }
        Ok(Self(address))
    }

    pub fn get(&self) -> Ipv4Addr {
        self.0
    }
}

impl fmt::Display for AddressFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { address } => {
                write!(f, "proxy address {address} is outside the guest pool")
            }
            Self::NotCanonical { text } => {
                write!(f, "proxy address {text:?} is not canonical IPv4")
            }
        }
    }
}

impl std::error::Error for AddressFault {}

impl Serialize for ProxyAddress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for ProxyAddress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let address = text
            .parse::<Ipv4Addr>()
            .ok()
            .filter(|a| a.to_string() == text);
        Self::new(address.ok_or_else(|| D::Error::custom(AddressFault::NotCanonical { text }))?)
            .map_err(D::Error::custom)
    }
}

/// The control methods offered by the guest, including the four workload verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlMethod {
    Hello,
    Observe,
    Install,
    Activate,
    Drain,
    Remove,
    Shutdown,
    Exec,
    ExecStatus,
    ExecOutput,
    WorkloadStop,
}

impl ControlMethod {
    pub const ALL: [Self; 11] = [
        Self::Hello,
        Self::Observe,
        Self::Install,
        Self::Activate,
        Self::Drain,
        Self::Remove,
        Self::Shutdown,
        Self::Exec,
        Self::ExecStatus,
        Self::ExecOutput,
        Self::WorkloadStop,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::Observe => "observe",
            Self::Install => "install",
            Self::Activate => "activate",
            Self::Drain => "drain",
            Self::Remove => "remove",
            Self::Shutdown => "shutdown",
            Self::Exec => "exec",
            Self::ExecStatus => "exec_status",
            Self::ExecOutput => "exec_output",
            Self::WorkloadStop => "workload_stop",
        }
    }

    /// Looks up an exact wire name, without trimming or changing its case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.name() == name)
    }

    pub fn admitted_on(self, lane: ControlLane) -> bool {
        lane == ControlLane::Normal || matches!(self, Self::Hello | Self::Observe | Self::Remove)
    }
}

impl Serialize for ControlMethod {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for ControlMethod {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::from_name(&name).ok_or_else(|| D::Error::custom("unknown control method"))
    }
}

/// The caller retains the assigned lane after accepting its first hello.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneState {
    AwaitingHello,
    Assigned(ControlLane),
}

/// Assigns a connection's role, or repeats its existing role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct HelloParams {
    pub lane: ControlLane,
}

/// Pages one observation; zero snapshot and offset start a capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ObserveParams {
    pub deadline_ms: NonZeroU64,
    pub snapshot: u64,
    pub offset: u64,
}

/// Installs a complete grant. `address` is required in JSON, non-null only for a proxy map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct InstallParams {
    pub operation: UniverseOp,
    #[serde(deserialize_with = "required_nullable")]
    pub address: Option<ProxyAddress>,
    pub deadline_ms: NonZeroU64,
}

/// Selects one grant for activate or drain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct GrantParams {
    pub grant: GrantId,
    pub deadline_ms: NonZeroU64,
}

/// Removes the exact inverse belonging to `owner`; `force` selects forced withdrawal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct RemoveParams {
    pub inverse: UniverseRemoval,
    pub owner: CellOwner,
    pub force: bool,
    pub deadline_ms: NonZeroU64,
}

/// Requests guest shutdown within a positive remaining budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ShutdownParams {
    pub deadline_ms: NonZeroU64,
}

object_serde!(HelloParams through HelloParams, ObserveParams through ObserveParams,
    GrantParams through GrantParams, RemoveParams through RemoveParams,
    ShutdownParams through ShutdownParams);

pub(crate) fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

/// Starts or retries a process. PID1 calls check_argv after decoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ExecParams {
    pub argv: Vec<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub request_key: Option<RequestKey>,
    pub deadline_ms: NonZeroU64,
}

/// Queries one retained process; wait uses only this request budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ExecStatusParams {
    pub exec_id: ExecId,
    pub wait: bool,
    pub deadline_ms: NonZeroU64,
}

/// Queries both retained output streams of one process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ExecOutputParams {
    pub exec_id: ExecId,
    pub deadline_ms: NonZeroU64,
}

/// Stops the workload within a positive remaining budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct WorkloadStopParams {
    pub deadline_ms: NonZeroU64,
}

/// A new process or the current state of a retained keyed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ExecResult {
    pub boot: BootToken,
    pub exec_id: ExecId,
    pub state: ExecState,
}

/// Both streams of a process; their EOF flags are independent of its status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ExecOutputResult {
    pub boot: BootToken,
    pub exec_id: ExecId,
    pub stdout: Stream,
    pub stderr: Stream,
}

/// Acknowledges that this boot stopped the workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct WorkloadStopResult {
    pub boot: BootToken,
    pub stopped: Yes,
}

object_serde!(ExecParams through ExecParams, ExecStatusParams through ExecStatusParams,
    ExecOutputParams through ExecOutputParams, WorkloadStopParams through WorkloadStopParams,
    ExecResult through ExecResult, ExecOutputResult through ExecOutputResult,
    WorkloadStopResult through WorkloadStopResult);

impl Serialize for InstallParams {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        InstallParams::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for InstallParams {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let params = InstallParams::deserialize(ObjectOnly::new(deserializer))?;
        if matches!(params.operation, UniverseOp::SetProxyMap { .. }) != params.address.is_some() {
            return Err(D::Error::custom(
                "address must be non-null exactly for SetProxyMap",
            ));
        }
        Ok(params)
    }
}

/// A typed control request, without its envelope id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlCall {
    Hello(HelloParams),
    Observe(ObserveParams),
    Install(InstallParams),
    Activate(GrantParams),
    Drain(GrantParams),
    Remove(RemoveParams),
    Shutdown(ShutdownParams),
    Exec(ExecParams),
    ExecStatus(ExecStatusParams),
    ExecOutput(ExecOutputParams),
    WorkloadStop(WorkloadStopParams),
}

impl ControlCall {
    pub fn method(&self) -> ControlMethod {
        match self {
            Self::Hello(_) => ControlMethod::Hello,
            Self::Observe(_) => ControlMethod::Observe,
            Self::Install(_) => ControlMethod::Install,
            Self::Activate(_) => ControlMethod::Activate,
            Self::Drain(_) => ControlMethod::Drain,
            Self::Remove(_) => ControlMethod::Remove,
            Self::Shutdown(_) => ControlMethod::Shutdown,
            Self::Exec(_) => ControlMethod::Exec,
            Self::ExecStatus(_) => ControlMethod::ExecStatus,
            Self::ExecOutput(_) => ControlMethod::ExecOutput,
            Self::WorkloadStop(_) => ControlMethod::WorkloadStop,
        }
    }

    /// The selected grant, or none for a call that does not select a grant.
    pub fn grant(&self) -> Option<GrantId> {
        match self {
            Self::Install(params) => Some(params.operation.id()),
            Self::Activate(params) | Self::Drain(params) => Some(params.grant),
            Self::Remove(params) => Some(params.inverse.id),
            _ => None,
        }
    }
}

/// Reads a request without its newline. Unknown or unadmitted methods are -32601,
/// observe needs a u64 id (-32600), and bad params or a changed hello lane are -32602.
/// Envelope faults retain `split_request`'s codes. The reader bounds the frame;
/// this function does no IO and does not change the caller's lane state.
pub fn decode_control_request(
    state: LaneState,
    line: &[u8],
) -> Result<InboundRequest<ControlCall>, RequestRefusal> {
    let (id, name, params) = split_request(line)?;
    let refuse = |code, message: &str| RequestRefusal {
        id: id.clone(),
        error: BridgeError::Protocol {
            code,
            message: cut(message),
        },
    };
    let method = ControlMethod::from_name(&name)
        .ok_or_else(|| refuse(ProtocolCode::MethodNotFound, "unknown control method"))?;
    let admitted = match state {
        LaneState::AwaitingHello => method == ControlMethod::Hello,
        LaneState::Assigned(lane) => method.admitted_on(lane),
    };
    if !admitted {
        return Err(refuse(
            ProtocolCode::MethodNotFound,
            "method is not offered on this lane",
        ));
    }
    if method == ControlMethod::Observe && id.as_u64().is_none() {
        return Err(refuse(
            ProtocolCode::InvalidRequest,
            "observe requires an unsigned 64-bit id",
        ));
    }
    let text = params.get();
    let call = match method {
        ControlMethod::Hello => serde_json::from_str(text).map(ControlCall::Hello),
        ControlMethod::Observe => serde_json::from_str(text).map(ControlCall::Observe),
        ControlMethod::Install => serde_json::from_str(text).map(ControlCall::Install),
        ControlMethod::Activate => serde_json::from_str(text).map(ControlCall::Activate),
        ControlMethod::Drain => serde_json::from_str(text).map(ControlCall::Drain),
        ControlMethod::Remove => serde_json::from_str(text).map(ControlCall::Remove),
        ControlMethod::Shutdown => serde_json::from_str(text).map(ControlCall::Shutdown),
        ControlMethod::Exec => serde_json::from_str(text).map(ControlCall::Exec),
        ControlMethod::ExecStatus => serde_json::from_str(text).map(ControlCall::ExecStatus),
        ControlMethod::ExecOutput => serde_json::from_str(text).map(ControlCall::ExecOutput),
        ControlMethod::WorkloadStop => serde_json::from_str(text).map(ControlCall::WorkloadStop),
    }
    .map_err(|error| refuse(ProtocolCode::InvalidParams, &error.to_string()))?;
    if let (LaneState::Assigned(lane), ControlCall::Hello(hello)) = (state, &call)
        && hello.lane != lane
    {
        return Err(refuse(
            ProtocolCode::InvalidParams,
            "hello cannot change the assigned lane",
        ));
    }
    Ok(InboundRequest { id, call })
}

/// Writes one compact request without a newline. Returns the shared encoder's error,
/// including a serialization failure in a caller-built `UniverseOp`.
pub fn encode_control_request(id: u64, call: &ControlCall) -> Result<Vec<u8>, EncodeError> {
    let method = call.method().name();
    match call {
        ControlCall::Hello(params) => encode_request(id, method, params),
        ControlCall::Observe(params) => encode_request(id, method, params),
        ControlCall::Install(params) => encode_request(id, method, params),
        ControlCall::Activate(params) | ControlCall::Drain(params) => {
            encode_request(id, method, params)
        }
        ControlCall::Remove(params) => encode_request(id, method, params),
        ControlCall::Shutdown(params) => encode_request(id, method, params),
        ControlCall::Exec(params) => encode_request(id, method, params),
        ControlCall::ExecStatus(params) => encode_request(id, method, params),
        ControlCall::ExecOutput(params) => encode_request(id, method, params),
        ControlCall::WorkloadStop(params) => encode_request(id, method, params),
    }
}

/// The original boot and the protected guest policy's digest, returned by hello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct HelloResult {
    pub boot: BootToken,
    pub policy: Digest,
}

/// Acknowledges that this boot installed the selected grant with admission closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct InstalledResult {
    pub boot: BootToken,
    pub grant: GrantId,
    pub installed: Yes,
}

/// Acknowledges that this boot activated the selected grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ActiveResult {
    pub boot: BootToken,
    pub grant: GrantId,
    pub active: Yes,
}

/// Acknowledges that this boot finished waiting for the selected grant's pending requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct DrainedResult {
    pub boot: BootToken,
    pub grant: GrantId,
    pub drained: Yes,
}

/// Acknowledges that this boot removed the selected grant's guest attachments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct RemovedResult {
    pub boot: BootToken,
    pub grant: GrantId,
    pub guest_removed: Yes,
}

/// Acknowledges that this boot accepted shutdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct ShutdownResult {
    pub boot: BootToken,
    pub shutdown_requested: Yes,
}

object_serde!(HelloResult through HelloResult, InstalledResult through InstalledResult,
    ActiveResult through ActiveResult, DrainedResult through DrainedResult,
    RemovedResult through RemovedResult, ShutdownResult through ShutdownResult);

/// A control success without its envelope. Observe is a page whose assembler checks
/// the account; acknowledgements alone do not prove that a capability is effective.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlResult {
    Hello(HelloResult),
    Observe(ObservationPage),
    Installed(InstalledResult),
    Active(ActiveResult),
    Drained(DrainedResult),
    Removed(RemovedResult),
    Shutdown(ShutdownResult),
    Exec(ExecResult),
    ExecStatus(ExecStatus),
    ExecOutput(ExecOutputResult),
    WorkloadStop(WorkloadStopResult),
}

/// Encodes the method's result as a compact success reply with no newline.
/// Echoes `id` unchanged and propagates the shared encoder's serialization errors.
pub fn encode_control_result(
    id: &RequestId,
    result: &ControlResult,
) -> Result<Vec<u8>, EncodeError> {
    match result {
        ControlResult::Hello(result) => encode_success(id, result),
        ControlResult::Observe(result) => encode_success(id, result),
        ControlResult::Installed(result) => encode_success(id, result),
        ControlResult::Active(result) => encode_success(id, result),
        ControlResult::Drained(result) => encode_success(id, result),
        ControlResult::Removed(result) => encode_success(id, result),
        ControlResult::Shutdown(result) => encode_success(id, result),
        ControlResult::Exec(result) => encode_success(id, result),
        ControlResult::ExecStatus(result) => encode_success(id, result),
        ControlResult::ExecOutput(result) => encode_success(id, result),
        ControlResult::WorkloadStop(result) => encode_success(id, result),
    }
}

/// Decodes the expected call's closed result, checking its boot when supplied and
/// its grant against the call. Pass no boot only for the first hello. Observe pages
/// are returned for the caller's assembler to check. The reader bounds the frame.
pub fn decode_control_reply(
    call: &ControlCall,
    expected_id: u64,
    boot: Option<&BootToken>,
    line: &[u8],
) -> Result<ControlResult, ReplyFault> {
    let raw = split_reply(expected_id, line)?;
    let text = raw.get();
    let result = match call {
        ControlCall::Hello(_) => serde_json::from_str(text).map(ControlResult::Hello),
        ControlCall::Observe(_) => serde_json::from_str(text).map(ControlResult::Observe),
        ControlCall::Install(_) => serde_json::from_str(text).map(ControlResult::Installed),
        ControlCall::Activate(_) => serde_json::from_str(text).map(ControlResult::Active),
        ControlCall::Drain(_) => serde_json::from_str(text).map(ControlResult::Drained),
        ControlCall::Remove(_) => serde_json::from_str(text).map(ControlResult::Removed),
        ControlCall::Shutdown(_) => serde_json::from_str(text).map(ControlResult::Shutdown),
        ControlCall::Exec(_) => serde_json::from_str(text).map(ControlResult::Exec),
        ControlCall::ExecStatus(_) => serde_json::from_str(text).map(ControlResult::ExecStatus),
        ControlCall::ExecOutput(_) => serde_json::from_str(text).map(ControlResult::ExecOutput),
        ControlCall::WorkloadStop(_) => serde_json::from_str(text).map(ControlResult::WorkloadStop),
    }
    .map_err(|error| ReplyFault::Result {
        detail: cut(&error.to_string()),
    })?;
    let (found_boot, found_grant) = match &result {
        ControlResult::Hello(r) => (r.boot, None),
        ControlResult::Observe(_) => return Ok(result),
        ControlResult::Installed(r) => (r.boot, Some(r.grant)),
        ControlResult::Active(r) => (r.boot, Some(r.grant)),
        ControlResult::Drained(r) => (r.boot, Some(r.grant)),
        ControlResult::Removed(r) => (r.boot, Some(r.grant)),
        ControlResult::Shutdown(r) => (r.boot, None),
        ControlResult::Exec(r) => (r.boot, None),
        ControlResult::ExecStatus(r) => (r.boot(), None),
        ControlResult::ExecOutput(r) => (r.boot, None),
        ControlResult::WorkloadStop(r) => (r.boot, None),
    };
    if let Some(expected) = boot
        && *expected != found_boot
    {
        return Err(ReplyFault::BootChanged {
            expected: *expected,
            found: found_boot,
        });
    }
    if let Some(expected) = call.grant()
        && Some(expected) != found_grant
    {
        return Err(ReplyFault::GrantMismatch {
            expected,
            found: found_grant.expect("grant result"),
        });
    }
    let expected_exec = match call {
        ControlCall::ExecStatus(p) => Some(p.exec_id),
        ControlCall::ExecOutput(p) => Some(p.exec_id),
        _ => None,
    };
    let found_exec = match &result {
        ControlResult::ExecStatus(r) => Some(r.exec_id()),
        ControlResult::ExecOutput(r) => Some(r.exec_id),
        _ => None,
    };
    if let Some(expected) = expected_exec
        && Some(expected) != found_exec
    {
        return Err(ReplyFault::ExecMismatch {
            expected,
            found: found_exec.expect("exec result"),
        });
    }
    Ok(result)
}
