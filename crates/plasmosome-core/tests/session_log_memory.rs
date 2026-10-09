use plasmosome_core::SessionLog;
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write;
use std::sync::atomic::{AtomicIsize, Ordering};

const LINES: u64 = 20_000;
const BOUND: usize = 256 * 1024;

struct Counting;

#[global_allocator]
static ALLOCATOR: Counting = Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static COUNTED: Cell<bool> = const { Cell::new(false) };
}

fn track(change: isize) {
    if COUNTED.try_with(Cell::get).unwrap_or(false) {
        let live = LIVE.fetch_add(change, Ordering::Relaxed) + change;
        PEAK.fetch_max(live, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let block = unsafe { System.alloc(layout) };
        if !block.is_null() {
            track(layout.size() as isize);
        }
        block
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let block = unsafe { System.alloc_zeroed(layout) };
        if !block.is_null() {
            track(layout.size() as isize);
        }
        block
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        unsafe { System.dealloc(block, layout) };
        track(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(block, layout, size) };
        if !moved.is_null() {
            track(size as isize);
            track(-(layout.size() as isize));
        }
        moved
    }
}

fn peak_while<T>(work: impl FnOnce() -> T) -> (T, usize) {
    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    COUNTED.set(true);
    let done = work();
    COUNTED.set(false);
    (done, PEAK.load(Ordering::Relaxed).max(0) as usize)
}

#[test]
fn opening_a_long_log_holds_one_line_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.ndjson");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let detail = "x".repeat(160);
    for seq in 1..=LINES {
        let line = json!({ "ts_ms": 0, "seq": seq, "kind": "tool_invoke", "detail": detail });
        writeln!(file, "{line}").unwrap();
    }
    file.into_inner().unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size > 16 * BOUND as u64, "the log is only {size} bytes");
    let (log, peak) = peak_while(|| SessionLog::open(path.clone()).unwrap());
    assert!(
        peak < BOUND,
        "opening a {size}-byte log held {peak} bytes at once"
    );
    assert_eq!(log.append("tool_invoke", json!({})).unwrap(), LINES + 1);
}
