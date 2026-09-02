use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::document::{
    DocumentError, DocumentKind, DocumentRecord, MarkdownShadow, ShadowDocument, is_document_id,
    is_lower_hex_sha, valid_lifecycle, validate_document_targets,
};
use crate::freshness::canonical_utc;

/// A refusal raised while encoding, importing, decoding, or comparing a shadow store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowError {
    code: &'static str,
    /// The first canonical logical key responsible for the refusal, when known.
    pub offending_key: Option<String>,
    /// The complete-set comparison category, when the refusal is a parity mismatch.
    pub mismatch: Option<String>,
}

impl ShadowError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for ShadowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for ShadowError {}

/// A disposable Beads store that receives one Markdown-shadow import.
#[derive(Clone, Debug)]
pub struct ShadowStore {
    /// The non-path label used in contract evidence.
    pub label: String,
    /// The disposable root that owns import JSONL files.
    pub temporary_root: PathBuf,
    /// The isolated fixture repository used as the Beads working directory.
    pub repository: PathBuf,
    /// The complete explicit environment for every Beads command.
    pub environment: BTreeMap<String, String>,
    /// The verified Beads binary supplied by the caller.
    pub binary: PathBuf,
}

impl ShadowStore {
    /// Builds a store descriptor without opening or mutating the store.
    pub fn new(
        label: impl Into<String>,
        temporary_root: PathBuf,
        repository: PathBuf,
        environment: BTreeMap<String, String>,
        binary: PathBuf,
    ) -> Self {
        Self {
            label: label.into(),
            temporary_root,
            repository,
            environment,
            binary,
        }
    }
}

/// Evidence returned after one real Beads import, export, and project-key verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowStoreImport {
    /// The non-path store label.
    pub label: String,
    /// Documents decoded from the store's Beads export.
    pub documents: Vec<ShadowDocument>,
    /// Redacted command plans run for this store.
    pub command_plans: Vec<String>,
}

/// Evidence returned after an import that includes task operational siblings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationalShadowStoreImport {
    /// The non-path label used in contract evidence.
    pub label: String,
    /// Documents and strict task operational siblings decoded from the Beads export.
    pub documents: Vec<OperationalDocument>,
    /// Redacted command plans run for this store.
    pub command_plans: Vec<String>,
}

/// A recorded task owner that remains live for an offline projection.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveOwner {
    /// The actor that holds the published task ownership record.
    pub actor: String,
    /// The agent session recorded with that ownership record.
    pub session_id: String,
    /// The ownership token needed by a future writer protocol.
    pub ownership_token: String,
    /// The semantic claim operation id.
    pub claim_operation_id: String,
    /// The canonical UTC acquisition time.
    pub acquired_at: String,
    /// The canonical UTC expiry time retained as an audit fact, not a local expiration rule.
    pub expires_at: String,
}

/// The Beads-backed operational sibling attached only to imported task rows.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalMetadata {
    /// The operational sibling schema version.
    pub schema_version: u64,
    /// The active owner, when a future writer has recorded one.
    pub active_owner: Option<ActiveOwner>,
    /// Ordered task keys that must be done before this task is locally ready.
    pub task_dependencies: Vec<String>,
}

/// A document decoded with its task-only Beads operational sibling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationalDocument {
    /// The immutable Markdown shadow document.
    pub document: ShadowDocument,
    /// The task operational sibling, absent for intents and specs.
    pub operational: Option<OperationalMetadata>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShadowMetadata {
    schema_version: u64,
    authority_mode: String,
    document_key: String,
    kind: DocumentKind,
    document_id: String,
    document_path: String,
    title: String,
    content_commit_sha: String,
    state_version: u64,
    intent_ids: Vec<String>,
    spec_ids: Vec<String>,
    lifecycle: String,
    priority: Option<u8>,
    pr: Option<String>,
    evidence: Option<String>,
}

fn refusal(code: &'static str, offending_key: Option<String>) -> ShadowError {
    ShadowError {
        code,
        offending_key,
        mismatch: None,
    }
}

fn comparison_refusal(code: &'static str, key: &str, mismatch: &str) -> ShadowError {
    ShadowError {
        code,
        offending_key: Some(key.to_owned()),
        mismatch: Some(mismatch.to_owned()),
    }
}

fn from_document_error(error: DocumentError) -> ShadowError {
    ShadowError {
        code: error.code(),
        offending_key: error.offending_key,
        mismatch: None,
    }
}

fn canonical_path(record: &DocumentRecord) -> bool {
    let Some(suffix) = record.document_path.strip_prefix(record.kind.directory()) else {
        return false;
    };
    let prefix = format!("{}-", record.document_id);
    suffix.starts_with(&prefix)
        && suffix.ends_with(".md")
        && suffix.len() > prefix.len() + 3
        && !suffix.contains('/')
}

fn validate_document(document: &ShadowDocument) -> Result<(), ShadowError> {
    let record = &document.record;
    let key = Some(record.document_key.clone());
    if !is_document_id(&record.document_id)
        || record.document_key != format!("{}:{}", record.kind.namespace(), record.document_id)
        || !canonical_path(record)
        || record.title.trim().is_empty()
        || !is_lower_hex_sha(&record.content_commit_sha)
        || record.state_version != 1
        || record.intent_ids.iter().any(|id| !is_document_id(id))
        || record.spec_ids.iter().any(|id| !is_document_id(id))
        || !valid_lifecycle(&record.kind, &document.shadow.lifecycle)
    {
        return Err(refusal("invalid_document", key));
    }
    match record.kind {
        DocumentKind::Intent => {
            if !record.intent_ids.is_empty()
                || !record.spec_ids.is_empty()
                || document.shadow.priority.is_some()
                || document.shadow.pr.is_some()
                || document.shadow.evidence.is_some()
            {
                return Err(refusal(
                    "invalid_document",
                    Some(record.document_key.clone()),
                ));
            }
        }
        DocumentKind::Spec => {
            if !record.spec_ids.is_empty()
                || document.shadow.priority.is_some()
                || document.shadow.pr.is_some()
                || document.shadow.evidence.is_some()
            {
                return Err(refusal(
                    "invalid_document",
                    Some(record.document_key.clone()),
                ));
            }
        }
        DocumentKind::Task => {
            if !matches!(document.shadow.priority, Some(1..=3))
                || document
                    .shadow
                    .pr
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                || document
                    .shadow
                    .evidence
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
            {
                return Err(refusal(
                    "invalid_document",
                    Some(record.document_key.clone()),
                ));
            }
        }
    }
    Ok(())
}

fn validate_documents(documents: &[ShadowDocument]) -> Result<(), ShadowError> {
    let mut keys = BTreeSet::new();
    for document in documents {
        validate_document(document)?;
        if !keys.insert(document.record.document_key.clone()) {
            return Err(refusal(
                "invalid_document",
                Some(document.record.document_key.clone()),
            ));
        }
    }
    validate_document_targets(documents).map_err(from_document_error)
}

fn invalid_store() -> ShadowError {
    refusal("invalid_store", None)
}

fn valid_task_key(value: &str) -> bool {
    value.strip_prefix("task:").is_some_and(is_document_id)
}

fn validate_operational_metadata(metadata: &OperationalMetadata) -> Result<(), ShadowError> {
    if metadata.schema_version != 1 {
        return Err(invalid_store());
    }
    if let Some(owner) = &metadata.active_owner
        && (owner.actor.trim().is_empty()
            || owner.session_id.trim().is_empty()
            || owner.ownership_token.trim().is_empty()
            || owner.claim_operation_id.trim().is_empty()
            || !canonical_utc(&owner.acquired_at)
            || !canonical_utc(&owner.expires_at))
    {
        return Err(invalid_store());
    }
    let mut dependencies = BTreeSet::new();
    if metadata
        .task_dependencies
        .iter()
        .any(|dependency| !valid_task_key(dependency) || !dependencies.insert(dependency))
    {
        return Err(invalid_store());
    }
    Ok(())
}

fn validate_operational_documents(documents: &[OperationalDocument]) -> Result<(), ShadowError> {
    let shadow_documents = documents
        .iter()
        .map(|document| document.document.clone())
        .collect::<Vec<_>>();
    validate_documents(&shadow_documents)?;
    let task_keys = documents
        .iter()
        .filter(|document| matches!(document.document.record.kind, DocumentKind::Task))
        .map(|document| document.document.record.document_key.as_str())
        .collect::<BTreeSet<_>>();
    for document in documents {
        validate_document(&document.document)?;
        match document.document.record.kind {
            DocumentKind::Task => {
                let metadata = document.operational.as_ref().ok_or_else(invalid_store)?;
                validate_operational_metadata(metadata)?;
                if metadata
                    .task_dependencies
                    .iter()
                    .any(|dependency| !task_keys.contains(dependency.as_str()))
                {
                    return Err(invalid_store());
                }
            }
            DocumentKind::Intent | DocumentKind::Spec if document.operational.is_some() => {
                return Err(invalid_store());
            }
            DocumentKind::Intent | DocumentKind::Spec => {}
        }
    }
    Ok(())
}

/// Creates the empty task operational siblings used by a first Markdown-shadow bootstrap.
pub fn initial_operational_metadata(
    documents: &[ShadowDocument],
) -> Result<BTreeMap<String, OperationalMetadata>, ShadowError> {
    validate_documents(documents)?;
    Ok(documents
        .iter()
        .filter(|document| matches!(document.record.kind, DocumentKind::Task))
        .map(|document| {
            (
                document.record.document_key.clone(),
                OperationalMetadata {
                    schema_version: 1,
                    active_owner: None,
                    task_dependencies: Vec::new(),
                },
            )
        })
        .collect())
}

/// Returns the stable Beads-native id for one logical record.
pub fn native_id(record: &DocumentRecord) -> String {
    format!(
        "plasmosome-{}{}",
        record.kind.namespace(),
        record.document_id
    )
}

fn effective_priority(document: &ShadowDocument) -> u8 {
    document.shadow.priority.unwrap_or(2)
}

fn shadow_row(document: &ShadowDocument) -> Value {
    let record = &document.record;
    let shadow = &document.shadow;
    json!({
        "id": native_id(record),
        "title": record.title,
        "description": "",
        "status": "open",
        "priority": effective_priority(document),
        "external_ref": record.document_key,
        "metadata": {
            "plasmosome_document": {
                "schema_version": 1,
                "authority_mode": "markdown-shadow",
                "document_key": record.document_key,
                "kind": record.kind,
                "document_id": record.document_id,
                "document_path": record.document_path,
                "title": record.title,
                "content_commit_sha": record.content_commit_sha,
                "state_version": record.state_version,
                "intent_ids": record.intent_ids,
                "spec_ids": record.spec_ids,
                "lifecycle": shadow.lifecycle,
                "priority": shadow.priority,
                "pr": shadow.pr,
                "evidence": shadow.evidence,
            }
        }
    })
}

fn operational_shadow_row(
    document: &ShadowDocument,
    operational: Option<&OperationalMetadata>,
) -> Result<Value, ShadowError> {
    let mut row = shadow_row(document);
    if let Some(operational) = operational {
        let metadata = row
            .get_mut("metadata")
            .and_then(Value::as_object_mut)
            .ok_or_else(invalid_store)?;
        metadata.insert(
            "plasmosome_operational".to_owned(),
            serde_json::to_value(operational).map_err(|_| invalid_store())?,
        );
    }
    Ok(row)
}

/// Serializes validated shadow documents as Beads-importable JSON Lines.
pub fn to_beads_jsonl(documents: &[ShadowDocument]) -> Result<String, ShadowError> {
    validate_documents(documents)?;
    documents
        .iter()
        .map(shadow_row)
        .map(|row| serde_json::to_string(&row).map_err(|_| refusal("invalid_document", None)))
        .collect::<Result<Vec<_>, _>>()
        .map(|rows| rows.join("\n"))
        .map(|rows| {
            if rows.is_empty() {
                rows
            } else {
                format!("{rows}\n")
            }
        })
}

/// Serializes Markdown documents with their strict task-only operational siblings.
pub fn to_operational_beads_jsonl(
    documents: &[ShadowDocument],
    operational: &BTreeMap<String, OperationalMetadata>,
) -> Result<String, ShadowError> {
    validate_documents(documents)?;
    let operational_documents = documents
        .iter()
        .map(|document| OperationalDocument {
            document: document.clone(),
            operational: operational.get(&document.record.document_key).cloned(),
        })
        .collect::<Vec<_>>();
    let task_keys = documents
        .iter()
        .filter(|document| matches!(document.record.kind, DocumentKind::Task))
        .map(|document| document.record.document_key.as_str())
        .collect::<BTreeSet<_>>();
    if operational
        .keys()
        .any(|key| !task_keys.contains(key.as_str()))
    {
        return Err(invalid_store());
    }
    validate_operational_documents(&operational_documents)?;
    operational_documents
        .iter()
        .map(|document| operational_shadow_row(&document.document, document.operational.as_ref()))
        .map(|row| {
            row.and_then(|row| {
                serde_json::to_string(&row).map_err(|_| refusal("invalid_document", None))
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|rows| rows.join("\n"))
        .map(|rows| {
            if rows.is_empty() {
                rows
            } else {
                format!("{rows}\n")
            }
        })
}

fn metadata_key(value: &Value) -> Option<String> {
    value
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("plasmosome_document"))
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("document_key"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn outer_string<'a>(
    value: &'a Value,
    field: &str,
    key: &Option<String>,
) -> Result<&'a str, ShadowError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| refusal("invalid_document", key.clone()))
}

fn outer_priority(value: &Value, key: &Option<String>) -> Result<u8, ShadowError> {
    value
        .get("priority")
        .and_then(Value::as_u64)
        .and_then(|priority| u8::try_from(priority).ok())
        .ok_or_else(|| refusal("invalid_document", key.clone()))
}

fn decode_row(value: &Value) -> Result<(String, ShadowDocument), ShadowError> {
    let key = metadata_key(value);
    let nested = value
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("plasmosome_document"))
        .cloned()
        .ok_or_else(|| refusal("invalid_document", key.clone()))?;
    let metadata: ShadowMetadata =
        serde_json::from_value(nested).map_err(|_| refusal("invalid_document", key.clone()))?;
    let document = ShadowDocument {
        record: DocumentRecord {
            document_key: metadata.document_key,
            kind: metadata.kind,
            document_id: metadata.document_id,
            document_path: metadata.document_path,
            title: metadata.title,
            content_commit_sha: metadata.content_commit_sha,
            state_version: metadata.state_version,
            intent_ids: metadata.intent_ids,
            spec_ids: metadata.spec_ids,
        },
        shadow: MarkdownShadow {
            lifecycle: metadata.lifecycle,
            priority: metadata.priority,
            pr: metadata.pr,
            evidence: metadata.evidence,
        },
    };
    let document_key = Some(document.record.document_key.clone());
    if metadata.schema_version != 1
        || metadata.authority_mode != "markdown-shadow"
        || outer_string(value, "id", &document_key)? != native_id(&document.record)
        || outer_string(value, "title", &document_key)? != document.record.title
        || outer_string(value, "status", &document_key)? != "open"
        || outer_priority(value, &document_key)? != effective_priority(&document)
        || outer_string(value, "external_ref", &document_key)? != document.record.document_key
    {
        return Err(refusal("invalid_document", document_key));
    }
    if let Some(description) = value.get("description")
        && description.as_str() != Some("")
    {
        return Err(refusal(
            "invalid_document",
            Some(document.record.document_key),
        ));
    }
    validate_document(&document)?;
    let native = native_id(&document.record);
    Ok((native, document))
}

/// Decodes a Beads JSONL export while rejecting an ambiguous or non-shadow projection.
pub fn decode_beads_jsonl(jsonl: &str) -> Result<Vec<ShadowDocument>, ShadowError> {
    let mut documents = Vec::new();
    let mut native_ids = BTreeSet::new();
    let mut logical_keys = BTreeSet::new();
    for line in jsonl.lines().filter(|line| !line.trim().is_empty()) {
        let value = serde_json::from_str(line).map_err(|_| refusal("invalid_document", None))?;
        let (native, document) = decode_row(&value)?;
        let key = document.record.document_key.clone();
        if !native_ids.insert(native) || !logical_keys.insert(key.clone()) {
            return Err(refusal("invalid_document", Some(key)));
        }
        documents.push(document);
    }
    validate_documents(&documents)?;
    Ok(documents)
}

fn operational_from_row(
    value: &Value,
    document: &ShadowDocument,
) -> Result<Option<OperationalMetadata>, ShadowError> {
    let metadata = value
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or_else(invalid_store)?;
    let operational = metadata.get("plasmosome_operational");
    match document.record.kind {
        DocumentKind::Task => operational
            .cloned()
            .ok_or_else(invalid_store)
            .and_then(|value| serde_json::from_value(value).map_err(|_| invalid_store()))
            .map(Some),
        DocumentKind::Intent | DocumentKind::Spec if operational.is_some() => Err(invalid_store()),
        DocumentKind::Intent | DocumentKind::Spec => Ok(None),
    }
}

/// Decodes a strict Beads JSONL export with task operational metadata.
pub fn decode_operational_beads_jsonl(
    jsonl: &str,
) -> Result<Vec<OperationalDocument>, ShadowError> {
    let mut documents = Vec::new();
    let mut native_ids = BTreeSet::new();
    let mut logical_keys = BTreeSet::new();
    for line in jsonl.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).map_err(|_| invalid_store())?;
        let (native, document) = decode_row(&value).map_err(|_| invalid_store())?;
        let key = document.record.document_key.clone();
        if !native_ids.insert(native) || !logical_keys.insert(key) {
            return Err(invalid_store());
        }
        let operational = operational_from_row(&value, &document)?;
        documents.push(OperationalDocument {
            document,
            operational,
        });
    }
    validate_operational_documents(&documents)?;
    Ok(documents)
}

/// Serializes a validated operational projection in canonical document-key order.
pub fn canonical_operational_projection(
    documents: &[OperationalDocument],
) -> Result<String, ShadowError> {
    validate_operational_documents(documents)?;
    let mut documents = documents.to_vec();
    documents.sort_by(|left, right| {
        left.document
            .record
            .document_key
            .cmp(&right.document.record.document_key)
    });
    serde_json::to_string(&documents).map_err(|_| invalid_store())
}

/// Returns the SHA-256 digest of a canonical operational projection.
pub fn operational_projection_digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

/// Serializes the typed shadow projection in canonical key order without Beads-native ids.
pub fn canonical_logical_export(documents: &[ShadowDocument]) -> Result<String, ShadowError> {
    validate_documents(documents)?;
    let mut documents = documents.to_vec();
    documents.sort_by(|left, right| left.record.document_key.cmp(&right.record.document_key));
    serde_json::to_string(&documents).map_err(|_| refusal("invalid_document", None))
}

/// Decodes a canonical logical export before it is imported into another fresh store.
pub fn decode_logical_export(value: &str) -> Result<Vec<ShadowDocument>, ShadowError> {
    let documents: Vec<ShadowDocument> =
        serde_json::from_str(value).map_err(|_| refusal("invalid_document", None))?;
    validate_documents(&documents)?;
    Ok(documents)
}

/// Returns the SHA-256 digest of a canonical logical export.
pub fn logical_export_digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn compare_documents(
    expected: &[ShadowDocument],
    actual: &[ShadowDocument],
    code: &'static str,
    include_shadow: bool,
) -> Result<(), ShadowError> {
    let mut expected_by_key = BTreeMap::new();
    let mut actual_by_key = BTreeMap::new();
    for document in expected {
        if expected_by_key
            .insert(document.record.document_key.as_str(), document)
            .is_some()
        {
            return Err(comparison_refusal(
                code,
                &document.record.document_key,
                "different",
            ));
        }
    }
    for document in actual {
        if actual_by_key
            .insert(document.record.document_key.as_str(), document)
            .is_some()
        {
            return Err(comparison_refusal(
                code,
                &document.record.document_key,
                "different",
            ));
        }
    }
    let keys = expected_by_key
        .keys()
        .chain(actual_by_key.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    for key in keys {
        match (expected_by_key.get(key), actual_by_key.get(key)) {
            (Some(_), None) => return Err(comparison_refusal(code, key, "missing")),
            (None, Some(_)) => return Err(comparison_refusal(code, key, "extra")),
            (Some(expected), Some(actual))
                if expected.record != actual.record
                    || (include_shadow && expected.shadow != actual.shadow) =>
            {
                return Err(comparison_refusal(code, key, "different"));
            }
            (Some(_), Some(_)) => {}
            (None, None) => unreachable!("a collected key exists in one mapping"),
        }
    }
    Ok(())
}

/// Compares every logical `DocumentRecord` field without changing list order or duplicates.
pub fn compare_document_mapping(
    expected: &[ShadowDocument],
    actual: &[ShadowDocument],
) -> Result<(), ShadowError> {
    compare_documents(expected, actual, "document_mapping_mismatch", false)
}

/// Compares every logical and Markdown-shadow field without changing list order or duplicates.
pub fn compare_shadow_parity(
    expected: &[ShadowDocument],
    actual: &[ShadowDocument],
) -> Result<(), ShadowError> {
    compare_documents(expected, actual, "shadow_parity_mismatch", true)
}

fn store_command(store: &ShadowStore, argv: Vec<String>, redacted: Vec<usize>) -> CommandSpec {
    CommandSpec {
        program: store.binary.clone(),
        argv,
        cwd: Some(store.repository.clone()),
        environment: store.environment.clone(),
        redacted_argv_positions: redacted,
    }
}

fn run_store_command<R: CommandRunner>(
    runner: &mut R,
    plans: &mut Vec<String>,
    command: CommandSpec,
    key: Option<String>,
) -> Result<CommandOutput, ShadowError> {
    plans.push(command.display());
    let output = runner
        .run(command)
        .map_err(|_| refusal("invalid_document", key.clone()))?;
    if output.status == 0 {
        Ok(output)
    } else {
        Err(refusal("invalid_document", key))
    }
}

fn validate_import_response(
    output: &str,
    expected_ids: &[String],
    key: Option<String>,
) -> Result<(), ShadowError> {
    let value: Value =
        serde_json::from_str(output).map_err(|_| refusal("invalid_document", key.clone()))?;
    let created = value
        .get("created")
        .and_then(Value::as_u64)
        .ok_or_else(|| refusal("invalid_document", key.clone()))?;
    let ids = value
        .get("ids")
        .and_then(Value::as_array)
        .ok_or_else(|| refusal("invalid_document", key.clone()))?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| refusal("invalid_document", key.clone()))?;
    if created != expected_ids.len() as u64
        || ids != expected_ids.iter().map(String::as_str).collect::<Vec<_>>()
        || value.get("skipped").and_then(Value::as_u64) != Some(0)
    {
        return Err(refusal("invalid_document", key));
    }
    Ok(())
}

/// Imports documents into one fresh store, exports them, and verifies its Markdown-shadow keys.
pub fn import_shadow_documents<R: CommandRunner>(
    runner: &mut R,
    store: &ShadowStore,
    source_commit: &str,
    documents: &[ShadowDocument],
) -> Result<ShadowStoreImport, ShadowError> {
    if !is_lower_hex_sha(source_commit) {
        return Err(refusal("invalid_source_ref", None));
    }
    validate_documents(documents)?;
    let jsonl = to_beads_jsonl(documents)?;
    let mut jsonl_file = NamedTempFile::new_in(&store.temporary_root)
        .map_err(|_| refusal("invalid_document", None))?;
    jsonl_file
        .write_all(jsonl.as_bytes())
        .map_err(|_| refusal("invalid_document", None))?;
    let jsonl_path = jsonl_file.path().display().to_string();
    let expected_ids = documents
        .iter()
        .map(|document| native_id(&document.record))
        .collect::<Vec<_>>();
    let key = documents
        .first()
        .map(|document| document.record.document_key.clone());
    let mut command_plans = Vec::new();
    let imported = run_store_command(
        runner,
        &mut command_plans,
        store_command(
            store,
            vec![
                "--sandbox".into(),
                "import".into(),
                jsonl_path,
                "--json".into(),
            ],
            vec![2],
        ),
        key.clone(),
    )?;
    validate_import_response(&imported.stdout, &expected_ids, key.clone())?;
    let exported = run_store_command(
        runner,
        &mut command_plans,
        store_command(store, vec!["--sandbox".into(), "export".into()], Vec::new()),
        key.clone(),
    )?;
    let exported_documents = decode_beads_jsonl(&exported.stdout)?;
    for (argv, expected) in [
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "set".into(),
                "plasmosome.authority-mode".into(),
                "markdown-shadow".into(),
            ],
            None,
        ),
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "set".into(),
                "plasmosome.source-commit".into(),
                source_commit.to_owned(),
            ],
            None,
        ),
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "get".into(),
                "plasmosome.authority-mode".into(),
            ],
            Some("markdown-shadow"),
        ),
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "get".into(),
                "plasmosome.source-commit".into(),
            ],
            Some(source_commit),
        ),
    ] {
        let output = run_store_command(
            runner,
            &mut command_plans,
            store_command(store, argv, Vec::new()),
            key.clone(),
        )?;
        if expected.is_some_and(|expected| output.stdout.trim() != expected) {
            return Err(refusal("invalid_document", key));
        }
    }
    Ok(ShadowStoreImport {
        label: store.label.clone(),
        documents: exported_documents,
        command_plans,
    })
}

/// Imports task operational siblings into a fresh store and verifies their strict export.
pub fn import_operational_shadow_documents<R: CommandRunner>(
    runner: &mut R,
    store: &ShadowStore,
    source_commit: &str,
    documents: &[ShadowDocument],
    operational: &BTreeMap<String, OperationalMetadata>,
) -> Result<OperationalShadowStoreImport, ShadowError> {
    if !is_lower_hex_sha(source_commit) {
        return Err(refusal("invalid_source_ref", None));
    }
    let jsonl = to_operational_beads_jsonl(documents, operational)?;
    let mut jsonl_file = NamedTempFile::new_in(&store.temporary_root)
        .map_err(|_| refusal("invalid_document", None))?;
    jsonl_file
        .write_all(jsonl.as_bytes())
        .map_err(|_| refusal("invalid_document", None))?;
    let jsonl_path = jsonl_file.path().display().to_string();
    let expected_ids = documents
        .iter()
        .map(|document| native_id(&document.record))
        .collect::<Vec<_>>();
    let key = documents
        .first()
        .map(|document| document.record.document_key.clone());
    let mut command_plans = Vec::new();
    let imported = run_store_command(
        runner,
        &mut command_plans,
        store_command(
            store,
            vec![
                "--sandbox".into(),
                "import".into(),
                jsonl_path,
                "--json".into(),
            ],
            vec![2],
        ),
        key.clone(),
    )?;
    validate_import_response(&imported.stdout, &expected_ids, key.clone())?;
    let exported = run_store_command(
        runner,
        &mut command_plans,
        store_command(store, vec!["--sandbox".into(), "export".into()], Vec::new()),
        key.clone(),
    )?;
    for (argv, expected) in [
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "set".into(),
                "plasmosome.authority-mode".into(),
                "markdown-shadow".into(),
            ],
            None,
        ),
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "set".into(),
                "plasmosome.source-commit".into(),
                source_commit.to_owned(),
            ],
            None,
        ),
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "get".into(),
                "plasmosome.authority-mode".into(),
            ],
            Some("markdown-shadow"),
        ),
        (
            vec![
                "--sandbox".into(),
                "kv".into(),
                "get".into(),
                "plasmosome.source-commit".into(),
            ],
            Some(source_commit),
        ),
    ] {
        let output = run_store_command(
            runner,
            &mut command_plans,
            store_command(store, argv, Vec::new()),
            key.clone(),
        )?;
        if expected.is_some_and(|expected| output.stdout.trim() != expected) {
            return Err(refusal("invalid_document", key));
        }
    }
    Ok(OperationalShadowStoreImport {
        label: store.label.clone(),
        documents: decode_operational_beads_jsonl(&exported.stdout)?,
        command_plans,
    })
}
