use super::digest::Digest;
use super::endpoints::RuntimeListener;
use super::hello::{BootToken, HelloRefusal};
use crate::readiness::{Interest, ProbeBudget};
use crate::vmm::{SupervisionError, VmmState};
use plasmosome_core::private_socket::PrivateSocketError;
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// The host ends of the shim's three streams after a complete handshake: the
/// normal lane, the withdrawal lane and the 4091 data association, with the
/// boot token both hellos returned, the guest policy they named and the peer
/// process the kernel reported for each stream.
///
/// It proves both hellos and the data association. Spec 001 §4.2 asks more
/// before a cell is ready: the workload namespaces and the workload control
/// group, which this handshake does not observe.
///
/// Keep it for as long as the cell lives. Dropping it closes all three host
/// ends, and the shim then sees each lane closed.
#[derive(Debug)]
pub struct Association {
    /// The boot token both hellos returned.
    pub boot: BootToken,
    /// The guest policy digest both hellos named, equal to the one expected.
    pub policy: Digest,
    /// The helper's process id, which every stream's peer read matched.
    pub peer: libc::pid_t,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "held so the lane stays open until the drop")
    )]
    normal: UnixStream,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "held so the lane stays open until the drop")
    )]
    withdrawal: UnixStream,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "held so the association stays open until the drop")
    )]
    data: UnixStream,
}

/// Which step of the handshake a failure happened at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Accepting the first 4090 connection.
    NormalAccept,
    /// Sending the normal lane's hello and reading its reply.
    NormalReply,
    /// Accepting the second 4090 connection.
    WithdrawalAccept,
    /// Sending the withdrawal lane's hello and reading its reply.
    WithdrawalReply,
    /// Accepting the 4091 data connection.
    DataAccept,
}

/// What the caller's liveness check says about the helper.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Liveness {
    /// The helper has not ended.
    Running,
    /// The helper ended in this state.
    Ended(VmmState),
    /// The helper's state could not be read.
    Unknown(SupervisionError),
}

/// Why the handshake did not complete. Every stream it had accepted is closed
/// when it returns one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandshakeFailure {
    /// Shutdown was asked for. Cancellation wins when the allowance is spent
    /// at the same check.
    Cancelled { stage: Stage },
    /// The budget's allowance was spent, or the step finished after it.
    TimedOut { stage: Stage },
    /// The liveness check said the helper ended.
    HelperEnded { stage: Stage, state: VmmState },
    /// The liveness check could not tell whether the helper runs.
    LivenessUnknown {
        stage: Stage,
        error: SupervisionError,
    },
    /// A stream's peer is process `found`, not the helper `expected`. A pid of
    /// 0 or below never matches.
    ForeignPeer {
        stage: Stage,
        expected: libc::pid_t,
        found: libc::pid_t,
    },
    /// The listener refused a peer, or the peer's process id could not be
    /// read: a peer of another uid, credentials the kernel would not give, or
    /// a stream that could not be set up.
    PeerRefused {
        stage: Stage,
        refusal: PrivateSocketError,
    },
    /// The hello reply was refused by the judge.
    Refused { stage: Stage, refusal: HelloRefusal },
    /// More than 1,048,576 bytes came before the reply's newline.
    FrameTooLarge { stage: Stage },
    /// Bytes followed the reply's newline in the same read.
    UnexpectedBytes { stage: Stage },
    /// The peer closed its end before the reply's newline.
    Closed { stage: Stage },
    /// An accept, read, write or wait failed with this errno.
    Io { stage: Stage, errno: i32 },
}

/// Runs the host side of the shim's handshake over the cell's two listeners,
/// and returns the [`Association`] once it is complete.
///
/// It accepts the first connection on `control` and checks that its peer is
/// `expected_peer`, the helper's pid. It sends the normal hello and judges the
/// reply against `expected_policy`. It accepts the second connection on
/// `control`, checks its peer and does the same for the withdrawal lane, whose
/// boot token must equal the normal lane's. Only then does it accept one
/// connection on `data` and check its peer. A data connection that came
/// earlier waits in that listener's queue until then.
///
/// Everything spends `budget`'s one allowance, and nothing renews it. Before
/// every wait it checks the budget, then calls `alive`; a helper that ended,
/// or a state it cannot read, fails at once. Waits last at most 25 ms of what
/// is left. A handshake that completes after the allowance is `TimedOut`.
///
/// A peer pid is read from the kernel when each stream is accepted, and again
/// after each hello reply is read. What a match proves differs by platform.
/// On Linux each read is `SO_PEERCRED`'s, the process that called `connect`,
/// so the reads after the replies add nothing. On Darwin each read is
/// `LOCAL_PEERPID`'s, the last process that created, connected or sent on the
/// peer's socket. A match at the accept shows only that the helper was the
/// last to use that socket, not that it connected; a match after a reply
/// shows that the helper was the last to send on it, so a reply sent by
/// another process fails. On neither platform does a pid show that the
/// process still runs.
///
/// Pass the listeners the cell bound before it launched the helper. On a
/// failure this closes what it accepted but not the listeners: a connection
/// still in a listener's queue closes when the caller closes the listener.
pub fn handshake(
    control: &RuntimeListener,
    data: &RuntimeListener,
    expected_peer: libc::pid_t,
    expected_policy: &Digest,
    budget: &ProbeBudget<'_>,
    alive: impl FnMut() -> Liveness,
) -> Result<Association, HandshakeFailure> {
    handshake_with(
        control,
        data,
        Expected {
            peer: expected_peer,
            policy: expected_policy,
        },
        budget,
        alive,
        Instant::now,
        crate::readiness::wait_fd,
    )
}

#[derive(Clone, Copy)]
struct Expected<'a> {
    peer: libc::pid_t,
    policy: &'a Digest,
}

fn handshake_with<A, N, W>(
    control: &RuntimeListener,
    data: &RuntimeListener,
    expected: Expected<'_>,
    budget: &ProbeBudget<'_>,
    alive: A,
    now: N,
    wait: W,
) -> Result<Association, HandshakeFailure>
where
    A: FnMut() -> Liveness,
    N: Fn() -> Instant,
    W: FnMut(RawFd, Interest, Duration) -> std::io::Result<bool>,
{
    let _ = (control, data, expected, budget, alive, now, wait);
    todo!()
}

fn authentic(expected: libc::pid_t, found: libc::pid_t) -> bool {
    let _ = (expected, found);
    todo!()
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::fmt::Display for HandshakeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for HandshakeFailure {}
