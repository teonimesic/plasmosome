use plasmosome_work_state::document::parse_document;
use plasmosome_work_state::freshness::{
    ObservationState, PendingMutations, RemoteRelation, classify,
};
use plasmosome_work_state::read::{
    ReadCommand, blocked_tasks, list_documents, project_read, ready_tasks, render_human,
    show_document,
};
use plasmosome_work_state::shadow::{
    ActiveOwner, OperationalDocument, initial_operational_metadata,
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
    blocked_tasks(snapshot)
        .unwrap()
        .into_iter()
        .find(|task| task.document_key == "task:014")
        .unwrap()
        .blockers
        .into_iter()
        .map(|blocker| blocker.code)
        .collect()
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
}

#[test]
fn active_and_terminal_tasks_are_not_called_blocked() {
    let mut snapshot = snapshot();
    snapshot.documents[2].document.shadow.lifecycle = "in_progress".into();
    assert!(ready_tasks(&snapshot).unwrap().is_empty());
    assert!(blocked_tasks(&snapshot).unwrap().is_empty());
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
