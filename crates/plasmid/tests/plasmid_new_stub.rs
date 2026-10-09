use std::process::Command;

#[test]
fn plasmid_new_is_a_loud_reservation_not_a_scaffold() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .current_dir(scratch.path())
        .args(["new", "my-thing"])
        .output()
        .expect("the plasmid stub runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("plasmid new"), "{stderr}");
    assert!(stderr.contains("not frozen"), "{stderr}");
    let created: Vec<_> = std::fs::read_dir(scratch.path())
        .expect("the scratch directory is readable")
        .map(|entry| entry.expect("a directory entry").file_name())
        .collect();
    assert!(
        created.is_empty(),
        "the reserved verb must scaffold nothing, but created {created:?}"
    );
}

#[test]
fn plasmid_help_names_the_reserved_verb() {
    let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .arg("--help")
        .output()
        .expect("the plasmid stub runs");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("plasmid new"), "{stdout}");
}

#[test]
fn an_unknown_verb_is_a_named_refusal() {
    let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .arg("attach")
        .output()
        .expect("the plasmid stub runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown verb `attach`"), "{stderr}");
}
