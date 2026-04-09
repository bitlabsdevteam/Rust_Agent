use crate::runtime_log;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_QUEUE_FILE: &str = "logs/ingress_queue.jsonl";
pub const DEFAULT_CONVERSATION_HISTORY_FILE: &str = "logs/conversation_history.MD";

const WORKSPACE_CONTEXT_DIR: &str = "Workspace";
const REPO_CONTEXT_FILES: &[&str] = &["AGENTS.md", "TDD.md"];
const MAX_CONTEXT_DOC_PREVIEW_CHARS: usize = 280;
const MAX_CONTEXT_PACKET_CHARS: usize = 2400;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IngressItemKind {
    Text,
    Image,
    Video,
    Audio,
}

impl IngressItemKind {
    fn from_label(label: &str) -> Option<Self> {
        match label.trim().to_ascii_lowercase().as_str() {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            _ => None,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

impl fmt::Display for IngressItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngressItem {
    pub kind: IngressItemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngressRequest {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default)]
    pub items: Vec<IngressItem>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngressAnalysis {
    pub total_items: usize,
    pub text_items: usize,
    pub image_items: usize,
    pub video_items: usize,
    pub audio_items: usize,
    pub text_characters: usize,
    pub referenced_assets: usize,
    pub routing_hint: String,
    pub classification: IngressClassification,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngressClassification {
    pub category: String,
    pub priority: String,
    pub target_queue: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueEntry {
    pub queue_id: String,
    pub received_at_epoch_ms: u128,
    pub status: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub analysis: IngressAnalysis,
    pub items: Vec<IngressItem>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAck {
    pub queue_id: String,
    pub queue_path: String,
    pub total_items: usize,
    pub classification: String,
    pub priority: String,
    pub target_queue: String,
    pub routing_hint: String,
    pub summary: String,
}

impl QueueAck {
    pub fn render(&self) -> String {
        format!(
            "Ingress classified and queued.\nQueue ID: {}\nItems: {}\nClassification: {}\nPriority: {}\nTarget queue: {}\nRouting hint: {}\nSummary: {}\nQueue file: {}",
            self.queue_id,
            self.total_items,
            self.classification,
            self.priority,
            self.target_queue,
            self.routing_hint,
            self.summary,
            self.queue_path
        )
    }
}

pub fn queue_ingress(user_input: &str, arguments: &Value) -> Result<QueueAck, String> {
    let request = enrich_request_with_context(parse_ingress_request(user_input, arguments)?)?;
    let analysis = analyze_request(&request);
    let queue_path = resolve_queue_path();
    let entry = build_queue_entry(&request, analysis.clone());
    append_queue_entry(&queue_path, &entry)?;
    runtime_log::info(
        "ingress_agent",
        format!(
            "persisted queue_id {} to {}",
            entry.queue_id,
            queue_path.display()
        ),
    );

    Ok(QueueAck {
        queue_id: entry.queue_id,
        queue_path: queue_path.display().to_string(),
        total_items: analysis.total_items,
        classification: analysis.classification.category.clone(),
        priority: analysis.classification.priority.clone(),
        target_queue: analysis.classification.target_queue.clone(),
        routing_hint: analysis.routing_hint,
        summary: analysis.summary,
    })
}

pub fn resolve_conversation_history_path() -> PathBuf {
    std::env::var("AGENT_CONVERSATION_HISTORY_FILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONVERSATION_HISTORY_FILE))
}

pub fn ensure_conversation_history_file() -> Result<PathBuf, String> {
    let path = resolve_conversation_history_path();
    if path.is_file() {
        return Ok(path);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create history directory {}: {error}",
                parent.display()
            )
        })?;
    }

    fs::write(
        &path,
        "# Conversation History\n\nThis file stores reusable conversation context for ingress compaction.\n",
    )
    .map_err(|error| format!("failed to create conversation history file {}: {error}", path.display()))?;

    Ok(path)
}

pub fn append_conversation_history(role: &str, content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let path = ensure_conversation_history_file()?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| {
            format!(
                "failed to open conversation history {}: {error}",
                path.display()
            )
        })?;

    let heading = match role.trim().to_ascii_lowercase().as_str() {
        "assistant" => "Assistant",
        _ => "User",
    };
    let entry = format!("\n## {heading}\n\n{}\n", trimmed);
    file.write_all(entry.as_bytes()).map_err(|error| {
        format!(
            "failed to append conversation history {}: {error}",
            path.display()
        )
    })
}

fn enrich_request_with_context(mut request: IngressRequest) -> Result<IngressRequest, String> {
    let history_path = ensure_conversation_history_file()?;
    let documents = load_markdown_documents(&resolve_workspace_context_dir())?;
    let compact_context = compact_context_packet(&documents, &history_path, &request);

    let metadata = request.metadata.take();
    request.metadata = merge_metadata(
        metadata,
        json!({
            "ingress_context": {
                "conversation_history_file": history_path.display().to_string(),
                "documents_loaded": documents.len(),
                "context_packet": compact_context,
            }
        }),
    );

    Ok(request)
}

pub fn parse_ingress_request(
    user_input: &str,
    arguments: &Value,
) -> Result<IngressRequest, String> {
    if let Some(request) = parse_request_value(arguments)? {
        return Ok(request);
    }

    let trimmed = user_input.trim();
    if trimmed.is_empty() {
        return Err("Ingress payload is empty.".to_string());
    }

    if looks_like_json(trimmed) {
        if let Some(request) = parse_request_value(
            &serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("Ingress payload JSON parse failed: {error}"))?,
        )? {
            return Ok(request);
        }
    }

    Ok(IngressRequest {
        source: "cli".to_string(),
        message_id: None,
        items: vec![IngressItem {
            kind: IngressItemKind::Text,
            text: Some(trimmed.to_string()),
            path: None,
            url: None,
            mime_type: Some("text/plain".to_string()),
            transcript: None,
            notes: None,
            metadata: Value::Null,
        }],
        metadata: Value::Null,
    })
}

pub fn analyze_request(request: &IngressRequest) -> IngressAnalysis {
    let mut text_items = 0;
    let mut image_items = 0;
    let mut video_items = 0;
    let mut audio_items = 0;
    let mut text_characters = 0;
    let mut referenced_assets = 0;
    let mut kind_labels = Vec::new();

    for item in &request.items {
        match item.kind {
            IngressItemKind::Text => text_items += 1,
            IngressItemKind::Image => image_items += 1,
            IngressItemKind::Video => video_items += 1,
            IngressItemKind::Audio => audio_items += 1,
        }
        if !kind_labels.iter().any(|label| label == item.kind.as_str()) {
            kind_labels.push(item.kind.as_str().to_string());
        }
        text_characters += item.text.as_deref().unwrap_or_default().chars().count();
        text_characters += item
            .transcript
            .as_deref()
            .unwrap_or_default()
            .chars()
            .count();
        if item.path.is_some() || item.url.is_some() {
            referenced_assets += 1;
        }
    }

    let routing_hint = if video_items > 0 || audio_items > 0 {
        "multimodal_media_review"
    } else if image_items > 0 && text_items > 0 {
        "multimodal_document_review"
    } else if image_items > 0 {
        "image_review"
    } else if text_items > 0 {
        "text_triage"
    } else {
        "generic_ingest"
    }
    .to_string();

    let classification = classify_request(request, &routing_hint);
    let summary = build_summary(request, &kind_labels, &classification);

    IngressAnalysis {
        total_items: request.items.len(),
        text_items,
        image_items,
        video_items,
        audio_items,
        text_characters,
        referenced_assets,
        routing_hint,
        classification,
        summary,
    }
}

pub fn build_queue_entry(request: &IngressRequest, analysis: IngressAnalysis) -> QueueEntry {
    let received_at_epoch_ms = now_epoch_ms();
    QueueEntry {
        queue_id: next_queue_id(received_at_epoch_ms),
        received_at_epoch_ms,
        status: "queued".to_string(),
        source: request.source.clone(),
        message_id: request.message_id.clone(),
        analysis,
        items: request.items.clone(),
        metadata: request.metadata.clone(),
    }
}

pub fn append_queue_entry(path: &Path, entry: &QueueEntry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create queue directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open queue file {}: {error}", path.display()))?;

    let encoded = serde_json::to_string(entry)
        .map_err(|error| format!("failed to encode queue entry: {error}"))?;
    file.write_all(encoded.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| {
            format!(
                "failed to append queue entry to {}: {error}",
                path.display()
            )
        })
}

pub fn resolve_queue_path() -> PathBuf {
    std::env::var("AGENT_QUEUE_FILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_QUEUE_FILE))
}

fn resolve_workspace_context_dir() -> PathBuf {
    std::env::var("AGENT_WORKSPACE_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(WORKSPACE_CONTEXT_DIR))
}

fn load_markdown_documents(workspace_dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut documents = Vec::new();

    for relative_path in REPO_CONTEXT_FILES {
        let path = PathBuf::from(relative_path);
        if !path.is_file() {
            continue;
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("read {} failed: {error}", path.display()))?;
        documents.push((path, contents));
    }

    if workspace_dir.is_dir() {
        let mut workspace_paths = fs::read_dir(workspace_dir)
            .map_err(|error| format!("could not read {}: {error}", workspace_dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(|extension| extension.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        workspace_paths.sort();

        for path in workspace_paths {
            let contents = fs::read_to_string(&path)
                .map_err(|error| format!("read {} failed: {error}", path.display()))?;
            documents.push((path, contents));
        }
    }

    Ok(documents)
}

fn compact_context_packet(
    documents: &[(PathBuf, String)],
    history_path: &Path,
    request: &IngressRequest,
) -> String {
    let request_preview = request
        .items
        .iter()
        .filter_map(|item| {
            item.text
                .as_deref()
                .or(item.transcript.as_deref())
                .or(item.notes.as_deref())
        })
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(shorten_for_context)
        .unwrap_or_else(|| "No inline text preview was available.".to_string());

    let mut sections = vec![format!(
        "Task contract\n- goal: classify the latest user ingress payload with repo-aware context\n- constraints: read repo/workspace markdown, durable memory, and conversation history before compacting content for downstream planning\n- acceptance_criteria: the queue entry preserves a compact context packet alongside the classified payload\n- relevant_files: {}, {}, {}\n- stop_condition: stop after the ingress payload is queued with compacted context",
        "AGENTS.md",
        history_path.display(),
        resolve_workspace_context_dir().display()
    )];

    sections.push(format!("User payload preview: {request_preview}"));

    let mut document_summaries = Vec::new();
    for (path, contents) in documents {
        let normalized = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let preview = shorten_with_limit(&normalized, MAX_CONTEXT_DOC_PREVIEW_CHARS);
        document_summaries.push(format!("- {} => {}", path.display(), preview));
    }

    if document_summaries.is_empty() {
        sections.push("Markdown context files: none".to_string());
    } else {
        sections.push(format!(
            "Markdown context digest:\n{}",
            document_summaries.join("\n")
        ));
    }

    shorten_with_limit(&sections.join("\n\n"), MAX_CONTEXT_PACKET_CHARS)
}

fn shorten_for_context(value: &str) -> String {
    shorten_with_limit(value, MAX_CONTEXT_DOC_PREVIEW_CHARS)
}

fn shorten_with_limit(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    let shortened = trimmed.chars().take(limit).collect::<String>();
    if trimmed.chars().count() > limit {
        format!("{shortened}...")
    } else {
        shortened
    }
}

fn merge_metadata(current: Value, addition: Value) -> Value {
    match (current, addition) {
        (Value::Object(mut left), Value::Object(right)) => {
            for (key, value) in right {
                left.insert(key, value);
            }
            Value::Object(left)
        }
        (Value::Null, value) => value,
        (value, Value::Null) => value,
        (value, _) => value,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn read_queue_entries(path: &Path) -> Result<Vec<QueueEntry>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to read queue file {}: {error}",
                path.display()
            ));
        }
    };

    let mut entries = Vec::new();
    for (line_number, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry = serde_json::from_str::<QueueEntry>(trimmed).map_err(|error| {
            format!(
                "failed to parse queue entry at {} line {}: {error}",
                path.display(),
                line_number + 1
            )
        })?;
        entries.push(entry);
    }

    Ok(entries)
}

pub fn read_queue_entries_lossy(path: &Path) -> Result<(Vec<QueueEntry>, Vec<String>), String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => {
            return Err(format!(
                "failed to read queue file {}: {error}",
                path.display()
            ));
        }
    };

    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for (line_number, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<QueueEntry>(trimmed) {
            Ok(entry) => entries.push(entry),
            Err(error) => warnings.push(format!(
                "skipping malformed queue entry at {} line {}: {error}",
                path.display(),
                line_number + 1
            )),
        }
    }

    Ok((entries, warnings))
}

fn parse_request_value(value: &Value) -> Result<Option<IngressRequest>, String> {
    if value.is_null() {
        return Ok(None);
    }

    if let Some(object) = value.as_object() {
        if object.is_empty() {
            return Ok(None);
        }

        if object.contains_key("items")
            || object.contains_key("source")
            || object.contains_key("message_id")
        {
            return parse_request_object(object).map(Some);
        }

        if object.contains_key("type") {
            let item = parse_item_object(object)?;
            return Ok(Some(IngressRequest {
                source: "cli".to_string(),
                message_id: None,
                items: vec![item],
                metadata: Value::Null,
            }));
        }
    }

    if let Some(items) = value.as_array() {
        let parsed_items = items
            .iter()
            .map(parse_item_value)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Some(IngressRequest {
            source: "cli".to_string(),
            message_id: None,
            items: parsed_items,
            metadata: Value::Null,
        }));
    }

    Ok(None)
}

fn parse_request_object(object: &Map<String, Value>) -> Result<IngressRequest, String> {
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("cli")
        .to_string();
    let message_id = object
        .get("message_id")
        .or_else(|| object.get("messageId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let metadata = object.get("metadata").cloned().unwrap_or(Value::Null);

    let items = match object.get("items") {
        Some(Value::Array(items)) => items
            .iter()
            .map(parse_item_value)
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err("Ingress request `items` must be an array.".to_string());
        }
        None => {
            if let Some(value) = object.get("text").and_then(Value::as_str) {
                vec![IngressItem {
                    kind: IngressItemKind::Text,
                    text: Some(value.trim().to_string()),
                    path: None,
                    url: None,
                    mime_type: object
                        .get("mime_type")
                        .or_else(|| object.get("mimeType"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| Some("text/plain".to_string())),
                    transcript: None,
                    notes: object
                        .get("notes")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    metadata: Value::Null,
                }]
            } else {
                return Err("Ingress request must include `items` or top-level `text`.".to_string());
            }
        }
    };

    if items.is_empty() {
        return Err("Ingress request must contain at least one item.".to_string());
    }

    Ok(IngressRequest {
        source,
        message_id,
        items,
        metadata,
    })
}

fn parse_item_value(value: &Value) -> Result<IngressItem, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Each ingress item must be a JSON object.".to_string())?;
    parse_item_object(object)
}

fn parse_item_object(object: &Map<String, Value>) -> Result<IngressItem, String> {
    let kind_label = object
        .get("type")
        .or_else(|| object.get("kind"))
        .and_then(Value::as_str)
        .ok_or_else(|| "Ingress item is missing required `type`.".to_string())?;
    let kind = IngressItemKind::from_label(kind_label)
        .ok_or_else(|| format!("Unsupported ingress item type `{kind_label}`."))?;

    let text = object
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mime_type = object
        .get("mime_type")
        .or_else(|| object.get("mimeType"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let transcript = object
        .get("transcript")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let notes = object
        .get("notes")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if matches!(kind, IngressItemKind::Text) && text.is_none() {
        return Err("Text ingress items require non-empty `text`.".to_string());
    }

    if !matches!(kind, IngressItemKind::Text)
        && path.is_none()
        && url.is_none()
        && text.is_none()
        && transcript.is_none()
    {
        return Err(format!(
            "{} ingress items require at least one of `path`, `url`, `text`, or `transcript`.",
            kind.as_str()
        ));
    }

    Ok(IngressItem {
        kind,
        text,
        path,
        url,
        mime_type,
        transcript,
        notes,
        metadata: object.get("metadata").cloned().unwrap_or(Value::Null),
    })
}

fn classify_request(request: &IngressRequest, routing_hint: &str) -> IngressClassification {
    let signal_text = request
        .items
        .iter()
        .flat_map(|item| {
            [
                item.text.as_deref(),
                item.transcript.as_deref(),
                item.notes.as_deref(),
            ]
        })
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    let has_support_signal = contains_any(
        &signal_text,
        &[
            "support",
            "help",
            "issue",
            "bug",
            "error",
            "broken",
            "not working",
            "cannot",
            "can't",
            "unable",
            "incident",
            "escalat",
        ],
    );
    let has_billing_signal = contains_any(
        &signal_text,
        &[
            "refund",
            "billing",
            "payment",
            "charged",
            "charge",
            "subscription",
            "invoice",
            "receipt",
            "order",
        ],
    );
    let has_sales_signal = contains_any(
        &signal_text,
        &[
            "pricing", "quote", "demo", "trial", "buy", "purchase", "sales",
        ],
    );
    let has_document_signal = contains_any(
        &signal_text,
        &[
            "document",
            "form",
            "report",
            "statement",
            "contract",
            "pdf",
            "invoice",
            "receipt",
        ],
    );
    let has_urgent_signal = contains_any(
        &signal_text,
        &[
            "urgent",
            "asap",
            "immediately",
            "emergency",
            "critical",
            "sev1",
            "sev-1",
        ],
    );

    let (category, target_queue, rationale) = if has_support_signal {
        (
            "support_request",
            "support_triage",
            "support language detected in the payload",
        )
    } else if has_billing_signal {
        (
            "billing_ops",
            "billing_review",
            "billing or order language detected in the payload",
        )
    } else if has_sales_signal {
        (
            "sales_inquiry",
            "sales_follow_up",
            "sales intent detected in the payload",
        )
    } else if has_document_signal {
        (
            "document_intake",
            "document_processing",
            "document-oriented content detected in the payload",
        )
    } else if routing_hint == "multimodal_media_review" || routing_hint == "image_review" {
        (
            "media_intake",
            "media_review",
            "media assets were supplied without a stronger domain signal",
        )
    } else {
        (
            "general_intake",
            "general_triage",
            "no stronger domain signal was detected",
        )
    };

    let priority = if has_urgent_signal { "high" } else { "normal" };

    IngressClassification {
        category: category.to_string(),
        priority: priority.to_string(),
        target_queue: target_queue.to_string(),
        rationale: rationale.to_string(),
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn build_summary(
    request: &IngressRequest,
    kind_labels: &[String],
    classification: &IngressClassification,
) -> String {
    let preview = request
        .items
        .iter()
        .find_map(|item| item.text.as_deref().or(item.transcript.as_deref()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let shortened = value.chars().take(96).collect::<String>();
            if value.chars().count() > 96 {
                format!("{shortened}...")
            } else {
                shortened
            }
        });

    match preview {
        Some(preview) => format!(
            "Source `{}` submitted {} item(s) [{}]. Classified as `{}` with `{}` priority for `{}`. Preview: {}",
            request.source,
            request.items.len(),
            kind_labels.join(", "),
            classification.category,
            classification.priority,
            classification.target_queue,
            preview
        ),
        None => format!(
            "Source `{}` submitted {} item(s) [{}]. Classified as `{}` with `{}` priority for `{}`.",
            request.source,
            request.items.len(),
            kind_labels.join(", "),
            classification.category,
            classification.priority,
            classification.target_queue
        ),
    }
}

fn looks_like_json(value: &str) -> bool {
    matches!(value.chars().next(), Some('{') | Some('['))
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn next_queue_id(received_at_epoch_ms: u128) -> String {
    static NEXT_QUEUE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_QUEUE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("ingress-{received_at_epoch_ms}-{sequence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_text_as_single_text_item() {
        let request = parse_ingress_request("hello world", &Value::Null).expect("should parse");

        assert_eq!(request.source, "cli");
        assert_eq!(request.items.len(), 1);
        assert_eq!(request.items[0].kind, IngressItemKind::Text);
        assert_eq!(request.items[0].text.as_deref(), Some("hello world"));
    }

    #[test]
    fn parses_multimodal_json_payload() {
        let request = parse_ingress_request(
            r#"{"source":"whatsapp","items":[{"type":"text","text":"invoice"},{"type":"image","path":"invoice.jpg","notes":"photo"}]}"#,
            &Value::Null,
        )
        .expect("should parse");

        assert_eq!(request.source, "whatsapp");
        assert_eq!(request.items.len(), 2);
        assert_eq!(request.items[1].kind, IngressItemKind::Image);
    }

    #[test]
    fn analyzes_multimodal_payload_for_routing() {
        let request = parse_ingress_request(
            r#"{"source":"app","items":[{"type":"text","text":"review this"},{"type":"audio","path":"call.wav","transcript":"customer escalated"}]}"#,
            &Value::Null,
        )
        .expect("should parse");

        let analysis = analyze_request(&request);
        assert_eq!(analysis.total_items, 2);
        assert_eq!(analysis.audio_items, 1);
        assert_eq!(analysis.routing_hint, "multimodal_media_review");
        assert_eq!(analysis.classification.category, "support_request");
        assert_eq!(analysis.classification.priority, "normal");
    }

    #[test]
    fn classifies_support_payloads_and_sets_target_queue() {
        let request = parse_ingress_request(
            r#"{"source":"app","items":[{"type":"text","text":"Urgent support issue. The checkout is broken and needs escalation."}]}"#,
            &Value::Null,
        )
        .expect("should parse");

        let analysis = analyze_request(&request);

        assert_eq!(analysis.classification.category, "support_request");
        assert_eq!(analysis.classification.priority, "high");
        assert_eq!(analysis.classification.target_queue, "support_triage");
    }

    #[test]
    fn appends_queue_entry_to_jsonl_file() {
        let request = parse_ingress_request("hello queue", &Value::Null).expect("should parse");
        let entry = build_queue_entry(&request, analyze_request(&request));
        let path = std::env::temp_dir().join(format!("ingress-test-{}.jsonl", now_epoch_ms()));

        append_queue_entry(&path, &entry).expect("queue write should succeed");
        let contents = fs::read_to_string(&path).expect("queue file should exist");

        assert!(contents.contains("\"status\":\"queued\""));
        assert!(contents.contains("\"classification\""));
        assert!(contents.contains("\"summary\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reads_queue_entries_from_jsonl_file() {
        let request = parse_ingress_request("queue reader", &Value::Null).expect("should parse");
        let entry = build_queue_entry(&request, analyze_request(&request));
        let path = std::env::temp_dir().join(format!("ingress-read-test-{}.jsonl", now_epoch_ms()));

        append_queue_entry(&path, &entry).expect("queue write should succeed");
        let entries = read_queue_entries(&path).expect("queue entries should load");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].queue_id, entry.queue_id);
        assert_eq!(entries[0].analysis.summary, entry.analysis.summary);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn lossy_queue_reader_skips_malformed_lines() {
        let request = parse_ingress_request("queue reader", &Value::Null).expect("should parse");
        let entry = build_queue_entry(&request, analyze_request(&request));
        let path =
            std::env::temp_dir().join(format!("ingress-lossy-test-{}.jsonl", now_epoch_ms()));

        fs::write(
            &path,
            format!(
                "{{\"legacy\":true}}\n{}\n",
                serde_json::to_string(&entry).expect("entry should serialize")
            ),
        )
        .expect("queue file should be written");

        let (entries, warnings) =
            read_queue_entries_lossy(&path).expect("lossy queue reader should load");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].queue_id, entry.queue_id);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("skipping malformed queue entry"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn queue_ack_render_mentions_queue_id_and_file() {
        let rendered = QueueAck {
            queue_id: "ingress-1".to_string(),
            queue_path: "logs/ingress_queue.jsonl".to_string(),
            total_items: 2,
            classification: "document_intake".to_string(),
            priority: "normal".to_string(),
            target_queue: "document_processing".to_string(),
            routing_hint: "multimodal_document_review".to_string(),
            summary: "summary".to_string(),
        }
        .render();

        assert!(rendered.contains("Ingress classified and queued."));
        assert!(rendered.contains("Classification: document_intake"));
        assert!(rendered.contains("Queue ID: ingress-1"));
        assert!(rendered.contains("Queue file: logs/ingress_queue.jsonl"));
    }

    #[test]
    fn compact_context_packet_includes_task_contract_and_request_preview() {
        let request =
            parse_ingress_request("Need support with a broken checkout flow.", &Value::Null)
                .expect("request should parse");
        let documents = vec![(
            PathBuf::from("Workspace/MEMORY.md"),
            "# MEMORY\n\n- durable constraint\n".to_string(),
        )];
        let context_packet = compact_context_packet(
            &documents,
            Path::new("logs/conversation_history.MD"),
            &request,
        );

        assert!(context_packet.contains("Task contract"));
        assert!(context_packet.contains("broken checkout flow"));
        assert!(context_packet.contains("Workspace/MEMORY.md"));
    }
}
