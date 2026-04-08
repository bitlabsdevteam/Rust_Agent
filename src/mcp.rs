use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MCP_PROTOCOL_VERSION_LATEST: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-03-26", "2024-11-05"];
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 15_000;

#[derive(Debug, Clone)]
pub struct McpCatalog {
    pub servers: Vec<McpServerSummary>,
    pub tools: Vec<McpToolRegistration>,
}

impl McpCatalog {
    fn empty() -> Self {
        Self {
            servers: Vec::new(),
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpServerSummary {
    pub name: String,
    pub command: String,
    pub protocol_version: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone)]
pub struct McpToolRegistration {
    pub local_name: String,
    pub server_name: String,
    pub remote_name: String,
    pub description: String,
    pub input_schema: Value,
    client: Arc<Mutex<McpClient>>,
}

impl McpToolRegistration {
    pub fn call(&self, arguments: &Value) -> Result<String, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| format!("MCP server `{}` lock poisoned", self.server_name))?;
        client.call_tool(&self.remote_name, arguments)
    }

    pub fn planning_description(&self) -> String {
        let schema = schema_summary(&self.input_schema);
        if schema.is_empty() {
            format!(
                "{}: {} [MCP server `{}` remote tool `{}`]",
                self.local_name, self.description, self.server_name, self.remote_name
            )
        } else {
            format!(
                "{}: {} [MCP server `{}` remote tool `{}`; input schema: {}]",
                self.local_name, self.description, self.server_name, self.remote_name, schema
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct McpServerConfig {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    request_timeout_ms: Option<u64>,
}

#[derive(Debug)]
struct McpClient {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<Value, String>>,
    next_id: u64,
    request_timeout: Duration,
    protocol_version: String,
}

impl McpClient {
    fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let mut command = Command::new(&config.command);
        command.args(&config.args);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.envs(config.env.iter());

        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to spawn MCP server `{}` with command `{}`: {error}",
                config.name, config.command
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("MCP server `{}` did not expose stdin", config.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("MCP server `{}` did not expose stdout", config.name))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| format!("MCP server `{}` did not expose stderr", config.name))?;

        let messages = spawn_reader_threads(config.name.clone(), stdout, stderr);
        let mut client = Self {
            child,
            stdin,
            messages,
            next_id: 1,
            request_timeout: Duration::from_millis(
                config
                    .request_timeout_ms
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_MS),
            ),
            protocol_version: String::new(),
        };
        client.initialize(config)?;
        Ok(client)
    }

    fn initialize(&mut self, config: &McpServerConfig) -> Result<(), String> {
        let response = self.send_request(
            "initialize",
            Some(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION_LATEST,
                "capabilities": {},
                "clientInfo": {
                    "name": "agent_in_rust",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        )?;

        let result = response
            .get("result")
            .ok_or_else(|| format!("MCP server `{}` returned no initialize result", config.name))?;
        let protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!(
                    "MCP server `{}` returned no protocol version during initialize",
                    config.name
                )
            })?;

        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&protocol_version) {
            return Err(format!(
                "MCP server `{}` negotiated unsupported protocol version `{}`",
                config.name, protocol_version
            ));
        }

        self.protocol_version = protocol_version.to_string();
        self.send_notification("notifications/initialized", None)?;
        Ok(())
    }

    fn list_tools(&mut self) -> Result<Vec<McpDiscoveredTool>, String> {
        let mut cursor: Option<String> = None;
        let mut discovered = Vec::new();

        loop {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({ "cursor": cursor }))
                .or_else(|| Some(json!({})));
            let response = self.send_request("tools/list", params)?;
            let result = response
                .get("result")
                .ok_or_else(|| "MCP `tools/list` returned no result".to_string())?;
            let page: McpToolsListResult = serde_json::from_value(result.clone())
                .map_err(|error| format!("MCP `tools/list` parse error: {error}"))?;
            discovered.extend(page.tools);

            match page.next_cursor {
                Some(next_cursor) if !next_cursor.trim().is_empty() => {
                    cursor = Some(next_cursor);
                }
                _ => break,
            }
        }

        Ok(discovered)
    }

    fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<String, String> {
        let response = self.send_request(
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": normalize_arguments(arguments),
            })),
        )?;
        let result: McpCallToolResult = serde_json::from_value(
            response
                .get("result")
                .cloned()
                .ok_or_else(|| format!("MCP tool `{name}` returned no result"))?,
        )
        .map_err(|error| format!("MCP `tools/call` parse error for `{name}`: {error}"))?;

        if result.is_error.unwrap_or(false) {
            return Err(format_mcp_tool_result(&result));
        }

        Ok(format_mcp_tool_result(&result))
    }

    fn send_notification(&mut self, method: &str, params: Option<Value>) -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });
        self.write_message(&payload)
    }

    fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let request_id = self.next_id;
        self.next_id += 1;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params.unwrap_or_else(|| json!({}))
        });
        self.write_message(&payload)?;

        loop {
            let message = match self.messages.recv_timeout(self.request_timeout) {
                Ok(message) => message?,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "timed out waiting for MCP response to `{method}` after {} ms",
                        self.request_timeout.as_millis()
                    ))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("MCP transport closed while waiting for a response".to_string())
                }
            };

            if let Some(error) = parse_json_rpc_error(&message) {
                return Err(error);
            }

            if let Some(response_id) = message.get("id").and_then(Value::as_u64) {
                if response_id == request_id {
                    return Ok(message);
                }
                continue;
            }
        }
    }

    fn write_message(&mut self, payload: &Value) -> Result<(), String> {
        let encoded = serde_json::to_string(payload)
            .map_err(|error| format!("MCP message encode error: {error}"))?;
        self.stdin
            .write_all(encoded.as_bytes())
            .map_err(|error| format!("MCP write error: {error}"))?;
        self.stdin
            .write_all(b"\n")
            .map_err(|error| format!("MCP write error: {error}"))?;
        self.stdin
            .flush()
            .map_err(|error| format!("MCP flush error: {error}"))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn load_mcp_catalog_from_env() -> Result<McpCatalog, String> {
    let Some(raw_config) = std::env::var("MCP_SERVERS")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(McpCatalog::empty());
    };

    let configs: Vec<McpServerConfig> = serde_json::from_str(&raw_config)
        .map_err(|error| format!("Failed to parse `MCP_SERVERS` as JSON: {error}"))?;

    if configs.is_empty() {
        return Ok(McpCatalog::empty());
    }

    let mut servers = Vec::new();
    let mut tools = Vec::new();

    for config in configs {
        if config.name.trim().is_empty() {
            return Err("Each MCP server entry requires a non-empty `name`.".to_string());
        }
        if config.command.trim().is_empty() {
            return Err(format!(
                "MCP server `{}` requires a non-empty `command`.",
                config.name
            ));
        }

        let mut client = McpClient::connect(&config)?;
        let protocol_version = client.protocol_version.clone();
        let discovered_tools = client.list_tools()?;
        let tool_count = discovered_tools.len();
        let client = Arc::new(Mutex::new(client));

        for tool in discovered_tools {
            let local_name = format!("mcp::{}::{}", config.name, tool.name);
            let description = empty_to_default(
                tool.description.as_deref(),
                "No description provided by the MCP server.",
            );

            tools.push(McpToolRegistration {
                local_name,
                server_name: config.name.clone(),
                remote_name: tool.name,
                description,
                input_schema: tool.input_schema,
                client: Arc::clone(&client),
            });
        }

        servers.push(McpServerSummary {
            name: config.name,
            command: config.command,
            protocol_version,
            tool_count,
        });
    }

    Ok(McpCatalog { servers, tools })
}

fn spawn_reader_threads(
    server_name: String,
    stdout: ChildStdout,
    stderr: ChildStderr,
) -> Receiver<Result<Value, String>> {
    let (sender, receiver) = mpsc::channel();
    let stdout_sender = sender.clone();
    thread::spawn(move || read_json_lines(server_name, stdout, stdout_sender));
    thread::spawn(move || drain_stderr(stderr));
    receiver
}

fn read_json_lines(
    server_name: String,
    stdout: ChildStdout,
    sender: mpsc::Sender<Result<Value, String>>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        match line {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let parsed = serde_json::from_str::<Value>(trimmed).map_err(|error| {
                    format!(
                        "MCP server `{}` emitted invalid JSON-RPC on stdout: {error}; line={trimmed}",
                        server_name
                    )
                });
                if sender.send(parsed).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(format!(
                    "MCP server `{}` stdout read error: {error}",
                    server_name
                )));
                break;
            }
        }
    }
}

fn drain_stderr(stderr: ChildStderr) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        if line.is_err() {
            break;
        }
    }
}

fn parse_json_rpc_error(message: &Value) -> Option<String> {
    let error = message.get("error")?;
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown MCP error");
    Some(format!("JSON-RPC error {code}: {message}"))
}

fn normalize_arguments(arguments: &Value) -> Value {
    match arguments {
        Value::Object(_) => arguments.clone(),
        Value::Null => json!({}),
        _ => json!({ "value": arguments.clone() }),
    }
}

fn format_mcp_tool_result(result: &McpCallToolResult) -> String {
    let mut sections = Vec::new();

    if !result.content.is_empty() {
        let rendered_blocks = result
            .content
            .iter()
            .map(render_content_block)
            .collect::<Vec<_>>()
            .join("\n");
        if !rendered_blocks.trim().is_empty() {
            sections.push(rendered_blocks);
        }
    }

    if let Some(structured) = &result.structured_content {
        if !structured.is_null() {
            sections.push(format!(
                "Structured content:\n{}",
                serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string())
            ));
        }
    }

    if sections.is_empty() {
        "MCP tool completed with no textual content.".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn render_content_block(block: &Value) -> String {
    let block_type = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match block_type {
        "text" => block
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "MCP text block was empty.".to_string()),
        "image" | "audio" | "resource" | "resource_link" => {
            serde_json::to_string_pretty(block).unwrap_or_else(|_| block.to_string())
        }
        _ => serde_json::to_string_pretty(block).unwrap_or_else(|_| block.to_string()),
    }
}

fn schema_summary(schema: &Value) -> String {
    let Some(object) = schema.as_object() else {
        return String::new();
    };

    let required = object
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .map(|(name, definition)| {
                    let type_name = definition
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("any");
                    if required.iter().any(|required_name| required_name == name) {
                        format!("{name}: {type_name} (required)")
                    } else {
                        format!("{name}: {type_name}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    properties
}

fn empty_to_default(value: Option<&str>, default: &str) -> String {
    match value {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => default.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct McpToolsListResult {
    #[serde(default)]
    tools: Vec<McpDiscoveredTool>,
    #[serde(default, rename = "nextCursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpDiscoveredTool {
    name: String,
    description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    input_schema: Value,
}

#[derive(Debug, Deserialize)]
struct McpCallToolResult {
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default, rename = "structuredContent")]
    structured_content: Option<Value>,
    #[serde(default, rename = "isError")]
    is_error: Option<bool>,
}

impl fmt::Display for McpServerSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (command: `{}`, protocol: {}, tools: {})",
            self.name, self.command, self.protocol_version, self.tool_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_summary_marks_required_fields() {
        let summary = schema_summary(&json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "recursive": { "type": "boolean" }
            },
            "required": ["path"]
        }));

        assert!(summary.contains("path: string (required)"));
        assert!(summary.contains("recursive: boolean"));
    }

    #[test]
    fn formats_structured_mcp_tool_result() {
        let rendered = format_mcp_tool_result(&McpCallToolResult {
            content: vec![json!({ "type": "text", "text": "hello" })],
            structured_content: Some(json!({ "ok": true })),
            is_error: Some(false),
        });

        assert!(rendered.contains("hello"));
        assert!(rendered.contains("Structured content"));
    }

    #[test]
    fn normalizes_non_object_arguments() {
        assert_eq!(normalize_arguments(&Value::Null), json!({}));
        assert_eq!(
            normalize_arguments(&json!("value")),
            json!({ "value": "value" })
        );
    }
}
