use std::cell::Cell;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use plasmosome_backend::Digest;
use plasmosome_backend::paging::{
    AssemblyFault, CaptureError, CaptureSlot, FrozenAccount, MAX_FRAME_BYTES, MAX_PAGE_BYTES,
    ObservationPage, PageAssembler, PageCursor, PageFault, PagingError,
};
use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SCOPE: &str = "cell-1";
const SECOND: Duration = Duration::from_secs(1);

macro_rules! assert_says {
    ($($error:expr => $text:expr;)*) => {
        $(assert_eq!($error.to_string(), $text);)*
    };
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    a: u32,
}

fn account() -> FrozenAccount {
    FrozenAccount::freeze(&Account { a: 1234 }).expect("a small record freezes")
}

fn paged(page_bytes: usize) -> CaptureSlot<&'static str> {
    CaptureSlot::with_page_bytes(page_bytes).expect("a page size from 1 to 65,536 is accepted")
}

fn start(
    slot: &mut CaptureSlot<&'static str>,
    account: FrozenAccount,
    now: Instant,
    expires: Instant,
) -> ObservationPage {
    slot.capture(SCOPE, expires, &|| now, || Ok::<_, Infallible>(account))
        .expect("a capture inside its deadline succeeds")
}

fn cursor(snapshot: u64, offset: u64) -> PageCursor {
    PageCursor { snapshot, offset }
}

fn read(
    slot: &mut CaptureSlot<&'static str>,
    snapshot: u64,
    offset: u64,
    now: Instant,
) -> Result<ObservationPage, PagingError> {
    slot.page(&SCOPE, cursor(snapshot, offset), now + 60 * SECOND, now)
}

fn ten_bytes_in_pages_of_four() -> (CaptureSlot<&'static str>, Instant, Vec<ObservationPage>) {
    let t = Instant::now();
    let mut slot = paged(4);
    let mut pages = vec![start(&mut slot, account(), t, t + 10 * SECOND)];
    for offset in [4, 8] {
        pages.push(read(&mut slot, 1, offset, t).expect("an in-range page is served"));
    }
    (slot, t, pages)
}

fn page(offset: u64, len: usize, total: u64) -> ObservationPage {
    let complete = offset.checked_add(len as u64) == Some(total);
    let sha256 = Digest::of(b"account");
    let bytes = vec![b'x'; len];
    ObservationPage {
        snapshot: 1,
        offset,
        total,
        sha256,
        bytes,
        complete,
    }
}

fn pages_of(bytes: &[u8], page_bytes: usize) -> Vec<ObservationPage> {
    let (total, sha256) = (bytes.len() as u64, Digest::of(bytes));
    let mut offsets = (0..total).step_by(page_bytes);
    let pages = bytes.chunks(page_bytes).map(|chunk| {
        let offset = offsets.next().expect("one offset per chunk");
        let complete = offset + chunk.len() as u64 == total;
        let bytes = chunk.to_vec();
        ObservationPage {
            snapshot: 7,
            offset,
            total,
            sha256,
            bytes,
            complete,
        }
    });
    pages.collect()
}

fn joined(pages: &[ObservationPage]) -> Vec<u8> {
    pages.iter().flat_map(|page| page.bytes.clone()).collect()
}

fn assemble<T: DeserializeOwned>(pages: Vec<ObservationPage>) -> Result<T, AssemblyFault> {
    let mut assembler = PageAssembler::new();
    for page in pages {
        assembler.accept(page)?;
    }
    assembler.finish()
}

fn refused_after(accepted: &[ObservationPage], next: ObservationPage) -> AssemblyFault {
    let mut assembler = PageAssembler::new();
    for page in accepted {
        assembler
            .accept(page.clone())
            .expect("an earlier page is accepted");
    }
    let before = assembler.next();
    let fault = assembler.accept(next).expect_err("the page is refused");
    assert_eq!(assembler.next(), before, "a refused page changes nothing");
    fault
}

fn refusal(text: &str) -> String {
    serde_json::from_str::<ObservationPage>(text)
        .expect_err(text)
        .to_string()
}

#[test]
fn page_record_is_strict() {
    let valid = serde_json::to_value(page(0, 3, 3)).expect("a page encodes");
    let decoded: ObservationPage = serde_json::from_str(&valid.to_string()).expect("it decodes");
    assert_eq!(decoded, page(0, 3, 3));
    let mut unknown = valid.clone();
    unknown["extra"] = json!(true);
    assert!(refusal(&unknown.to_string()).contains("unknown field `extra`"));
    let fields = ["snapshot", "offset", "total", "sha256", "bytes", "complete"];
    for field in fields {
        let mut missing = valid.clone();
        missing
            .as_object_mut()
            .expect("a page is an object")
            .remove(field);
        let reason = refusal(&missing.to_string());
        assert!(
            reason.contains(&format!("missing field `{field}`")),
            "{reason}"
        );
    }
    let repeated = valid.to_string().replacen('{', "{\"snapshot\":1,", 1);
    assert!(refusal(&repeated).contains("duplicate field `snapshot`"));
    let mut wide = valid.clone();
    wide["bytes"] = json!([1, 256]);
    assert!(refusal(&wide.to_string()).contains("integer `256`"));
    let mut negative = valid.clone();
    negative["offset"] = json!(-1);
    assert!(refusal(&negative.to_string()).contains("integer `-1`"));
    let positional = Value::Array(fields.iter().map(|field| valid[*field].clone()).collect());
    assert!(refusal(&positional.to_string()).contains("invalid type: sequence"));
    let through_value = serde_json::from_value::<ObservationPage>(positional);
    let reason = through_value
        .expect_err("a positional array is not a page")
        .to_string();
    assert!(reason.contains("invalid type: sequence"), "{reason}");
}

#[test]
fn page_check_refuses_each_fault() {
    let mut zero_snapshot = page(0, 1, 1);
    zero_snapshot.snapshot = 0;
    assert_eq!(zero_snapshot.check(), Err(PageFault::ZeroSnapshot));
    assert_eq!(page(0, 1, 0).check(), Err(PageFault::ZeroTotal));
    assert_eq!(page(0, 0, 5).check(), Err(PageFault::EmptyBytes));
    let len = 65_537;
    assert_eq!(
        page(0, len, 70_000).check(),
        Err(PageFault::TooManyBytes { len })
    );
    let (offset, len) = (u64::MAX, 1);
    assert_eq!(
        page(offset, len, u64::MAX).check(),
        Err(PageFault::OffsetOverflow { offset, len })
    );
    assert_eq!(
        page(4, 4, 6).check(),
        Err(PageFault::BeyondTotal { end: 8, total: 6 })
    );
    let mut early = page(0, 4, 10);
    early.complete = true;
    let (stated, actual) = (true, false);
    assert_eq!(
        early.check(),
        Err(PageFault::CompleteFlag { stated, actual })
    );
    let mut late = page(6, 4, 10);
    late.complete = false;
    let (stated, actual) = (false, true);
    assert_eq!(
        late.check(),
        Err(PageFault::CompleteFlag { stated, actual })
    );
    assert_eq!(page(6, 4, 10).check(), Ok(()));
    assert_eq!(page(0, 4, 10).check(), Ok(()));
}

#[test]
fn page_of_65536_bytes_is_valid_and_65537_is_not() {
    assert_eq!(page(0, 65_536, 65_536).check(), Ok(()));
    assert_eq!(page(0, 65_536, 200_000).check(), Ok(()));
    let len = 65_537;
    assert_eq!(
        page(0, len, 65_537).check(),
        Err(PageFault::TooManyBytes { len })
    );
}

#[derive(Serialize)]
struct Response<'a> {
    id: u64,
    result: &'a ObservationPage,
}

#[test]
fn largest_page_fits_the_frame() {
    let mut largest = page(u64::MAX - 65_536, 65_536, u64::MAX);
    largest.snapshot = u64::MAX;
    largest.bytes.fill(255);
    assert_eq!(largest.check(), Ok(()));
    let line = serde_json::to_string(&Response {
        id: u64::MAX,
        result: &largest,
    })
    .expect("a page encodes");
    let prefix = "{\"id\":18446744073709551615,\"result\":{\"snapshot\":18446744073709551615,";
    assert!(line.starts_with(prefix), "{}", &line[..80]);
    assert!(line.len() <= MAX_FRAME_BYTES, "{} bytes", line.len());
    assert!(!line.contains(' ') && !line.contains('\n'));
    let result = &line["{\"id\":18446744073709551615,\"result\":".len()..line.len() - 1];
    let decoded: ObservationPage = serde_json::from_str(result).expect("the page decodes back");
    assert_eq!(decoded, largest);
}

#[test]
fn frame_bound_is_the_spec_001_line_limit() {
    assert_eq!(MAX_FRAME_BYTES, 1_048_576);
    assert_eq!(MAX_PAGE_BYTES, 65_536);
}

#[derive(Serialize)]
struct Sample {
    name: &'static str,
    values: Vec<u32>,
    nested: BTreeMap<&'static str, bool>,
}

#[test]
fn freeze_is_compact_and_hashed() {
    let nested = BTreeMap::from([("ok", true)]);
    let sample = Sample {
        name: "cell-1",
        values: vec![1, 2, 3],
        nested,
    };
    let frozen = FrozenAccount::freeze(&sample).expect("a record freezes");
    let expected: &[u8] = br#"{"name":"cell-1","values":[1,2,3],"nested":{"ok":true}}"#;
    assert_eq!(frozen.bytes(), expected);
    assert_eq!(frozen.total(), expected.len() as u64);
    assert_eq!(frozen.sha256(), &Digest::of(expected));
    assert_eq!(frozen.clone(), frozen);
}

#[test]
fn freeze_refuses_what_json_cannot_hold() {
    let map = BTreeMap::from([(vec![1u8], 1u8)]);
    let error = FrozenAccount::freeze(&map).expect_err("a map with non-string keys is not JSON");
    let reason = "key must be a string";
    assert!(matches!(&error, PagingError::Encode { detail } if detail.contains(reason)));
    assert_eq!(error.code(), -32603);
}

#[test]
fn capture_then_pages_reproduce_the_account() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let shape: Vec<_> = pages
        .iter()
        .map(|p| (p.offset, p.bytes.len(), p.complete))
        .collect();
    assert_eq!(shape, [(0, 4, false), (4, 4, false), (8, 2, true)]);
    for page in &pages {
        assert_eq!(
            (page.snapshot, page.total, page.sha256),
            (1, 10, *account().sha256())
        );
        assert_eq!(page.check(), Ok(()));
    }
    assert_eq!(joined(&pages), account().bytes());

    let t = Instant::now();
    let mut halves = paged(5);
    let first = start(&mut halves, account(), t, t + SECOND);
    let second = read(&mut halves, 1, 5, t).expect("the second half is served");
    let shape = [
        (first.bytes.len(), first.complete),
        (second.bytes.len(), second.complete),
    ];
    assert_eq!(shape, [(5, false), (5, true)]);
}

#[test]
fn repeated_offset_returns_the_same_page() {
    let (mut slot, t, pages) = ten_bytes_in_pages_of_four();
    for page in &pages {
        assert_eq!(read(&mut slot, 1, page.offset, t).as_ref(), Ok(page));
        assert_eq!(read(&mut slot, 1, page.offset, t).as_ref(), Ok(page));
    }
    let inside = read(&mut slot, 1, 3, t).expect("any in-range offset is served");
    assert_eq!(
        (&inside.bytes[..], inside.complete),
        (&account().bytes()[3..7], false)
    );
    let last = read(&mut slot, 1, 9, t).expect("the last byte is served");
    assert_eq!((last.bytes.len(), last.complete), (1, true));
}

#[test]
fn later_pages_read_the_frozen_bytes() {
    let t = Instant::now();
    let mut source = Account { a: 1234 };
    let mut slot = paged(4);
    let first = slot.capture(SCOPE, t + SECOND, &|| t, || FrozenAccount::freeze(&source));
    let mut pages = vec![first.expect("the capture succeeds")];
    source.a = 9876;
    let changed = FrozenAccount::freeze(&source).expect("the changed source freezes");
    for offset in [4, 8] {
        pages.push(read(&mut slot, 1, offset, t).expect("a later page is served"));
    }
    assert_ne!(changed.sha256(), account().sha256());
    assert!(pages.iter().all(|page| page.sha256 == *account().sha256()));
    assert_eq!(assemble::<Account>(pages), Ok(Account { a: 1234 }));
}

#[test]
fn snapshot_ids_never_repeat_on_a_connection() {
    let t = Instant::now();
    let mut slot = paged(4);
    let first = start(&mut slot, account(), t, t + SECOND).snapshot;
    slot.discard();
    assert!(!slot.is_holding());
    let second = start(&mut slot, account(), t, t + SECOND).snapshot;
    let third = start(&mut slot, account(), t, t + SECOND).snapshot;
    assert_eq!([first, second, third], [1, 2, 3]);
}

#[test]
fn new_capture_replaces_the_old_one() {
    let t = Instant::now();
    let mut slot = paged(4);
    start(&mut slot, account(), t, t + SECOND);
    start(&mut slot, account(), t, t + SECOND);
    let refused = read(&mut slot, 1, 4, t).expect_err("the old capture is gone");
    assert_eq!(refused, PagingError::UnknownSnapshot { snapshot: 1 });
    assert_eq!(refused.code(), -32602);
    assert_eq!(read(&mut slot, 2, 4, t).map(|page| page.snapshot), Ok(2));
}

#[test]
fn failed_restart_discards_the_previous_capture() {
    let t = Instant::now();
    let mut slot = paged(4);
    start(&mut slot, account(), t, t + SECOND);
    let failed = slot.capture(SCOPE, t + SECOND, &|| t, || Err::<FrozenAccount, _>("gone"));
    assert!(
        matches!(failed, Err(CaptureError::Collect("gone"))),
        "{failed:?}"
    );
    assert!(!slot.is_holding());
    assert_eq!(
        read(&mut slot, 1, 4, t),
        Err(PagingError::UnknownSnapshot { snapshot: 1 })
    );
}

#[test]
fn page_refuses_changed_scope() {
    let (mut slot, t, _) = ten_bytes_in_pages_of_four();
    let refused = slot.page(&"cell-2", cursor(1, 4), t + SECOND, t);
    assert_eq!(refused, Err(PagingError::ScopeChanged { snapshot: 1 }));
    assert_eq!(refused.map_err(|error| error.code()), Err(-32602));
}

#[test]
fn page_refuses_zero_snapshot_with_offset() {
    let (mut slot, t, _) = ten_bytes_in_pages_of_four();
    for offset in [4, 0] {
        let refused = read(&mut slot, 0, offset, t).expect_err("snapshot zero names no page");
        assert_eq!(
            refused,
            PagingError::InvalidCursor {
                snapshot: 0,
                offset
            }
        );
        assert_eq!(refused.code(), -32602);
    }
    assert!(PageCursor::START.is_start());
    assert!(!cursor(0, 4).is_start() && !cursor(1, 0).is_start());
}

#[test]
fn page_refuses_offset_at_total() {
    let (mut slot, t, _) = ten_bytes_in_pages_of_four();
    for offset in [10, 11, u64::MAX] {
        let refused = read(&mut slot, 1, offset, t).expect_err("an offset past the end");
        assert_eq!(refused, PagingError::OffsetOutOfRange { offset, total: 10 });
        assert_eq!(refused.code(), -32602);
    }
    assert!(read(&mut slot, 1, 9, t).is_ok());
}

#[test]
fn expired_capture_is_internal_error_and_dropped() {
    let t = Instant::now();
    let expires = t + SECOND;
    let mut slot = paged(4);
    start(&mut slot, account(), t, expires);
    assert!(read(&mut slot, 1, 4, expires - Duration::from_millis(1)).is_ok());
    let refused = read(&mut slot, 1, 4, expires).expect_err("the deadline has passed");
    assert_eq!(
        (refused.clone(), refused.code()),
        (PagingError::Expired, -32603)
    );
    assert!(!slot.is_holding());
    assert_eq!(
        read(&mut slot, 1, 4, t),
        Err(PagingError::UnknownSnapshot { snapshot: 1 })
    );
}

#[test]
fn later_deadline_shortens_but_never_renews() {
    let t = Instant::now();
    let mut shortened = paged(4);
    start(&mut shortened, account(), t, t + 10 * SECOND);
    assert!(
        shortened
            .page(&SCOPE, cursor(1, 4), t + 5 * SECOND, t + SECOND)
            .is_ok()
    );
    let late = shortened.page(&SCOPE, cursor(1, 8), t + 20 * SECOND, t + 6 * SECOND);
    assert_eq!(late, Err(PagingError::Expired));

    let mut kept = paged(4);
    start(&mut kept, account(), t, t + 10 * SECOND);
    assert!(
        kept.page(&SCOPE, cursor(1, 4), t + 20 * SECOND, t + SECOND)
            .is_ok()
    );
    assert!(
        kept.page(&SCOPE, cursor(1, 8), t + 30 * SECOND, t + 9 * SECOND)
            .is_ok()
    );
    let late = kept.page(&SCOPE, cursor(1, 8), t + 30 * SECOND, t + 10 * SECOND);
    assert_eq!(late, Err(PagingError::Expired));
}

#[test]
fn collection_past_the_deadline_retains_nothing() {
    let t = Instant::now();
    let expires = t + SECOND;
    let readings = Cell::new(0);
    let now = || {
        readings.set(readings.get() + 1);
        if readings.get() == 1 { t } else { expires }
    };
    let mut slot = paged(4);
    let outcome = slot.capture(SCOPE, expires, &now, || Ok::<_, Infallible>(account()));
    assert!(matches!(
        outcome,
        Err(CaptureError::Paging(PagingError::Expired))
    ));
    assert_eq!(readings.get(), 2);
    assert!(!slot.is_holding());
}

#[test]
fn capture_at_its_deadline_collects_nothing_and_drops_the_old_capture() {
    let t = Instant::now();
    let mut slot = paged(4);
    start(&mut slot, account(), t, t + 10 * SECOND);
    let collected = Cell::new(false);
    let outcome = slot.capture(SCOPE, t, &|| t, || {
        collected.set(true);
        Ok::<_, Infallible>(account())
    });
    assert!(matches!(
        outcome,
        Err(CaptureError::Paging(PagingError::Expired))
    ));
    assert!(!collected.get());
    assert!(!slot.is_holding());
}

#[test]
fn capture_stays_after_the_last_page_until_expiry() {
    let (mut slot, t, pages) = ten_bytes_in_pages_of_four();
    assert!(pages[2].complete);
    assert_eq!(
        read(&mut slot, 1, 0, t + 9 * SECOND).as_ref(),
        Ok(&pages[0])
    );
    slot.expire(t + 9 * SECOND);
    assert!(slot.is_holding());
    slot.expire(t + 10 * SECOND);
    assert!(!slot.is_holding());
    assert_eq!(
        read(&mut slot, 1, 0, t),
        Err(PagingError::UnknownSnapshot { snapshot: 1 })
    );
    slot.expire(t + 11 * SECOND);
    assert!(!slot.is_holding());
}

#[test]
fn with_page_bytes_bounds() {
    for page_bytes in [0, 65_537] {
        let refused = CaptureSlot::<&str>::with_page_bytes(page_bytes).err();
        assert_eq!(refused, Some(PagingError::BadPageSize { page_bytes }));
    }
    let t = Instant::now();
    let big = FrozenAccount::freeze(&"x".repeat(70_000)).expect("a long string freezes");
    let slots = [
        (CaptureSlot::new(), 65_536),
        (CaptureSlot::default(), 65_536),
        (paged(65_536), 65_536),
        (paged(1), 1),
    ];
    for (mut slot, served) in slots {
        assert_eq!(
            start(&mut slot, big.clone(), t, t + SECOND).bytes.len(),
            served
        );
    }
}

#[test]
fn paging_error_codes() {
    let (snapshot, offset, total, page_bytes) = (1, 10, 10, 0);
    let invalid_params = [
        PagingError::InvalidCursor { snapshot, offset },
        PagingError::UnknownSnapshot { snapshot },
        PagingError::ScopeChanged { snapshot },
        PagingError::OffsetOutOfRange { offset, total },
    ];
    for error in invalid_params {
        assert_eq!(error.code(), -32602, "{error}");
    }
    let detail = "boom".to_string();
    let internal = [
        PagingError::Expired,
        PagingError::SnapshotsExhausted,
        PagingError::BadPageSize { page_bytes },
        PagingError::Encode { detail },
    ];
    for error in internal {
        assert_eq!(error.code(), -32603, "{error}");
    }
}

#[test]
fn errors_describe_themselves() {
    let (stated, computed) = (Digest::of(b"stated"), Digest::of(b"computed"));
    let mismatch = format!("the account hashes to {computed}, not the stated {stated}");
    let detail = || "boom".to_string();
    assert_says! {
        PageFault::ZeroSnapshot => "the page names snapshot 0, which only starts a capture";
        PageFault::ZeroTotal => "the page states an empty account";
        PageFault::EmptyBytes => "the page carries no bytes";
        PageFault::TooManyBytes { len: 65_537 } => "the page carries 65537 bytes, more than 65536";
        PageFault::OffsetOverflow { offset: 9, len: 1 } => "offset 9 plus 1 bytes overflows a u64";
        PageFault::BeyondTotal { end: 8, total: 6 }
            => "the page ends at byte 8, past the account's 6 bytes";
        PageFault::CompleteFlag { stated: true, actual: false }
            => "the page says complete is true, but where it ends makes it false";
        PagingError::InvalidCursor { snapshot: 0, offset: 4 }
            => "snapshot 0 with offset 4 names no page; zero/zero starts a capture";
        PagingError::UnknownSnapshot { snapshot: 2 }
            => "snapshot 2 is not the capture this connection holds";
        PagingError::ScopeChanged { snapshot: 1 } => "snapshot 1 was captured for another scope";
        PagingError::OffsetOutOfRange { offset: 10, total: 10 }
            => "offset 10 is outside the 10-byte account";
        PagingError::Expired => "the capture's deadline has passed";
        PagingError::SnapshotsExhausted => "this connection has issued every snapshot ID";
        PagingError::BadPageSize { page_bytes: 0 } => "a page of 0 bytes is outside 1 to 65536";
        PagingError::Encode { detail: detail() } => "the account cannot be encoded as JSON: boom";
        CaptureError::<&str>::Paging(PagingError::Expired) => "the capture's deadline has passed";
        CaptureError::Collect("disk gone") => "collecting the account failed: disk gone";
        AssemblyFault::Page(PageFault::EmptyBytes) => "the page carries no bytes";
        AssemblyFault::FirstPageNotAtZero { offset: 4 }
            => "the first page starts at offset 4, not 0";
        AssemblyFault::SnapshotChanged { expected: 1, found: 2 }
            => "a page from snapshot 2 arrived while assembling snapshot 1";
        AssemblyFault::TotalChanged { expected: 10, found: 12 }
            => "a page states a 12-byte account, not 10 bytes";
        AssemblyFault::HashChanged => "a page states a different account hash than the first page";
        AssemblyFault::OutOfOrder { expected: 4, found: 8 }
            => "a page starts at offset 8, not at the first missing byte 4";
        AssemblyFault::AfterComplete => "a page arrived after the completing page";
        AssemblyFault::Incomplete { received: 8, total: Some(10) } => "only 8 of 10 bytes arrived";
        AssemblyFault::Incomplete { received: 0, total: None } => "no page arrived";
        AssemblyFault::HashMismatch { stated, computed } => mismatch;
        AssemblyFault::Whitespace
            => "the account starts or ends with whitespace, so it is not compact JSON";
        AssemblyFault::Decode { detail: detail() }
            => "the account is not the expected record: boom";
    }
}

#[test]
fn assembler_accepts_a_complete_transfer_and_decodes() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let mut assembler = PageAssembler::default();
    let mut cursors = vec![assembler.next()];
    for page in pages {
        assembler
            .accept(page)
            .expect("each page in order is accepted");
        cursors.push(assembler.next());
    }
    let expected = [
        Some(PageCursor::START),
        Some(cursor(1, 4)),
        Some(cursor(1, 8)),
        None,
    ];
    assert_eq!(cursors, expected);
    assert_eq!(assembler.finish::<Account>(), Ok(Account { a: 1234 }));
}

#[test]
fn assembler_refuses_a_page_that_breaks_its_own_rules() {
    let mut zero = page(0, 1, 1);
    zero.snapshot = 0;
    assert_eq!(
        refused_after(&[], zero),
        AssemblyFault::Page(PageFault::ZeroSnapshot)
    );
}

#[test]
fn assembler_refuses_a_first_page_not_at_zero() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let fault = refused_after(&[], pages[1].clone());
    assert_eq!(fault, AssemblyFault::FirstPageNotAtZero { offset: 4 });
}

#[test]
fn assembler_refuses_a_mixed_snapshot() {
    let (mut slot, t, pages) = ten_bytes_in_pages_of_four();
    start(&mut slot, account(), t, t + 10 * SECOND);
    let other = read(&mut slot, 2, 4, t).expect("the second capture serves its page");
    let (expected, found) = (1, 2);
    let fault = refused_after(&pages[..1], other);
    assert_eq!(fault, AssemblyFault::SnapshotChanged { expected, found });
}

#[test]
fn assembler_refuses_a_changed_total() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let mut longer = pages[1].clone();
    longer.total = 12;
    let (expected, found) = (10, 12);
    let fault = refused_after(&pages[..1], longer);
    assert_eq!(fault, AssemblyFault::TotalChanged { expected, found });
}

#[test]
fn assembler_refuses_a_changed_hash() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let mut other = pages[1].clone();
    other.sha256 = Digest::of(b"another account");
    assert_eq!(
        refused_after(&pages[..1], other),
        AssemblyFault::HashChanged
    );
}

#[test]
fn assembler_refuses_a_gap() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let (expected, found) = (4, 8);
    let fault = refused_after(&pages[..1], pages[2].clone());
    assert_eq!(fault, AssemblyFault::OutOfOrder { expected, found });
}

#[test]
fn assembler_refuses_an_overlap() {
    let (mut slot, t, pages) = ten_bytes_in_pages_of_four();
    let overlapping = read(&mut slot, 1, 2, t).expect("offset 2 is in range");
    let (expected, found) = (4, 2);
    let fault = refused_after(&pages[..1], overlapping);
    assert_eq!(fault, AssemblyFault::OutOfOrder { expected, found });
    let (expected, found) = (8, 0);
    let fault = refused_after(&pages[..2], pages[0].clone());
    assert_eq!(fault, AssemblyFault::OutOfOrder { expected, found });
}

#[test]
fn assembler_refuses_a_page_after_complete() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    for again in [&pages[0], &pages[2]] {
        assert_eq!(
            refused_after(&pages, again.clone()),
            AssemblyFault::AfterComplete
        );
    }
}

#[test]
fn assembler_refuses_a_prefix() {
    let (_, _, pages) = ten_bytes_in_pages_of_four();
    let (received, total) = (8, Some(10));
    let refused = assemble::<Account>(pages[..2].to_vec());
    assert_eq!(refused, Err(AssemblyFault::Incomplete { received, total }));
    let (received, total) = (0, None);
    let refused = PageAssembler::new().finish::<Account>();
    assert_eq!(refused, Err(AssemblyFault::Incomplete { received, total }));
}

#[test]
fn assembler_refuses_a_hash_mismatch() {
    let (_, _, mut pages) = ten_bytes_in_pages_of_four();
    pages[1].bytes[0] ^= 0x01;
    let (stated, computed) = (*account().sha256(), Digest::of(&joined(&pages)));
    let refused = assemble::<Account>(pages);
    assert_eq!(
        refused,
        Err(AssemblyFault::HashMismatch { stated, computed })
    );
}

#[test]
fn assembler_refuses_surrounding_whitespace() {
    for text in ["{\"a\":1}\n", " {\"a\":1}", "\t{\"a\":1}", "{\"a\":1}\r"] {
        let refused = assemble::<Value>(pages_of(text.as_bytes(), 4));
        assert_eq!(refused, Err(AssemblyFault::Whitespace), "{text:?}");
    }
    assert_eq!(
        assemble::<Value>(pages_of(b"{\"a\":1}", 4)),
        Ok(json!({"a": 1}))
    );
}

#[test]
fn assembler_refuses_undecodable_content() {
    let wider = FrozenAccount::freeze(&json!({"a": 1, "b": 2})).expect("an object freezes");
    let refused = assemble::<Account>(pages_of(wider.bytes(), 4));
    let reason = "unknown field `b`";
    assert!(matches!(&refused, Err(AssemblyFault::Decode { detail }) if detail.contains(reason)));
    let refused = assemble::<Account>(pages_of(b"nope", 4));
    assert!(
        matches!(&refused, Err(AssemblyFault::Decode { .. })),
        "{refused:?}"
    );
}

#[test]
fn assembler_does_not_trust_total_for_allocation() {
    let mut first = page(0, 1, u64::MAX);
    first.bytes = vec![b'{'];
    let mut assembler = PageAssembler::new();
    assert_eq!(assembler.accept(first), Ok(()));
    assert_eq!(assembler.next(), Some(cursor(1, 1)));
    let (received, total) = (1, Some(u64::MAX));
    let refused = assembler.finish::<Value>();
    assert_eq!(refused, Err(AssemblyFault::Incomplete { received, total }));
}

fn transfer(text: &str, page_bytes: usize) -> (FrozenAccount, Vec<ObservationPage>, String) {
    let t = Instant::now();
    let frozen = FrozenAccount::freeze(&text).expect("a string freezes");
    let mut slot = paged(page_bytes);
    let mut assembler = PageAssembler::new();
    let mut pages = Vec::new();
    let mut page = start(&mut slot, frozen.clone(), t, t + 60 * SECOND);
    loop {
        pages.push(page.clone());
        assembler
            .accept(page)
            .expect("each served page is accepted");
        let Some(next) = assembler.next() else { break };
        page = read(&mut slot, next.snapshot, next.offset, t).expect("the next page is served");
    }
    let decoded = assembler.finish::<String>().expect("the account assembles");
    (frozen, pages, decoded)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn round_trip_any_account_any_page_size(
        chars in proptest::collection::vec(any::<char>(), 0..75_000),
        page_bytes in 1..=MAX_PAGE_BYTES,
        flip in any::<prop::sample::Index>(),
    ) {
        let text: String = chars.into_iter().collect();
        let (frozen, mut pages, decoded) = transfer(&text, page_bytes);
        prop_assert_eq!(&decoded, &text);
        prop_assert_eq!(pages.len() as u64, frozen.total().div_ceil(page_bytes as u64));
        let at = flip.index(frozen.total() as usize);
        pages[at / page_bytes].bytes[at % page_bytes] ^= 0x01;
        let refused = assemble::<String>(pages);
        prop_assert!(matches!(refused, Err(AssemblyFault::HashMismatch { .. })), "{:?}", refused);
    }
}
