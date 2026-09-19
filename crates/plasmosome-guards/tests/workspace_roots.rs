use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use plasmosome_guards::check_workspace;

const PUBLICATION: &str = "only_the_held_names_are_publishable_to_a_registry";
const UNHELD: &str = "plasmosome-backend";

#[derive(Clone)]
struct Consumer {
    executable: PathBuf,
    filter: &'static str,
    violation: &'static str,
}

fn copy_tree(source: &Path, destination: &Path) {
    let metadata = fs::symlink_metadata(source).expect("the fixture source is readable");
    if metadata.file_type().is_symlink() {
        symlink(
            fs::read_link(source).expect("the source symlink is readable"),
            destination,
        )
        .expect("the fixture preserves the source symlink");
    } else if metadata.is_dir() {
        fs::create_dir_all(destination).expect("the fixture directory is created");
        for entry in fs::read_dir(source).expect("the source directory is readable") {
            let entry = entry.expect("the source directory entry is readable");
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | "target" | ".worktrees")) {
                continue;
            }
            if entry.path().is_dir() && entry.path().join(".git").exists() {
                continue;
            }
            copy_tree(&entry.path(), &destination.join(name));
        }
    } else {
        assert!(metadata.is_file(), "fixtures copy only files and symlinks");
        fs::copy(source, destination).expect("the current working source is copied");
    }
}

fn copy_source(source: &Path, destination: &Path) {
    fs::create_dir_all(destination.join("docs/specs")).expect("the fixture spec directory exists");
    for name in ["Cargo.toml", "Cargo.lock", "crates"] {
        copy_tree(&source.join(name), &destination.join(name));
    }
}

fn cargo(root: &Path, target: &Path) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target)
        .env_remove("CARGO_MANIFEST_DIR")
        .env("CARGO_TERM_COLOR", "never")
        .env("RUST_BACKTRACE", "0");
    command
}

fn transcript(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn build_consumers(root: &Path, target: &Path) -> Vec<Consumer> {
    assert!(
        !target.exists(),
        "each compilation uses an unused owned target"
    );
    let output = cargo(root, target)
        .args([
            "test",
            "--locked",
            "-p",
            "plasmosome-guards",
            "--test",
            "workspace_guards",
            "--no-run",
            "--message-format=json",
        ])
        .output()
        .expect("Cargo compiles the actual consumer test executables");
    assert!(output.status.success(), "{}", transcript(&output));
    let mut publication = None;
    for line in output.stdout.split(|byte| *byte == b'\n') {
        let Ok(artifact) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if artifact["reason"] != "compiler-artifact" || artifact["profile"]["test"] != true {
            continue;
        }
        let Some(executable) = artifact["executable"].as_str() else {
            continue;
        };
        let slot = match artifact["target"]["name"].as_str() {
            Some("workspace_guards") => &mut publication,
            _ => continue,
        };
        assert_eq!(
            artifact["fresh"], false,
            "the consumer was actually rebuilt"
        );
        assert!(slot.replace(PathBuf::from(executable)).is_none());
    }
    [Consumer {
        executable: publication.expect("Cargo reports the publication integration executable"),
        filter: PUBLICATION,
        violation: UNHELD,
    }]
    .to_vec()
}

fn copied_consumers(consumers: &[Consumer], destination: &Path) -> Vec<Consumer> {
    fs::create_dir_all(destination).expect("the copied binary directory exists");
    consumers
        .iter()
        .map(|original| {
            let executable = destination.join(original.executable.file_name().unwrap());
            fs::copy(&original.executable, &executable)
                .expect("the linked executable is copied unchanged");
            Consumer {
                executable,
                filter: original.filter,
                violation: original.violation,
            }
        })
        .collect()
}

fn run(consumer: &Consumer, cwd: &Path, target: &Path, manifest: Option<&Path>) -> Output {
    let mut command = Command::new(&consumer.executable);
    command
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", target)
        .env("RUST_BACKTRACE", "0")
        .env_remove("CARGO_MANIFEST_DIR")
        .args([
            consumer.filter,
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ]);
    if let Some(manifest) = manifest {
        command.env("CARGO_MANIFEST_DIR", manifest);
    }
    command
        .output()
        .expect("the selected real consumer finishes")
}

enum Verdict<'a> {
    Pass,
    Stale {
        built: &'a Path,
        runtime: &'a Path,
        violated: bool,
    },
    Unavailable,
}

fn observe(
    failures: &mut Vec<String>,
    case: &str,
    consumer: &Consumer,
    output: Output,
    expected: Verdict<'_>,
) {
    let text = transcript(&output);
    let panic = text.split_once("panicked at").map(|(_, cause)| cause);
    let valid = match expected {
        Verdict::Pass => {
            output.status.success()
                && text.contains("1 passed; 0 failed")
                && !text.contains("StaleTarget")
                && !text.contains("WorkspaceRootUnavailable")
                && !text.contains(consumer.violation)
        }
        Verdict::Stale {
            built,
            runtime,
            violated,
        } => {
            let before_panic = text.split_once("panicked at").map(|(before, _)| before);
            !output.status.success()
                && text.contains("0 passed; 1 failed")
                && before_panic.is_some_and(|before| before.contains("StaleTarget"))
                && text.contains(built.to_str().unwrap())
                && text.contains(runtime.to_str().unwrap())
                && !text.contains("WorkspaceRootUnavailable")
                && if violated {
                    panic.is_some_and(|cause| cause.contains(consumer.violation))
                } else {
                    panic.is_some_and(|cause| cause.contains("StaleTarget"))
                        && !text.contains(consumer.violation)
                }
        }
        Verdict::Unavailable => {
            !output.status.success()
                && text.contains("0 passed; 1 failed")
                && panic.is_some_and(|cause| cause.contains("WorkspaceRootUnavailable"))
                && !text.contains("StaleTarget")
                && !text.contains(consumer.violation)
        }
    };
    if valid {
        eprintln!("workspace roots: {case}: {}: observed", consumer.filter);
    } else {
        failures.push(format!("{case}: {}\n{text}", consumer.filter));
    }
}

fn mutate_copy(root: &Path, consumer: &Consumer) -> (PathBuf, String) {
    assert_eq!(
        consumer.filter, PUBLICATION,
        "one consumer drives the mutation"
    );
    let path = root.join("crates/plasmosome-backend/Cargo.toml");
    let original = fs::read_to_string(&path).expect("the actual consumer input is readable");
    let manifest: toml::Value = original.parse().expect("the member manifest is valid TOML");
    assert_eq!(manifest["package"]["name"].as_str(), Some(UNHELD));
    assert_eq!(manifest["package"]["publish"].as_bool(), Some(false));
    assert_eq!(original.matches("publish = false").count(), 1);
    let changed = original.replacen("publish = false", "publish = [\"crates-io\"]", 1);
    fs::write(&path, changed).expect("only the copy's consumer input changes");
    (path, original)
}

#[test]
fn prebuilt_real_consumers_inspect_the_invocation_tree_but_cannot_certify_a_copy() {
    check_workspace(|source| {
        let directory = tempfile::tempdir().expect("the regression owns its disposable workspaces");
        let fixture = directory
            .path()
            .canonicalize()
            .expect("the fixture has a canonical path");
        let a = fixture.join("a");
        let b = fixture.join("b");
        let outside = fixture.join("outside");
        let external_target = fixture.join("external-target");
        fs::create_dir(&outside).unwrap();
        copy_source(source, &a);
        copy_source(&a, &b);
        let consumers = build_consumers(&a, &external_target);
        let copied = copied_consumers(&consumers, &b.join("target/prebuilt"));
        let alias = fixture.join("a-alias");
        symlink(&a, &alias).expect("a symlink provides an equivalent workspace spelling");
        let mut failures = Vec::new();

        for (original, copy) in consumers.iter().zip(&copied) {
            for manifest in [None, Some(b.join("crates/plasmosome-membrane"))] {
                let manifest = manifest.as_deref();
                let environment = if manifest.is_some() {
                    "misleading manifest env"
                } else {
                    "absent manifest env"
                };
                for (case, executable, cwd) in [
                    ("external target, cwd A", original, &a),
                    ("binary under B, cwd A", copy, &a),
                    ("symlink-equivalent cwd A", copy, &alias),
                ] {
                    observe(
                        &mut failures,
                        &format!("{case}, {environment}"),
                        copy,
                        run(executable, cwd, &external_target, manifest),
                        Verdict::Pass,
                    );
                }
                observe(
                    &mut failures,
                    &format!("outside workspace, {environment}"),
                    copy,
                    run(copy, &outside, &external_target, manifest),
                    Verdict::Unavailable,
                );
            }

            let (input, original_input) = mutate_copy(&b, copy);
            for manifest in [None, Some(a.join("crates/plasmosome-membrane"))] {
                for (case, executable) in [
                    ("copy-only violation", copy),
                    ("external target copy-only violation", original),
                ] {
                    observe(
                        &mut failures,
                        case,
                        copy,
                        run(executable, &b, &external_target, manifest.as_deref()),
                        Verdict::Stale {
                            built: &a,
                            runtime: &b,
                            violated: true,
                        },
                    );
                }
                observe(
                    &mut failures,
                    "A retained and unmodified",
                    original,
                    run(original, &a, &external_target, manifest.as_deref()),
                    Verdict::Pass,
                );
            }
            fs::write(input, original_input).expect("the copy-only mutation is restored");
            for manifest in [None, Some(a.join("crates/plasmosome-membrane"))] {
                for (case, executable) in [
                    ("restored copy remains stale", copy),
                    ("external target restored copy remains stale", original),
                ] {
                    observe(
                        &mut failures,
                        case,
                        copy,
                        run(executable, &b, &external_target, manifest.as_deref()),
                        Verdict::Stale {
                            built: &a,
                            runtime: &b,
                            violated: false,
                        },
                    );
                }
            }
        }

        let moved = fixture.join("moved-a");
        fs::rename(&a, &moved).expect("only the disposable original workspace is moved");
        assert!(!a.exists(), "the original build root no longer exists");
        for consumer in &consumers {
            observe(
                &mut failures,
                "moved workspace without original path",
                consumer,
                run(consumer, &moved, &external_target, Some(&a)),
                Verdict::Stale {
                    built: &a,
                    runtime: &moved,
                    violated: false,
                },
            );
        }

        let rebuilt_target = b.join("target/rebuilt");
        let rebuilt = build_consumers(&b, &rebuilt_target);
        for consumer in &rebuilt {
            observe(
                &mut failures,
                "actual B rebuild clears staleness",
                consumer,
                run(consumer, &b, &rebuilt_target, Some(&a)),
                Verdict::Pass,
            );
            let mut command = cargo(&outside, &rebuilt_target);
            command
                .args(["test", "--locked", "--manifest-path"])
                .arg(b.join("Cargo.toml"));
            command.args(["-p", "plasmosome-guards", "--test", "workspace_guards"]);
            let output = command
                .args([
                    "--",
                    consumer.filter,
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .output()
                .expect("Cargo selects B from outside through its manifest");
            observe(
                &mut failures,
                "outside Cargo manifest selection after B rebuild",
                consumer,
                output,
                Verdict::Pass,
            );
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    });
}
