use super::fixture;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, LazyLock, mpsc};
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
            "fixture_cache::the_supervision_fixture_is_cached_inside_the_target_directory",
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
    let sources = tempfile::tempdir().unwrap();
    let runs = sources.path().join("runs");
    let source = sources.path().join("counting.c");
    std::fs::write(
        &source,
        format!(
            "#include <stdio.h>\nint main(void) {{ FILE *runs = fopen(\"{}\", \"a\"); if (!runs) return 1; fputc('x', runs); fclose(runs); return 64; }}\n",
            runs.display()
        ),
    )
    .expect("a source that records each run is written");
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
    assert_eq!(
        std::fs::read(&runs)
            .expect("the callers ran the fixture")
            .len(),
        CALLERS,
        "every concurrent caller ran the fixture once before returning it"
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

#[test]
fn the_same_fixture_source_at_another_path_reuses_the_published_fixture() {
    let sources = tempfile::tempdir().unwrap();
    let copy = sources.path().join("copy.c");
    std::fs::copy(fixture::supervision_worker_source(), &copy)
        .expect("the fixture source is copied to another path");
    let published = fixture::supervision_fixture();
    let root = published
        .parent()
        .and_then(Path::parent)
        .expect("the fixture sits two directories below its cache root");

    assert_eq!(
        fixture::compile_supervision_fixture(root, &copy),
        published,
        "the same source at another path is cached under the same key and reuses the published fixture"
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
    let keys = entries(cache.path());
    assert_eq!(keys.len(), 1, "one source is cached under one key");
    assert_eq!(
        entries(&cache.path().join(&keys[0])),
        ["supervision-worker"],
        "a failed warm-up leaves the published executable and no temporary name"
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
fn a_stale_fixture_that_cannot_run_fails_setup_naming_it() {
    let cache = tempfile::tempdir().unwrap();
    let key = cache.path().join(key_of(&fixture::supervision_fixture()));
    std::fs::create_dir(&key).expect("the key directory is created");
    let stale = key.join("supervision-worker");
    std::fs::write(&stale, "").expect("a stale file that cannot run is left at the real key");

    let refused = std::panic::catch_unwind(|| {
        fixture::compile_supervision_fixture(cache.path(), &fixture::supervision_worker_source())
    })
    .expect_err("a reused file that cannot run fails setup");
    let message = panic_message(refused);
    assert!(
        message.starts_with(&format!(
            "the compiled supervision worker fixture {} runs: ",
            stale.display()
        )) && message.contains("remove it or run `cargo clean`"),
        "setup names the file that would not run and how to clear it: {message}"
    );
}

#[test]
fn a_fixture_source_that_cannot_be_read_fails_setup_naming_it() {
    let cache = tempfile::tempdir().unwrap();
    let missing = cache.path().join("missing.c");

    let refused =
        std::panic::catch_unwind(|| fixture::compile_supervision_fixture(cache.path(), &missing))
            .expect_err("a source that cannot be read fails setup");
    let message = panic_message(refused);
    assert!(
        message.starts_with(&format!(
            "the supervision worker source {} is readable: ",
            missing.display()
        )),
        "setup names the source it could not read: {message}"
    );
    assert_eq!(
        entries(cache.path()),
        Vec::<String>::new(),
        "an unreadable source creates no key directory"
    );
}

#[test]
fn a_cache_root_that_is_not_a_directory_fails_setup_naming_it() {
    let cache = tempfile::tempdir().unwrap();
    let blocked = cache.path().join("blocked");
    std::fs::write(&blocked, "").expect("a file stands where the cache root would be");

    let refused = std::panic::catch_unwind(|| {
        fixture::compile_supervision_fixture(&blocked, &fixture::supervision_worker_source())
    })
    .expect_err("a cache root that cannot hold a key directory fails setup");
    let message = panic_message(refused);
    assert!(
        message.starts_with(&format!(
            "the fixture cache directory {}/",
            blocked.display()
        )) && message.contains("is created, readable only by its owner: "),
        "setup names the key directory it could not create: {message}"
    );
}

#[test]
fn the_fixture_cache_sits_in_the_nearest_directory_cargo_tagged_whatever_the_layout_below() {
    let target = tempfile::tempdir().unwrap();
    std::fs::write(target.path().join("CACHEDIR.TAG"), TAG)
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

#[test]
fn the_supervision_fixture_is_cached_inside_the_target_directory() {
    let executable = std::env::current_exe().expect("the running test executable has a path");
    let fixture = fixture::supervision_fixture();
    let root = fixture
        .parent()
        .and_then(Path::parent)
        .expect("the fixture sits two directories below its cache root");
    let target = root
        .parent()
        .expect("the cache root sits in the target directory");
    assert!(
        root.ends_with("plasmosome-supervision-fixture")
            && target.join("CACHEDIR.TAG").is_file()
            && executable.starts_with(target),
        "the compiled fixture {} is cached in the Cargo target directory that holds the test executable {}, not in a shared temporary directory",
        fixture.display(),
        executable.display()
    );
}

const TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n";

fn untagged_temporary_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    if let Some(tagged) = directory
        .path()
        .ancestors()
        .find(|ancestor| ancestor.join("CACHEDIR.TAG").is_file())
    {
        panic!(
            "this test needs a temporary directory outside any Cargo target directory; {} has a tagged ancestor {}",
            directory.path().display(),
            tagged.display()
        );
    }
    directory
}

const GENERIC_FIX: &str = "Let Cargo create the directory, or write the tag into the Cargo target directory that holds the test executable: echo 'Signature: 8a477f597d28d172789f06886806bc55' > <that directory>/CACHEDIR.TAG";

fn declared_fix(tag: &str) -> String {
    format!(
        "Let Cargo create the directory, or write the tag into CARGO_TARGET_DIR: echo 'Signature: 8a477f597d28d172789f06886806bc55' > {tag}"
    )
}

#[test]
fn a_missing_tag_names_the_executable_and_the_fix() {
    let target = untagged_temporary_directory();
    let executable = target.path().join("debug/deps/plasmosome_membrane-0123");
    let refusal = |declared: Option<&Path>| {
        fixture::cache_root(&executable, "a-cache", "the test fixture", declared)
            .expect_err("an untagged tree holds no cache")
    };

    let generic = refusal(None);
    assert!(
        generic.starts_with(&format!(
            "the test executable {} has no ancestor holding CACHEDIR.TAG, so the test fixture has no cache inside a Cargo target directory. ",
            executable.display()
        )) && generic.contains(
            "Cargo writes that tag only when it creates the target directory itself; a directory made by hand or by cargo llvm-cov has none. "
        ) && generic.ends_with(GENERIC_FIX),
        "the refusal names the executable, why the tag is missing and the command that writes it: {generic}"
    );
    let declared = refusal(Some(target.path()));
    assert!(
        declared.ends_with(&declared_fix(&format!(
            "'{}/CACHEDIR.TAG'",
            target.path().display()
        ))),
        "a CARGO_TARGET_DIR holding the executable is named in the command: {declared}"
    );
    assert_eq!(
        declared.replace(
            &declared_fix(&format!("'{}/CACHEDIR.TAG'", target.path().display())),
            GENERIC_FIX
        ),
        generic,
        "only the command differs"
    );
    let elsewhere = untagged_temporary_directory();
    assert_eq!(
        refusal(Some(elsewhere.path())),
        generic,
        "a CARGO_TARGET_DIR that does not hold the executable gets the generic command"
    );
    assert_eq!(
        refusal(Some(Path::new(""))),
        generic,
        "an empty CARGO_TARGET_DIR gets the generic command"
    );
}

#[test]
fn the_fix_command_quotes_the_target_for_the_shell() {
    let parent = untagged_temporary_directory();
    let target = parent.path().join("it's a target");
    let executable = target.join("debug/deps/membraned-0123");

    let refusal = fixture::cache_root(&executable, "a-cache", "the test fixture", Some(&target))
        .expect_err("an untagged tree holds no cache");

    assert!(
        refusal.ends_with(&declared_fix(&format!(
            "'{}/it'\\''s a target/CACHEDIR.TAG'",
            parent.path().display()
        ))),
        "the path is one shell word, with its quote escaped: {refusal}"
    );
}

#[test]
fn a_target_named_through_a_symbolic_link_still_holds_the_executable() {
    let parent = untagged_temporary_directory();
    let real = parent.path().join("real");
    let executable = real.join("debug/deps/plasmosome_membrane-0123");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, "").expect("a stand-in executable is written");
    let link = parent.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("a link to the target is made");
    let canonical = std::fs::canonicalize(&executable).expect("the stand-in has a canonical path");

    let refusal = fixture::cache_root(&canonical, "a-cache", "the test fixture", Some(&link))
        .expect_err("an untagged tree holds no cache");

    assert!(
        refusal.ends_with(&declared_fix(&format!("'{}/CACHEDIR.TAG'", link.display()))),
        "CARGO_TARGET_DIR is named as the user spelled it: {refusal}"
    );
}

#[test]
fn a_tagged_target_still_holds_the_cache() {
    let target = tempfile::tempdir().unwrap();
    std::fs::write(target.path().join("CACHEDIR.TAG"), TAG).expect("the tag is written");
    let elsewhere = tempfile::tempdir().unwrap();

    for below in [
        "membraned-0123",
        "debug/deps/membraned-0123",
        "debug/build/plasmosome-membrane/0123abcd/out/membraned-0123abcd",
    ] {
        for declared in [None, Some(elsewhere.path())] {
            assert_eq!(
                fixture::cache_root(
                    &target.path().join(below),
                    "a-cache",
                    "the test fixture",
                    declared
                ),
                Ok(target.path().join("a-cache")),
                "an executable at {below} has its cache in the tagged target, whatever CARGO_TARGET_DIR says"
            );
        }
    }
}

#[test]
fn the_nearest_tagged_ancestor_holds_the_cache() {
    let outer = tempfile::tempdir().unwrap();
    std::fs::write(outer.path().join("CACHEDIR.TAG"), TAG).expect("the outer tag is written");
    let inner = outer.path().join("inner");
    std::fs::create_dir(&inner).expect("the inner target is created");
    std::fs::write(inner.join("CACHEDIR.TAG"), TAG).expect("the inner tag is written");

    assert_eq!(
        fixture::cache_root(
            &inner.join("debug/deps/membraned-0123"),
            "a-cache",
            "the test fixture",
            None
        ),
        Ok(inner.join("a-cache"))
    );
}

#[test]
fn a_directory_named_cachedir_tag_is_not_a_tag() {
    let target = untagged_temporary_directory();
    std::fs::create_dir(target.path().join("CACHEDIR.TAG"))
        .expect("a directory takes the tag's name");

    assert!(
        fixture::cache_root(
            &target.path().join("debug/deps/membraned-0123"),
            "a-cache",
            "the test fixture",
            None
        )
        .is_err()
    );
}

#[test]
fn a_failed_fixture_setup_fails_every_caller_with_its_own_message() {
    static FAILING: LazyLock<Result<PathBuf, String>> =
        LazyLock::new(|| fixture::set_up(|| panic!("the first cause")));

    for caller in ["first", "second"] {
        let refused = std::panic::catch_unwind(|| fixture::prepared(&FAILING))
            .expect_err("a fixture whose setup failed fails its caller");
        assert_eq!(
            panic_message(refused),
            "the first cause",
            "the {caller} caller fails with the setup's own message, not a poisoned lock"
        );
    }
}

#[test]
fn a_fixture_that_set_up_returns_its_path_to_every_caller() {
    static READY: LazyLock<Result<PathBuf, String>> =
        LazyLock::new(|| fixture::set_up(|| PathBuf::from("/a/fixture")));

    for _ in 0..2 {
        assert_eq!(fixture::prepared(&READY), PathBuf::from("/a/fixture"));
    }
}

#[test]
fn a_setup_keeps_a_formatted_message_and_names_a_payload_that_is_not_text() {
    const KIND: &str = "formatted";
    assert_eq!(
        fixture::set_up(|| panic!("a {KIND} cause")),
        Err("a formatted cause".to_owned())
    );
    assert_eq!(
        fixture::set_up(|| std::panic::panic_any(7_u8)),
        Err("the fixture's setup panicked with a payload that is not text".to_owned())
    );
}

const NEEDS_THE_FIXTURE: [&str; 2] = [
    "fixture_cache::the_same_fixture_source_at_another_path_reuses_the_published_fixture",
    "fixture_cache::the_supervision_fixture_is_cached_inside_the_target_directory",
];

fn failure_of<'a>(report: &'a str, test: &str) -> &'a str {
    let heading = format!("---- {test} stdout ----\n");
    let start = report
        .find(&heading)
        .unwrap_or_else(|| panic!("the report shows {test} failing:\n{report}"))
        + heading.len();
    let rest = &report[start..];
    &rest[..rest
        .find("\n---- ")
        .or_else(|| rest.find("\nfailures:"))
        .unwrap_or(rest.len())]
}

#[test]
fn every_test_needing_the_fixture_fails_with_the_first_cause_outside_a_tagged_target() {
    let outside = untagged_temporary_directory();
    let copy = outside.path().join("debug/deps/membraned-copy");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(
        std::env::current_exe().expect("the running test executable has a path"),
        &copy,
    )
    .expect("the test executable is copied outside the target directory");

    let child = Command::new(&copy)
        .args(["--exact", "--test-threads=1"])
        .args(NEEDS_THE_FIXTURE)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("the copied test executable starts");

    let report = String::from_utf8_lossy(&child.stdout);
    assert!(
        !child.status.success() && report.contains("test result: FAILED. 0 passed; 2 failed"),
        "both tests fail when the fixture has no cache:\n{report}"
    );
    assert!(
        !report.contains("poisoned"),
        "no test fails with a poisoned lock:\n{report}"
    );
    let cause = format!(
        "the test executable {} has no ancestor holding CACHEDIR.TAG, so the supervision fixture has no cache inside a Cargo target directory. ",
        copy.display()
    );
    for test in NEEDS_THE_FIXTURE {
        let failure = failure_of(&report, test);
        assert!(
            failure.contains(&cause) && failure.contains(GENERIC_FIX),
            "{test} fails with the first cause and the command that fixes it:\n{failure}"
        );
    }
}
