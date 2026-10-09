use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::digest::Digest;

/// The most account bytes one page carries (spec 001 §4.1).
pub const MAX_PAGE_BYTES: usize = 65_536;

/// The longest line a peer accepts, in bytes before its newline (spec 001 §1). The largest legal
/// page fits in it, inside a response with the largest request ID.
pub const MAX_FRAME_BYTES: usize = 1_048_576;

/// One page of a frozen account: the wire result of every paged observation method.
///
/// It decodes only from a JSON object that holds each of these six fields exactly once. Decoding
/// checks the shape only: pass the page to a [`PageAssembler`], or call
/// [`ObservationPage::check`], before trusting any field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservationPage {
    /// The capture this page belongs to. Never zero.
    pub snapshot: u64,
    /// Where `bytes` starts in the account.
    pub offset: u64,
    /// The account's length in bytes. Never zero.
    pub total: u64,
    /// The SHA-256 of the whole account, not of this page.
    pub sha256: Digest,
    /// 1 to [`MAX_PAGE_BYTES`] account bytes, starting at `offset`.
    pub bytes: Vec<u8>,
    /// True exactly when this page ends at `total`.
    pub complete: bool,
}

impl<'de> Deserialize<'de> for ObservationPage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ObservationPage, D::Error> {
        let _ = deserializer;
        todo!()
    }
}

impl ObservationPage {
    /// Checks the rules spec 001 §4.1 sets on one page and returns the first one broken, in this
    /// order: a zero snapshot, a zero total, no bytes, more than [`MAX_PAGE_BYTES`], an end past
    /// `u64::MAX`, an end past `total`, and a `complete` flag that differs from whether the end
    /// equals `total`. It cannot check the hash; only the assembled account can.
    pub fn check(&self) -> Result<(), PageFault> {
        todo!()
    }
}

/// The first page rule an [`ObservationPage`] breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFault {
    /// The snapshot is zero, which only a request that starts a capture carries.
    ZeroSnapshot,
    /// The total is zero; an account is never empty.
    ZeroTotal,
    /// The page carries no bytes.
    EmptyBytes,
    /// The page carries `len` bytes, more than [`MAX_PAGE_BYTES`].
    TooManyBytes { len: usize },
    /// `offset` plus `len` does not fit in a u64.
    OffsetOverflow { offset: u64, len: usize },
    /// The page ends at byte `end`, past the account's `total`.
    BeyondTotal { end: u64, total: u64 },
    /// The page says `complete` is `stated`, but where it ends makes it `actual`.
    CompleteFlag { stated: bool, actual: bool },
}

/// Which page to ask for. Zero/zero starts a capture; any other cursor names a byte offset in
/// the capture the responder holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    /// The capture's ID, or zero to start one.
    pub snapshot: u64,
    /// The first account byte wanted.
    pub offset: u64,
}

impl PageCursor {
    /// The zero/zero cursor that starts a new capture.
    pub const START: PageCursor = PageCursor {
        snapshot: 0,
        offset: 0,
    };

    /// True for zero/zero, which a responder answers with [`CaptureSlot::capture`].
    pub fn is_start(&self) -> bool {
        todo!()
    }
}

/// One logical result, encoded once as compact JSON and hashed. Its bytes never change, and
/// clones share them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenAccount {
    bytes: Arc<[u8]>,
    sha256: Digest,
}

impl FrozenAccount {
    /// Encodes `value` with serde_json's compact writer, without a trailing newline, and hashes
    /// the bytes. Freeze a value only after collecting and validating it. A value JSON cannot
    /// hold, such as a map with non-string keys, is refused with `PagingError::Encode`.
    pub fn freeze<T: Serialize>(value: &T) -> Result<FrozenAccount, PagingError> {
        let _ = value;
        todo!()
    }

    /// The account's length in bytes, the `total` of each of its pages; at least 1.
    pub fn total(&self) -> u64 {
        todo!()
    }

    /// The SHA-256 of [`FrozenAccount::bytes`].
    pub fn sha256(&self) -> &Digest {
        todo!()
    }

    /// The compact JSON bytes.
    pub fn bytes(&self) -> &[u8] {
        todo!()
    }
}

/// The responder's half of one paged method on one connection: at most one frozen capture,
/// served in pages until its deadline.
///
/// Keep one slot per connection and per paged method, and drop it when the connection closes,
/// which discards the capture. `S` is the capture's scope, such as the resolved cell: a page
/// request with a different scope is refused. The slot reads no clock; the caller passes `now`.
pub struct CaptureSlot<S> {
    page_bytes: usize,
    last_snapshot: u64,
    current: Option<Capture<S>>,
}

struct Capture<S> {
    snapshot: u64,
    scope: S,
    account: FrozenAccount,
    expires: Instant,
}

impl<S: PartialEq> CaptureSlot<S> {
    /// A slot that serves pages of [`MAX_PAGE_BYTES`].
    pub fn new() -> CaptureSlot<S> {
        todo!()
    }

    /// A slot that serves pages of `page_bytes`, which must be 1 to [`MAX_PAGE_BYTES`];
    /// anything else is `PagingError::BadPageSize`.
    pub fn with_page_bytes(page_bytes: usize) -> Result<CaptureSlot<S>, PagingError> {
        let _ = page_bytes;
        todo!()
    }

    /// Answers a zero/zero request: drops any capture this slot holds, calls `collect` once,
    /// keeps its account under the next snapshot ID and returns the page at offset 0.
    ///
    /// `expires` is the first request's deadline, and it bounds the capture and every page.
    /// `now` is read before and after `collect`; if either reading is at or past `expires`, the
    /// result is `Expired` and nothing is kept. A `collect` error comes back as
    /// `CaptureError::Collect`. The previous capture is gone whatever the outcome. Snapshot IDs
    /// start at 1 and this slot never reuses one, not even after [`CaptureSlot::discard`].
    pub fn capture<E>(
        &mut self,
        scope: S,
        expires: Instant,
        now: &dyn Fn() -> Instant,
        collect: impl FnOnce() -> Result<FrozenAccount, E>,
    ) -> Result<ObservationPage, CaptureError<E>> {
        let _ = (scope, expires, now, collect);
        todo!()
    }

    /// Answers a request for a page of the capture this slot holds.
    ///
    /// Refuses a zero snapshot (`InvalidCursor`), a snapshot other than the one held
    /// (`UnknownSnapshot`), a `scope` other than the capture's (`ScopeChanged`), a capture whose
    /// deadline is at or before `now` (`Expired`, which also drops it) and an offset at or past
    /// the account's end (`OffsetOutOfRange`). Otherwise it lowers the capture's deadline to
    /// `request_expires` when that is earlier, never raising it, and returns up to the slot's
    /// page size of bytes from `cursor.offset`. The same offset always returns the same page.
    pub fn page(
        &mut self,
        scope: &S,
        cursor: PageCursor,
        request_expires: Instant,
        now: Instant,
    ) -> Result<ObservationPage, PagingError> {
        let _ = (scope, cursor, request_expires, now);
        todo!()
    }

    /// Drops the capture if `now` is at or past its deadline. Call it while no request arrives,
    /// so retained bytes do not outlive the deadline.
    pub fn expire(&mut self, now: Instant) {
        let _ = now;
        todo!()
    }

    /// Drops the capture, if any. Snapshot IDs keep counting from where they were.
    pub fn discard(&mut self) {
        todo!()
    }

    /// True while a capture is held.
    pub fn is_holding(&self) -> bool {
        todo!()
    }
}

impl<S: PartialEq> Default for CaptureSlot<S> {
    fn default() -> CaptureSlot<S> {
        CaptureSlot::new()
    }
}

/// Why a responder refused a capture or a page. Answer with [`PagingError::code`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PagingError {
    /// A page request named snapshot zero; only zero/zero, which starts a capture, may.
    InvalidCursor { snapshot: u64, offset: u64 },
    /// The slot holds no capture with this ID.
    UnknownSnapshot { snapshot: u64 },
    /// The request's scope differs from the one the capture was taken for.
    ScopeChanged { snapshot: u64 },
    /// The offset is at or past the end of the `total`-byte account.
    OffsetOutOfRange { offset: u64, total: u64 },
    /// The capture's deadline passed; its bytes are gone.
    Expired,
    /// The slot has issued every snapshot ID a u64 holds.
    SnapshotsExhausted,
    /// A page size outside 1 to [`MAX_PAGE_BYTES`].
    BadPageSize { page_bytes: usize },
    /// The account could not be encoded as JSON.
    Encode { detail: String },
}

impl PagingError {
    /// The JSON-RPC error code spec 001 §4.1 assigns: `-32602` when the request named a page that
    /// does not exist, and `-32603` when the responder failed or the capture expired.
    pub fn code(&self) -> i64 {
        todo!()
    }
}

/// Why [`CaptureSlot::capture`] returned no page: a paging rule, or the caller's collection.
#[derive(Debug)]
pub enum CaptureError<E> {
    /// A paging rule refused the capture.
    Paging(PagingError),
    /// `collect` failed with this error.
    Collect(E),
}

/// The caller's half: collects the pages of one capture, and yields the decoded account only when
/// every byte arrived and matches the stated length and hash.
///
/// Ask [`PageAssembler::next`] which cursor to request, pass each reply to
/// [`PageAssembler::accept`] and call [`PageAssembler::finish`] once `next` is `None`. A refused
/// page changes nothing, and spec 001 makes any refusal fail the whole transfer.
pub struct PageAssembler {
    started: Option<Started>,
    buffer: Vec<u8>,
}

struct Started {
    snapshot: u64,
    total: u64,
    sha256: Digest,
    complete: bool,
}

impl PageAssembler {
    /// An assembler that has received nothing.
    pub fn new() -> PageAssembler {
        todo!()
    }

    /// The cursor to request next: zero/zero before any page, then the capture's snapshot at the
    /// first missing byte, and `None` once the completing page was accepted.
    pub fn next(&self) -> Option<PageCursor> {
        todo!()
    }

    /// Checks `page` with [`ObservationPage::check`] and against the pages before it, then keeps
    /// its bytes. The first page must start at offset 0. Each later page must carry the first
    /// page's snapshot, total and hash, start at the first missing byte, and not follow the
    /// completing page. Memory grows only with the bytes received, never with a stated total.
    pub fn accept(&mut self, page: ObservationPage) -> Result<(), AssemblyFault> {
        let _ = page;
        todo!()
    }

    /// Yields the account decoded as `T` once every byte arrived. Refuses missing bytes
    /// (`Incomplete`), bytes whose SHA-256 is not the stated hash (`HashMismatch`), leading or
    /// trailing whitespace (`Whitespace`) and content serde cannot decode as `T` (`Decode`).
    /// Serde checks shape only: validate the record's own rules before using it.
    pub fn finish<T: DeserializeOwned>(self) -> Result<T, AssemblyFault> {
        todo!()
    }
}

impl Default for PageAssembler {
    fn default() -> PageAssembler {
        PageAssembler::new()
    }
}

/// Why a [`PageAssembler`] refused a page or a finished account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyFault {
    /// The page broke a rule of its own.
    Page(PageFault),
    /// The first page started at `offset`, not 0.
    FirstPageNotAtZero { offset: u64 },
    /// A page came from snapshot `found` while assembling snapshot `expected`.
    SnapshotChanged { expected: u64, found: u64 },
    /// A page stated a total of `found` bytes, not `expected`.
    TotalChanged { expected: u64, found: u64 },
    /// A page stated a different account hash than the first page.
    HashChanged,
    /// A page started at `found`, not at the first missing byte `expected`.
    OutOfOrder { expected: u64, found: u64 },
    /// A page arrived after the completing page.
    AfterComplete,
    /// Only `received` bytes arrived, of `total` if any page stated one.
    Incomplete { received: u64, total: Option<u64> },
    /// The assembled bytes hash to `computed`, not the `stated` hash.
    HashMismatch { stated: Digest, computed: Digest },
    /// The account starts or ends with whitespace, so it is not compact JSON.
    Whitespace,
    /// The account is not a `T`.
    Decode { detail: String },
}

impl fmt::Display for PageFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl fmt::Display for PagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl<E: fmt::Display> fmt::Display for CaptureError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl fmt::Display for AssemblyFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for PageFault {}

impl std::error::Error for PagingError {}

impl<E: std::error::Error> std::error::Error for CaptureError<E> {}

impl std::error::Error for AssemblyFault {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::time::Duration;

    #[test]
    fn snapshot_ids_run_out_instead_of_wrapping_to_zero() {
        let now = Instant::now();
        let expires = now + Duration::from_secs(1);
        let account = FrozenAccount::freeze(&1).expect("a number freezes");
        let mut slot = CaptureSlot::<()>::new();
        slot.last_snapshot = u64::MAX - 1;
        let last = slot
            .capture((), expires, &|| now, || {
                Ok::<_, Infallible>(account.clone())
            })
            .expect("the last snapshot ID is issued");
        assert_eq!(last.snapshot, u64::MAX);
        let refused = slot.capture((), expires, &|| now, || {
            Ok::<_, Infallible>(account.clone())
        });
        assert!(
            matches!(
                refused,
                Err(CaptureError::Paging(PagingError::SnapshotsExhausted))
            ),
            "{refused:?}"
        );
        assert!(!slot.is_holding());
    }
}
