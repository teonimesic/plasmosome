use std::fs;
use std::path::{Path, PathBuf};

use plasmosome_work_state::command::{CommandOutput, RecordingCommandRunner};
use plasmosome_work_state::contract::isolated_environment;
use plasmosome_work_state::document::{ShadowDocument, parse_document};
use plasmosome_work_state::shadow::{
    ShadowError, ShadowStore, canonical_logical_export, compare_document_mapping,
    compare_shadow_parity, decode_beads_jsonl, decode_logical_export, import_shadow_documents,
    logical_export_digest, native_id, to_beads_jsonl,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn sha(character: char) -> String {
    character.to_string().repeat(40)
}

fn documents() -> Vec<ShadowDocument> {
    let content_commit = sha('a');
    vec![
        parse_document(
            "docs/intents/001-intent.md",
            "---\nid: 001\ntitle: Intent\nstatus: approved\n---\n",
            &content_commit,
        )
        .unwrap(),
        parse_document(
            "docs/specs/001-spec.md",
            "---\nid: 001\ntitle: Spec\nstatus: accepted\nintents: [001]\n---\n",
            &content_commit,
        )
        .unwrap(),
        parse_document(
            "tasks/001-task.md",
            "---\nid: 001\ntitle: Task\nstatus: in_review\npriority: 3\nspecs: [001]\nintents: [001]\npr: 75\nevidence: >-\n  reviewed\n  locally\n---\n",
            &content_commit,
        )
        .unwrap(),
    ]
}

fn store(root: &Path) -> ShadowStore {
    let repository = root.join("repository");
    fs::create_dir_all(&repository).unwrap();
    ShadowStore::new(
        "clone-a",
        root.to_path_buf(),
        repository,
        isolated_environment(root),
        PathBuf::from("/private/pinned/bd"),
    )
}

fn import_output(documents: &[ShadowDocument]) -> CommandOutput {
    CommandOutput::success(
        serde_json::json!({
            "created": documents.len(),
            "ids": documents
                .iter()
                .map(|document| native_id(&document.record))
                .collect::<Vec<_>>(),
            "skipped": 0,
        })
        .to_string(),
    )
}

fn valid_store_outputs(
    documents: &[ShadowDocument],
    source_commit: &str,
) -> Vec<Result<CommandOutput, String>> {
    vec![
        Ok(import_output(documents)),
        Ok(CommandOutput::success(to_beads_jsonl(documents).unwrap())),
        Ok(CommandOutput::success("")),
        Ok(CommandOutput::success("")),
        Ok(CommandOutput::success("markdown-shadow\n")),
        Ok(CommandOutput::success(format!("{source_commit}\n"))),
    ]
}

fn assert_refusal(error: ShadowError, code: &str, key: Option<&str>) {
    assert_eq!(error.code(), code);
    assert_eq!(error.offending_key.as_deref(), key);
}

fn encoded_value(document: &ShadowDocument) -> Value {
    let all = documents();
    let jsonl = to_beads_jsonl(&all).unwrap();
    jsonl
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Value>, _>>()
        .unwrap()
        .into_iter()
        .find(|value| value["external_ref"] == document.record.document_key)
        .unwrap()
}

#[test]
fn beads_jsonl_round_trip_preserves_every_typed_field() {
    let documents = documents();

    let decoded = decode_beads_jsonl(&to_beads_jsonl(&documents).unwrap()).unwrap();

    assert_eq!(decoded, documents);
    assert_eq!(
        decoded
            .iter()
            .map(|document| native_id(&document.record))
            .collect::<Vec<_>>(),
        vec![
            "plasmosome-intent001",
            "plasmosome-spec001",
            "plasmosome-task001",
        ]
    );
}

#[test]
fn first_shadow_import_sets_state_version_one() {
    let documents = documents();
    let jsonl = to_beads_jsonl(&documents).unwrap();

    assert!(
        documents
            .iter()
            .all(|document| document.record.state_version == 1)
    );
    for line in jsonl.lines() {
        let value: Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["metadata"]["plasmosome_document"]["state_version"], 1);
    }
}

#[test]
fn non_hex_source_commit_refuses_before_a_beads_command() {
    let root = tempdir().unwrap();
    let documents = documents();
    let store = store(root.path());
    let mut runner = RecordingCommandRunner::scripted(Vec::new());

    let error = import_shadow_documents(&mut runner, &store, &sha('g'), &documents).unwrap_err();

    assert_eq!(error.code(), "invalid_source_ref");
    assert!(runner.commands().is_empty());
    assert!(runner.finish().is_ok());
}

#[test]
fn shadow_commands_are_isolated_local_and_redact_the_import_path() {
    let root = tempdir().unwrap();
    let documents = documents();
    let source_commit = sha('b');
    let store = store(root.path());
    let mut runner =
        RecordingCommandRunner::scripted(valid_store_outputs(&documents, &source_commit));

    let result = import_shadow_documents(&mut runner, &store, &source_commit, &documents).unwrap();

    assert_eq!(result.label, "clone-a");
    assert_eq!(result.documents, documents);
    assert_eq!(runner.commands().len(), 6);
    assert_eq!(
        runner.commands()[0].argv[..2],
        ["--sandbox".to_owned(), "import".to_owned()]
    );
    assert_eq!(runner.commands()[0].argv[3], "--json");
    assert_eq!(runner.commands()[0].redacted_argv_positions, vec![2]);
    assert_eq!(runner.commands()[1].argv, vec!["--sandbox", "export"]);
    assert_eq!(
        runner.commands()[2].argv,
        vec![
            "--sandbox",
            "kv",
            "set",
            "plasmosome.authority-mode",
            "markdown-shadow",
        ]
    );
    for command in runner.commands() {
        assert_eq!(command.cwd.as_deref(), Some(store.repository.as_path()));
        assert_eq!(command.environment, store.environment);
        assert!(
            !command
                .argv
                .iter()
                .any(|argument| argument.contains("/source-worktree"))
        );
    }
    assert!(
        result
            .command_plans
            .iter()
            .all(|plan| !plan.contains(&root.path().display().to_string()))
    );
    assert!(
        result
            .command_plans
            .iter()
            .all(|plan| !plan.contains("/private/pinned/bd"))
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn mode_and_source_commit_are_verified_in_each_store() {
    let root = tempdir().unwrap();
    let documents = documents();
    let source_commit = sha('b');
    let store = store(root.path());
    let mut runner =
        RecordingCommandRunner::scripted(valid_store_outputs(&documents, &source_commit));

    import_shadow_documents(&mut runner, &store, &source_commit, &documents).unwrap();

    assert_eq!(
        runner.commands()[3].argv,
        vec![
            "--sandbox",
            "kv",
            "set",
            "plasmosome.source-commit",
            &source_commit,
        ]
    );
    assert_eq!(
        runner.commands()[4].argv,
        vec!["--sandbox", "kv", "get", "plasmosome.authority-mode"]
    );
    assert_eq!(
        runner.commands()[5].argv,
        vec!["--sandbox", "kv", "get", "plasmosome.source-commit"]
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn unexpected_authority_mode_refuses_before_source_commit_lookup() {
    let root = tempdir().unwrap();
    let documents = documents();
    let source_commit = sha('b');
    let store = store(root.path());
    let mut outputs = valid_store_outputs(&documents, &source_commit);
    outputs[4] = Ok(CommandOutput::success("beads-authoritative\n"));
    outputs.truncate(5);
    let mut runner = RecordingCommandRunner::scripted(outputs);

    let error = import_shadow_documents(&mut runner, &store, &source_commit, &documents)
        .expect_err("unexpected authority mode refuses");

    assert_refusal(error, "invalid_document", Some("intent:001"));
    assert_eq!(runner.commands().len(), 5);
    assert_eq!(
        runner.commands()[4].argv,
        vec!["--sandbox", "kv", "get", "plasmosome.authority-mode"]
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn duplicate_native_or_logical_ids_refuse_on_export() {
    let documents = documents();
    let jsonl = to_beads_jsonl(&documents).unwrap();
    let jsonl = format!("{jsonl}{}\n", jsonl.lines().last().unwrap());

    assert_refusal(
        decode_beads_jsonl(&jsonl).unwrap_err(),
        "invalid_document",
        Some("task:001"),
    );
}

#[test]
fn missing_unknown_or_inconsistent_metadata_refuses() {
    let document = documents().pop().unwrap();
    let mut missing = encoded_value(&document);
    missing.as_object_mut().unwrap().remove("metadata");
    assert_eq!(
        decode_beads_jsonl(&missing.to_string()).unwrap_err().code(),
        "invalid_document"
    );

    let mut unknown = encoded_value(&document);
    unknown["metadata"]["plasmosome_document"]["unexpected"] = Value::Bool(true);
    assert_eq!(
        decode_beads_jsonl(&unknown.to_string()).unwrap_err().code(),
        "invalid_document"
    );

    let mut inconsistent = encoded_value(&document);
    inconsistent["title"] = Value::String("Other title".into());
    assert_refusal(
        decode_beads_jsonl(&inconsistent.to_string()).unwrap_err(),
        "invalid_document",
        Some("task:001"),
    );
}

#[test]
fn import_result_must_name_the_exact_ids_and_count() {
    let root = tempdir().unwrap();
    let documents = documents();
    let store = store(root.path());
    let mut runner = RecordingCommandRunner::scripted(vec![Ok(CommandOutput::success(
        serde_json::json!({
            "created": 2,
            "ids": ["plasmosome-intent001", "plasmosome-spec001"],
            "skipped": 0,
        })
        .to_string(),
    ))]);

    let error = import_shadow_documents(&mut runner, &store, &sha('b'), &documents).unwrap_err();

    assert_refusal(error, "invalid_document", Some("intent:001"));
    assert_eq!(runner.commands().len(), 1);
    assert!(runner.finish().is_ok());
}

#[test]
fn changed_link_order_refuses_with_the_offending_key() {
    let expected = documents();
    let mut actual = expected.clone();
    actual[2].record.spec_ids = vec!["002".into(), "001".into()];

    let error = compare_document_mapping(&expected, &actual).unwrap_err();

    assert_refusal(error.clone(), "document_mapping_mismatch", Some("task:001"));
    assert_eq!(error.mismatch.as_deref(), Some("different"));
}

#[test]
fn missing_extra_and_changed_mapping_records_each_refuse() {
    let expected = documents();
    let missing = &expected[..2];
    let missing_error = compare_document_mapping(&expected, missing).unwrap_err();
    assert_eq!(missing_error.offending_key.as_deref(), Some("task:001"));
    assert_eq!(missing_error.mismatch.as_deref(), Some("missing"));

    let mut extra = expected.clone();
    let mut duplicate = extra[2].clone();
    duplicate.record.document_key = "task:002".into();
    duplicate.record.document_id = "002".into();
    duplicate.record.document_path = "tasks/002-extra.md".into();
    extra.push(duplicate);
    let extra_error = compare_document_mapping(&expected, &extra).unwrap_err();
    assert_eq!(extra_error.offending_key.as_deref(), Some("task:002"));
    assert_eq!(extra_error.mismatch.as_deref(), Some("extra"));

    let mut changed = expected.clone();
    changed[1].record.title = "Changed".into();
    let changed_error = compare_document_mapping(&expected, &changed).unwrap_err();
    assert_eq!(changed_error.offending_key.as_deref(), Some("spec:001"));
    assert_eq!(changed_error.mismatch.as_deref(), Some("different"));
}

#[test]
fn changed_lifecycle_priority_pr_or_evidence_each_refuses() {
    let expected = documents();
    let mutations: [fn(&mut ShadowDocument); 4] = [
        |document: &mut ShadowDocument| document.shadow.lifecycle = "done".into(),
        |document: &mut ShadowDocument| document.shadow.priority = Some(1),
        |document: &mut ShadowDocument| document.shadow.pr = Some("76".into()),
        |document: &mut ShadowDocument| document.shadow.evidence = Some("other".into()),
    ];
    for mutate in mutations {
        let mut actual = expected.clone();
        mutate(&mut actual[2]);
        compare_document_mapping(&expected, &actual)
            .expect("mapping ignores Markdown shadow fields");
        let error = compare_shadow_parity(&expected, &actual).unwrap_err();
        assert_eq!(error.code(), "shadow_parity_mismatch");
        assert_eq!(error.offending_key.as_deref(), Some("task:001"));
        assert_eq!(error.mismatch.as_deref(), Some("different"));
    }
}

#[test]
fn canonical_logical_export_round_trips_without_native_ids() {
    let documents = documents();
    let export = canonical_logical_export(&documents).unwrap();

    assert!(!export.contains("plasmosome-task001"));
    assert_eq!(decode_logical_export(&export).unwrap(), documents);
    let mut reordered = documents.clone();
    reordered.reverse();
    let reordered_export = canonical_logical_export(&reordered).unwrap();
    assert_eq!(reordered_export, export);
    assert_eq!(
        logical_export_digest(&export),
        logical_export_digest(&reordered_export)
    );
    assert_eq!(
        logical_export_digest(&export),
        format!("{:x}", Sha256::digest(export.as_bytes()))
    );
}
