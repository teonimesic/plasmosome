use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::document::DocumentKind;
use crate::shadow::{OperationalDocument, OperationalMetadata};
use crate::store::FencedSnapshot;

/// A stable refusal while projecting one verified local snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    code: &'static str,
    /// The kind-qualified key associated with the refusal, when it is safe to expose.
    pub document_key: Option<String>,
}

impl ReadError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for ReadError {}

fn refusal(code: &'static str, document_key: Option<String>) -> ReadError {
    ReadError { code, document_key }
}

/// One canonical lightweight document row returned by `list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListedDocument {
    /// The immutable kind-qualified document key.
    pub document_key: String,
    /// The document namespace.
    pub kind: DocumentKind,
    /// The three-digit namespace-local id.
    pub document_id: String,
    /// The Markdown title.
    pub title: String,
    /// The Markdown-owned lifecycle.
    pub lifecycle: String,
    /// The task priority, absent for intents and specs.
    pub priority: Option<u8>,
}

/// The complete stored Markdown projection for one exact kind-qualified key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShownDocument {
    /// The immutable kind-qualified document key.
    pub document_key: String,
    /// The document namespace.
    pub kind: DocumentKind,
    /// The three-digit namespace-local id.
    pub document_id: String,
    /// The canonical Markdown-relative path.
    pub document_path: String,
    /// The Markdown title.
    pub title: String,
    /// The content-establishing Git commit.
    pub content_commit_sha: String,
    /// The persisted document state version.
    pub state_version: u64,
    /// Ordered copied intent ids.
    pub intent_ids: Vec<String>,
    /// Ordered copied spec ids.
    pub spec_ids: Vec<String>,
    /// The Markdown-owned lifecycle.
    pub lifecycle: String,
    /// The task priority, absent for intents and specs.
    pub priority: Option<u8>,
    /// The stored Markdown pull-request value.
    pub pr: Option<String>,
    /// The stored Markdown evidence value.
    pub evidence: Option<String>,
    /// The task-only operational sibling.
    pub operational: Option<OperationalMetadata>,
}

/// One deterministic reason why a task is not locally ready.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReadinessBlocker {
    /// The stable blocker category.
    pub code: String,
    /// The related document key when the category identifies one.
    pub document_key: Option<String>,
}

/// One task returned by `ready` or `blocked`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReadinessTask {
    /// The immutable task key.
    pub document_key: String,
    /// The Markdown task title.
    pub title: String,
    /// The deterministic complete blocker list.
    pub blockers: Vec<ReadinessBlocker>,
    /// Local projection answers never authorize starting work.
    pub authorizes_start: bool,
}

/// The four read-only wrapper projections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadCommand {
    /// Lists every canonical document.
    List,
    /// Shows one exact kind-qualified document key.
    Show(String),
    /// Lists locally ready planned tasks.
    Ready,
    /// Lists locally blocked todo or planned tasks.
    Blocked,
}

/// One complete local read response, including its fixed freshness envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReadResponse {
    /// The requested projection name.
    pub command: String,
    /// The installed Beads authority mode.
    pub authority_mode: String,
    /// The Markdown source commit stored by bootstrap.
    pub source_commit: String,
    /// The complete persisted local freshness envelope.
    pub freshness: crate::freshness::FreshnessEnvelope,
    /// The list payload, when `command` is `list`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<ListedDocument>>,
    /// The show payload, when `command` is `show`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<ShownDocument>,
    /// The ready payload, when `command` is `ready`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<Vec<ReadinessTask>>,
    /// The blocked payload, when `command` is `blocked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Vec<ReadinessTask>>,
}

fn valid_key(value: &str) -> bool {
    let Some((kind, id)) = value.split_once(':') else {
        return false;
    };
    matches!(kind, "intent" | "spec" | "task")
        && id.len() == 3
        && id.bytes().all(|byte| byte.is_ascii_digit())
        && value.matches(':').count() == 1
}

fn validated_documents(snapshot: &FencedSnapshot) -> Result<Vec<&OperationalDocument>, ReadError> {
    let mut keys = BTreeMap::new();
    for document in &snapshot.documents {
        let key = document.document.record.document_key.clone();
        if !valid_key(&key) || keys.insert(key.clone(), document).is_some() {
            return Err(refusal("invalid_store", Some(key)));
        }
        match document.document.record.kind {
            DocumentKind::Task if document.operational.is_none() => {
                return Err(refusal("invalid_store", Some(key)));
            }
            DocumentKind::Intent | DocumentKind::Spec if document.operational.is_some() => {
                return Err(refusal("invalid_store", Some(key)));
            }
            DocumentKind::Task | DocumentKind::Intent | DocumentKind::Spec => {}
        }
    }
    let mut documents = keys.into_values().collect::<Vec<_>>();
    documents.sort_by(|left, right| {
        left.document
            .record
            .document_key
            .cmp(&right.document.record.document_key)
    });
    Ok(documents)
}

fn shown(document: &OperationalDocument) -> ShownDocument {
    let record = &document.document.record;
    let shadow = &document.document.shadow;
    ShownDocument {
        document_key: record.document_key.clone(),
        kind: record.kind.clone(),
        document_id: record.document_id.clone(),
        document_path: record.document_path.clone(),
        title: record.title.clone(),
        content_commit_sha: record.content_commit_sha.clone(),
        state_version: record.state_version,
        intent_ids: record.intent_ids.clone(),
        spec_ids: record.spec_ids.clone(),
        lifecycle: shadow.lifecycle.clone(),
        priority: shadow.priority,
        pr: shadow.pr.clone(),
        evidence: shadow.evidence.clone(),
        operational: document.operational.clone(),
    }
}

/// Lists all verified snapshot documents in canonical kind and numeric-id order.
pub fn list_documents(snapshot: &FencedSnapshot) -> Result<Vec<ListedDocument>, ReadError> {
    validated_documents(snapshot).map(|documents| {
        documents
            .into_iter()
            .map(|document| {
                let record = &document.document.record;
                let shadow = &document.document.shadow;
                ListedDocument {
                    document_key: record.document_key.clone(),
                    kind: record.kind.clone(),
                    document_id: record.document_id.clone(),
                    title: record.title.clone(),
                    lifecycle: shadow.lifecycle.clone(),
                    priority: shadow.priority,
                }
            })
            .collect()
    })
}

/// Returns the complete stored projection for one exact namespace-qualified key.
pub fn show_document(snapshot: &FencedSnapshot, key: &str) -> Result<ShownDocument, ReadError> {
    if !valid_key(key) {
        return Err(refusal("invalid_document_key", None));
    }
    let documents = validated_documents(snapshot)?;
    documents
        .into_iter()
        .find(|document| document.document.record.document_key == key)
        .map(shown)
        .ok_or_else(|| refusal("document_not_found", Some(key.to_owned())))
}

fn blocker(code: &str, document_key: Option<String>) -> ReadinessBlocker {
    ReadinessBlocker {
        code: code.to_owned(),
        document_key,
    }
}

fn indexed_documents(
    snapshot: &FencedSnapshot,
) -> Result<BTreeMap<&str, &OperationalDocument>, ReadError> {
    let documents = validated_documents(snapshot)?;
    Ok(documents
        .into_iter()
        .map(|document| (document.document.record.document_key.as_str(), document))
        .collect())
}

fn required_document<'a>(
    documents: &BTreeMap<&str, &'a OperationalDocument>,
    key: &str,
    kind: DocumentKind,
) -> Result<&'a OperationalDocument, ReadError> {
    let document = documents
        .get(key)
        .copied()
        .ok_or_else(|| refusal("invalid_store", Some(key.to_owned())))?;
    if document.document.record.kind != kind {
        return Err(refusal("invalid_store", Some(key.to_owned())));
    }
    Ok(document)
}

fn task_blockers(
    task: &OperationalDocument,
    documents: &BTreeMap<&str, &OperationalDocument>,
) -> Result<Vec<ReadinessBlocker>, ReadError> {
    let record = &task.document.record;
    let mut blockers = Vec::new();
    if task.document.shadow.lifecycle == "todo" {
        blockers.push(blocker("task_not_planned", None));
    }
    let operational = task
        .operational
        .as_ref()
        .ok_or_else(|| refusal("invalid_store", Some(record.document_key.clone())))?;
    if operational.active_owner.is_some() {
        blockers.push(blocker("live_owner", None));
    }
    let mut dependency_keys = BTreeSet::new();
    for dependency in &operational.task_dependencies {
        if !dependency_keys.insert(dependency.as_str()) {
            return Err(refusal("invalid_store", Some(record.document_key.clone())));
        }
        let dependent = required_document(documents, dependency, DocumentKind::Task)?;
        if dependent.document.shadow.lifecycle != "done" {
            blockers.push(blocker("dependency_not_done", Some(dependency.clone())));
        }
    }
    if record.spec_ids.is_empty() {
        blockers.push(blocker("missing_spec_links", None));
    }
    let mut expected_intents = Vec::new();
    let mut expected_seen = BTreeSet::new();
    for spec_id in &record.spec_ids {
        let spec_key = format!("spec:{spec_id}");
        let spec = required_document(documents, &spec_key, DocumentKind::Spec)?;
        if spec.document.shadow.lifecycle != "accepted" {
            blockers.push(blocker("spec_not_accepted", Some(spec_key)));
        }
        for intent_id in &spec.document.record.intent_ids {
            if expected_seen.insert(intent_id.as_str()) {
                expected_intents.push(intent_id.clone());
            }
        }
    }
    if record.intent_ids != expected_intents {
        blockers.push(blocker("intent_closure_mismatch", None));
    }
    if expected_intents.is_empty() {
        blockers.push(blocker("missing_intent_links", None));
    }
    let mut intent_ids = Vec::new();
    let mut intent_seen = BTreeSet::new();
    for intent_id in expected_intents.iter().chain(record.intent_ids.iter()) {
        if intent_seen.insert(intent_id.as_str()) {
            intent_ids.push(intent_id);
        }
    }
    for intent_id in intent_ids {
        let intent_key = format!("intent:{intent_id}");
        let intent = required_document(documents, &intent_key, DocumentKind::Intent)?;
        if intent.document.shadow.lifecycle != "approved" {
            blockers.push(blocker("intent_not_approved", Some(intent_key)));
        }
    }
    Ok(blockers)
}

fn task_projections(snapshot: &FencedSnapshot) -> Result<Vec<ReadinessTask>, ReadError> {
    let documents = indexed_documents(snapshot)?;
    let mut tasks = documents
        .values()
        .copied()
        .filter(|document| matches!(document.document.record.kind, DocumentKind::Task))
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        left.document
            .record
            .document_key
            .cmp(&right.document.record.document_key)
    });
    tasks
        .into_iter()
        .filter(|task| matches!(task.document.shadow.lifecycle.as_str(), "todo" | "planned"))
        .map(|task| {
            Ok(ReadinessTask {
                document_key: task.document.record.document_key.clone(),
                title: task.document.record.title.clone(),
                blockers: task_blockers(task, &documents)?,
                authorizes_start: false,
            })
        })
        .collect()
}

/// Returns every planned task whose complete local governance projection has no blocker.
pub fn ready_tasks(snapshot: &FencedSnapshot) -> Result<Vec<ReadinessTask>, ReadError> {
    task_projections(snapshot).map(|tasks| {
        tasks
            .into_iter()
            .filter(|task| task.blockers.is_empty())
            .collect()
    })
}

/// Returns every todo or planned task with its complete deterministic blocker list.
pub fn blocked_tasks(snapshot: &FencedSnapshot) -> Result<Vec<ReadinessTask>, ReadError> {
    task_projections(snapshot).map(|tasks| {
        tasks
            .into_iter()
            .filter(|task| !task.blockers.is_empty())
            .collect()
    })
}

/// Projects one complete local read response without opening a network connection or clock.
pub fn project_read(
    command: ReadCommand,
    snapshot: &FencedSnapshot,
    authority_mode: &str,
    source_commit: &str,
) -> Result<ReadResponse, ReadError> {
    let mut response = ReadResponse {
        command: match &command {
            ReadCommand::List => "list",
            ReadCommand::Show(_) => "show",
            ReadCommand::Ready => "ready",
            ReadCommand::Blocked => "blocked",
        }
        .to_owned(),
        authority_mode: authority_mode.to_owned(),
        source_commit: source_commit.to_owned(),
        freshness: snapshot.freshness.clone(),
        documents: None,
        document: None,
        ready: None,
        blocked: None,
    };
    match command {
        ReadCommand::List => response.documents = Some(list_documents(snapshot)?),
        ReadCommand::Show(key) => response.document = Some(show_document(snapshot, &key)?),
        ReadCommand::Ready => response.ready = Some(ready_tasks(snapshot)?),
        ReadCommand::Blocked => response.blocked = Some(blocked_tasks(snapshot)?),
    }
    Ok(response)
}

fn displayed_option(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("unknown")
}

fn freshness_name(freshness: &crate::freshness::Freshness) -> &'static str {
    match freshness {
        crate::freshness::Freshness::SynchronizedAsOf => "synchronized_as_of",
        crate::freshness::Freshness::Stale => "stale",
        crate::freshness::Freshness::Unknown => "unknown",
        crate::freshness::Freshness::Unpublished => "unpublished",
        crate::freshness::Freshness::StaleWithUnpublished => "stale_with_unpublished",
        crate::freshness::Freshness::UnknownWithUnpublished => "unknown_with_unpublished",
    }
}

/// Renders one response without representing any freshness value as current or up to date.
pub fn render_human(response: &ReadResponse) -> String {
    let freshness = &response.freshness;
    let freshness_line = match freshness.freshness {
        crate::freshness::Freshness::SynchronizedAsOf => format!(
            "freshness: synchronized as of {}",
            displayed_option(&freshness.last_successful_sync_at)
        ),
        _ => format!("freshness: {}", freshness_name(&freshness.freshness)),
    };
    let mut lines = vec![
        format!("command: {}", response.command),
        format!("authority mode: {}", response.authority_mode),
        format!("source commit: {}", response.source_commit),
        format!("local generation: {}", freshness.local_generation),
        format!(
            "remote generation: {}",
            displayed_option(&freshness.remote_generation)
        ),
        format!(
            "remote observed at: {}",
            displayed_option(&freshness.remote_observed_at)
        ),
        format!(
            "pending mutations: {} [{}]",
            freshness.pending_mutations.count,
            freshness.pending_mutations.operation_ids.join(", ")
        ),
        freshness_line,
    ];
    if let Some(documents) = &response.documents {
        lines.extend(documents.iter().map(|document| {
            format!(
                "{} {} {}",
                document.document_key, document.lifecycle, document.title
            )
        }));
    }
    if let Some(document) = &response.document {
        lines.push(format!("{} {}", document.document_key, document.title));
    }
    if response.ready.is_some() || response.blocked.is_some() {
        lines.push("local projection; does not authorize start".into());
    }
    for tasks in [&response.ready, &response.blocked].into_iter().flatten() {
        lines.extend(tasks.iter().map(|task| {
            let blockers = task
                .blockers
                .iter()
                .map(|blocker| blocker.code.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{} {} local projection; does not authorize start{}",
                task.document_key,
                task.title,
                if !blockers.is_empty() {
                    format!("; blockers: {blockers}")
                } else {
                    String::new()
                }
            )
        }));
    }
    format!("{}\n", lines.join("\n"))
}
