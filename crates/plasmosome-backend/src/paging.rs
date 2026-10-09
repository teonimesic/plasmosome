use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use serde::de::value::MapAccessDeserializer;
use serde::de::{DeserializeOwned, MapAccess, Visitor};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageFields {
    snapshot: u64,
    offset: u64,
    total: u64,
    sha256: Digest,
    bytes: Vec<u8>,
    complete: bool,
}

struct PageObject;

impl<'de> Visitor<'de> for PageObject {
    type Value = ObservationPage;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an observation page object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<ObservationPage, A::Error> {
        let fields = PageFields::deserialize(MapAccessDeserializer::new(map))?;
        Ok(ObservationPage {
            snapshot: fields.snapshot,
            offset: fields.offset,
            total: fields.total,
            sha256: fields.sha256,
            bytes: fields.bytes,
            complete: fields.complete,
        })
    }
}

impl<'de> Deserialize<'de> for ObservationPage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ObservationPage, D::Error> {
        deserializer.deserialize_map(PageObject)
    }
}

impl ObservationPage {
    /// Checks the rules spec 001 §4.1 sets on one page and returns the first one broken, in this
    /// order: a zero snapshot, a zero total, no bytes, more than [`MAX_PAGE_BYTES`], an end past
    /// `u64::MAX`, an end past `total`, and a `complete` flag that differs from whether the end
    /// equals `total`. It cannot check the hash; only the assembled account can.
    pub fn check(&self) -> Result<(), PageFault> {
        let len = self.bytes.len();
        if self.snapshot == 0 {
            return Err(PageFault::ZeroSnapshot);
        }
        if self.total == 0 {
            return Err(PageFault::ZeroTotal);
        }
        if len == 0 {
            return Err(PageFault::EmptyBytes);
        }
        if len > MAX_PAGE_BYTES {
            return Err(PageFault::TooManyBytes { len });
        }
        let offset = self.offset;
        let end = offset
            .checked_add(len as u64)
            .ok_or(PageFault::OffsetOverflow { offset, len })?;
        if end > self.total {
            return Err(PageFault::BeyondTotal {
                end,
                total: self.total,
            });
        }
        let actual = end == self.total;
        if self.complete != actual {
            return Err(PageFault::CompleteFlag {
                stated: self.complete,
                actual,
            });
        }
        Ok(())
    }
}

/// The first page rule an [`ObservationPage`] breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFault {
    ZeroSnapshot,
    ZeroTotal,
    EmptyBytes,
    TooManyBytes {
        len: usize,
    },
    OffsetOverflow {
        offset: u64,
        len: usize,
    },
    BeyondTotal {
        end: u64,
        total: u64,
    },
    /// `stated` is the page's flag; `actual` is whether the page ends at `total`.
    CompleteFlag {
        stated: bool,
        actual: bool,
    },
}

/// Which page to ask for. Zero/zero starts a capture; any other cursor names a byte offset in
/// the capture the responder holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    pub snapshot: u64,
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
        *self == PageCursor::START
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
        let bytes = serde_json::to_vec(value).map_err(|error| PagingError::Encode {
            detail: error.to_string(),
        })?;
        let sha256 = Digest::of(&bytes);
        Ok(FrozenAccount {
            bytes: bytes.into(),
            sha256,
        })
    }

    /// The account's length in bytes, the `total` of each of its pages; at least 1.
    pub fn total(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// The SHA-256 of [`FrozenAccount::bytes`].
    pub fn sha256(&self) -> &Digest {
        &self.sha256
    }

    /// The compact JSON bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
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

impl<S> Capture<S> {
    fn page_at(&self, offset: u64, page_bytes: usize) -> ObservationPage {
        let account = self.account.bytes();
        let start = offset as usize;
        let end = account.len().min(start + page_bytes);
        ObservationPage {
            snapshot: self.snapshot,
            offset,
            total: self.account.total(),
            sha256: self.account.sha256,
            bytes: account[start..end].to_vec(),
            complete: end == account.len(),
        }
    }
}

impl<S: PartialEq> CaptureSlot<S> {
    /// A slot that serves pages of [`MAX_PAGE_BYTES`].
    pub fn new() -> CaptureSlot<S> {
        CaptureSlot {
            page_bytes: MAX_PAGE_BYTES,
            last_snapshot: 0,
            current: None,
        }
    }

    /// A slot that serves pages of `page_bytes`, which must be 1 to [`MAX_PAGE_BYTES`];
    /// anything else is `PagingError::BadPageSize`.
    pub fn with_page_bytes(page_bytes: usize) -> Result<CaptureSlot<S>, PagingError> {
        if !(1..=MAX_PAGE_BYTES).contains(&page_bytes) {
            return Err(PagingError::BadPageSize { page_bytes });
        }
        Ok(CaptureSlot {
            page_bytes,
            ..CaptureSlot::new()
        })
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
        self.current = None;
        if now() >= expires {
            return Err(CaptureError::Paging(PagingError::Expired));
        }
        let account = collect().map_err(CaptureError::Collect)?;
        if now() >= expires {
            return Err(CaptureError::Paging(PagingError::Expired));
        }
        let snapshot = self
            .last_snapshot
            .checked_add(1)
            .ok_or(CaptureError::Paging(PagingError::SnapshotsExhausted))?;
        self.last_snapshot = snapshot;
        let capture = Capture {
            snapshot,
            scope,
            account,
            expires,
        };
        let first = capture.page_at(0, self.page_bytes);
        self.current = Some(capture);
        Ok(first)
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
        let PageCursor { snapshot, offset } = cursor;
        if snapshot == 0 {
            return Err(PagingError::InvalidCursor { snapshot, offset });
        }
        let Some(capture) = self
            .current
            .as_mut()
            .filter(|held| held.snapshot == snapshot)
        else {
            return Err(PagingError::UnknownSnapshot { snapshot });
        };
        if capture.scope != *scope {
            return Err(PagingError::ScopeChanged { snapshot });
        }
        if now >= capture.expires {
            self.current = None;
            return Err(PagingError::Expired);
        }
        let total = capture.account.total();
        if offset >= total {
            return Err(PagingError::OffsetOutOfRange { offset, total });
        }
        capture.expires = capture.expires.min(request_expires);
        Ok(capture.page_at(offset, self.page_bytes))
    }

    /// Drops the capture if `now` is at or past its deadline. Call it while no request arrives,
    /// so retained bytes do not outlive the deadline.
    pub fn expire(&mut self, now: Instant) {
        if self
            .current
            .as_ref()
            .is_some_and(|held| now >= held.expires)
        {
            self.current = None;
        }
    }

    /// Drops the capture, if any. Snapshot IDs keep counting from where they were.
    pub fn discard(&mut self) {
        self.current = None;
    }

    /// True while a capture is held.
    pub fn is_holding(&self) -> bool {
        self.current.is_some()
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
    InvalidCursor {
        snapshot: u64,
        offset: u64,
    },
    UnknownSnapshot {
        snapshot: u64,
    },
    ScopeChanged {
        snapshot: u64,
    },
    OffsetOutOfRange {
        offset: u64,
        total: u64,
    },
    /// The capture's deadline passed; its bytes are gone.
    Expired,
    SnapshotsExhausted,
    BadPageSize {
        page_bytes: usize,
    },
    /// The account could not be encoded as JSON.
    Encode {
        detail: String,
    },
}

impl PagingError {
    /// The JSON-RPC error code spec 001 §4.1 assigns: `-32602` when the request named a page that
    /// does not exist, and `-32603` when the responder failed or the capture expired.
    pub fn code(&self) -> i64 {
        match self {
            PagingError::InvalidCursor { .. }
            | PagingError::UnknownSnapshot { .. }
            | PagingError::ScopeChanged { .. }
            | PagingError::OffsetOutOfRange { .. } => -32602,
            PagingError::Expired
            | PagingError::SnapshotsExhausted
            | PagingError::BadPageSize { .. }
            | PagingError::Encode { .. } => -32603,
        }
    }
}

/// Why [`CaptureSlot::capture`] returned no page: a paging rule, or the caller's collection.
#[derive(Debug)]
pub enum CaptureError<E> {
    Paging(PagingError),
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
        PageAssembler {
            started: None,
            buffer: Vec::new(),
        }
    }

    fn received(&self) -> u64 {
        self.buffer.len() as u64
    }

    /// The cursor to request next: zero/zero before any page, then the capture's snapshot at the
    /// first missing byte, and `None` once the completing page was accepted.
    pub fn next(&self) -> Option<PageCursor> {
        match &self.started {
            None => Some(PageCursor::START),
            Some(started) if started.complete => None,
            Some(started) => Some(PageCursor {
                snapshot: started.snapshot,
                offset: self.received(),
            }),
        }
    }

    /// Checks `page` with [`ObservationPage::check`] and against the pages before it, then keeps
    /// its bytes. The first page must start at offset 0. Each later page must carry the first
    /// page's snapshot, total and hash, start at the first missing byte, and not follow the
    /// completing page. Memory grows only with the bytes received, never with a stated total.
    pub fn accept(&mut self, page: ObservationPage) -> Result<(), AssemblyFault> {
        page.check().map_err(AssemblyFault::Page)?;
        match &self.started {
            None if page.offset != 0 => {
                return Err(AssemblyFault::FirstPageNotAtZero {
                    offset: page.offset,
                });
            }
            None => {}
            Some(started) => self.follows(started, &page)?,
        }
        self.buffer.extend_from_slice(&page.bytes);
        self.started = Some(Started {
            snapshot: page.snapshot,
            total: page.total,
            sha256: page.sha256,
            complete: page.complete,
        });
        Ok(())
    }

    fn follows(&self, started: &Started, page: &ObservationPage) -> Result<(), AssemblyFault> {
        if started.complete {
            return Err(AssemblyFault::AfterComplete);
        }
        if page.snapshot != started.snapshot {
            return Err(AssemblyFault::SnapshotChanged {
                expected: started.snapshot,
                found: page.snapshot,
            });
        }
        if page.total != started.total {
            return Err(AssemblyFault::TotalChanged {
                expected: started.total,
                found: page.total,
            });
        }
        if page.sha256 != started.sha256 {
            return Err(AssemblyFault::HashChanged);
        }
        if page.offset != self.received() {
            return Err(AssemblyFault::OutOfOrder {
                expected: self.received(),
                found: page.offset,
            });
        }
        Ok(())
    }

    /// Yields the account decoded as `T` once every byte arrived. Refuses missing bytes
    /// (`Incomplete`), bytes whose SHA-256 is not the stated hash (`HashMismatch`), leading or
    /// trailing whitespace (`Whitespace`) and content serde cannot decode as `T` (`Decode`).
    /// Serde checks shape only: validate the record's own rules before using it.
    pub fn finish<T: DeserializeOwned>(self) -> Result<T, AssemblyFault> {
        let received = self.received();
        let stated = match &self.started {
            Some(started) if received == started.total => started.sha256,
            started => {
                return Err(AssemblyFault::Incomplete {
                    received,
                    total: started.as_ref().map(|started| started.total),
                });
            }
        };
        let computed = Digest::of(&self.buffer);
        if computed != stated {
            return Err(AssemblyFault::HashMismatch { stated, computed });
        }
        let surrounded = [self.buffer.first(), self.buffer.last()];
        if surrounded
            .into_iter()
            .flatten()
            .any(u8::is_ascii_whitespace)
        {
            return Err(AssemblyFault::Whitespace);
        }
        serde_json::from_slice(&self.buffer).map_err(|error| AssemblyFault::Decode {
            detail: error.to_string(),
        })
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
    Page(PageFault),
    FirstPageNotAtZero {
        offset: u64,
    },
    SnapshotChanged {
        expected: u64,
        found: u64,
    },
    TotalChanged {
        expected: u64,
        found: u64,
    },
    HashChanged,
    /// A page started at `found`, not at the first missing byte `expected`.
    OutOfOrder {
        expected: u64,
        found: u64,
    },
    AfterComplete,
    /// Only `received` bytes arrived, of `total` if any page stated one.
    Incomplete {
        received: u64,
        total: Option<u64>,
    },
    HashMismatch {
        stated: Digest,
        computed: Digest,
    },
    /// The account starts or ends with whitespace, so it is not compact JSON.
    Whitespace,
    /// The account is not a `T`.
    Decode {
        detail: String,
    },
}

impl fmt::Display for PageFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageFault::ZeroSnapshot => {
                f.write_str("the page names snapshot 0, which only starts a capture")
            }
            PageFault::ZeroTotal => f.write_str("the page states an empty account"),
            PageFault::EmptyBytes => f.write_str("the page carries no bytes"),
            PageFault::TooManyBytes { len } => write!(
                f,
                "the page carries {len} bytes, more than {MAX_PAGE_BYTES}"
            ),
            PageFault::OffsetOverflow { offset, len } => {
                write!(f, "offset {offset} plus {len} bytes overflows a u64")
            }
            PageFault::BeyondTotal { end, total } => write!(
                f,
                "the page ends at byte {end}, past the account's {total} bytes"
            ),
            PageFault::CompleteFlag { stated, actual } => write!(
                f,
                "the page says complete is {stated}, but where it ends makes it {actual}"
            ),
        }
    }
}

impl fmt::Display for PagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PagingError::InvalidCursor { snapshot, offset } => write!(
                f,
                "snapshot {snapshot} with offset {offset} names no page; zero/zero starts a capture"
            ),
            PagingError::UnknownSnapshot { snapshot } => write!(
                f,
                "snapshot {snapshot} is not the capture this connection holds"
            ),
            PagingError::ScopeChanged { snapshot } => {
                write!(f, "snapshot {snapshot} was captured for another scope")
            }
            PagingError::OffsetOutOfRange { offset, total } => {
                write!(f, "offset {offset} is outside the {total}-byte account")
            }
            PagingError::Expired => f.write_str("the capture's deadline has passed"),
            PagingError::SnapshotsExhausted => {
                f.write_str("this connection has issued every snapshot ID")
            }
            PagingError::BadPageSize { page_bytes } => write!(
                f,
                "a page of {page_bytes} bytes is outside 1 to {MAX_PAGE_BYTES}"
            ),
            PagingError::Encode { detail } => {
                write!(f, "the account cannot be encoded as JSON: {detail}")
            }
        }
    }
}

impl<E: fmt::Display> fmt::Display for CaptureError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::Paging(error) => error.fmt(f),
            CaptureError::Collect(error) => write!(f, "collecting the account failed: {error}"),
        }
    }
}

impl fmt::Display for AssemblyFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssemblyFault::Page(fault) => fault.fmt(f),
            AssemblyFault::FirstPageNotAtZero { offset } => {
                write!(f, "the first page starts at offset {offset}, not 0")
            }
            AssemblyFault::SnapshotChanged { expected, found } => write!(
                f,
                "a page from snapshot {found} arrived while assembling snapshot {expected}"
            ),
            AssemblyFault::TotalChanged { expected, found } => write!(
                f,
                "a page states a {found}-byte account, not {expected} bytes"
            ),
            AssemblyFault::HashChanged => {
                f.write_str("a page states a different account hash than the first page")
            }
            AssemblyFault::OutOfOrder { expected, found } => write!(
                f,
                "a page starts at offset {found}, not at the first missing byte {expected}"
            ),
            AssemblyFault::AfterComplete => f.write_str("a page arrived after the completing page"),
            AssemblyFault::Incomplete {
                received,
                total: Some(total),
            } => write!(f, "only {received} of {total} bytes arrived"),
            AssemblyFault::Incomplete { total: None, .. } => f.write_str("no page arrived"),
            AssemblyFault::HashMismatch { stated, computed } => write!(
                f,
                "the account hashes to {computed}, not the stated {stated}"
            ),
            AssemblyFault::Whitespace => {
                f.write_str("the account starts or ends with whitespace, so it is not compact JSON")
            }
            AssemblyFault::Decode { detail } => {
                write!(f, "the account is not the expected record: {detail}")
            }
        }
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
