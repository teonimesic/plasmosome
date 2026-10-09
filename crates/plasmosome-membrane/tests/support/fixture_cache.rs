use super::fixture;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("the directory entry is readable")
                .file_name()
                .into_string()
                .expect("the entry name is UTF-8")
        })
        .collect();
    names.sort();
    names
}

fn fixture_identity(path: &Path) -> (u64, SystemTime, SystemTime) {
    let file = std::fs::metadata(path).expect("the published fixture exists");
    let key = std::fs::metadata(path.parent().expect("the fixture has a key directory"))
        .expect("the key directory exists");
    (
        file.ino(),
        file.modified()
            .expect("the fixture's modification time is readable"),
        key.modified()
            .expect("the key directory's modification time is readable"),
    )
}

#[test]
fn a_second_test_process_publishes_no_new_key_and_leaves_no_temporary_files() {
    let published = fixture::supervision_fixture();
    let root = published
        .parent()
        .and_then(Path::parent)
        .expect("the fixture sits two directories below its cache root");
    let keys = entries(root);
    let temporary = tempfile::tempdir().unwrap();

    let child = Command::new(std::env::current_exe().expect("the test executable has a path"))
        .args([
            "--exact",
            "fixture::the_supervision_fixture_is_cached_inside_the_target_directory",
            "fixture_cache::a_freshly_compiled_fixture_is_run_once_before_it_is_returned",
        ])
        .env("TMPDIR", temporary.path())
        .output()
        .expect("a second test process starts");

    let report = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success() && report.contains("test result: ok. 2 passed"),
        "the second test process reuses the published fixture and compiles a fresh one:\n{report}\n{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(
        entries(root),
        keys,
        "a second test process publishes no new key"
    );
    assert_eq!(
        entries(temporary.path()),
        Vec::<String>::new(),
        "a second test process leaves nothing in its temporary directory"
    );
}

const RACE_BUDGET: Duration = Duration::from_secs(60);

fn versions_published_under(root: &Path, finished: &AtomicBool) -> HashSet<(u64, SystemTime)> {
    let mut seen = HashSet::new();
    loop {
        let last = finished.load(Ordering::Acquire);
        for key in entries(root) {
            if let Ok(metadata) = std::fs::metadata(root.join(key).join("supervision-worker")) {
                seen.insert((
                    metadata.ino(),
                    metadata
                        .modified()
                        .expect("the fixture's modification time is readable"),
                ));
            }
        }
        if last {
            return seen;
        }
        thread::yield_now();
    }
}

#[test]
fn concurrent_callers_share_one_published_fixture_that_is_never_replaced() {
    const CALLERS: usize = 8;
    let cache = tempfile::tempdir().unwrap();
    let source = fixture::supervision_worker_source();
    let start = Arc::new(Barrier::new(CALLERS));
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = {
        let root = cache.path().to_path_buf();
        let finished = Arc::clone(&finished);
        thread::spawn(move || versions_published_under(&root, &finished))
    };
    let (report, reports) = mpsc::channel();
    let callers: Vec<_> = (0..CALLERS)
        .map(|_| {
            let start = Arc::clone(&start);
            let root = cache.path().to_path_buf();
            let source = source.clone();
            let report = report.clone();
            thread::spawn(move || {
                start.wait();
                let outcome = std::panic::catch_unwind(|| {
                    let path = fixture::compile_supervision_fixture(&root, &source);
                    let inode = std::fs::metadata(&path)
                        .expect("the returned fixture exists")
                        .ino();
                    (path, inode)
                });
                let _ = report.send(outcome.map_err(panic_message));
            })
        })
        .collect();
    drop(report);

    let deadline = Instant::now() + RACE_BUDGET;
    let outcomes: Vec<_> = (0..CALLERS)
        .map_while(|_| {
            reports
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .ok()
        })
        .collect();
    finished.store(true, Ordering::Release);
    let observed = watcher.join().expect("the watcher returns");
    assert_eq!(
        outcomes.len(),
        CALLERS,
        "every concurrent caller returns within {RACE_BUDGET:?}"
    );
    for caller in callers {
        caller
            .join()
            .expect("a caller thread exits after reporting");
    }
    assert_eq!(
        observed.len(),
        1,
        "the published fixture was never replaced or rewritten while the callers raced: {observed:?}"
    );
    let returned: Vec<(PathBuf, u64)> = outcomes
        .into_iter()
        .map(|outcome| {
            outcome.unwrap_or_else(|message| panic!("a concurrent caller failed: {message}"))
        })
        .collect();

    let published = &returned[0].0;
    let settled = fixture_identity(published);
    assert_eq!(
        observed,
        HashSet::from([(settled.0, settled.1)]),
        "the watcher saw the file the callers returned"
    );
    for (path, inode) in &returned {
        assert_eq!(
            path, published,
            "every concurrent caller returns the same path"
        );
        assert_eq!(
            *inode, settled.0,
            "no caller's executable was replaced after that caller returned"
        );
    }
    assert_eq!(
        entries(cache.path()).len(),
        1,
        "one source is cached under one key"
    );
    assert_eq!(
        entries(published.parent().expect("the fixture has a key directory")),
        ["supervision-worker"],
        "the key directory holds one executable and no temporary names"
    );

    let again = fixture::compile_supervision_fixture(cache.path(), &source);
    assert_eq!(
        &again, published,
        "a later call with the same cache root returns the same path"
    );
    assert_eq!(
        fixture_identity(&again),
        settled,
        "a later call reuses the published executable without compiling it again"
    );
}

fn key_of(fixture: &Path) -> &std::ffi::OsStr {
    fixture
        .parent()
        .and_then(Path::file_name)
        .expect("a cached fixture sits in its key directory")
}

#[test]
fn a_changed_fixture_source_compiles_to_a_new_key() {
    let cache = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();
    let changed = sources.path().join("supervision_worker.c");
    let mut text = std::fs::read_to_string(fixture::supervision_worker_source())
        .expect("the fixture source is readable");
    text.push_str("\n/* a changed source */\n");
    std::fs::write(&changed, text).expect("the changed source is written");

    let original = fixture::supervision_fixture();
    let rebuilt = fixture::compile_supervision_fixture(cache.path(), &changed);

    assert_ne!(
        key_of(&rebuilt),
        key_of(&original),
        "a changed source is cached under a new key"
    );
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map(|message| message.to_string())
            .unwrap_or_default(),
    }
}

#[test]
fn a_fixture_source_that_does_not_compile_publishes_nothing() {
    let cache = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();
    let broken = sources.path().join("broken.c");
    std::fs::write(&broken, "int main(void) { return }\n").expect("the broken source is written");

    let refused =
        std::panic::catch_unwind(|| fixture::compile_supervision_fixture(cache.path(), &broken))
            .expect_err("a source that does not compile fails setup");

    let message = panic_message(refused);
    assert!(
        message.starts_with("the supervision worker fixture compiles")
            && message.contains(&broken.display().to_string()),
        "setup reports the failed compile and names its source, not a later step: {message}"
    );
    for key in entries(cache.path()) {
        assert_eq!(
            entries(&cache.path().join(key)),
            Vec::<String>::new(),
            "a failed compile leaves neither an executable nor a temporary name"
        );
    }
}

#[test]
fn a_freshly_compiled_fixture_is_run_once_before_it_is_returned() {
    let cache = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();
    let refusing = sources.path().join("refusing.c");
    std::fs::write(&refusing, "int main(void) { return 63; }\n")
        .expect("the refusing source is written");

    let refused =
        std::panic::catch_unwind(|| fixture::compile_supervision_fixture(cache.path(), &refusing))
            .expect_err("a freshly compiled executable that does not exit 64 fails setup");
    let message = panic_message(refused);
    assert!(
        message.contains("refuses a call with no arguments")
            && message.contains("Some(63)")
            && message.contains(&cache.path().display().to_string()),
        "setup fails at the warm-up's exit-64 assertion and names the executable: {message}"
    );
}

#[test]
fn a_reused_fixture_is_run_once_before_it_is_returned() {
    let cache = tempfile::tempdir().unwrap();
    let key = cache.path().join(key_of(&fixture::supervision_fixture()));
    std::fs::create_dir(&key).expect("the key directory is created");
    std::os::unix::fs::symlink("/usr/bin/false", key.join("supervision-worker"))
        .expect("a stand-in exiting 1 is published at the real source's key");

    let refused = std::panic::catch_unwind(|| {
        fixture::compile_supervision_fixture(cache.path(), &fixture::supervision_worker_source())
    })
    .expect_err("a reused executable that does not exit 64 fails setup");
    let message = panic_message(refused);
    assert!(
        message.contains("refuses a call with no arguments")
            && message.contains("Some(1)")
            && message.contains(&key.join("supervision-worker").display().to_string()),
        "setup fails at the warm-up's exit-64 assertion and names the stale executable: {message}"
    );
}

#[test]
fn the_fixture_cache_sits_in_the_nearest_directory_cargo_tagged_whatever_the_layout_below() {
    let target = tempfile::tempdir().unwrap();
    std::fs::write(
        target.path().join("CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    )
    .expect("the cache directory tag is written");
    let executable = target
        .path()
        .join("debug/build/plasmosome-membrane/0123abcd/out/membraned-0123abcd");

    assert_eq!(
        fixture::fixture_cache_root(&executable),
        target.path().join("plasmosome-supervision-fixture"),
        "the cache root sits in the tagged target directory, not beside a deps directory"
    );
}

#[test]
#[should_panic(expected = "/work/target/debug/membraned")]
fn the_fixture_cache_refuses_an_executable_outside_a_cargo_target_directory() {
    fixture::fixture_cache_root(Path::new("/work/target/debug/membraned"));
}
