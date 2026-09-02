use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::command::{CommandOutput, CommandRunner, CommandSpec};

/// The namespace a durable Plasmosome work document belongs to.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    /// A durable goal proposed or approved by the project owner.
    Intent,
    /// A testable contract that serves one or more intents.
    Spec,
    /// A unit of implementation work governed by one or more specs.
    Task,
}

impl DocumentKind {
    pub(crate) fn namespace(&self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Spec => "spec",
            Self::Task => "task",
        }
    }

    pub(crate) fn directory(&self) -> &'static str {
        match self {
            Self::Intent => "docs/intents/",
            Self::Spec => "docs/specs/",
            Self::Task => "tasks/",
        }
    }

    fn order(&self) -> u8 {
        match self {
            Self::Intent => 0,
            Self::Spec => 1,
            Self::Task => 2,
        }
    }
}

/// The immutable logical projection of one Markdown document.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct DocumentRecord {
    /// The kind-qualified, immutable logical identity.
    pub document_key: String,
    /// The document namespace.
    pub kind: DocumentKind,
    /// The exact three-digit id taken from the canonical path.
    pub document_id: String,
    /// The canonical Git-relative Markdown path.
    pub document_path: String,
    /// The current Markdown frontmatter title.
    pub title: String,
    /// The Git commit that established the imported path and contents.
    pub content_commit_sha: String,
    /// The initial durable state version for this disposable import.
    pub state_version: u64,
    /// Ordered intent ids copied from Markdown without repair.
    pub intent_ids: Vec<String>,
    /// Ordered spec ids copied from Markdown without repair.
    pub spec_ids: Vec<String>,
}

/// The Markdown-owned volatile fields represented in a one-way Beads shadow.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MarkdownShadow {
    /// The kind-specific lifecycle value from Markdown frontmatter.
    pub lifecycle: String,
    /// The Markdown task priority, when this is a task.
    pub priority: Option<u8>,
    /// The task PR field, when present and nonblank.
    pub pr: Option<String>,
    /// The task evidence field, when present and nonblank.
    pub evidence: Option<String>,
}

/// A logical document and its Markdown-owned shadow fields.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ShadowDocument {
    /// The immutable logical record.
    pub record: DocumentRecord,
    /// The Markdown shadow projection.
    pub shadow: MarkdownShadow,
}

/// A source snapshot resolved once from one requested Git ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDocuments {
    /// The ref supplied by the caller before it was resolved.
    pub requested_ref: String,
    /// The immutable source commit used for every later Git read.
    pub source_commit: String,
    /// Canonically ordered and target-validated documents.
    pub documents: Vec<ShadowDocument>,
    /// Redacted-safe Git command plans used to construct the snapshot.
    pub command_plans: Vec<String>,
}

/// A stable source-model refusal and its document key when one is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentError {
    code: &'static str,
    /// The source document that caused the refusal, when its path identifies one.
    pub offending_key: Option<String>,
}

impl DocumentError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for DocumentError {}

#[derive(Clone, Debug)]
struct DocumentPath {
    kind: DocumentKind,
    id: String,
    path: String,
    key: String,
}

#[derive(Clone, Debug)]
enum FieldValue {
    Plain(String),
    Block(String),
}

#[derive(Clone, Copy, Debug)]
struct BlockStyle {
    folded: bool,
    chomp: Option<char>,
}

fn refusal(code: &'static str, offending_key: Option<String>) -> DocumentError {
    DocumentError {
        code,
        offending_key,
    }
}

fn invalid_document(key: Option<String>) -> DocumentError {
    refusal("invalid_document", key)
}

fn key(kind: &DocumentKind, id: &str) -> String {
    format!("{}:{id}", kind.namespace())
}

pub(crate) fn is_document_id(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn is_lower_hex_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn starts_with_document_id(value: &str) -> bool {
    value
        .as_bytes()
        .get(..3)
        .is_some_and(|prefix| prefix.iter().all(u8::is_ascii_digit))
}

fn kind_for_path(path: &str) -> Option<DocumentKind> {
    [DocumentKind::Intent, DocumentKind::Spec, DocumentKind::Task]
        .into_iter()
        .find(|kind| path.starts_with(kind.directory()))
}

fn key_from_numeric_path(kind: &DocumentKind, path: &str) -> Option<String> {
    let basename = path.rsplit('/').next()?;
    starts_with_document_id(basename).then(|| key(kind, &basename[..3]))
}

fn canonical_document_path(path: &str) -> Result<DocumentPath, DocumentError> {
    let Some(kind) = kind_for_path(path) else {
        return Err(invalid_document(None));
    };
    let suffix = path
        .strip_prefix(kind.directory())
        .expect("matched namespace has its prefix");
    let offending_key = key_from_numeric_path(&kind, path);
    if suffix.contains('/')
        || suffix.len() < 8
        || !starts_with_document_id(suffix)
        || suffix.as_bytes().get(3) != Some(&b'-')
        || !suffix.ends_with(".md")
        || suffix[4..suffix.len() - 3].is_empty()
    {
        return Err(invalid_document(offending_key));
    }
    let id = suffix[..3].to_owned();
    Ok(DocumentPath {
        key: key(&kind, &id),
        kind,
        id,
        path: path.to_owned(),
    })
}

fn discovered_paths(contents: &str) -> Result<Vec<DocumentPath>, DocumentError> {
    let mut documents = Vec::new();
    for path in contents.split('\0').filter(|path| !path.is_empty()) {
        let Some(kind) = kind_for_path(path) else {
            continue;
        };
        if path == format!("{}README.md", kind.directory()) {
            continue;
        }
        let basename = path.rsplit('/').next().unwrap_or_default();
        if starts_with_document_id(basename) {
            documents.push(canonical_document_path(path)?);
        }
    }
    documents.sort_by(|left, right| {
        left.kind
            .order()
            .cmp(&right.kind.order())
            .then(left.id.cmp(&right.id))
            .then(left.path.cmp(&right.path))
    });
    let mut seen = BTreeSet::new();
    for document in &documents {
        if !seen.insert(document.key.clone()) {
            return Err(refusal("duplicate_document_id", Some(document.key.clone())));
        }
    }
    Ok(documents)
}

fn block_style(value: &str) -> Option<BlockStyle> {
    let value = value.trim();
    let mut characters = value.chars();
    let marker = characters.next()?;
    if marker != '|' && marker != '>' {
        return None;
    }
    let chomp = match characters.next() {
        Some(chomp @ ('+' | '-')) => Some(chomp),
        Some(_) => return None,
        None => None,
    };
    characters.next().is_none().then_some(BlockStyle {
        folded: marker == '>',
        chomp,
    })
}

fn leading_whitespace(value: &str) -> usize {
    value
        .bytes()
        .take_while(|byte| *byte == b' ' || *byte == b'\t')
        .count()
}

fn folded_block(lines: &[String]) -> String {
    let mut value = String::new();
    let mut wrote = false;
    let mut blank_lines = 0;
    for line in lines {
        if line.is_empty() {
            if wrote {
                blank_lines += 1;
            }
            continue;
        }
        if wrote {
            if blank_lines == 0 {
                value.push(' ');
            } else {
                for _ in 0..blank_lines {
                    value.push('\n');
                }
            }
        }
        value.push_str(line);
        wrote = true;
        blank_lines = 0;
    }
    if wrote {
        for _ in 0..blank_lines {
            value.push('\n');
        }
    }
    value
}

fn decode_block(style: BlockStyle, raw_lines: &[&str]) -> String {
    let indentation = raw_lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| leading_whitespace(line))
        .min()
        .unwrap_or(0);
    let lines = raw_lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                line[indentation..].to_owned()
            }
        })
        .collect::<Vec<_>>();
    let mut value = if style.folded {
        folded_block(&lines)
    } else {
        lines.join("\n")
    };
    match style.chomp {
        Some('-') => value.trim_end_matches('\n').to_owned(),
        Some('+') => {
            if !lines.is_empty() {
                value.push('\n');
            }
            value
        }
        None => {
            value = value.trim_end_matches('\n').to_owned();
            if !value.is_empty() {
                value.push('\n');
            }
            value
        }
        Some(_) => unreachable!("block chomp only permits plus or minus"),
    }
}

fn frontmatter(
    contents: &str,
    key: &str,
) -> Result<BTreeMap<String, Vec<FieldValue>>, DocumentError> {
    let lines = contents.lines().collect::<Vec<_>>();
    if lines.first() != Some(&"---") {
        return Err(invalid_document(Some(key.to_owned())));
    }
    let Some(closing) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, line)| (*line == "---").then_some(index))
    else {
        return Err(invalid_document(Some(key.to_owned())));
    };
    let mut fields = BTreeMap::<String, Vec<FieldValue>>::new();
    let mut index = 1;
    while index < closing {
        let line = lines[index];
        if line.is_empty() || line.starts_with([' ', '\t']) {
            index += 1;
            continue;
        }
        let Some((name, raw_value)) = line.split_once(':') else {
            index += 1;
            continue;
        };
        let raw_value = raw_value.trim_start();
        let value = if raw_value.starts_with(['|', '>']) {
            let Some(style) = block_style(raw_value) else {
                return Err(invalid_document(Some(key.to_owned())));
            };
            index += 1;
            let start = index;
            while index < closing
                && (lines[index].is_empty() || lines[index].starts_with([' ', '\t']))
            {
                index += 1;
            }
            FieldValue::Block(decode_block(style, &lines[start..index]))
        } else {
            index += 1;
            FieldValue::Plain(raw_value.to_owned())
        };
        fields.entry(name.to_owned()).or_default().push(value);
    }
    Ok(fields)
}

fn normalized_plain(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn required_plain(
    fields: &BTreeMap<String, Vec<FieldValue>>,
    field: &str,
    key: &str,
) -> Result<String, DocumentError> {
    let Some(values) = fields.get(field) else {
        return Err(invalid_document(Some(key.to_owned())));
    };
    if values.len() != 1 {
        return Err(invalid_document(Some(key.to_owned())));
    }
    let FieldValue::Plain(value) = &values[0] else {
        return Err(invalid_document(Some(key.to_owned())));
    };
    let value = normalized_plain(value);
    if value.is_empty() {
        Err(invalid_document(Some(key.to_owned())))
    } else {
        Ok(value)
    }
}

fn optional_scalar(
    fields: &BTreeMap<String, Vec<FieldValue>>,
    field: &str,
    key: &str,
) -> Result<Option<String>, DocumentError> {
    match fields.get(field) {
        None => Ok(None),
        Some(values) if values.len() == 1 => match &values[0] {
            FieldValue::Plain(value) => {
                let value = normalized_plain(value);
                Ok((!value.trim().is_empty()).then_some(value))
            }
            FieldValue::Block(value) => Ok((!value.trim().is_empty()).then_some(value.clone())),
        },
        Some(_) => Err(invalid_document(Some(key.to_owned()))),
    }
}

fn flow_list(
    fields: &BTreeMap<String, Vec<FieldValue>>,
    field: &str,
    key: &str,
) -> Result<Vec<String>, DocumentError> {
    let value = required_plain(fields, field, key)?;
    let Some(value) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return Err(invalid_document(Some(key.to_owned())));
    };
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|id| id.trim().to_owned())
        .map(|id| {
            if is_document_id(&id) {
                Ok(id)
            } else {
                Err(invalid_document(Some(key.to_owned())))
            }
        })
        .collect()
}

pub(crate) fn valid_lifecycle(kind: &DocumentKind, lifecycle: &str) -> bool {
    match kind {
        DocumentKind::Intent => matches!(lifecycle, "draft" | "approved"),
        DocumentKind::Spec => matches!(lifecycle, "draft" | "accepted" | "superseded"),
        DocumentKind::Task => matches!(
            lifecycle,
            "todo" | "planned" | "in_progress" | "in_review" | "done"
        ),
    }
}

/// Parses one canonical Markdown document without repairing any of its fields.
pub fn parse_document(
    document_path: &str,
    contents: &str,
    content_commit_sha: &str,
) -> Result<ShadowDocument, DocumentError> {
    let path = canonical_document_path(document_path)?;
    if !is_lower_hex_sha(content_commit_sha) {
        return Err(refusal("content_commit_mismatch", Some(path.key.clone())));
    }
    let fields = frontmatter(contents, &path.key)?;
    let id = required_plain(&fields, "id", &path.key)?;
    if id != path.id {
        return Err(invalid_document(Some(path.key.clone())));
    }
    let title = required_plain(&fields, "title", &path.key)?;
    let lifecycle = required_plain(&fields, "status", &path.key)?;
    if !valid_lifecycle(&path.kind, &lifecycle) {
        return Err(invalid_document(Some(path.key.clone())));
    }
    let (priority, intent_ids, spec_ids, pr, evidence) = match path.kind {
        DocumentKind::Intent => (None, Vec::new(), Vec::new(), None, None),
        DocumentKind::Spec => (
            None,
            flow_list(&fields, "intents", &path.key)?,
            Vec::new(),
            None,
            None,
        ),
        DocumentKind::Task => {
            let priority = required_plain(&fields, "priority", &path.key)?
                .parse::<u8>()
                .ok()
                .filter(|priority| (1..=3).contains(priority))
                .ok_or_else(|| invalid_document(Some(path.key.clone())))?;
            (
                Some(priority),
                flow_list(&fields, "intents", &path.key)?,
                flow_list(&fields, "specs", &path.key)?,
                optional_scalar(&fields, "pr", &path.key)?,
                optional_scalar(&fields, "evidence", &path.key)?,
            )
        }
    };
    Ok(ShadowDocument {
        record: DocumentRecord {
            document_key: path.key,
            kind: path.kind,
            document_id: id,
            document_path: path.path,
            title,
            content_commit_sha: content_commit_sha.to_owned(),
            state_version: 1,
            intent_ids,
            spec_ids,
        },
        shadow: MarkdownShadow {
            lifecycle,
            priority,
            pr,
            evidence,
        },
    })
}

/// Validates every typed source link after all documents have been parsed.
pub fn validate_document_targets(documents: &[ShadowDocument]) -> Result<(), DocumentError> {
    let mut keys = BTreeSet::new();
    let mut intent_ids = BTreeSet::new();
    let mut spec_ids = BTreeSet::new();
    for document in documents {
        if !keys.insert(document.record.document_key.clone()) {
            return Err(refusal(
                "duplicate_document_id",
                Some(document.record.document_key.clone()),
            ));
        }
        match document.record.kind {
            DocumentKind::Intent => {
                intent_ids.insert(document.record.document_id.clone());
            }
            DocumentKind::Spec => {
                spec_ids.insert(document.record.document_id.clone());
            }
            DocumentKind::Task => {}
        }
    }
    for document in documents {
        let source_key = Some(document.record.document_key.clone());
        match document.record.kind {
            DocumentKind::Intent => {}
            DocumentKind::Spec => {
                if document
                    .record
                    .intent_ids
                    .iter()
                    .any(|id| !intent_ids.contains(id))
                {
                    return Err(refusal("missing_document_target", source_key));
                }
            }
            DocumentKind::Task => {
                if document
                    .record
                    .intent_ids
                    .iter()
                    .any(|id| !intent_ids.contains(id))
                    || document
                        .record
                        .spec_ids
                        .iter()
                        .any(|id| !spec_ids.contains(id))
                {
                    return Err(refusal("missing_document_target", source_key));
                }
            }
        }
    }
    Ok(())
}

fn source_command(
    source_root: &Path,
    environment: &BTreeMap<String, String>,
    argv: Vec<String>,
) -> CommandSpec {
    let mut environment = environment.clone();
    environment.insert("GIT_NO_LAZY_FETCH".into(), "1".into());
    CommandSpec {
        program: "git".into(),
        argv,
        cwd: Some(source_root.to_path_buf()),
        environment,
        redacted_argv_positions: Vec::new(),
    }
}

fn run_source_command<R: CommandRunner>(
    runner: &mut R,
    plans: &mut Vec<String>,
    source_root: &Path,
    environment: &BTreeMap<String, String>,
    argv: Vec<String>,
    code: &'static str,
    offending_key: Option<String>,
) -> Result<CommandOutput, DocumentError> {
    let command = source_command(source_root, environment, argv);
    plans.push(command.display());
    let output = runner
        .run(command)
        .map_err(|_| refusal(code, offending_key.clone()))?;
    if output.status != 0 {
        Err(refusal(code, offending_key))
    } else {
        Ok(output)
    }
}

fn one_lower_sha(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    let value = lines.next()?;
    (lines.next().is_none() && is_lower_hex_sha(value)).then_some(value.to_owned())
}

/// Resolves one source ref and reconstructs every numeric work document from that Git tree.
pub fn load_documents<R: CommandRunner>(
    runner: &mut R,
    source_root: &Path,
    environment: &BTreeMap<String, String>,
    source_ref: &str,
) -> Result<SourceDocuments, DocumentError> {
    if source_ref.trim().is_empty() || source_ref.contains(['\n', '\r']) {
        return Err(refusal("invalid_source_ref", None));
    }
    let mut command_plans = Vec::new();
    let resolution = run_source_command(
        runner,
        &mut command_plans,
        source_root,
        environment,
        vec![
            "rev-parse".into(),
            "--verify".into(),
            "--end-of-options".into(),
            format!("{source_ref}^{{commit}}"),
        ],
        "invalid_source_ref",
        None,
    )?;
    let source_commit =
        one_lower_sha(&resolution.stdout).ok_or_else(|| refusal("invalid_source_ref", None))?;
    let tree = run_source_command(
        runner,
        &mut command_plans,
        source_root,
        environment,
        vec![
            "ls-tree".into(),
            "-r".into(),
            "--name-only".into(),
            "-z".into(),
            source_commit.clone(),
            "--".into(),
            "docs/intents".into(),
            "docs/specs".into(),
            "tasks".into(),
        ],
        "invalid_document",
        None,
    )?;
    let paths = discovered_paths(&tree.stdout)?;
    let mut documents = Vec::new();
    for path in paths {
        let selected_contents = run_source_command(
            runner,
            &mut command_plans,
            source_root,
            environment,
            vec!["show".into(), format!("{}:{}", source_commit, path.path)],
            "invalid_document",
            Some(path.key.clone()),
        )?
        .stdout;
        let content_commit = run_source_command(
            runner,
            &mut command_plans,
            source_root,
            environment,
            vec![
                "log".into(),
                "-1".into(),
                "--format=%H".into(),
                source_commit.clone(),
                "--".into(),
                format!(":(literal){}", path.path),
            ],
            "content_commit_mismatch",
            Some(path.key.clone()),
        )?;
        let content_commit = one_lower_sha(&content_commit.stdout)
            .ok_or_else(|| refusal("content_commit_mismatch", Some(path.key.clone())))?;
        let established_contents = run_source_command(
            runner,
            &mut command_plans,
            source_root,
            environment,
            vec!["show".into(), format!("{}:{}", content_commit, path.path)],
            "content_commit_mismatch",
            Some(path.key.clone()),
        )?
        .stdout;
        if established_contents != selected_contents {
            return Err(refusal("content_commit_mismatch", Some(path.key.clone())));
        }
        documents.push(parse_document(
            &path.path,
            &selected_contents,
            &content_commit,
        )?);
    }
    validate_document_targets(&documents)?;
    Ok(SourceDocuments {
        requested_ref: source_ref.to_owned(),
        source_commit,
        documents,
        command_plans,
    })
}
