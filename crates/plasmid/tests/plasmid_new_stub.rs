use std::process::Command;

#[test]
fn plasmid_new_is_a_loud_reservation_not_a_scaffold() {
    let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .args(["new", "my-thing"])
        .output()
        .expect("the plasmid stub runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("plasmid new"), "{stderr}");
    assert!(stderr.contains("not frozen"), "{stderr}");
    assert!(
        !std::path::Path::new("my-thing").exists(),
        "the reserved verb must not scaffold a directory"
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
