use plasmosome_work_state::document::parse_document;
use plasmosome_work_state::freshness::{
    ObservationState, PendingMutations, RemoteRelation, classify,
};
use plasmosome_work_state::read::{
    ReadCommand, blocked_tasks, list_documents, project_read, ready_tasks, render_human,
    show_document,
};
use plasmosome_work_state::shadow::{
    ActiveOwner, OperationalDocument, OperationalMetadata, initial_operational_metadata,
};
use plasmosome_work_state::store::FencedSnapshot;

fn documents() -> Vec<OperationalDocument> {
    let source = "a".repeat(40);
    let markdown = vec![
        parse_document(
            "docs/intents/014-intent.md",
            "---\nid: 014\ntitle: Intent 014\nstatus: approved\n---\n",
            &source,
        )
        .unwrap(),
        parse_document(
            "docs/specs/014-spec.md",
            "---\nid: 014\ntitle: Spec 014\nstatus: accepted\nintents: [014]\n---\n",
            &source,
        )
        .unwrap(),
        parse_document(
            "tasks/014-task.md",
            "---\nid: 014\ntitle: Task 014\nstatus: planned\npriority: 1\nintents: [014]\nspecs: [014]\n---\n",
            &source,
        )
        .unwrap(),
    ];
    let operational = initial_operational_metadata(&markdown).unwrap();
    markdown
        .into_iter()
        .map(|document| OperationalDocument {
            operational: operational.get(&document.record.document_key).cloned(),
            document,
        })
        .collect()
}

fn snapshot() -> FencedSnapshot {
    FencedSnapshot {
        documents: documents(),
        freshness: classify(ObservationState {
            last_successful_sync_at: None,
            local_generation: "local-generation".into(),
            remote_generation: None,
            remote_observed_at: None,
            observed_local_generation: None,
            remote_relation: RemoteRelation::Unknown,
            pending_mutations: PendingMutations {
                operation_ids: Vec::new(),
            },
        })
        .unwrap(),
    }
}

#[test]
fn list_and_show_preserve_canonical_namespaces() {
    let snapshot = snapshot();

    let listed = list_documents(&snapshot).expect("valid snapshot lists every namespace");
    assert_eq!(
        listed
            .iter()
            .map(|document| document.document_key.as_str())
            .collect::<Vec<_>>(),
        vec!["intent:014", "spec:014", "task:014"]
    );
    assert_eq!(listed[2].priority, Some(1));
    assert_eq!(
        show_document(&snapshot, "spec:014").unwrap().title,
        "Spec 014"
    );
    assert_eq!(
        show_document(&snapshot, "014").unwrap_err().code(),
        "invalid_document_key"
    );
    assert_eq!(
        show_document(&snapshot, "task:999").unwrap_err().code(),
        "document_not_found"
    );
}

fn blocker_codes(snapshot: &FencedSnapshot) -> Vec<String> {
    blocker_codes_for(snapshot, "task:014")
}

fn blocker_codes_for(snapshot: &FencedSnapshot, key: &str) -> Vec<String> {
    blocker_details_for(snapshot, key)
        .into_iter()
        .map(|(code, _)| code)
        .collect()
}

fn blocker_details_for(snapshot: &FencedSnapshot, key: &str) -> Vec<(String, Option<String>)> {
    blocked_tasks(snapshot)
        .unwrap()
        .into_iter()
        .find(|task| task.document_key == key)
        .unwrap()
        .blockers
        .into_iter()
        .map(|blocker| (blocker.code, blocker.document_key))
        .collect()
}

fn multi_spec_snapshot() -> FencedSnapshot {
    let source = "a".repeat(40);
    let mut markdown = documents()
        .into_iter()
        .map(|document| document.document)
        .collect::<Vec<_>>();
    markdown.extend([
        parse_document(
            "docs/intents/015-intent.md",
            "---\nid: 015\ntitle: Intent 015\nstatus: approved\n---\n",
            &source,
        )
        .unwrap(),
        parse_document(
            "docs/specs/015-spec.md",
            "---\nid: 015\ntitle: Spec 015\nstatus: accepted\nintents: [014, 015]\n---\n",
            &source,
        )
        .unwrap(),
        parse_document(
            "tasks/015-task.md",
            "---\nid: 015\ntitle: Task 015\nstatus: planned\npriority: 2\nintents: [014, 015]\nspecs: [014, 015]\n---\n",
            &source,
        )
        .unwrap(),
    ]);
    let operational = initial_operational_metadata(&markdown).unwrap();
    FencedSnapshot {
        documents: markdown
            .into_iter()
            .map(|document| OperationalDocument {
                operational: operational.get(&document.record.document_key).cloned(),
                document,
            })
            .collect(),
        freshness: snapshot().freshness,
    }
}

#[test]
fn ready_requires_every_governance_gate() {
    let ready = ready_tasks(&snapshot()).unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].document_key, "task:014");
    assert!(!ready[0].authorizes_start);

    let mut todo = snapshot();
    todo.documents[2].document.shadow.lifecycle = "todo".into();
    assert_eq!(blocker_codes(&todo), vec!["task_not_planned"]);

    let mut owned = snapshot();
    owned.documents[2]
        .operational
        .as_mut()
        .unwrap()
        .active_owner = Some(ActiveOwner {
        actor: "agent".into(),
        session_id: "session".into(),
        ownership_token: "token".into(),
        claim_operation_id: "operation".into(),
        acquired_at: "2026-09-02T12:00:00Z".into(),
        expires_at: "2026-09-02T13:00:00Z".into(),
    });
    assert_eq!(blocker_codes(&owned), vec!["live_owner"]);

    let mut invalid_links = snapshot();
    invalid_links.documents[1].document.shadow.lifecycle = "draft".into();
    invalid_links.documents[2]
        .document
        .record
        .intent_ids
        .clear();
    invalid_links.documents[0].document.shadow.lifecycle = "draft".into();
    assert_eq!(
        blocker_codes(&invalid_links),
        vec![
            "spec_not_accepted",
            "intent_closure_mismatch",
            "intent_not_approved",
        ]
    );

    let mut missing = snapshot();
    missing.documents[2].document.record.spec_ids.clear();
    missing.documents[2].document.record.intent_ids.clear();
    assert_eq!(
        blocker_codes(&missing),
        vec!["missing_spec_links", "missing_intent_links"]
    );

    let multi_spec = multi_spec_snapshot();
    assert_eq!(
        ready_tasks(&multi_spec)
            .unwrap()
            .into_iter()
            .map(|task| task.document_key)
            .collect::<Vec<_>>(),
        vec!["task:014", "task:015"],
        "the ordered union of every copied spec intent closes exactly"
    );

    let mut dependency = multi_spec.clone();
    dependency
        .documents
        .iter_mut()
        .find(|document| document.document.record.document_key == "task:014")
        .unwrap()
        .document
        .shadow
        .lifecycle = "todo".into();
    dependency
        .documents
        .iter_mut()
        .find(|document| document.document.record.document_key == "task:015")
        .unwrap()
        .operational
        .as_mut()
        .unwrap()
        .task_dependencies
        .extend(["task:015".into(), "task:014".into()]);
    assert_eq!(
        blocker_details_for(&dependency, "task:015"),
        vec![
            ("dependency_not_done".into(), Some("task:015".into())),
            ("dependency_not_done".into(), Some("task:014".into())),
        ]
    );

    for intent_ids in [
        vec!["014".to_owned(), "015".to_owned(), "015".to_owned()],
        vec!["014".to_owned()],
        vec!["015".to_owned(), "014".to_owned()],
    ] {
        let mut closure = multi_spec.clone();
        closure
            .documents
            .iter_mut()
            .find(|document| document.document.record.document_key == "task:015")
            .unwrap()
            .document
            .record
            .intent_ids = intent_ids;
        assert_eq!(
            blocker_codes_for(&closure, "task:015"),
            vec!["intent_closure_mismatch"]
        );
    }
}

#[test]
fn active_and_terminal_tasks_are_not_called_blocked() {
    for lifecycle in ["in_progress", "in_review", "done"] {
        let mut snapshot = snapshot();
        snapshot.documents[2].document.shadow.lifecycle = lifecycle.into();
        assert!(ready_tasks(&snapshot).unwrap().is_empty(), "{lifecycle}");
        assert!(blocked_tasks(&snapshot).unwrap().is_empty(), "{lifecycle}");
    }
}

#[test]
fn human_and_json_reads_carry_the_same_envelope() {
    let mut snapshot = snapshot();
    snapshot.freshness = classify(ObservationState {
        last_successful_sync_at: Some("2026-09-02T12:00:00Z".into()),
        local_generation: "local-generation".into(),
        remote_generation: Some("a".repeat(40)),
        remote_observed_at: Some("2026-09-02T12:00:00Z".into()),
        observed_local_generation: Some("local-generation".into()),
        remote_relation: RemoteRelation::Equivalent,
        pending_mutations: PendingMutations {
            operation_ids: Vec::new(),
        },
    })
    .unwrap();
    for command in [
        ReadCommand::List,
        ReadCommand::Show("task:014".into()),
        ReadCommand::Ready,
        ReadCommand::Blocked,
    ] {
        let response = project_read(command, &snapshot, "markdown-shadow", "a-source").unwrap();
        let value = serde_json::to_value(&response).unwrap();
        for field in ["command", "authority_mode", "source_commit", "freshness"] {
            assert!(value.get(field).is_some(), "missing {field}");
        }
        assert_eq!(value["freshness"]["pending_mutations"]["count"], 0);
        let human = render_human(&response);
        assert!(human.contains("synchronized as of 2026-09-02T12:00:00Z"));
        assert!(!human.contains("current"));
        assert!(!human.contains("up to date"));
    }

    snapshot.freshness = classify(ObservationState {
        last_successful_sync_at: Some("2026-09-02T11:00:00Z".into()),
        local_generation: "local-generation".into(),
        remote_generation: Some("b".repeat(40)),
        remote_observed_at: Some("2026-09-02T12:00:00Z".into()),
        observed_local_generation: Some("another-generation".into()),
        remote_relation: RemoteRelation::Unknown,
        pending_mutations: PendingMutations {
            operation_ids: vec!["operation-046".into()],
        },
    })
    .unwrap();
    let response =
        project_read(ReadCommand::List, &snapshot, "markdown-shadow", "a-source").unwrap();
    let human = render_human(&response);
    assert!(
        human.contains("last successful sync at: 2026-09-02T11:00:00Z"),
        "a non-synchronized freshness state must retain its persisted last successful sync fact"
    );
}

#[test]
fn human_reads_render_complete_payloads_and_related_blocker_keys() {
    let source = "a".repeat(40);
    let snapshot = snapshot();
    let list = project_read(ReadCommand::List, &snapshot, "markdown-shadow", "a-source").unwrap();
    let list_human = render_human(&list);
    assert!(
        list_human
            .contains("task:014 kind=task id=014 lifecycle=planned priority=1 title=Task 014")
    );

    let shown = project_read(
        ReadCommand::Show("task:014".into()),
        &snapshot,
        "markdown-shadow",
        "a-source",
    )
    .unwrap();
    let shown_human = render_human(&shown);
    for expected in [
        "document key: task:014".to_owned(),
        "kind: task".to_owned(),
        "document id: 014".to_owned(),
        "document path: tasks/014-task.md".to_owned(),
        "title: Task 014".to_owned(),
        format!("content commit: {source}"),
        "state version: 1".to_owned(),
        "intent ids: [014]".to_owned(),
        "spec ids: [014]".to_owned(),
        "lifecycle: planned".to_owned(),
        "priority: 1".to_owned(),
        "pr: unknown".to_owned(),
        "evidence: unknown".to_owned(),
        "operational: {\"schema_version\":1".to_owned(),
    ] {
        assert!(
            shown_human.contains(&expected),
            "missing {expected} from {shown_human}"
        );
    }

    let mut blocked_snapshot = snapshot;
    blocked_snapshot.documents[2].document.shadow.lifecycle = "todo".into();
    let dependent = parse_document(
        "tasks/015-dependent.md",
        "---\nid: 015\ntitle: Dependent Task\nstatus: planned\npriority: 2\nintents: [014]\nspecs: [014]\n---\n",
        &source,
    )
    .unwrap();
    blocked_snapshot.documents.push(OperationalDocument {
        document: dependent,
        operational: Some(OperationalMetadata {
            schema_version: 1,
            active_owner: None,
            task_dependencies: vec!["task:014".into()],
        }),
    });
    let blocked = project_read(
        ReadCommand::Blocked,
        &blocked_snapshot,
        "markdown-shadow",
        "a-source",
    )
    .unwrap();
    assert!(render_human(&blocked).contains("dependency_not_done (task:014)"));
}

#[test]
fn empty_readiness_output_still_disclaims_start_authority() {
    let response = project_read(
        ReadCommand::Blocked,
        &snapshot(),
        "markdown-shadow",
        "a-source",
    )
    .unwrap();
    assert!(response.blocked.as_ref().is_some_and(Vec::is_empty));
    assert!(
        render_human(&response).contains("local projection; does not authorize start"),
        "the empty blocked projection must still state its non-authority"
    );
}
