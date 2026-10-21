#[path = "fuzz/control_line.rs"]
mod control_line;
#[path = "fuzz/manifest.rs"]
mod manifest;
#[path = "fuzz/replay.rs"]
mod replay;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Mutex;

use replay::{MAX_CORPUS_BYTES, MAX_INPUT_BYTES, replay_dir};

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

fn panic_text(run: impl FnOnce()) -> Option<String> {
    let payload = catch_unwind(AssertUnwindSafe(run)).err()?;
    Some(
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_else(|| "a panic that carries no text".to_string()),
    )
}

fn corpus(files: &[(&str, Vec<u8>)]) -> tempfile::TempDir {
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
    let dir = corpus(&[(".keep", b"seed".to_vec())]);
    let text = panic_text(|| replay_dir(dir.path(), accept)).expect("an empty corpus panics");
    assert!(
        text.contains(&shown(dir.path())) && text.contains("no input"),
        "the panic names the directory as holding no input: {text}"
    );
}

#[test]
fn an_input_over_the_bound_panics_naming_it_and_one_at_the_bound_passes() {
    let at = corpus(&[("seed-at", vec![b'a'; MAX_INPUT_BYTES])]);
    assert_eq!(panic_text(|| replay_dir(at.path(), accept)), None);

    let past = corpus(&[("seed-past", vec![b'a'; MAX_INPUT_BYTES + 1])]);
    let text = panic_text(|| replay_dir(past.path(), accept)).expect("a long input panics");
    assert!(
        text.contains(&shown(&past.path().join("seed-past"))),
        "the panic names the long input: {text}"
    );
}

#[test]
fn a_corpus_over_its_total_panics_naming_it_and_one_at_the_total_passes() {
    let inputs = |count: usize| -> Vec<(String, Vec<u8>)> {
        (0..count)
            .map(|index| (format!("seed-{index:02}"), vec![b'a'; MAX_INPUT_BYTES]))
            .collect()
    };
    let named = |inputs: &[(String, Vec<u8>)]| -> tempfile::TempDir {
        let files: Vec<(&str, Vec<u8>)> = inputs
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect();
        corpus(&files)
    };
    assert_eq!(16 * MAX_INPUT_BYTES, MAX_CORPUS_BYTES);

    let at = named(&inputs(16));
    assert_eq!(panic_text(|| replay_dir(at.path(), accept)), None);

    let past = named(&inputs(17));
    let text = panic_text(|| replay_dir(past.path(), accept)).expect("a large corpus panics");
    assert!(
        text.contains(&shown(past.path())) && text.contains(&MAX_CORPUS_BYTES.to_string()),
        "the panic names the directory and its bound: {text}"
    );
}

#[test]
fn every_panicking_input_is_named_in_one_failure() {
    let dir = corpus(&[
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
    let files: Vec<(&str, Vec<u8>)> = written
        .iter()
        .map(|name| (name.as_str(), name.as_bytes().to_vec()))
        .collect();
    let dir = corpus(&files);
    replay_dir(dir.path(), record);
    let mut sorted = written.clone();
    sorted.sort();
    let expected: Vec<Vec<u8>> = sorted.iter().map(|name| name.as_bytes().to_vec()).collect();
    let seen = SEEN.lock().expect("no recording check panics").clone();
    assert_eq!(seen, expected, "the inputs arrive sorted by name");
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
    let mut over = Vec::new();
    for entry in std::fs::read_dir(dir).expect("the corpus is listed") {
        let path = entry.expect("the corpus is listed").path();
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("the input is read");
        let opening = bytes.iter().filter(|byte| LEVEL_BYTES.contains(byte)).count();
        if opening > MAX_LEVEL_BYTES {
            over.push(format!("{}: {opening}", path.display()));
        }
    }
    over
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
    let dir = corpus(&[
        ("seed-49", vec![b'['; MAX_LEVEL_BYTES]),
        ("seed-50", vec![b'['; MAX_LEVEL_BYTES + 1]),
        ("seed-50-mixed", b"[{.".repeat(17)[..50].to_vec()),
    ]);
    let mut over = over_the_level_bytes(dir.path());
    over.sort();
    let mut expected = vec![
        format!("{}: 50", dir.path().join("seed-50").display()),
        format!("{}: 50", dir.path().join("seed-50-mixed").display()),
    ];
    expected.sort();
    assert_eq!(over, expected);
}
