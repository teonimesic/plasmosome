#[path = "fuzz/control_line.rs"]
mod control_line;
#[path = "fuzz/manifest.rs"]
mod manifest;
#[path = "fuzz/replay.rs"]
mod replay;

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Mutex;

use replay::{
    MAX_CORPUS_BYTES, MAX_INPUT_BYTES, panic_message, read_corpus, read_input, replay_dir,
};

fn accept(_: &[u8]) {}

fn refuse_marked(data: &[u8]) {
    if data.starts_with(b"panic") {
        panic!("the check refuses a marked input");
    }
}

static SEEN: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

fn record(data: &[u8]) {
    SEEN.lock()
        .expect("no recording check panics")
        .push(data.to_vec());
}

static THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record_thread(_: &[u8]) {
    let mut names = THREADS.lock().expect("no recording check panics");
    names.push(std::thread::current().name().unwrap_or_default().into());
}

fn panic_text(run: impl FnOnce()) -> Option<String> {
    let payload = catch_unwind(AssertUnwindSafe(run)).err()?;
    Some(panic_message(payload.as_ref()))
}

fn corpus(
    files: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>)>,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    for (name, bytes) in files {
        std::fs::write(dir.path().join(name), bytes).expect("the input is written");
    }
    dir
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

#[test]
fn a_missing_corpus_panics_naming_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("no-such-target");
    let text = panic_text(|| replay_dir(&missing, accept)).expect("a missing corpus panics");
    assert!(
        text.contains(&shown(&missing)),
        "the panic names the missing directory: {text}"
    );
}

#[test]
fn a_corpus_holding_only_a_dot_file_panics_as_empty() {
    let dir = corpus([(".keep", b"seed".to_vec())]);
    let text = panic_text(|| replay_dir(dir.path(), accept)).expect("an empty corpus panics");
    assert!(
        text.contains(&shown(dir.path())) && text.contains("no input"),
        "the panic names the directory as holding no input: {text}"
    );
}

#[test]
fn an_input_over_the_bound_panics_naming_it_and_one_at_the_bound_passes() {
    let at = corpus([("seed-at", vec![b'a'; MAX_INPUT_BYTES])]);
    assert_eq!(panic_text(|| replay_dir(at.path(), accept)), None);

    let past = corpus([("seed-past", vec![b'a'; MAX_INPUT_BYTES + 1])]);
    let text = panic_text(|| replay_dir(past.path(), accept)).expect("a long input panics");
    assert!(
        text.contains(&shown(&past.path().join("seed-past"))),
        "the panic names the long input: {text}"
    );
}

#[test]
fn a_corpus_over_its_total_panics_naming_it_and_one_at_the_total_passes() {
    let inputs = |count: usize| {
        (0..count).map(|index| (format!("seed-{index:02}"), [b'a'; MAX_INPUT_BYTES]))
    };
    assert_eq!(16 * MAX_INPUT_BYTES, MAX_CORPUS_BYTES);

    let at = corpus(inputs(16));
    assert_eq!(panic_text(|| replay_dir(at.path(), accept)), None);

    let past = corpus(inputs(17));
    let text = panic_text(|| replay_dir(past.path(), accept)).expect("a large corpus panics");
    assert!(
        text.contains(&shown(past.path())) && text.contains(&MAX_CORPUS_BYTES.to_string()),
        "the panic names the directory and its bound: {text}"
    );
}

#[test]
fn every_panicking_input_is_named_in_one_failure() {
    let dir = corpus([
        ("seed-a", b"fine".to_vec()),
        ("seed-b", b"panic first".to_vec()),
        ("seed-c", b"fine".to_vec()),
        ("seed-d", b"panic second".to_vec()),
    ]);
    let text = panic_text(|| replay_dir(dir.path(), refuse_marked)).expect("the replay fails");
    assert!(
        text.contains("seed-b") && text.contains("seed-d"),
        "the failure names both panicking inputs: {text}"
    );
    assert!(
        !text.contains("seed-a") && !text.contains("seed-c"),
        "the failure names only the panicking inputs: {text}"
    );
}

#[test]
fn inputs_arrive_in_name_order() {
    let written: Vec<String> = (0..26u8)
        .map(|index| format!("seed-{}", (b'a' + index * 7 % 26) as char))
        .collect();
    let dir = corpus(written.iter().map(|name| (name, name)));
    replay_dir(dir.path(), record);
    let mut sorted = written.clone();
    sorted.sort();
    let expected: Vec<Vec<u8>> = sorted.iter().map(|name| name.as_bytes().to_vec()).collect();
    let seen = SEEN.lock().expect("no recording check panics").clone();
    assert_eq!(seen, expected, "the inputs arrive sorted by name");
}

#[test]
fn each_entry_that_is_not_a_regular_file_is_named_before_any_input_is_read() {
    let dir = corpus([
        ("a-long", vec![b'a'; MAX_INPUT_BYTES + 1]),
        ("seed-real", b"fine".to_vec()),
    ]);
    let copied = dir.path().join("manifest");
    std::fs::create_dir(&copied).expect("the subdirectory is made");
    std::fs::write(copied.join("crash-1"), b"panic").expect("the input is written");
    for (link, target) in [("seed-link", "seed-real"), ("seed-zero", "/dev/zero")] {
        std::os::unix::fs::symlink(dir.path().join(target), dir.path().join(link))
            .expect("the link is made");
    }
    let text = panic_text(|| replay_dir(dir.path(), accept)).expect("the replay refuses them");
    for entry in ["manifest", "seed-link", "seed-zero"] {
        let named = shown(&dir.path().join(entry));
        assert!(text.contains(&named), "the panic names {named}: {text}");
    }
}

#[test]
fn an_input_is_read_no_further_than_one_byte_past_its_bound() {
    let mut source = std::io::repeat(0).take(1 << 20);
    let text = panic_text(|| drop(read_input(Path::new("endless"), &mut source)))
        .expect("an endless input panics");
    assert!(
        text.contains("endless holds more than 4096 bytes"),
        "the panic names the input: {text}"
    );
    assert_eq!((1 << 20) - source.limit(), MAX_INPUT_BYTES as u64 + 1);
}

#[test]
fn each_input_runs_on_a_thread_named_after_it() {
    let dir = corpus([("seed-a", b"a".to_vec()), ("seed-b", b"b".to_vec())]);
    replay_dir(dir.path(), record_thread);
    let names = THREADS.lock().expect("no recording check panics").clone();
    assert_eq!(names, ["input seed-a", "input seed-b"]);
}

#[test]
fn the_control_line_corpus_replays() {
    replay::replay("control_line", control_line::check);
}

#[test]
fn the_manifest_corpus_replays() {
    replay::replay("manifest", manifest::check);
}

#[test]
fn the_tree_walk_counts_the_documented_levels() {
    assert_eq!(manifest::tree_depth("a = 1"), Some(1));
    assert_eq!(manifest::tree_depth("a.b = [{ c = 1 }]"), Some(4));
    assert_eq!(manifest::tree_depth("[[a]]\n[a.b]\nc = []"), Some(5));
    assert_eq!(manifest::tree_depth("a = "), None);
}

const LEVEL_BYTES: &[u8] = b"[{.";

const MAX_LEVEL_BYTES: usize = 49;

fn over_the_level_bytes(dir: &Path) -> Vec<String> {
    read_corpus(dir)
        .into_iter()
        .filter_map(|(name, bytes)| {
            let opening = bytes
                .iter()
                .filter(|byte| LEVEL_BYTES.contains(byte))
                .count();
            (opening > MAX_LEVEL_BYTES).then(|| format!("{name}: {opening}"))
        })
        .collect()
}

#[test]
fn no_saved_declaration_can_nest_more_than_99_levels() {
    let corpus = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fuzz/corpus/manifest"
    ));
    let over = over_the_level_bytes(corpus);
    assert!(
        over.is_empty(),
        "a saved declaration holds more than {MAX_LEVEL_BYTES} of [, {{ and ., so it may nest \
         more than 99 levels, and this public repository holds none deeper than 100: {over:?}"
    );
}

#[test]
fn a_declaration_holding_50_brackets_is_over_the_guard() {
    let dir = corpus([
        ("seed-49", vec![b'['; MAX_LEVEL_BYTES]),
        ("seed-50", vec![b'['; MAX_LEVEL_BYTES + 1]),
        ("seed-50-mixed", b"[{.".repeat(17)[..50].to_vec()),
    ]);
    assert_eq!(
        over_the_level_bytes(dir.path()),
        ["seed-50: 50", "seed-50-mixed: 50"]
    );
}

#[test]
fn a_declaration_in_a_subdirectory_fails_the_guard_naming_it() {
    let dir = corpus([("seed-49", vec![b'['; MAX_LEVEL_BYTES])]);
    let crashes = dir.path().join("crashes");
    std::fs::create_dir(&crashes).expect("the subdirectory is made");
    std::fs::write(crashes.join("crash-1"), vec![b'['; MAX_LEVEL_BYTES + 1])
        .expect("the input is written");
    let text = panic_text(|| drop(over_the_level_bytes(dir.path())))
        .expect("the guard refuses the subdirectory");
    assert!(
        text.contains(&shown(&crashes)),
        "the guard names it: {text}"
    );
}
