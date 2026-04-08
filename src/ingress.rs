use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_QUEUE_FILE: &str = "Workspace/ingress_queue.jsonl";

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
    pub summary: String,
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
    pub routing_hint: String,
    pub summary: String,
}

impl QueueAck {
    pub fn render(&self) -> String {
        format!(
            "Ingress accepted and queued.\nQueue ID: {}\nItems: {}\nRouting hint: {}\nSummary: {}\nQueue file: {}",
            self.queue_id, self.total_items, self.routing_hint, self.summary, self.queue_path
        )
    }
}

pub fn queue_ingress(user_input: &str, arguments: &Value) -> Result<QueueAck, String> {
    let request = parse_ingress_request(user_input, arguments)?;
    let analysis = analyze_request(&request);
    let queue_path = resolve_queue_path();
    let entry = build_queue_entry(&request, analysis.clone());
    append_queue_entry(&queue_path, &entry)?;

    Ok(QueueAck {
        queue_id: entry.queue_id,
        queue_path: queue_path.display().to_string(),
        total_items: analysis.total_items,
        routing_hint: analysis.routing_hint,
        summary: analysis.summary,
    })
}

pub fn parse_ingress_request(user_input: &str, arguments: &Value) -> Result<IngressRequest, String> {
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
        text_characters += item.transcript.as_deref().unwrap_or_default().chars().count();
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

    let summary = build_summary(request, &kind_labels);

    IngressAnalysis {
        total_items: request.items.len(),
        text_items,
        image_items,
        video_items,
        audio_items,
        text_characters,
        referenced_assets,
        routing_hint,
        summary,
    }
}

pub fn build_queue_entry(request: &IngressRequest, analysis: IngressAnalysis) -> QueueEntry {
    let received_at_epoch_ms = now_epoch_ms();
    QueueEntry {
        queue_id: format!("ingress-{received_at_epoch_ms}"),
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
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create queue directory {}: {error}", parent.display()))?;
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
        .map_err(|error| format!("failed to append queue entry to {}: {error}", path.display()))
}

pub fn resolve_queue_path() -> PathBuf {
    std::env::var("AGENT_QUEUE_FILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_QUEUE_FILE))
}

fn parse_request_value(value: &Value) -> Result<Option<IngressRequest>, String> {
    if value.is_null() {
        return Ok(None);
    }

    if let Some(object) = value.as_object() {
        if object.is_empty() {
            return Ok(None);
        }

        if object.contains_key("items") || object.contains_key("source") || object.contains_key("message_id") {
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

    if !matches!(kind, IngressItemKind::Text) && path.is_none() && url.is_none() && text.is_none() && transcript.is_none() {
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

fn build_summary(request: &IngressRequest, kind_labels: &[String]) -> String {
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
            "Source `{}` submitted {} item(s) [{}]. Preview: {}",
            request.source,
            request.items.len(),
            kind_labels.join(", "),
            preview
        ),
        None => format!(
            "Source `{}` submitted {} item(s) [{}].",
            request.source,
            request.items.len(),
            kind_labels.join(", ")
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
    }

    #[test]
    fn appends_queue_entry_to_jsonl_file() {
        let request = parse_ingress_request("hello queue", &Value::Null).expect("should parse");
        let entry = build_queue_entry(&request, analyze_request(&request));
        let path = std::env::temp_dir().join(format!("ingress-test-{}.jsonl", now_epoch_ms()));

        append_queue_entry(&path, &entry).expect("queue write should succeed");
        let contents = fs::read_to_string(&path).expect("queue file should exist");

        assert!(contents.contains("\"status\":\"queued\""));
        assert!(contents.contains("\"summary\""));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn queue_ack_render_mentions_queue_id_and_file() {
        let rendered = QueueAck {
            queue_id: "ingress-1".to_string(),
            queue_path: "Workspace/ingress_queue.jsonl".to_string(),
            total_items: 2,
            routing_hint: "multimodal_document_review".to_string(),
            summary: "summary".to_string(),
        }
        .render();

        assert!(rendered.contains("Ingress accepted and queued."));
        assert!(rendered.contains("Queue ID: ingress-1"));
        assert!(rendered.contains("Queue file: Workspace/ingress_queue.jsonl"));
    }
}
