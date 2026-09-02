use std::path::Path;

use plasmosome_work_state::command::{CommandOutput, RecordingCommandRunner};
use plasmosome_work_state::contract::isolated_environment;
use plasmosome_work_state::document::{
    DocumentKind, load_documents, parse_document, validate_document_targets,
};

fn sha(character: char) -> String {
    character.to_string().repeat(40)
}

fn intent(id: &str, title: &str, status: &str) -> String {
    format!("---\nid: {id}\ntitle: {title}\nstatus: {status}\n---\n")
}

fn spec(id: &str, title: &str, status: &str, intents: &str) -> String {
    format!("---\nid: {id}\ntitle: {title}\nstatus: {status}\nintents: {intents}\n---\n")
}

fn task(id: &str, title: &str, status: &str, priority: &str, specs: &str, intents: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: {title}\nstatus: {status}\npriority: {priority}\nspecs: {specs}\nintents: {intents}\npr:\nevidence:\n---\n"
    )
}

fn source_outputs(
    source_commit: &str,
    documents: &[(&str, &str, &str)],
) -> Vec<Result<CommandOutput, String>> {
    let paths = documents
        .iter()
        .map(|(path, _, _)| *path)
        .collect::<Vec<_>>()
        .join("\0");
    let mut outputs = vec![
        Ok(CommandOutput::success(format!("{source_commit}\n"))),
        Ok(CommandOutput::success(format!("{paths}\0"))),
    ];
    let mut ordered = documents.to_vec();
    ordered.sort_by_key(|(path, _, _)| {
        let path = *path;
        match path {
            path if path.starts_with("docs/intents/") => (0, path),
            path if path.starts_with("docs/specs/") => (1, path),
            _ => (2, path),
        }
    });
    for (_, contents, content_commit) in ordered {
        outputs.push(Ok(CommandOutput::success(contents)));
        outputs.push(Ok(CommandOutput::success(format!("{content_commit}\n"))));
        outputs.push(Ok(CommandOutput::success(contents)));
    }
    outputs
}

fn load(
    runner: &mut RecordingCommandRunner,
    source_ref: &str,
) -> Result<
    plasmosome_work_state::document::SourceDocuments,
    plasmosome_work_state::document::DocumentError,
> {
    load_documents(
        runner,
        Path::new("/source-worktree"),
        &isolated_environment(Path::new("/isolated-document-environment")),
        source_ref,
    )
}

#[test]
fn requested_ref_is_resolved_once_before_numeric_documents_are_read() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let contents = intent("001", "One", "approved");
    let mut runner = RecordingCommandRunner::scripted(source_outputs(
        &source_commit,
        &[("docs/intents/001-one.md", &contents, &content_commit)],
    ));

    let source = load(&mut runner, "origin/main").expect("source documents load");

    assert_eq!(source.requested_ref, "origin/main");
    assert_eq!(source.source_commit, source_commit);
    assert_eq!(runner.commands().len(), 5);
    assert_eq!(
        runner.commands()[0].argv,
        vec![
            "rev-parse",
            "--verify",
            "--end-of-options",
            "origin/main^{commit}",
        ]
    );
    assert_eq!(
        runner
            .commands()
            .iter()
            .filter(|command| command.argv.first() == Some(&"rev-parse".into()))
            .count(),
        1
    );
    for command in &runner.commands()[1..] {
        assert!(
            !command
                .argv
                .iter()
                .any(|argument| argument.contains("origin/main"))
        );
    }
    for command in &runner.commands()[1..4] {
        assert!(
            command.argv.iter().any(|argument| {
                argument == &source_commit || argument.starts_with(&format!("{source_commit}:"))
            }),
            "expected resolved source SHA in {:?}",
            command.argv
        );
    }
    assert!(
        runner.commands()[4]
            .argv
            .iter()
            .any(|argument| argument.starts_with(&format!("{content_commit}:")))
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn source_resolution_requires_a_lowercase_hex_commit() {
    let mut runner = RecordingCommandRunner::scripted(vec![Ok(CommandOutput::success(format!(
        "{}\n",
        sha('g')
    )))]);

    let error = load(&mut runner, "selected-ref").unwrap_err();

    assert_eq!(error.code(), "invalid_source_ref");
    assert_eq!(runner.commands().len(), 1);
    assert!(runner.finish().is_ok());
}

#[test]
fn requested_tree_paths_are_discovered_without_a_configured_count() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let intent_contents = intent("001", "Intent", "approved");
    let spec_contents = spec("001", "Spec", "accepted", "[001]");
    let task_contents = task("001", "Task", "planned", "1", "[001]", "[001]");
    let mut runner = RecordingCommandRunner::scripted(source_outputs(
        &source_commit,
        &[
            ("tasks/001-task.md", &task_contents, &content_commit),
            ("docs/specs/001-spec.md", &spec_contents, &content_commit),
            (
                "docs/intents/001-intent.md",
                &intent_contents,
                &content_commit,
            ),
        ],
    ));

    let source = load(&mut runner, "selected-ref").expect("all numeric paths load");

    assert_eq!(source.documents.len(), 3);
    assert_eq!(
        source
            .documents
            .iter()
            .map(|document| document.record.document_key.as_str())
            .collect::<Vec<_>>(),
        vec!["intent:001", "spec:001", "task:001"]
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn source_tree_paths_are_nul_delimited_and_literal() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let contents = intent("001", "One", "approved");
    let mut runner = RecordingCommandRunner::scripted(source_outputs(
        &source_commit,
        &[("docs/intents/001-µ.md", &contents, &content_commit)],
    ));

    let source = load(&mut runner, "selected-ref").expect("literal path loads");

    assert_eq!(
        source.documents[0].record.document_path,
        "docs/intents/001-µ.md"
    );
    assert_eq!(
        runner.commands()[1].argv,
        vec![
            "ls-tree",
            "-r",
            "--name-only",
            "-z",
            &source_commit,
            "--",
            "docs/intents",
            "docs/specs",
            "tasks",
        ]
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn equal_ids_in_three_namespaces_make_three_distinct_keys() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let intent_contents = intent("014", "Intent", "approved");
    let spec_contents = spec("014", "Spec", "accepted", "[014]");
    let task_contents = task("014", "Task", "planned", "2", "[014]", "[014]");
    let mut runner = RecordingCommandRunner::scripted(source_outputs(
        &source_commit,
        &[
            (
                "docs/intents/014-intent.md",
                &intent_contents,
                &content_commit,
            ),
            ("docs/specs/014-spec.md", &spec_contents, &content_commit),
            ("tasks/014-task.md", &task_contents, &content_commit),
        ],
    ));

    let source = load(&mut runner, "selected-ref").unwrap();

    assert_eq!(
        source
            .documents
            .iter()
            .map(|document| document.record.document_key.as_str())
            .collect::<Vec<_>>(),
        vec!["intent:014", "spec:014", "task:014"]
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn content_commit_establishes_the_selected_literal_path_and_contents() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let contents = intent("001", "One", "approved");
    let mut runner = RecordingCommandRunner::scripted(source_outputs(
        &source_commit,
        &[("docs/intents/001-a[b].md", &contents, &content_commit)],
    ));

    let source = load(&mut runner, "selected-ref").unwrap();

    assert_eq!(
        source.documents[0].record.content_commit_sha,
        content_commit
    );
    assert_eq!(
        runner.commands()[3].argv,
        vec![
            "log".into(),
            "-1".into(),
            "--format=%H".into(),
            source_commit.clone(),
            "--".into(),
            ":(literal)docs/intents/001-a[b].md".into(),
        ]
    );
    assert_eq!(
        runner.commands()[4].argv,
        vec![
            "show".into(),
            format!("{content_commit}:docs/intents/001-a[b].md"),
        ]
    );
    assert!(runner.finish().is_ok());
}

#[test]
fn frontmatter_reconstructs_status_priority_pr_and_evidence_forms() {
    let content_commit = sha('b');
    let folded = parse_document(
        "tasks/045-shadow.md",
        "---\nid: 045\ntitle: Shadow\nstatus: in_review\npriority: 3\nspecs: [014, 014]\nintents: [015, 015]\npr: >-\n  https://github.com/teonimesic/plasmosome/pull/75\nevidence: |-\n  first line\n  second line\n---\n\nevidence: body prose is ignored\n",
        &content_commit,
    )
    .unwrap();
    assert_eq!(folded.record.kind, DocumentKind::Task);
    assert_eq!(folded.record.spec_ids, vec!["014", "014"]);
    assert_eq!(folded.record.intent_ids, vec!["015", "015"]);
    assert_eq!(folded.shadow.lifecycle, "in_review");
    assert_eq!(folded.shadow.priority, Some(3));
    assert_eq!(
        folded.shadow.pr.as_deref(),
        Some("https://github.com/teonimesic/plasmosome/pull/75")
    );
    assert_eq!(
        folded.shadow.evidence.as_deref(),
        Some("first line\nsecond line")
    );

    let plain = parse_document(
        "tasks/046-plain.md",
        "---\nid: 046\ntitle: Plain\nstatus: in_review\npriority: 1\nspecs: []\nintents: []\npr: 75\nevidence: https://example.invalid/evidence\n---\n",
        &content_commit,
    )
    .unwrap();
    assert_eq!(plain.shadow.pr.as_deref(), Some("75"));
    assert_eq!(
        plain.shadow.evidence.as_deref(),
        Some("https://example.invalid/evidence")
    );

    let blank = parse_document(
        "tasks/047-blank.md",
        "---\nid: 047\ntitle: Blank\nstatus: todo\npriority: 2\nspecs: []\nintents: []\npr:\nevidence:\n---\n",
        &content_commit,
    )
    .unwrap();
    assert_eq!(blank.shadow.pr, None);
    assert_eq!(blank.shadow.evidence, None);
}

#[test]
fn legacy_link_copies_are_preserved_without_repair() {
    let content_commit = sha('b');
    let intent = parse_document(
        "docs/intents/001-intent.md",
        &intent("001", "Intent", "approved"),
        &content_commit,
    )
    .unwrap();
    let spec = parse_document(
        "docs/specs/001-spec.md",
        &spec("001", "Spec", "accepted", "[001]"),
        &content_commit,
    )
    .unwrap();
    let task = parse_document(
        "tasks/001-task.md",
        &task("001", "Task", "todo", "1", "[001, 001]", "[]"),
        &content_commit,
    )
    .unwrap();

    let documents = vec![intent, spec, task];
    validate_document_targets(&documents).expect("present links need no repair");
    assert_eq!(documents[2].record.spec_ids, vec!["001", "001"]);
    assert!(documents[2].record.intent_ids.is_empty());
}

#[test]
fn numeric_noncanonical_path_or_path_id_mismatch_refuses() {
    let content_commit = sha('b');
    let mismatch = parse_document(
        "tasks/001-task.md",
        &task("002", "Task", "todo", "1", "[]", "[]"),
        &content_commit,
    )
    .unwrap_err();
    assert_eq!(mismatch.code(), "invalid_document");
    assert_eq!(mismatch.offending_key.as_deref(), Some("task:001"));

    let source_commit = sha('a');
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(CommandOutput::success(format!("{source_commit}\n"))),
        Ok(CommandOutput::success("tasks/001.md\0")),
    ]);
    let error = load(&mut runner, "selected-ref").unwrap_err();
    assert_eq!(error.code(), "invalid_document");
    assert_eq!(error.offending_key.as_deref(), Some("task:001"));
    assert!(runner.finish().is_ok());
}

#[test]
fn non_ascii_path_prefix_refuses_without_panicking() {
    let error = parse_document(
        "docs/intents/µµ-x.md",
        &intent("001", "One", "approved"),
        &sha('b'),
    )
    .unwrap_err();

    assert_eq!(error.code(), "invalid_document");
    assert_eq!(error.offending_key, None);
}

#[test]
fn missing_or_duplicate_required_frontmatter_refuses() {
    let content_commit = sha('b');
    for contents in [
        "---\nid: 001\nstatus: approved\n---\n",
        "---\nid: 001\ntitle: One\nstatus: approved\nstatus: draft\n---\n",
    ] {
        let error =
            parse_document("docs/intents/001-one.md", contents, &content_commit).unwrap_err();
        assert_eq!(error.code(), "invalid_document");
        assert_eq!(error.offending_key.as_deref(), Some("intent:001"));
    }
}

#[test]
fn invalid_lifecycle_or_task_priority_refuses() {
    let content_commit = sha('b');
    for (path, contents, key) in [
        (
            "docs/intents/001-one.md",
            intent("001", "One", "unknown"),
            "intent:001",
        ),
        (
            "tasks/001-task.md",
            task("001", "Task", "todo", "4", "[]", "[]"),
            "task:001",
        ),
    ] {
        let error = parse_document(path, &contents, &content_commit).unwrap_err();
        assert_eq!(error.code(), "invalid_document");
        assert_eq!(error.offending_key.as_deref(), Some(key));
    }
}

#[test]
fn duplicate_id_within_one_kind_names_the_key_before_import() {
    let source_commit = sha('a');
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(CommandOutput::success(format!("{source_commit}\n"))),
        Ok(CommandOutput::success(
            "tasks/001-first.md\0tasks/001-second.md\0",
        )),
    ]);

    let error = load(&mut runner, "selected-ref").unwrap_err();

    assert_eq!(error.code(), "duplicate_document_id");
    assert_eq!(error.offending_key.as_deref(), Some("task:001"));
    assert_eq!(runner.commands().len(), 2);
    assert!(runner.finish().is_ok());
}

#[test]
fn missing_typed_target_names_the_source_key_before_import() {
    let content_commit = sha('b');
    let task = parse_document(
        "tasks/001-task.md",
        &task("001", "Task", "todo", "1", "[001]", "[]"),
        &content_commit,
    )
    .unwrap();

    let error = validate_document_targets(&[task]).unwrap_err();

    assert_eq!(error.code(), "missing_document_target");
    assert_eq!(error.offending_key.as_deref(), Some("task:001"));
}

#[test]
fn content_commit_mismatch_names_the_key_before_import() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let contents = intent("001", "One", "approved");
    let mut runner = RecordingCommandRunner::scripted(vec![
        Ok(CommandOutput::success(format!("{source_commit}\n"))),
        Ok(CommandOutput::success("docs/intents/001-one.md\0")),
        Ok(CommandOutput::success(&contents)),
        Ok(CommandOutput::success(format!("{content_commit}\n"))),
        Ok(CommandOutput::success(intent(
            "001",
            "Different",
            "approved",
        ))),
    ]);

    let error = load(&mut runner, "selected-ref").unwrap_err();

    assert_eq!(error.code(), "content_commit_mismatch");
    assert_eq!(error.offending_key.as_deref(), Some("intent:001"));
    assert_eq!(runner.commands().len(), 5);
    assert!(runner.finish().is_ok());
}

#[test]
fn source_git_plans_disable_lazy_fetch() {
    let source_commit = sha('a');
    let content_commit = sha('b');
    let contents = intent("001", "One", "approved");
    let mut runner = RecordingCommandRunner::scripted(source_outputs(
        &source_commit,
        &[("docs/intents/001-one.md", &contents, &content_commit)],
    ));

    load(&mut runner, "selected-ref").unwrap();

    for command in runner.commands() {
        assert_eq!(command.program, Path::new("git"));
        assert_eq!(command.cwd.as_deref(), Some(Path::new("/source-worktree")));
        assert_eq!(
            command
                .environment
                .get("GIT_NO_LAZY_FETCH")
                .map(String::as_str),
            Some("1")
        );
    }
    assert!(runner.finish().is_ok());
}
