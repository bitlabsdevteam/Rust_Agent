use crate::dispatch::{DispatchRequest, DispatchResponse, Dispatcher, LocalDispatcher, ProcessDispatcher};
use crate::evals::PlannerEvalActualDecision;
use crate::mcp::{load_mcp_catalog_from_env, McpServerSummary};
use crate::memory_agent::{FileMemoryAgent, MemoryAgent, MemoryAgentSnapshot};
use crate::planner::{Decision, PlannerFailureKind, PlannerRouter, PlannerRun as PlannerRunGeneric};
#[cfg(test)]
use crate::planner::{classify_planner_failure_kind, planner_failure_run};
use crate::observability::{self, Observability};
use crate::prompt_layers::{
    ActiveSkillLayer, ChannelMetadataLayer, ObservationLayer, PlannerPromptLayers,
    ProjectMemoryLayer, SystemPromptLayer, TaskContractLayer,
};
use crate::Tools::{default_tools, Tool, LEGACY_WEB_SEARCH_TOOL_NAME, WEB_SEARCH_TOOL_NAME};
use opentelemetry::KeyValue;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are a harness-first coding agent. Start from an explicit task contract, load project memory before acting, take exactly one action per loop iteration, keep context compact, and make tools, delegation, retries, and stop conditions legible in the trace.";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4";
pub const DEFAULT_PERPLEXITY_MODEL: &str = "sonar-pro";

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1/responses";
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MEM0_BASE_URL: &str = "https://api.mem0.ai";
const FALLBACK_MODEL: &str = "Opus 4.6";
const MAX_RETRIES: u8 = 3;
const MAX_LOOP_STEPS: u8 = 8;
const MAX_IMPORT_DEPTH: usize = 5;
const MAX_TRACE_ENTRIES: usize = 24;
const MAX_COMPACTED_HISTORY_ITEMS: usize = 6;
const MAX_ACTIVE_HISTORY_ITEMS: usize = 8;
const MAX_ACTIVE_HISTORY_CHARS: usize = 3_600;
const RETAINED_RECENT_HISTORY_ITEMS: usize = 4;
const MAX_COMPACTED_SUMMARY_CHARS: usize = 1_600;
const MAX_RELEVANT_FILES: usize = 5;
const PROJECT_MEMORY_FILE: &str = "CLAUDE.md";
const SYSTEM_PROMPT_DIR: &str = "system_prompt";
const SYSTEM_PROMPT_FILE: &str = "system_prompt.md";
const CLAUDE_DIR: &str = ".claude";
const AGENTS_DIR: &str = "agents";
const COMMANDS_DIR: &str = "commands";
const SKILLS_DIR: &str = "skills";
const WORKSPACE_DIR: &str = "Workspace";
const SHORT_TERM_MEMORY_FILE: &str = "short-term.json";
const LONG_TERM_MEMORY_FILE: &str = "MEMORY.md";
const HISTORY_DIR: &str = "history";
const CONTEXT_HISTORY_FILE: &str = "context_history.json";
const MAX_SHORT_TERM_HISTORY_ITEMS: usize = 8;
const MAX_LONG_TERM_MEMORY_ITEMS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Success(String),
    SkillActivated(SkillActivation),
    Retry(String),
}

impl fmt::Display for StepOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success(output) => write!(f, "success: {output}"),
            Self::SkillActivated(activation) => write!(f, "skill activated: {}", activation.name),
            Self::Retry(reason) => write!(f, "retry: {reason}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResult {
    pub output: String,
    pub trace: Vec<String>,
    pub usage: Vec<TokenUsageRecord>,
    pub stopped: bool,
}

impl AgentResult {
    fn completed(output: String, trace: Vec<String>, usage: Vec<TokenUsageRecord>) -> Self {
        Self {
            output,
            trace,
            usage,
            stopped: false,
        }
    }

    fn stopped(
        output: impl Into<String>,
        trace: Vec<String>,
        usage: Vec<TokenUsageRecord>,
    ) -> Self {
        Self {
            output: output.into(),
            trace,
            usage,
            stopped: true,
        }
    }

    pub fn render_usage_summary(&self) -> String {
        render_usage_summary(&self.usage)
    }
}

#[derive(Debug, Clone)]
struct AgentConfig {
    system_prompt: String,
    default_model: &'static str,
    fallback_model: &'static str,
    planner_backend: String,
    max_retries: u8,
    allow_heuristic_planner: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConversationMessage {
    role: MessageRole,
    content: String,
}

impl ConversationMessage {
    fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct SessionState {
    history: Vec<ConversationMessage>,
    trace: Vec<String>,
    compacted_summary: Option<String>,
    observations: Vec<String>,
    active_skill: Option<ActiveSkill>,
}

impl SessionState {
    fn record(&mut self, entry: impl Into<String>) {
        self.trace.push(entry.into());
        if self.trace.len() > MAX_TRACE_ENTRIES {
            let remove_count = self.trace.len() - MAX_TRACE_ENTRIES;
            self.trace.drain(0..remove_count);
        }
    }

    fn clear(&mut self) {
        self.history.clear();
        self.trace.clear();
        self.compacted_summary = None;
        self.observations.clear();
        self.active_skill = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitModeConfig {
    pub prompt_label: String,
    pub show_trace: bool,
    pub bin_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MemoryScope {
    User,
    Project,
    Imported,
}

impl fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Imported => "imported",
        };
        write!(f, "{label}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySource {
    pub scope: String,
    pub path: PathBuf,
    pub contents: String,
    pub imported_from: Option<PathBuf>,
    pub selector_hint: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryLoadRequest {
    pub focus_paths: Vec<PathBuf>,
}

impl MemoryLoadRequest {
    pub fn normalized_focus_paths(&self, root_dir: &Path) -> Vec<PathBuf> {
        let mut normalized = Vec::new();
        let mut seen = BTreeSet::new();

        for path in &self.focus_paths {
            let candidate = if path.is_absolute() {
                path.strip_prefix(root_dir).ok().map(Path::to_path_buf)
            } else {
                Some(path.clone())
            };
            let Some(candidate) = candidate else {
                continue;
            };
            let Some(candidate) = normalize_repo_relative_path(&candidate) else {
                continue;
            };
            if seen.insert(candidate.clone()) {
                normalized.push(candidate);
            }
        }

        normalized
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryStack {
    pub sources: Vec<MemorySource>,
    pub merged_instructions: String,
    pub load_request: MemoryLoadRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPacket {
    pub goal: String,
    pub constraints: Vec<String>,
    pub relevant_files: Vec<String>,
    pub known_facts: Vec<String>,
    pub missing_facts: Vec<String>,
    pub next_action: String,
    pub stop_condition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskContract {
    goal: String,
    constraints: Vec<String>,
    acceptance_criteria: Vec<String>,
    relevant_files: Vec<String>,
    stop_condition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentMemoryPaths {
    short_term_snapshot: PathBuf,
    long_term_notes: PathBuf,
}

#[derive(Debug, Clone)]
struct Mem0Config {
    api_key: String,
    base_url: String,
    user_id: String,
    agent_id: String,
    app_id: String,
    org_id: Option<String>,
    project_id: Option<String>,
    workspace_label: String,
}

impl Mem0Config {
    fn from_env(root_dir: &Path) -> Option<Self> {
        let api_key = configured_env("MEM0_API_KEY")?;
        if api_key.starts_with("replace-with-") || api_key.starts_with("your-") {
            return None;
        }

        let base_url = configured_env("MEM0_BASE_URL")
            .unwrap_or_else(|| DEFAULT_MEM0_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let workspace_label = fs::canonicalize(root_dir)
            .unwrap_or_else(|_| root_dir.to_path_buf())
            .display()
            .to_string();
        let user_id = configured_env("MEM0_USER_ID")
            .or_else(|| configured_env("USER"))
            .or_else(|| configured_env("USERNAME"))
            .unwrap_or_else(|| "local-user".to_string());
        let agent_id =
            configured_env("MEM0_AGENT_ID").unwrap_or_else(|| env!("CARGO_PKG_NAME").to_string());
        let app_id = configured_env("MEM0_APP_ID").unwrap_or_else(|| workspace_label.clone());

        Some(Self {
            api_key,
            base_url,
            user_id,
            agent_id,
            app_id,
            org_id: configured_env("MEM0_ORG_ID"),
            project_id: configured_env("MEM0_PROJECT_ID"),
            workspace_label,
        })
    }

    fn source_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "mem0://user/{}/agent/{}/app/{}",
            sanitize_mem0_path_segment(&self.user_id),
            sanitize_mem0_path_segment(&self.agent_id),
            sanitize_mem0_path_segment(&self.app_id),
        ))
    }

    fn fetch_long_term_memory(&self) -> io::Result<MemorySource> {
        let client = http_client()?;
        let mut payload = json!({
            "filters": {
                "AND": [
                    { "user_id": self.user_id.clone() },
                    { "agent_id": self.agent_id.clone() },
                    { "app_id": self.app_id.clone() }
                ]
            },
            "page": 1,
            "page_size": MAX_LONG_TERM_MEMORY_ITEMS
        });
        insert_optional_json_field(&mut payload, "org_id", self.org_id.clone());
        insert_optional_json_field(&mut payload, "project_id", self.project_id.clone());

        let response = client
            .post(format!("{}/v2/memories", self.base_url))
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .map_err(mem0_io_error)?;

        let status = response.status();
        let body = response.text().map_err(mem0_io_error)?;
        if !status.is_success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Mem0 get memories failed with HTTP {}: {}",
                    status.as_u16(),
                    body
                ),
            ));
        }

        Ok(MemorySource {
            scope: MemoryScope::Project.to_string(),
            path: self.source_path(),
            contents: render_mem0_memory_contents(self, &parse_mem0_list_response(&body)?),
            imported_from: None,
            selector_hint: None,
        })
    }

    fn append_long_term_memory(&self, note: &str) -> io::Result<()> {
        let client = http_client()?;
        let mut payload = json!({
            "messages": [
                {
                    "role": "user",
                    "content": note.trim()
                }
            ],
            "user_id": self.user_id.clone(),
            "agent_id": self.agent_id.clone(),
            "app_id": self.app_id.clone(),
            "async_mode": false,
            "output_format": "v1.1",
            "version": "v2",
            "metadata": {
                "source": "agent_in_rust",
                "kind": "long_term_memory",
                "workspace": self.workspace_label.clone()
            }
        });
        insert_optional_json_field(&mut payload, "org_id", self.org_id.clone());
        insert_optional_json_field(&mut payload, "project_id", self.project_id.clone());

        let response = client
            .post(format!("{}/v1/memories", self.base_url))
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .map_err(mem0_io_error)?;

        let status = response.status();
        let body = response.text().map_err(mem0_io_error)?;
        if !status.is_success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Mem0 add memory failed with HTTP {}: {}",
                    status.as_u16(),
                    body
                ),
            ));
        }
        if parse_mem0_add_response(&body)?.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Mem0 add memory returned no results.",
            ));
        }
        Ok(())
    }
}

struct Mem0MemoryAgent<'a> {
    config: &'a Mem0Config,
}

impl<'a> Mem0MemoryAgent<'a> {
    fn new(config: &'a Mem0Config) -> Self {
        Self { config }
    }
}

impl MemoryAgent for Mem0MemoryAgent<'_> {
    fn backend_label(&self) -> &str {
        "Mem0"
    }

    fn read_long_term_memory(&self) -> io::Result<MemoryAgentSnapshot> {
        let source = self.config.fetch_long_term_memory()?;
        Ok(MemoryAgentSnapshot {
            path: source.path,
            contents: source.contents,
        })
    }

    fn append_long_term_note(&self, note: &str) -> io::Result<()> {
        self.config.append_long_term_memory(note)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ShortTermMemorySnapshot {
    compacted_summary: Option<String>,
    observations: Vec<String>,
    recent_history: Vec<ConversationMessage>,
    active_skill: Option<ActiveSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    pub compacted: bool,
    pub reason: String,
    pub compacted_messages: usize,
    pub retained_messages: usize,
    pub summary: Option<String>,
}

impl CompactionResult {
    fn no_op(reason: impl Into<String>, state: &SessionState) -> Self {
        Self {
            compacted: false,
            reason: reason.into(),
            compacted_messages: 0,
            retained_messages: state.history.len(),
            summary: state.compacted_summary.clone(),
        }
    }

    pub fn render(&self) -> String {
        if !self.compacted {
            return format!("Context compaction skipped.\nReason: {}", self.reason);
        }

        let summary = self
            .summary
            .as_deref()
            .unwrap_or("No compacted summary was produced.");
        format!(
            "Context compacted.\nCompacted messages: {}\nRetained recent messages: {}\nReason: {}\nSummary: {}",
            self.compacted_messages, self.retained_messages, self.reason, summary
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DefinitionScope {
    User,
    Project,
}

impl fmt::Display for DefinitionScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::User => "user",
            Self::Project => "project",
        };
        write!(f, "{label}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSpec {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub tool_allowlist: Option<Vec<String>>,
    pub scope: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub version: Option<String>,
    pub tags: Vec<String>,
    pub when_to_use: Option<String>,
    pub when_not_to_use: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub preferred_subagents: Vec<String>,
    pub input_contract: Option<String>,
    pub output_contract: Option<String>,
    pub scope: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSkill {
    pub name: String,
    pub source_path: PathBuf,
    pub activated_by: String,
    pub task_fit: String,
    pub allowed_tools: Option<Vec<String>>,
    pub preferred_subagents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillActivation {
    pub name: String,
    pub source_path: PathBuf,
    pub scope: String,
    pub activated_by: String,
    pub task_fit: String,
    pub allowed_tools: Option<Vec<String>>,
    pub preferred_subagents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillValidationIssue {
    path: PathBuf,
    message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LoadedSkills {
    entries: BTreeMap<String, SkillSpec>,
    issues: Vec<SkillValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsageRecord {
    pub actor: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub execution: String,
    pub note: String,
}

impl TokenUsageRecord {
    fn zero(
        actor: impl Into<String>,
        execution: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            actor: actor.into(),
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            execution: execution.into(),
            note: note.into(),
        }
    }

    fn trace_line(&self) -> String {
        format!(
            "{}: {} input / {} output / {} total ({})",
            self.actor, self.input_tokens, self.output_tokens, self.total_tokens, self.note
        )
    }

    fn summary_line(&self) -> String {
        format!(
            "- {}: {} input, {} output, {} total [{}; {}]",
            self.actor,
            self.input_tokens,
            self.output_tokens,
            self.total_tokens,
            self.execution,
            self.note
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DelegationOutcome {
    response: DispatchResponse,
    launch_mode: &'static str,
}

type PlannerRun = PlannerRunGeneric<TokenUsageRecord>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CustomCommand {
    name: String,
    description: String,
    template: String,
    source_path: PathBuf,
    scope: DefinitionScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCreateRequest {
    pub name: String,
    pub description: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallRequest {
    pub source: String,
    pub skill_name: Option<String>,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SlashCommand {
    Help,
    Agents,
    Skills,
    Memory,
    Remember(String),
    Model,
    Clear,
    Compact,
    Mcp,
    Review(String),
    Skill { name: String, task: String },
    Init,
    Agent { name: String, task: String },
    Exit,
    Custom { name: String, arguments: String },
    Unknown(String),
}

pub struct MainAgent {
    root_dir: PathBuf,
    home_dir: Option<PathBuf>,
    memory_paths: AgentMemoryPaths,
    mem0: Option<Mem0Config>,
    config: AgentConfig,
    llm_engine: Option<OpenAiEngine>,
    fallback_engine: Option<AnthropicEngine>,
    tools: Vec<Tool>,
    mcp_servers: Vec<McpServerSummary>,
    observability: Observability,
    memory_stack: MemoryStack,
    short_term_memory: ShortTermMemorySnapshot,
    subagents: BTreeMap<String, SubagentSpec>,
    skills: BTreeMap<String, SkillSpec>,
    skill_validation_issues: Vec<SkillValidationIssue>,
    commands: BTreeMap<String, CustomCommand>,
}

impl MainAgent {
    pub fn from_env(system_prompt: String) -> io::Result<Self> {
        let root_dir = env::current_dir()?;
        let home_dir = env::var("HOME").ok().map(PathBuf::from);
        let catalog = load_mcp_catalog_from_env()
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
        let mut tools = default_tools();
        tools.extend(catalog.tools.into_iter().map(Tool::from_mcp));
        let llm_engine = OpenAiEngine::from_env();
        let fallback_engine = AnthropicEngine::from_env();
        let mem0 = Mem0Config::from_env(&root_dir);

        let planner_backend = match (&llm_engine, &fallback_engine) {
            (Some(primary), Some(fallback)) => format!(
                "OpenAI Responses planner ({}) -> Anthropic planner ({})",
                primary.model, fallback.model
            ),
            (Some(primary), None) => {
                format!("OpenAI Responses planner ({})", primary.model)
            }
            (None, Some(fallback)) => format!("Anthropic planner ({})", fallback.model),
            (None, None) => "local heuristic planner".to_string(),
        };

        let mut agent = Self {
            root_dir,
            home_dir,
            memory_paths: AgentMemoryPaths {
                short_term_snapshot: PathBuf::new(),
                long_term_notes: PathBuf::new(),
            },
            mem0,
            config: AgentConfig {
                system_prompt,
                default_model: "OpenAI GPT-5.4",
                fallback_model: FALLBACK_MODEL,
                planner_backend,
                max_retries: MAX_RETRIES,
                allow_heuristic_planner: llm_engine.is_none() && fallback_engine.is_none(),
            },
            llm_engine,
            fallback_engine,
            tools,
            mcp_servers: catalog.servers,
            observability: Observability::from_env(),
            memory_stack: MemoryStack::default(),
            short_term_memory: ShortTermMemorySnapshot::default(),
            subagents: BTreeMap::new(),
            skills: BTreeMap::new(),
            skill_validation_issues: Vec::new(),
            commands: BTreeMap::new(),
        };
        agent.memory_paths = agent_memory_paths(&agent.root_dir);
        agent.refresh_project_state()?;
        Ok(agent)
    }

    pub fn init_project_files(&mut self) -> io::Result<Vec<PathBuf>> {
        let created = scaffold_claude_project(&self.root_dir)?;
        self.refresh_project_state()?;
        Ok(created)
    }

    pub fn create_skill(&mut self, request: SkillCreateRequest) -> io::Result<PathBuf> {
        let scope = parse_definition_scope(&request.scope)?;
        let path = create_skill_scaffold(
            &self.root_dir,
            self.home_dir.as_deref(),
            scope,
            &request.name,
            &request.description,
        )?;
        self.refresh_project_state()?;
        Ok(path)
    }

    pub fn install_skill(&mut self, request: SkillInstallRequest) -> io::Result<PathBuf> {
        let scope = parse_definition_scope(&request.scope)?;
        let path = install_skill_from_source(
            &self.root_dir,
            self.home_dir.as_deref(),
            scope,
            &request.source,
            request.skill_name.as_deref(),
        )?;
        self.refresh_project_state()?;
        Ok(path)
    }

    pub fn preview_planner_decision(
        &self,
        input: &str,
        observations: &[String],
    ) -> PlannerEvalActualDecision {
        if input.trim().is_empty() {
            return PlannerEvalActualDecision {
                action: "stop".to_string(),
                tool_name: None,
                tool_arguments_json: None,
                skill_name: None,
                subagent_name: None,
                reason: "user input was empty".to_string(),
                planner_backend: self.config.planner_backend.clone(),
                reasoning: vec![
                    "The eval preview applies the harness stop condition before planner routing."
                        .to_string(),
                ],
                failure_class: None,
            };
        }
        if let Some(observation) = observations.last() {
            if observation.starts_with("Recoverable ")
                || observation.starts_with("Planner retry request:")
            {
                return PlannerEvalActualDecision {
                    action: "retry".to_string(),
                    tool_name: None,
                    tool_arguments_json: None,
                    skill_name: None,
                    subagent_name: None,
                    reason: "the latest observation still reflects a recoverable failure"
                        .to_string(),
                    planner_backend: self.config.planner_backend.clone(),
                    reasoning: vec![
                        "The eval preview preserves harness retry semantics from the latest observation."
                            .to_string(),
                    ],
                    failure_class: Some(PlannerFailureKind::Recoverable.label().to_string()),
                };
            }

            return PlannerEvalActualDecision {
                action: "finish".to_string(),
                tool_name: None,
                tool_arguments_json: None,
                skill_name: None,
                subagent_name: None,
                reason: "the latest observation is now the best available answer".to_string(),
                planner_backend: self.config.planner_backend.clone(),
                reasoning: vec![
                    "The eval preview finishes when the latest observation already answers the task."
                        .to_string(),
                ],
                failure_class: None,
            };
        }
        let state = SessionState::default();
        let run = heuristic_plan(
            input,
            &self.tools,
            &self.subagents,
            &self.skills,
            state.active_skill.as_ref(),
            observations,
        );
        let planner_backend = if self.config.planner_backend == "local heuristic planner" {
            self.config.planner_backend.clone()
        } else {
            format!(
                "local heuristic eval preview (configured: {})",
                self.config.planner_backend
            )
        };
        planner_eval_actual_decision(run, &planner_backend)
    }

    fn refresh_project_state(&mut self) -> io::Result<()> {
        self.memory_stack =
            load_memory_stack(&self.root_dir, self.home_dir.as_deref(), self.mem0.as_ref())?;
        self.short_term_memory = load_short_term_memory(&self.memory_paths.short_term_snapshot)?;
        self.subagents = load_subagent_specs(&self.root_dir, self.home_dir.as_deref())?;
        let loaded_skills = load_skill_specs(
            &self.root_dir,
            self.home_dir.as_deref(),
            &self.tools,
            &self.subagents,
        )?;
        self.skills = loaded_skills.entries;
        self.skill_validation_issues = loaded_skills.issues;
        self.commands = load_custom_commands(&self.root_dir, self.home_dir.as_deref())?;
        Ok(())
    }

    pub fn wait_for_context(&mut self, config: WaitModeConfig) -> io::Result<()> {
        let mut show_trace = config.show_trace;
        let mut state = self.load_session_state();

        println!("Claude-style session");
        println!(
            "Default model: {} | Fallback model: {}",
            self.default_model(),
            self.fallback_model()
        );
        println!("Planner backend: {}", self.planner_backend_label());
        println!("{}", self.chat_help_text(&config.bin_name));

        let stdin = io::stdin();
        loop {
            print!("{}> ", config.prompt_label);
            io::stdout().flush()?;

            let mut buffer = String::new();
            let bytes_read = stdin.read_line(&mut buffer)?;
            if bytes_read == 0 {
                println!("\nSession ended.");
                break;
            }

            let input = buffer.trim();
            if input.is_empty() {
                continue;
            }

            if input.starts_with('/') {
                match self.handle_slash_command(&mut state, input, &config.bin_name)? {
                    CommandOutcome::Continue(output) => {
                        println!("{output}");
                    }
                    CommandOutcome::Exit(output) => {
                        println!("{output}");
                        break;
                    }
                    CommandOutcome::ToggleTrace => {
                        show_trace = !show_trace;
                        println!(
                            "Trace output is now {}.",
                            if show_trace { "on" } else { "off" }
                        );
                    }
                }
                continue;
            }

            let result = self.run_with_state(&mut state, input);
            println!("{}", result.output);
            println!("\n{}", result.render_usage_summary());
            if show_trace {
                println!("\nTrace:");
                for entry in &result.trace {
                    println!("- {entry}");
                }
            }
        }

        Ok(())
    }

    pub fn run(&self, input: &str) -> AgentResult {
        let mut state = self.load_session_state();
        self.run_with_state(&mut state, input)
    }

    pub fn compact_context(&mut self) -> io::Result<CompactionResult> {
        let mut state = self.load_session_state();
        let result = compact_session_state(&mut state, true);
        if result.compacted {
            save_short_term_memory(&self.memory_paths.short_term_snapshot, &state)?;
            self.short_term_memory =
                load_short_term_memory(&self.memory_paths.short_term_snapshot)?;
        }
        Ok(result)
    }

    fn load_session_state(&self) -> SessionState {
        let snapshot = load_short_term_memory(&self.memory_paths.short_term_snapshot)
            .unwrap_or_else(|_| self.short_term_memory.clone());
        SessionState {
            history: snapshot.recent_history,
            trace: Vec::new(),
            compacted_summary: snapshot.compacted_summary,
            observations: snapshot.observations,
            active_skill: snapshot.active_skill,
        }
    }

    fn run_with_state(&self, state: &mut SessionState, input: &str) -> AgentResult {
        self.observability.with_span(
            "main_agent.run",
            vec![
                KeyValue::new("planner.backend", self.config.planner_backend.clone()),
                KeyValue::new("models.default", self.config.default_model.to_string()),
                KeyValue::new("models.fallback", self.config.fallback_model.to_string()),
                KeyValue::new("input.preview", observability::compact_text(input, 512)),
            ],
            || {
                if input.trim().is_empty() {
                    return AgentResult::stopped(
                        "Empty input. Nothing to do.",
                        vec!["Stop condition met: user input was empty.".to_string()],
                        vec![TokenUsageRecord::zero(
                            "main agent request",
                            "no-op",
                            "empty input; no model API call",
                        )],
                    );
                }

                let mut trace = vec![
                    format!(
                        "Loaded {} memory source(s).",
                        self.memory_stack.sources.len()
                    ),
                    format!("Loaded {} subagent spec(s).", self.subagents.len()),
                    format!("Loaded {} skill spec(s).", self.skills.len()),
                    format!(
                        "Skipped {} invalid skill file(s).",
                        self.skill_validation_issues.len()
                    ),
                    format!("Loaded {} custom command(s).", self.commands.len()),
                ];
                let compaction = compact_session_state(state, false);
                if compaction.compacted {
                    trace.push(format!(
                        "Auto-compacted session context: {} message(s) summarized; {} recent message(s) retained.",
                        compaction.compacted_messages, compaction.retained_messages
                    ));
                }
                if let Some(summary) = &state.compacted_summary {
                    trace.push(format!(
                        "Compacted session summary is active: {}",
                        observability::compact_text(summary, 220)
                    ));
                }
                trace.push(format!(
                    "User input received: {}",
                    observability::compact_text(input, 240)
                ));
                let mut usage = Vec::new();
                let mut retry_count = 0u8;
                let mut step_count = 0u8;
                let mut observations = state.observations.clone();

                loop {
                    if step_count >= MAX_LOOP_STEPS {
                        let reason = format!(
                            "Stop condition met: loop reached {MAX_LOOP_STEPS} steps without a final answer."
                        );
                        trace.push(reason.clone());
                        if usage.is_empty() {
                            usage.push(TokenUsageRecord::zero(
                                "main agent request",
                                "local heuristic router",
                                "no model API call",
                            ));
                        }
                        trace.extend(usage_trace_lines(&usage));
                        record_usage_observability(&usage);
                        for entry in &trace {
                            state.record(entry.clone());
                        }
                        state.observations = observations;
                        state.active_skill = None;
                        self.persist_short_term_memory(state, &mut trace);
                        return AgentResult::stopped(reason, trace, usage);
                    }

                    step_count += 1;
                    let task_contract = self.build_task_contract(input, &observations);
                    trace.push(format!("Iteration {step_count}: begin."));
                    trace.push(format!(
                        "Iteration {step_count}: model policy default=`{}` fallback=`{}`.",
                        self.config.default_model, self.config.fallback_model
                    ));
                    trace.push(format!(
                        "Iteration {step_count}: planner backend `{}`.",
                        self.config.planner_backend
                    ));
                    trace.push(format!(
                        "Iteration {step_count}: task goal = {}",
                        task_contract.goal
                    ));
                    trace.push(format!(
                        "Iteration {step_count}: acceptance criteria = {}",
                        task_contract.acceptance_criteria.join(" | ")
                    ));
                    trace.push(format!(
                        "Iteration {step_count}: relevant files = {}",
                        if task_contract.relevant_files.is_empty() {
                            "none inferred".to_string()
                        } else {
                            task_contract.relevant_files.join(", ")
                        }
                    ));
                    if let Some(observation) = observations.last() {
                        trace.push(format!(
                            "Iteration {step_count}: latest observation = {}",
                            observability::compact_text(observation, 240)
                        ));
                    }
                    if let Some(active_skill) = &state.active_skill {
                        trace.push(format!(
                            "Iteration {step_count}: active skill = `{}` (activated_by={}, task_fit={}).",
                            active_skill.name,
                            active_skill.activated_by,
                            active_skill.task_fit
                        ));
                    }
                    if retry_count > 0 {
                        trace.push(format!(
                            "Iteration {step_count}: retry budget used {retry_count}/{}.",
                            self.config.max_retries
                        ));
                    }

                    let planner_run =
                        self.decide_next_step(state, input, &task_contract, &observations, retry_count);
                    trace.push(format!(
                        "Iteration {step_count}: planner decision = {}",
                        planner_run.decision
                    ));
                    trace.extend(planner_run.trace.iter().map(|item| {
                        format!("Iteration {step_count}: {item}")
                    }));
                    trace.extend(planner_run.reasoning.iter().map(|item| {
                        format!("Iteration {step_count}: planner reasoning = {item}")
                    }));
                    if let Some(failure_kind) = planner_run.failure_kind {
                        trace.push(format!(
                            "Iteration {step_count}: planner failure class = {}.",
                            failure_kind.label()
                        ));
                    }
                    if let Some(record) = planner_run.usage {
                        usage.push(record);
                    } else if !usage.iter().any(|record| record.actor == "main agent request") {
                        usage.push(TokenUsageRecord::zero(
                            "main agent request",
                            "local heuristic router",
                            "no model API call",
                        ));
                    }

                    match planner_run.decision {
                        Decision::CallTool {
                            tool_name,
                            arguments,
                            reason,
                        } => {
                            trace.push(format!(
                                "Iteration {step_count}: executing tool `{tool_name}` because {reason}"
                            ));
                            match self.call_tool(
                                &tool_name,
                                input,
                                &arguments,
                                state.active_skill.as_ref(),
                            ) {
                                StepOutcome::Success(output) => {
                                    let observation =
                                        format!("Tool `{tool_name}` observation:\n{output}");
                                    trace.push(format!(
                                        "Iteration {step_count}: tool observation recorded."
                                    ));
                                    observations.push(observation);
                                }
                                StepOutcome::Retry(reason) => {
                                    retry_count += 1;
                                    let observation = format!(
                                        "Recoverable tool failure from `{tool_name}`: {reason}"
                                    );
                                    trace.push(format!(
                                        "Iteration {step_count}: {observation}"
                                    ));
                                    observations.push(observation);
                                    if retry_count >= self.config.max_retries {
                                        let stop_reason = format!(
                                            "Retry limit reached after recoverable tool failure: {reason}"
                                        );
                                        trace.push(format!(
                                            "Iteration {step_count}: stop reason = {stop_reason}"
                                        ));
                                        if usage.is_empty() {
                                            usage.push(TokenUsageRecord::zero(
                                                "main agent request",
                                                "local heuristic router",
                                                "no model API call",
                                            ));
                                        }
                                        trace.extend(usage_trace_lines(&usage));
                                        record_usage_observability(&usage);
                                        for entry in &trace {
                                            state.record(entry.clone());
                                        }
                                        state.observations = observations;
                                        state.active_skill = None;
                                        self.persist_short_term_memory(state, &mut trace);
                                        return AgentResult::stopped(stop_reason, trace, usage);
                                    }
                                }
                                StepOutcome::SkillActivated(_) => {
                                    observations.push(format!(
                                        "Recoverable tool failure from `{tool_name}`: tool execution returned a skill activation unexpectedly"
                                    ));
                                }
                            }
                        }
                        Decision::UseSkill { skill_name, reason } => {
                            trace.push(format!(
                                "Iteration {step_count}: activating skill `{skill_name}` because {reason}"
                            ));
                            match self.apply_skill(&skill_name, input, "planner") {
                                StepOutcome::SkillActivated(activation) => {
                                    let was_replaced = state.active_skill.as_ref().map(|skill| skill.name.clone());
                                    state.active_skill = Some(ActiveSkill {
                                        name: activation.name.clone(),
                                        source_path: activation.source_path.clone(),
                                        activated_by: activation.activated_by.clone(),
                                        task_fit: activation.task_fit.clone(),
                                        allowed_tools: activation.allowed_tools.clone(),
                                        preferred_subagents: activation.preferred_subagents.clone(),
                                    });
                                    trace.push(format!(
                                        "Iteration {step_count}: active skill is now `{}` [{}] from {}.",
                                        activation.name,
                                        activation.scope,
                                        activation.source_path.display()
                                    ));
                                    if let Some(previous) = was_replaced {
                                        if previous != activation.name {
                                            trace.push(format!(
                                                "Iteration {step_count}: replaced previously active skill `{previous}`."
                                            ));
                                        }
                                    }
                                    continue;
                                }
                                StepOutcome::Retry(reason) => {
                                    retry_count += 1;
                                    let observation = format!(
                                        "Recoverable skill failure from `{skill_name}`: {reason}"
                                    );
                                    trace.push(format!(
                                        "Iteration {step_count}: {observation}"
                                    ));
                                    observations.push(observation);
                                    if retry_count >= self.config.max_retries {
                                        let stop_reason = format!(
                                            "Retry limit reached after recoverable skill failure: {reason}"
                                        );
                                        trace.push(format!(
                                            "Iteration {step_count}: stop reason = {stop_reason}"
                                        ));
                                        if usage.is_empty() {
                                            usage.push(TokenUsageRecord::zero(
                                                "main agent request",
                                                "local heuristic router",
                                                "no model API call",
                                            ));
                                        }
                                        trace.extend(usage_trace_lines(&usage));
                                        record_usage_observability(&usage);
                                        for entry in &trace {
                                            state.record(entry.clone());
                                        }
                                        state.observations = observations;
                                        state.active_skill = None;
                                        self.persist_short_term_memory(state, &mut trace);
                                        return AgentResult::stopped(stop_reason, trace, usage);
                                    }
                                }
                                StepOutcome::Success(output) => {
                                    observations.push(output);
                                }
                            }
                        }
                        Decision::DelegateSubagent {
                            subagent_name,
                            reason,
                        } => {
                            trace.push(format!(
                                "Iteration {step_count}: delegating to subagent `{subagent_name}` because {reason}"
                            ));
                            match self.delegate_to_subagent(&subagent_name, state, input) {
                                Ok(outcome) => {
                                    observability::record_subagent_dispatch_event(
                                        "result",
                                        outcome.response.dispatch_id.as_str(),
                                        &subagent_name,
                                        outcome.launch_mode,
                                        None,
                                    );
                                    crate::runtime_log::info(
                                        "subagent",
                                        format!(
                                            "dispatch_id={} subagent={} launch_mode={} status=result summary={}",
                                            outcome.response.dispatch_id.as_str(),
                                            subagent_name,
                                            outcome.launch_mode,
                                            observability::compact_text(
                                                &outcome.response.summary,
                                                200
                                            )
                                        ),
                                    );
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent launch mode = `{}`.",
                                        outcome.launch_mode
                                    ));
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent dispatch_id = `{}`.",
                                        outcome.response.dispatch_id.as_str()
                                    ));
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent responder = {}.",
                                        outcome.response.responder
                                    ));
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent result summary = {}",
                                        outcome.response.summary
                                    ));
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent recommended next action = {}",
                                        outcome.response.recommended_next_action
                                    ));
                                    usage.extend(outcome.response.usage.clone());
                                    observations.push(format!(
                                        "Subagent `{subagent_name}` observation:\n{}",
                                        outcome.response.render()
                                    ));
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent observation recorded."
                                    ));
                                }
                                Err(reason) => {
                                    observability::record_subagent_dispatch_event(
                                        "failure",
                                        "unknown",
                                        &subagent_name,
                                        "unknown",
                                        Some(reason.clone()),
                                    );
                                    crate::runtime_log::warn(
                                        "subagent",
                                        format!(
                                            "subagent={} status=failure error={}",
                                            subagent_name, reason
                                        ),
                                    );
                                    retry_count += 1;
                                    let observation =
                                        format!("Recoverable delegation failure: {reason}");
                                    trace.push(format!(
                                        "Iteration {step_count}: {observation}"
                                    ));
                                    observations.push(observation);
                                    if retry_count >= self.config.max_retries {
                                        let stop_reason = format!(
                                            "Retry limit reached after delegation failure: {reason}"
                                        );
                                        trace.push(format!(
                                            "Iteration {step_count}: stop reason = {stop_reason}"
                                        ));
                                        if usage.is_empty() {
                                            usage.push(TokenUsageRecord::zero(
                                                "main agent request",
                                                "local heuristic router",
                                                "no model API call",
                                            ));
                                        }
                                        trace.extend(usage_trace_lines(&usage));
                                        record_usage_observability(&usage);
                                        for entry in &trace {
                                            state.record(entry.clone());
                                        }
                                        state.observations = observations;
                                        state.active_skill = None;
                                        self.persist_short_term_memory(state, &mut trace);
                                        return AgentResult::stopped(stop_reason, trace, usage);
                                    }
                                }
                            }
                        }
                        Decision::Finish { answer, reason } => {
                            trace.push(format!(
                                "Iteration {step_count}: stop reason = final answer produced ({reason})."
                            ));
                            state.history.push(ConversationMessage::user(input));
                            state
                                .history
                                .push(ConversationMessage::assistant(answer.clone()));
                            state.observations = observations;
                            state.active_skill = None;
                            self.persist_short_term_memory(state, &mut trace);
                            if usage.is_empty() {
                                usage.push(TokenUsageRecord::zero(
                                    "main agent request",
                                    "local heuristic router",
                                    "no model API call",
                                ));
                            }
                            trace.extend(usage_trace_lines(&usage));
                            record_usage_observability(&usage);
                            for entry in &trace {
                                state.record(entry.clone());
                            }
                            return AgentResult::completed(answer, trace, usage);
                        }
                        Decision::Retry(reason) => {
                            retry_count += 1;
                            trace.push(format!(
                                "Iteration {step_count}: planner requested retry because {reason}"
                            ));
                            observations.push(format!("Planner retry request: {reason}"));
                            if retry_count >= self.config.max_retries {
                                let stop_reason =
                                    format!("Retry limit reached because planner requested retry: {reason}");
                                trace.push(format!(
                                    "Iteration {step_count}: stop reason = {stop_reason}"
                                ));
                                if usage.is_empty() {
                                    usage.push(TokenUsageRecord::zero(
                                        "main agent request",
                                        "local heuristic router",
                                        "no model API call",
                                    ));
                                }
                                trace.extend(usage_trace_lines(&usage));
                                record_usage_observability(&usage);
                                for entry in &trace {
                                    state.record(entry.clone());
                                }
                                state.observations = observations;
                                state.active_skill = None;
                                self.persist_short_term_memory(state, &mut trace);
                                return AgentResult::stopped(stop_reason, trace, usage);
                            }
                        }
                        Decision::Stop(reason) => {
                            trace.push(format!(
                                "Iteration {step_count}: stop reason = {reason}"
                            ));
                            if usage.is_empty() {
                                usage.push(TokenUsageRecord::zero(
                                    "main agent request",
                                    "local heuristic router",
                                    "no model API call",
                                ));
                            }
                            trace.extend(usage_trace_lines(&usage));
                            record_usage_observability(&usage);
                            for entry in &trace {
                                state.record(entry.clone());
                            }
                            state.observations = observations;
                            state.active_skill = None;
                            self.persist_short_term_memory(state, &mut trace);
                            return AgentResult::stopped(reason, trace, usage);
                        }
                    }
                }
            },
        )
    }

    fn decide_next_step(
        &self,
        state: &SessionState,
        input: &str,
        task_contract: &TaskContract,
        observations: &[String],
        retry_count: u8,
    ) -> PlannerRun {
        let router = PlannerRouter::new(
            self.config.planner_backend.clone(),
            self.config.default_model,
            self.config.fallback_model,
            self.config.allow_heuristic_planner,
        );

        router.decide(
            self.llm_engine.as_ref().map(|engine| {
                let root_dir = &self.root_dir;
                let config = &self.config;
                let memory_context = &self.memory_stack.merged_instructions;
                let tools = &self.tools;
                let subagents = &self.subagents;
                let skills = &self.skills;
                let active_skill = state.active_skill.as_ref();
                let state = state;
                move || {
                    engine.plan(
                        root_dir,
                        config,
                        memory_context,
                        tools,
                        subagents,
                        skills,
                        active_skill,
                        state,
                        input,
                        task_contract,
                        observations,
                        retry_count,
                    )
                }
            }),
            self.fallback_engine.as_ref().map(|engine| {
                let root_dir = &self.root_dir;
                let config = &self.config;
                let memory_context = &self.memory_stack.merged_instructions;
                let tools = &self.tools;
                let subagents = &self.subagents;
                let skills = &self.skills;
                let active_skill = state.active_skill.as_ref();
                let state = state;
                move || {
                    engine.plan(
                        root_dir,
                        config,
                        memory_context,
                        tools,
                        subagents,
                        skills,
                        active_skill,
                        state,
                        input,
                        task_contract,
                        observations,
                        retry_count,
                    )
                }
            }),
            || {
                heuristic_plan(
                    input,
                    &self.tools,
                    &self.subagents,
                    &self.skills,
                    state.active_skill.as_ref(),
                    observations,
                )
            },
        )
    }

    fn call_tool(
        &self,
        name: &str,
        user_input: &str,
        arguments: &Value,
        active_skill: Option<&ActiveSkill>,
    ) -> StepOutcome {
        if let Some(skill) = active_skill {
            if let Some(allowed_tools) = &skill.allowed_tools {
                if !allowed_tools
                    .iter()
                    .map(|tool_name| normalize_tool_name(tool_name))
                    .any(|tool_name| tool_name == name)
                {
                    return StepOutcome::Retry(format!(
                        "Active skill `{}` does not allow tool `{name}`. Allowed tools: {}.",
                        skill.name,
                        allowed_tools.join(", ")
                    ));
                }
            }
        }
        match self.tools.iter().find(|tool| tool.is_named(name)) {
            Some(tool) => tool.run(user_input, arguments),
            None => StepOutcome::Retry(format!("Unknown tool `{name}`.")),
        }
    }

    fn apply_skill(&self, skill_name: &str, task: &str, activated_by: &str) -> StepOutcome {
        let Some(skill) = self.skills.get(skill_name) else {
            return StepOutcome::Retry(format!("Unknown skill `{skill_name}`."));
        };

        StepOutcome::SkillActivated(SkillActivation {
            name: skill.name.clone(),
            source_path: skill.source_path.clone(),
            scope: skill.scope.clone(),
            activated_by: activated_by.to_string(),
            task_fit: observability::compact_text(task, 240),
            allowed_tools: skill.allowed_tools.clone(),
            preferred_subagents: skill.preferred_subagents.clone(),
        })
    }

    fn render_skill_details(&self, skill_name: &str) -> io::Result<String> {
        let Some(skill) = self.skills.get(skill_name) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Unknown skill `{skill_name}`."),
            ));
        };

        let lines = vec![
            format!("Skill: {}", skill.name),
            format!("Scope: {}", skill.scope),
            format!("Path: {}", skill.source_path.display()),
            format!("Description: {}", skill.description),
            format!(
                "Version: {}",
                skill.version.as_deref().unwrap_or("unspecified")
            ),
            format!(
                "Tags: {}",
                if skill.tags.is_empty() {
                    "none".to_string()
                } else {
                    skill.tags.join(", ")
                }
            ),
            format!(
                "Allowed tools: {}",
                skill
                    .allowed_tools
                    .as_ref()
                    .map(|items| items.join(", "))
                    .unwrap_or_else(|| "inherit all visible tools".to_string())
            ),
            format!(
                "Preferred subagents: {}",
                if skill.preferred_subagents.is_empty() {
                    "none".to_string()
                } else {
                    skill.preferred_subagents.join(", ")
                }
            ),
            format!(
                "Use when: {}",
                skill.when_to_use.as_deref().unwrap_or("not specified")
            ),
            format!(
                "Do not use when: {}",
                skill.when_not_to_use.as_deref().unwrap_or("not specified")
            ),
            format!(
                "Input contract: {}",
                skill.input_contract.as_deref().unwrap_or("not specified")
            ),
            format!(
                "Output contract: {}",
                skill.output_contract.as_deref().unwrap_or("not specified")
            ),
            String::new(),
            "Instructions:".to_string(),
            skill.instructions.clone(),
        ];
        Ok(lines.join("\n"))
    }

    fn render_skill_validation(&self, skill_name: Option<&str>) -> String {
        let mut lines = vec!["Skill validation:".to_string()];
        let mut matched = false;

        for issue in &self.skill_validation_issues {
            if let Some(skill_name) = skill_name {
                let expected = format!("/{skill_name}/");
                let path_text = issue.path.to_string_lossy();
                if !path_text.contains(&expected)
                    && !path_text.ends_with(&format!("/{skill_name}/SKILL.md"))
                {
                    continue;
                }
            }
            matched = true;
            lines.push(format!(
                "- ERROR {} :: {}",
                issue.path.display(),
                issue.message
            ));
        }

        if let Some(skill_name) = skill_name {
            if self.skills.contains_key(skill_name) {
                matched = true;
                lines.push(format!("- OK `{skill_name}` loaded successfully."));
            }
        } else {
            for spec in self.skills.values() {
                lines.push(format!(
                    "- OK `{}` [{}] {}",
                    spec.name,
                    spec.scope,
                    spec.source_path.display()
                ));
            }
            matched = true;
        }

        if !matched {
            lines.push("- No matching skill was found.".to_string());
        }

        lines.join("\n")
    }

    fn delegate_to_subagent(
        &self,
        subagent_name: &str,
        state: &SessionState,
        task: &str,
    ) -> Result<DelegationOutcome, String> {
        let spec = self
            .subagents
            .get(subagent_name)
            .ok_or_else(|| format!("Unknown subagent `{subagent_name}`."))?;
        let context_packet = self.build_context_packet(task, state);
        let file_refs = if context_packet.relevant_files.is_empty() {
            suggest_relevant_files(&self.root_dir, task)
        } else {
            context_packet.relevant_files.clone()
        };
        let memory_refs = self
            .memory_stack
            .sources
            .iter()
            .map(|source| source.path.display().to_string())
            .collect::<Vec<_>>();
        let observations = state
            .history
            .iter()
            .rev()
            .take(4)
            .map(|message| {
                let role = match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                };
                format!(
                    "{role}: {}",
                    observability::compact_text(&message.content, 180)
                )
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let request = DispatchRequest::for_subagent(
            "main-worker",
            spec.name.clone(),
            task.to_string(),
            context_packet,
            memory_refs,
            file_refs,
            observations,
        );
        let dispatch_id = request.dispatch_id.as_str().to_string();
        match ProcessDispatcher::from_current_exe(self.root_dir.clone(), Duration::from_secs(30))
        {
            Ok(dispatcher) => {
                observability::record_subagent_dispatch_event(
                    "launch",
                    &dispatch_id,
                    &spec.name,
                    "spawned-process",
                    None,
                );
                crate::runtime_log::info(
                    "subagent",
                    format!(
                        "dispatch_id={} subagent={} launch_mode=spawned-process status=launch",
                        dispatch_id, spec.name
                    ),
                );
                self.observability
                    .with_span(
                        format!("subagent.{}", spec.name),
                        vec![
                            KeyValue::new("subagent.name", spec.name.clone()),
                            KeyValue::new("subagent.scope", spec.scope.clone()),
                            KeyValue::new("subagent.launch_mode", "spawned-process"),
                            KeyValue::new("subagent.dispatch_id", dispatch_id.clone()),
                        ],
                        || dispatcher.dispatch(request),
                    )
                    .map(|response| DelegationOutcome {
                        response,
                        launch_mode: "spawned-process",
                    })
            }
            Err(error) => {
                observability::record_subagent_dispatch_event(
                    "launch",
                    &dispatch_id,
                    &spec.name,
                    "local-fallback",
                    Some(error.to_string()),
                );
                crate::runtime_log::warn(
                    "subagent",
                    format!(
                        "dispatch_id={} subagent={} launch_mode=local-fallback status=launch error={}",
                        dispatch_id, spec.name, error
                    ),
                );
                let dispatcher = LocalDispatcher::new(|request| Ok(execute_subagent(spec, request)));
                self.observability
                    .with_span(
                        format!("subagent.{}", spec.name),
                        vec![
                            KeyValue::new("subagent.name", spec.name.clone()),
                            KeyValue::new("subagent.scope", spec.scope.clone()),
                            KeyValue::new("subagent.launch_mode", "local-fallback"),
                            KeyValue::new("subagent.launch_error", error.to_string()),
                            KeyValue::new("subagent.dispatch_id", dispatch_id.clone()),
                        ],
                        || dispatcher.dispatch(request),
                    )
                    .map(|response| DelegationOutcome {
                        response,
                        launch_mode: "local-fallback",
                    })
            }
        }
    }

    fn build_context_packet(&self, task: &str, state: &SessionState) -> ContextPacket {
        let task_contract = self.build_task_contract(task, &state.observations);
        let relevant_files = task_contract.relevant_files.clone();
        let mut known_facts = vec![
            format!(
                "Project memory sources loaded: {}",
                self.memory_stack.sources.len()
            ),
            format!(
                "Available subagents: {}",
                self.subagents
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            format!(
                "Available skills: {}",
                self.skills.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            format!(
                "Available tools: {}",
                self.tools
                    .iter()
                    .map(|tool| tool.name().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ];
        if let Some(summary) = &state.compacted_summary {
            known_facts.push(format!(
                "Compacted prior session summary: {}",
                observability::compact_text(summary, 220)
            ));
        }
        ContextPacket {
            goal: task_contract.goal,
            constraints: task_contract.constraints,
            relevant_files,
            known_facts,
            missing_facts: vec![
                "Whether network-backed model execution is enabled for this run.".to_string(),
                "Whether the user wants execution or review only for any planned changes."
                    .to_string(),
            ],
            next_action: "Return a concise delegated result to the parent session.".to_string(),
            stop_condition: task_contract.stop_condition,
        }
    }

    fn build_task_contract(&self, task: &str, observations: &[String]) -> TaskContract {
        let mut constraints = vec![
            "Start from project memory and repository-local instructions before acting."
                .to_string(),
            "Take exactly one action per loop iteration: call a tool, delegate to a subagent, retry, stop, or finalize."
                .to_string(),
            "Keep the working set compact and prefer explicit observations over hidden reasoning."
                .to_string(),
        ];
        if !observations.is_empty() {
            constraints.push(
                "Use the latest recorded observation before asking for more actions.".to_string(),
            );
        }

        TaskContract {
            goal: format!("Resolve the user task: {task}"),
            constraints,
            acceptance_criteria: vec![
                "The next action is justified from the current task contract.".to_string(),
                "Tool, delegation, retry, and stop behavior remain visible in the execution trace."
                    .to_string(),
                "The run stops with a final answer or an explicit stop reason.".to_string(),
            ],
            relevant_files: suggest_relevant_files(&self.root_dir, task),
            stop_condition:
                "Stop when a final answer is available, the planner decides to stop, or the retry budget is exhausted."
                    .to_string(),
        }
    }

    fn persist_short_term_memory(&self, state: &SessionState, trace: &mut Vec<String>) {
        match save_short_term_memory(&self.memory_paths.short_term_snapshot, state) {
            Ok(()) => trace.push(format!(
                "Persisted short-term memory to {}.",
                self.memory_paths.short_term_snapshot.display()
            )),
            Err(error) => trace.push(format!("Short-term memory persistence failed: {error}")),
        }
    }

    fn remember_long_term(&mut self, note: &str) -> io::Result<()> {
        append_long_term_memory(self.mem0.as_ref(), &self.memory_paths.long_term_notes, note)?;
        self.refresh_project_state()?;
        Ok(())
    }

    fn handle_slash_command(
        &mut self,
        state: &mut SessionState,
        input: &str,
        bin_name: &str,
    ) -> io::Result<CommandOutcome> {
        match parse_slash_command(input, &self.commands) {
            SlashCommand::Help => Ok(CommandOutcome::Continue(self.chat_help_text(bin_name))),
            SlashCommand::Agents => Ok(CommandOutcome::Continue(self.render_agents())),
            SlashCommand::Skills => Ok(CommandOutcome::Continue(self.render_skills())),
            SlashCommand::Memory => Ok(CommandOutcome::Continue(self.render_memory())),
            SlashCommand::Remember(note) => {
                if note.trim().is_empty() {
                    return Ok(CommandOutcome::Continue(
                        "Usage: /remember <durable note>".to_string(),
                    ));
                }
                self.remember_long_term(note.trim())?;
                Ok(CommandOutcome::Continue(if self.mem0.is_some() {
                    "Saved long-term memory entry to Mem0.".to_string()
                } else {
                    format!(
                        "Saved long-term memory entry to {}.",
                        self.memory_paths.long_term_notes.display()
                    )
                }))
            }
            SlashCommand::Model => Ok(CommandOutcome::Continue(self.render_model())),
            SlashCommand::Clear => {
                state.clear();
                save_short_term_memory(&self.memory_paths.short_term_snapshot, state)?;
                Ok(CommandOutcome::Continue(
                    "Cleared session history and trace. Project memory remains loaded and short-term memory was reset.".to_string(),
                ))
            }
            SlashCommand::Compact => {
                let result = compact_session_state(state, true);
                save_short_term_memory(&self.memory_paths.short_term_snapshot, state)?;
                Ok(CommandOutcome::Continue(result.render()))
            }
            SlashCommand::Mcp => Ok(CommandOutcome::Continue(self.render_mcp())),
            SlashCommand::Review(task) => {
                Ok(CommandOutcome::Continue(self.run_review(state, &task)))
            }
            SlashCommand::Skill { name, task } => Ok(CommandOutcome::Continue(
                match self.apply_skill(&name, &task, "user") {
                    StepOutcome::SkillActivated(activation) => {
                        state.active_skill = Some(ActiveSkill {
                            name: activation.name.clone(),
                            source_path: activation.source_path.clone(),
                            activated_by: activation.activated_by.clone(),
                            task_fit: activation.task_fit.clone(),
                            allowed_tools: activation.allowed_tools.clone(),
                            preferred_subagents: activation.preferred_subagents.clone(),
                        });
                        format!(
                            "Activated skill `{}` [{}] from {}.\nTask fit: {}\nAllowed tools: {}\nPreferred subagents: {}",
                            activation.name,
                            activation.scope,
                            activation.source_path.display(),
                            activation.task_fit,
                            activation
                                .allowed_tools
                                .as_ref()
                                .map(|items| items.join(", "))
                                .unwrap_or_else(|| "inherit all visible tools".to_string()),
                            if activation.preferred_subagents.is_empty() {
                                "none".to_string()
                            } else {
                                activation.preferred_subagents.join(", ")
                            }
                        )
                    }
                    StepOutcome::Retry(reason) => reason,
                    StepOutcome::Success(output) => output,
                },
            )),
            SlashCommand::Init => {
                let created = self.init_project_files()?;
                if created.is_empty() {
                    Ok(CommandOutcome::Continue(
                        "Claude-style project files already exist.".to_string(),
                    ))
                } else {
                    Ok(CommandOutcome::Continue(format!(
                        "Created Claude-style project files:\n{}",
                        created
                            .iter()
                            .map(|path| format!("- {}", path.display()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )))
                }
            }
            SlashCommand::Agent { name, task } => {
                match self.delegate_to_subagent(&name, state, &task) {
                    Ok(result) => Ok(CommandOutcome::Continue(result.response.render())),
                    Err(reason) => Ok(CommandOutcome::Continue(reason)),
                }
            }
            SlashCommand::Exit => Ok(CommandOutcome::Exit("Session ended.".to_string())),
            SlashCommand::Custom { name, arguments } => {
                let Some(command) = self.commands.get(&name) else {
                    return Ok(CommandOutcome::Continue(format!(
                        "Unknown slash command `/{name}`."
                    )));
                };
                let expanded = expand_custom_command(command, &arguments);
                Ok(CommandOutcome::Continue(
                    self.run_with_state(state, &expanded).output,
                ))
            }
            SlashCommand::Unknown(name) if name == "trace" => Ok(CommandOutcome::ToggleTrace),
            SlashCommand::Unknown(name) => Ok(CommandOutcome::Continue(format!(
                "Unknown slash command `/{name}`."
            ))),
        }
    }

    fn run_review(&self, state: &SessionState, task: &str) -> String {
        let focus = if task.trim().is_empty() {
            "general repository review".to_string()
        } else {
            task.trim().to_string()
        };
        let files = suggest_relevant_files(&self.root_dir, &focus);
        let findings = if files.is_empty() {
            vec!["No obvious target files matched the current review focus.".to_string()]
        } else {
            files
                .iter()
                .map(|path| format!("Review target worth checking: {path}"))
                .collect::<Vec<_>>()
        };
        let prior_context = state
            .compacted_summary
            .as_deref()
            .map(|summary| format!("Compacted session context: {summary}\n\n"))
            .unwrap_or_default();
        format!(
            "Review focus: {focus}\n\n{prior_context}Findings:\n{}\n\nOpen questions:\n- Verify behavior with targeted tests.\n- Confirm whether any project command or subagent should own this workflow.\n\nChange summary:\n- Review runs as a workflow, not as a permanent validator agent.",
            findings
                .into_iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    fn render_agents(&self) -> String {
        let mut lines = vec!["Subagents:".to_string()];
        if self.subagents.is_empty() {
            lines.push("- none".to_string());
        } else {
            for spec in self.subagents.values() {
                let tools = spec
                    .tool_allowlist
                    .as_ref()
                    .map(|items| items.join(", "))
                    .unwrap_or_else(|| "inherit parent tools".to_string());
                lines.push(format!(
                    "- {} [{}] tools={} path={}",
                    spec.name,
                    spec.scope,
                    tools,
                    spec.source_path.display()
                ));
            }
        }
        lines.join("\n")
    }

    fn render_skills(&self) -> String {
        let mut lines = vec!["Skills:".to_string()];
        if self.skills.is_empty() {
            lines.push("- none".to_string());
        } else {
            for spec in self.skills.values() {
                lines.push(format!(
                    "- {} [{}] path={} :: {} | allowed_tools={} | preferred_subagents={}",
                    spec.name,
                    spec.scope,
                    spec.source_path.display(),
                    spec.description,
                    spec.allowed_tools
                        .as_ref()
                        .map(|items| items.join(", "))
                        .unwrap_or_else(|| "inherit all visible tools".to_string()),
                    if spec.preferred_subagents.is_empty() {
                        "none".to_string()
                    } else {
                        spec.preferred_subagents.join(", ")
                    }
                ));
            }
        }
        if !self.skill_validation_issues.is_empty() {
            lines.push(format!(
                "Validation issues: {} invalid skill file(s) skipped.",
                self.skill_validation_issues.len()
            ));
        }
        lines.join("\n")
    }

    fn render_memory(&self) -> String {
        let short_term_memory =
            load_short_term_memory(&self.memory_paths.short_term_snapshot).unwrap_or_default();
        let mut lines = vec!["Memory sources:".to_string()];
        if self.memory_stack.sources.is_empty() {
            lines.push("- none".to_string());
        } else {
            for source in &self.memory_stack.sources {
                let imported = source
                    .imported_from
                    .as_ref()
                    .map(|path| format!(" imported_from={}", path.display()))
                    .unwrap_or_default();
                lines.push(format!(
                    "- [{}] {}{}",
                    source.scope,
                    source.path.display(),
                    imported
                ));
            }
        }
        lines.push(String::new());
        lines.push("Short-term memory:".to_string());
        lines.push(format!(
            "- snapshot: {}",
            self.memory_paths.short_term_snapshot.display()
        ));
        lines.push(format!(
            "- recent history items: {}",
            short_term_memory.recent_history.len()
        ));
        lines.push(format!(
            "- observation count: {}",
            short_term_memory.observations.len()
        ));
        lines.push(format!(
            "- compacted summary: {}",
            short_term_memory
                .compacted_summary
                .as_deref()
                .map(|summary| observability::compact_text(summary, 160))
                .unwrap_or_else(|| "none".to_string())
        ));
        lines.push(String::new());
        lines.push("Long-term memory:".to_string());
        let backend = if self.mem0.is_some() { "Mem0" } else { "file" };
        let source_path = self
            .long_term_memory_source()
            .map(|source| source.path.display().to_string())
            .unwrap_or_else(|| self.memory_paths.long_term_notes.display().to_string());
        lines.push(format!("- backend: {backend}"));
        lines.push(format!("- source: {source_path}"));
        let long_term_preview = self
            .long_term_memory_source()
            .map(|source| observability::compact_text(&source.contents, 400))
            .unwrap_or_else(|| "No durable notes recorded yet.".to_string());
        lines.push(long_term_preview);
        lines.push(String::new());
        lines.push("Merged memory preview:".to_string());
        let focus_paths = if self.memory_stack.load_request.focus_paths.is_empty() {
            "inactive".to_string()
        } else {
            self.memory_stack
                .load_request
                .focus_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(format!("Path-scoped selection: {focus_paths}"));
        lines.push(observability::compact_text(
            &self.memory_stack.merged_instructions,
            800,
        ));
        lines.join("\n")
    }

    fn render_model(&self) -> String {
        format!(
            "Model policy\n- default: {}\n- fallback: {}\n- planner backend: {}",
            self.config.default_model, self.config.fallback_model, self.config.planner_backend
        )
    }

    fn render_mcp(&self) -> String {
        let mut lines = vec!["MCP status:".to_string()];
        if self.mcp_servers.is_empty() {
            lines.push("- no MCP servers loaded".to_string());
        } else {
            for server in &self.mcp_servers {
                lines.push(format!("- {}", server));
            }
        }
        let tool_names = self
            .tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>();
        lines.push(String::new());
        lines.push(format!("Visible tools: {}", tool_names.join(", ")));
        lines.join("\n")
    }

    fn chat_help_text(&self, bin_name: &str) -> String {
        let mut lines = vec![
            format!("Use `{bin_name} list` to inspect loaded memory, subagents, skills, commands, and tools."),
            "Built-in commands:".to_string(),
            "/help, /agents, /skills, /memory, /remember <note>, /model, /clear, /compact, /mcp, /review [task], /skill <name> [task], /init, /agent <name> <task>, /trace, /exit".to_string(),
        ];
        if !self.commands.is_empty() {
            lines.push("Project commands:".to_string());
            for command in self.commands.values() {
                lines.push(format!("/{} - {}", command.name, command.description));
            }
        }
        lines.join("\n")
    }

    pub fn default_model(&self) -> &str {
        self.config.default_model
    }

    pub fn system_prompt(&self) -> &str {
        &self.config.system_prompt
    }

    pub fn fallback_model(&self) -> &str {
        self.config.fallback_model
    }

    pub fn planner_backend_label(&self) -> &str {
        &self.config.planner_backend
    }

    pub fn observability_enabled(&self) -> bool {
        self.observability.is_enabled()
    }

    pub fn observability_targets(&self) -> &[&'static str] {
        self.observability.enabled_targets()
    }

    pub fn observability_warnings(&self) -> &[String] {
        self.observability.warnings()
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn mcp_servers(&self) -> &[McpServerSummary] {
        &self.mcp_servers
    }

    pub fn memory_sources(&self) -> &[MemorySource] {
        &self.memory_stack.sources
    }

    pub fn subagents(&self) -> Vec<&SubagentSpec> {
        self.subagents.values().collect()
    }

    pub fn skills(&self) -> Vec<&SkillSpec> {
        self.skills.values().collect()
    }

    pub fn show_skill(&self, skill_name: &str) -> io::Result<String> {
        self.render_skill_details(skill_name)
    }

    pub fn validate_skills(&self, skill_name: Option<&str>) -> String {
        self.render_skill_validation(skill_name)
    }

    pub fn command_summaries(&self) -> Vec<String> {
        self.commands
            .values()
            .map(|command| {
                format!(
                    "/{} [{}] {}",
                    command.name, command.scope, command.description
                )
            })
            .collect()
    }

    fn long_term_memory_source(&self) -> Option<&MemorySource> {
        let mem0_path = self.mem0.as_ref().map(Mem0Config::source_path);
        self.memory_stack.sources.iter().find(|source| {
            source.path == self.memory_paths.long_term_notes
                || mem0_path
                    .as_ref()
                    .map(|path| source.path == *path)
                    .unwrap_or(false)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandOutcome {
    Continue(String),
    Exit(String),
    ToggleTrace,
}

fn parse_slash_command(input: &str, commands: &BTreeMap<String, CustomCommand>) -> SlashCommand {
    let trimmed = input.trim().trim_start_matches('/');
    let (name, remainder) = match trimmed.split_once(' ') {
        Some((name, remainder)) => (name, remainder.trim()),
        None => (trimmed, ""),
    };

    match name {
        "help" => SlashCommand::Help,
        "agents" => SlashCommand::Agents,
        "skills" => SlashCommand::Skills,
        "memory" => SlashCommand::Memory,
        "remember" => SlashCommand::Remember(remainder.to_string()),
        "model" => SlashCommand::Model,
        "clear" => SlashCommand::Clear,
        "compact" => SlashCommand::Compact,
        "mcp" => SlashCommand::Mcp,
        "review" => SlashCommand::Review(remainder.to_string()),
        "init" => SlashCommand::Init,
        "trace" => SlashCommand::Unknown("trace".to_string()),
        "exit" | "quit" => SlashCommand::Exit,
        "skill" => {
            let Some((skill_name, task)) = remainder.split_once(' ') else {
                if remainder.is_empty() {
                    return SlashCommand::Unknown("skill".to_string());
                }
                return SlashCommand::Skill {
                    name: remainder.to_string(),
                    task: remainder.to_string(),
                };
            };
            SlashCommand::Skill {
                name: skill_name.trim().to_string(),
                task: task.trim().to_string(),
            }
        }
        "agent" => {
            let Some((agent_name, task)) = remainder.split_once(' ') else {
                return SlashCommand::Unknown("agent".to_string());
            };
            SlashCommand::Agent {
                name: agent_name.trim().to_string(),
                task: task.trim().to_string(),
            }
        }
        other if commands.contains_key(other) => SlashCommand::Custom {
            name: other.to_string(),
            arguments: remainder.to_string(),
        },
        other => SlashCommand::Unknown(other.to_string()),
    }
}

fn parse_explicit_tool_request(input: &str) -> Option<(String, Value)> {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = "use tool ";
    if !lower.starts_with(prefix) {
        return None;
    }

    let remainder = trimmed[prefix.len()..].trim();
    let (tool_name, raw_arguments) = match remainder.split_once(" with ") {
        Some((name, json_body)) => (name.trim(), json_body.trim()),
        None => (remainder, ""),
    };
    if tool_name.is_empty() {
        return None;
    }

    let arguments = if raw_arguments.is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(raw_arguments).unwrap_or(Value::Object(Default::default()))
    };

    Some((tool_name.to_string(), arguments))
}

fn infer_tool_request(input: &str, tools: &[Tool]) -> Option<(String, Value, String)> {
    if let Some((tool_name, arguments)) = parse_explicit_tool_request(input) {
        let normalized_tool_name = normalize_tool_name(&tool_name);
        return Some((
            normalized_tool_name.clone(),
            arguments,
            if normalized_tool_name == tool_name {
                format!("Explicit tool request detected for `{tool_name}`.")
            } else {
                format!(
                    "Explicit tool request detected for `{tool_name}` and normalized to `{normalized_tool_name}`."
                )
            },
        ));
    }

    if tools.iter().any(|tool| tool.is_named(WEB_SEARCH_TOOL_NAME)) && should_use_web_search(input)
    {
        return Some((
            WEB_SEARCH_TOOL_NAME.to_string(),
            Value::Object(Default::default()),
            format!(
                "Planner inferred `{WEB_SEARCH_TOOL_NAME}` for an external or time-sensitive query."
            ),
        ));
    }

    None
}

fn parse_explicit_skill_request(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = "use skill ";
    if !lower.starts_with(prefix) {
        return None;
    }

    let remainder = trimmed[prefix.len()..].trim();
    let name = remainder
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn infer_skill_request(
    input: &str,
    skills: &BTreeMap<String, SkillSpec>,
) -> Option<(String, String)> {
    if skills.is_empty() {
        return None;
    }

    if let Some(skill_name) = parse_explicit_skill_request(input) {
        if skills.contains_key(&skill_name) {
            return Some((
                skill_name.clone(),
                format!("Explicit skill request detected for `{skill_name}`."),
            ));
        }
    }

    let lower = input.to_ascii_lowercase();
    let mut best_match: Option<(usize, String, String)> = None;
    for skill in skills.values() {
        let mut score = 0usize;
        if lower.contains(&skill.name.to_ascii_lowercase()) {
            score += 4;
        }
        for token in skill
            .description
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
            .map(|token| token.trim().to_ascii_lowercase())
            .filter(|token| token.len() >= 5)
        {
            if lower.contains(&token) {
                score += 1;
            }
        }
        if score == 0 {
            continue;
        }

        let reason = format!(
            "Planner inferred skill `{}` from the request and skill description match.",
            skill.name
        );
        match &best_match {
            Some((best_score, _, _)) if *best_score >= score => {}
            _ => {
                best_match = Some((score, skill.name.clone(), reason));
            }
        }
    }

    best_match.map(|(_, name, reason)| (name, reason))
}

fn should_use_web_search(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();

    let explicit_phrases = [
        "search the web",
        "search web",
        "web search",
        "look up",
        "lookup",
        "research ",
        "find online",
        "on the web",
        "perplexity",
    ];
    if explicit_phrases.iter().any(|phrase| lower.contains(phrase)) {
        return true;
    }

    let recency_signals = [
        "latest",
        "current",
        "today",
        "recent",
        "recently",
        "newest",
        "up-to-date",
        "up to date",
        "breaking",
        "news",
    ];
    let knowledge_targets = [
        "docs",
        "documentation",
        "release notes",
        "announcement",
        "announcements",
        "pricing",
        "price",
        "api",
        "api changes",
        "version",
        "versions",
        "model",
        "models",
        "policy",
        "policies",
    ];

    recency_signals.iter().any(|signal| lower.contains(signal))
        && knowledge_targets
            .iter()
            .any(|target| lower.contains(target))
}

fn infer_subagent_name<'a>(
    input: &str,
    subagents: &'a BTreeMap<String, SubagentSpec>,
) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    if let Some(explicit) = lower.strip_prefix("use agent ") {
        let name = explicit
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim();
        if subagents.contains_key(name) {
            return Some(name.to_string());
        }
    }

    if lower.contains("plan") && subagents.contains_key("plan") {
        return Some("plan".to_string());
    }
    if (lower.contains("explore")
        || lower.contains("investigate")
        || lower.contains("find ")
        || lower.contains("where "))
        && subagents.contains_key("explore")
    {
        return Some("explore".to_string());
    }
    if subagents.contains_key("general-purpose") {
        return Some("general-purpose".to_string());
    }
    subagents.keys().next().cloned()
}

fn parse_explicit_subagent_request(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = "use agent ";
    if !lower.starts_with(prefix) {
        return None;
    }

    let remainder = trimmed[prefix.len()..].trim();
    let name = remainder
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn heuristic_plan(
    input: &str,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
    skills: &BTreeMap<String, SkillSpec>,
    active_skill: Option<&ActiveSkill>,
    observations: &[String],
) -> PlannerRun {
    if let Some(observation) = observations.last() {
        if observation.starts_with("Recoverable ")
            || observation.starts_with("Planner retry request:")
        {
            return PlannerRun::failure(
                Decision::Retry(
                    "the latest observation still reflects a recoverable failure".to_string(),
                ),
                vec![
                    "The heuristic router preserves retry semantics when the latest observation is a recoverable failure."
                        .to_string(),
                ],
                PlannerFailureKind::Recoverable,
            );
        }

        return PlannerRun::success(
            Decision::Finish {
                answer: observation.clone(),
                reason: "the latest observation is now the best available answer".to_string(),
            },
            vec![
                "The heuristic router finishes after a successful observation instead of taking another action."
                    .to_string(),
            ],
            None,
        );
    }

    if let Some((tool_name, arguments, route_reason)) = infer_tool_request(input, tools) {
        return PlannerRun::success(
            Decision::CallTool {
                tool_name,
                arguments,
                reason: route_reason.clone(),
            },
            vec![route_reason],
            None,
        );
    }

    if let Some(subagent_name) = parse_explicit_subagent_request(input) {
        if subagents.contains_key(&subagent_name) {
            return PlannerRun::success(
                Decision::DelegateSubagent {
                    subagent_name: subagent_name.clone(),
                    reason: format!("Explicit subagent request detected for `{subagent_name}`."),
                },
                vec![format!(
                    "The local heuristic router honored the explicit subagent request for `{subagent_name}`."
                )],
                None,
            );
        }
    }

    if active_skill.is_none() {
        if let Some((skill_name, route_reason)) = infer_skill_request(input, skills) {
            return PlannerRun::success(
                Decision::UseSkill {
                    skill_name,
                    reason: route_reason.clone(),
                },
                vec![route_reason],
                None,
            );
        }
    }

    if let Some(active_skill) = active_skill {
        if let Some(subagent_name) = active_skill
            .preferred_subagents
            .iter()
            .find(|name| subagents.contains_key(*name))
            .cloned()
        {
            return PlannerRun::success(
                Decision::DelegateSubagent {
                    subagent_name,
                    reason: format!(
                        "Active skill `{}` prefers delegated execution through that subagent.",
                        active_skill.name
                    ),
                },
                vec![format!(
                    "The local heuristic router deferred to the active skill `{}`.",
                    active_skill.name
                )],
                None,
            );
        }
    }

    let subagent_name =
        infer_subagent_name(input, subagents).unwrap_or_else(|| "general-purpose".to_string());
    PlannerRun::success(
        Decision::DelegateSubagent {
            subagent_name,
            reason: "No explicit tool request was inferred.".to_string(),
        },
        vec![
            "No model-backed planner succeeded; using the local heuristic router.".to_string(),
        ],
        None,
    )
}

fn planner_eval_actual_decision(
    run: PlannerRun,
    planner_backend: &str,
) -> PlannerEvalActualDecision {
    let PlannerRun {
        decision,
        reasoning,
        trace: _,
        usage: _,
        failure_kind,
    } = run;
    let failure_class = failure_kind.map(|kind| kind.label().to_string());
    match decision {
        Decision::CallTool {
            tool_name,
            arguments,
            reason,
        } => PlannerEvalActualDecision {
            action: "tool".to_string(),
            tool_name: Some(tool_name),
            skill_name: None,
            subagent_name: None,
            tool_arguments_json: Some(arguments),
            reason,
            planner_backend: planner_backend.to_string(),
            reasoning,
            failure_class,
        },
        Decision::UseSkill { skill_name, reason } => PlannerEvalActualDecision {
            action: "skill".to_string(),
            tool_name: None,
            skill_name: Some(skill_name),
            subagent_name: None,
            tool_arguments_json: None,
            reason,
            planner_backend: planner_backend.to_string(),
            reasoning,
            failure_class,
        },
        Decision::DelegateSubagent {
            subagent_name,
            reason,
        } => PlannerEvalActualDecision {
            action: "delegate".to_string(),
            tool_name: None,
            skill_name: None,
            subagent_name: Some(subagent_name),
            tool_arguments_json: None,
            reason,
            planner_backend: planner_backend.to_string(),
            reasoning,
            failure_class,
        },
        Decision::Finish { reason, .. } => PlannerEvalActualDecision {
            action: "finish".to_string(),
            tool_name: None,
            skill_name: None,
            subagent_name: None,
            tool_arguments_json: None,
            reason,
            planner_backend: planner_backend.to_string(),
            reasoning,
            failure_class,
        },
        Decision::Retry(reason) => PlannerEvalActualDecision {
            action: "retry".to_string(),
            tool_name: None,
            skill_name: None,
            subagent_name: None,
            tool_arguments_json: None,
            reason,
            planner_backend: planner_backend.to_string(),
            reasoning,
            failure_class,
        },
        Decision::Stop(reason) => PlannerEvalActualDecision {
            action: "stop".to_string(),
            tool_name: None,
            skill_name: None,
            subagent_name: None,
            tool_arguments_json: None,
            reason,
            planner_backend: planner_backend.to_string(),
            reasoning,
            failure_class,
        },
    }
}

#[derive(Debug, Clone)]
struct OpenAiEngine {
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiEngine {
    fn from_env() -> Option<Self> {
        let api_key = env::var("OPENAI_API_KEY").ok()?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty()
            || api_key.starts_with("replace-with-")
            || api_key.starts_with("your-")
        {
            return None;
        }

        let base_url = env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
        let model = env::var("OPENAI_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string());

        Some(Self {
            api_key,
            base_url,
            model,
        })
    }

    fn plan(
        &self,
        root_dir: &Path,
        config: &AgentConfig,
        memory_context: &str,
        tools: &[Tool],
        subagents: &BTreeMap<String, SubagentSpec>,
        skills: &BTreeMap<String, SkillSpec>,
        active_skill: Option<&ActiveSkill>,
        state: &SessionState,
        user_input: &str,
        task_contract: &TaskContract,
        observations: &[String],
        retry_count: u8,
    ) -> Result<PlannerRun, String> {
        let planner_prompt = planner_prompt(
            state,
            memory_context,
            tools,
            subagents,
            skills,
            active_skill,
            task_contract,
            observations,
            retry_count,
        );
        let planner_input = build_planner_input_from_prompt(state, user_input, &planner_prompt);
        let request = json!({
            "model": self.model,
            "instructions": config.system_prompt,
            "input": planner_input,
            "reasoning": {
                "summary": "auto"
            },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "agent_decision",
                    "strict": true,
                    "schema": planner_schema()
                }
            }
        });
        let context_entry = capture_context_history_entry(
            "OpenAI Responses API",
            &self.model,
            &config.system_prompt,
            &planner_prompt,
            state,
            user_input,
            &request,
        );
        emit_context_preview(root_dir, &context_entry);

        let response = self.send_json_request(request)?;
        let raw_text = response
            .output_text()
            .ok_or_else(|| "OpenAI response did not contain text output.".to_string())?;
        let payload: PlannerPayload = serde_json::from_str(&raw_text)
            .map_err(|error| format!("Planner JSON parse failed: {error}; raw={raw_text}"))?;
        let decision = planner_payload_to_decision(&payload, tools, subagents, skills)?;
        let reasoning = vec![reasoning_summary_for_decision(&decision)];

        Ok(PlannerRun::success(
            decision,
            reasoning,
            response
                .usage
                .as_ref()
                .map(|usage| usage.as_token_record(&self.model)),
        ))
    }

    fn send_json_request(&self, request: Value) -> Result<OpenAiResponse, String> {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| format!("client build error: {error}"))?;

        let response = client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .map_err(|error| format!("request error: {error}"))?;

        let status = response.status();
        let response_body = response
            .text()
            .map_err(|error| format!("response read error: {error}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status.as_u16(), response_body));
        }

        let parsed: OpenAiResponse = serde_json::from_str(&response_body)
            .map_err(|error| format!("response JSON parse error: {error}; body={response_body}"))?;
        if let Some(error) = &parsed.error {
            return Err(format!(
                "{}: {}",
                error.code.as_deref().unwrap_or("api_error"),
                error.message
            ));
        }

        Ok(parsed)
    }
}

#[derive(Debug, Clone)]
struct AnthropicEngine {
    api_key: String,
    base_url: String,
    model: String,
    version: String,
}

impl AnthropicEngine {
    fn from_env() -> Option<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY").ok()?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty()
            || api_key.starts_with("replace-with-")
            || api_key.starts_with("your-")
        {
            return None;
        }

        let base_url = env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_BASE_URL.to_string());
        let model = env::var("ANTHROPIC_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| FALLBACK_MODEL.to_string());
        let version = env::var("ANTHROPIC_VERSION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_VERSION.to_string());

        Some(Self {
            api_key,
            base_url,
            model,
            version,
        })
    }

    fn plan(
        &self,
        root_dir: &Path,
        config: &AgentConfig,
        memory_context: &str,
        tools: &[Tool],
        subagents: &BTreeMap<String, SubagentSpec>,
        skills: &BTreeMap<String, SkillSpec>,
        active_skill: Option<&ActiveSkill>,
        state: &SessionState,
        user_input: &str,
        task_contract: &TaskContract,
        observations: &[String],
        retry_count: u8,
    ) -> Result<PlannerRun, String> {
        let prompt = planner_prompt(
            state,
            memory_context,
            tools,
            subagents,
            skills,
            active_skill,
            task_contract,
            observations,
            retry_count,
        );
        let request = json!({
            "model": self.model,
            "max_tokens": 900,
            "system": format!("{}\n\n{}", config.system_prompt, prompt),
            "messages": [
                {
                    "role": "user",
                    "content": user_input
                }
            ]
        });
        let context_entry = capture_context_history_entry(
            "Anthropic Messages API",
            &self.model,
            &config.system_prompt,
            &prompt,
            state,
            user_input,
            &request,
        );
        emit_context_preview(root_dir, &context_entry);

        let response = self.send_json_request(request)?;
        let raw_text = response
            .output_text()
            .ok_or_else(|| "Anthropic response did not contain text output.".to_string())?;
        let payload: PlannerPayload = serde_json::from_str(&raw_text)
            .map_err(|error| format!("Planner JSON parse failed: {error}; raw={raw_text}"))?;
        let decision = planner_payload_to_decision(&payload, tools, subagents, skills)?;
        let reasoning = vec![reasoning_summary_for_decision(&decision)];

        Ok(PlannerRun::success(
            decision,
            reasoning,
            response
                .usage
                .as_ref()
                .map(|usage| usage.as_token_record(&self.model)),
        ))
    }

    fn send_json_request(&self, request: Value) -> Result<AnthropicResponse, String> {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| format!("client build error: {error}"))?;

        let response = client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.version)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .map_err(|error| format!("request error: {error}"))?;

        let status = response.status();
        let response_body = response
            .text()
            .map_err(|error| format!("response read error: {error}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status.as_u16(), response_body));
        }

        serde_json::from_str(&response_body)
            .map_err(|error| format!("response JSON parse error: {error}; body={response_body}"))
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    status: Option<String>,
    error: Option<OpenAiErrorBody>,
    usage: Option<OpenAiUsageBody>,
    #[serde(default)]
    output: Vec<OpenAiOutputItem>,
}

impl OpenAiResponse {
    fn output_text(&self) -> Option<String> {
        let parts = self
            .output
            .iter()
            .flat_map(|item| item.content.iter())
            .filter(|content| {
                content.content_type == "output_text" || content.content_type == "text"
            })
            .filter_map(|content| content.text.clone())
            .collect::<Vec<_>>();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorBody {
    code: Option<String>,
    message: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsageBody {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

impl OpenAiUsageBody {
    fn as_token_record(&self, model: &str) -> TokenUsageRecord {
        TokenUsageRecord {
            actor: "main agent request".to_string(),
            input_tokens: self.input_tokens.unwrap_or(0),
            output_tokens: self.output_tokens.unwrap_or(0),
            total_tokens: self.total_tokens.unwrap_or(0),
            execution: format!("OpenAI Responses API ({model})"),
            note: "planner call".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiOutputItem {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    content: Vec<OpenAiContentItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiContentItem {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    #[serde(default)]
    content: Vec<AnthropicContentBlock>,
    usage: Option<AnthropicUsageBody>,
}

impl AnthropicResponse {
    fn output_text(&self) -> Option<String> {
        let text = self
            .content
            .iter()
            .filter(|item| item.content_type == "text")
            .filter_map(|item| item.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageBody {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

impl AnthropicUsageBody {
    fn as_token_record(&self, model: &str) -> TokenUsageRecord {
        let input_tokens = self.input_tokens.unwrap_or(0);
        let output_tokens = self.output_tokens.unwrap_or(0);
        TokenUsageRecord {
            actor: "main agent request".to_string(),
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            execution: format!("Anthropic Messages API ({model})"),
            note: "fallback planner call".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlannerPayload {
    action: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_arguments_json: String,
    #[serde(default)]
    skill_name: String,
    #[serde(default)]
    subagent_name: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ApiInputMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ActualContextSnapshot {
    system_prompt: String,
    planner_prompt: String,
    prior_history: Vec<ApiInputMessage>,
    current_user_input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FinalContextSnapshot {
    request_body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ContextHistoryEntry {
    datetime: String,
    provider: String,
    model: String,
    actual_context: ActualContextSnapshot,
    final_context: FinalContextSnapshot,
}

fn planner_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "action": {
                "type": "string",
                "enum": ["tool", "skill", "delegate", "finish", "retry", "stop"]
            },
            "tool_name": { "type": "string" },
            "tool_arguments_json": { "type": "string" },
            "skill_name": { "type": "string" },
            "subagent_name": { "type": "string" },
            "answer": { "type": "string" },
            "reason": { "type": "string" }
        },
        "required": ["action", "tool_name", "tool_arguments_json", "skill_name", "subagent_name", "answer", "reason"]
    })
}

fn build_planner_input_from_prompt(
    state: &SessionState,
    user_input: &str,
    planner_prompt: &str,
) -> Vec<ApiInputMessage> {
    let mut messages = vec![ApiInputMessage {
        role: "developer".to_string(),
        content: planner_prompt.to_string(),
    }];

    messages.extend(state.history.iter().map(|message| ApiInputMessage {
        role: match message.role {
            MessageRole::User => "user".to_string(),
            MessageRole::Assistant => "assistant".to_string(),
        },
        content: message.content.clone(),
    }));

    messages.push(ApiInputMessage {
        role: "user".to_string(),
        content: user_input.to_string(),
    });

    messages
}

fn context_history_path(root_dir: &Path) -> PathBuf {
    root_dir.join(HISTORY_DIR).join(CONTEXT_HISTORY_FILE)
}

fn capture_context_history_entry(
    provider: &str,
    model: &str,
    system_prompt: &str,
    planner_prompt: &str,
    state: &SessionState,
    user_input: &str,
    request_body: &Value,
) -> ContextHistoryEntry {
    ContextHistoryEntry {
        datetime: iso8601ish_now(),
        provider: provider.to_string(),
        model: model.to_string(),
        actual_context: ActualContextSnapshot {
            system_prompt: system_prompt.to_string(),
            planner_prompt: planner_prompt.to_string(),
            prior_history: state
                .history
                .iter()
                .map(|message| ApiInputMessage {
                    role: match message.role {
                        MessageRole::User => "user".to_string(),
                        MessageRole::Assistant => "assistant".to_string(),
                    },
                    content: message.content.clone(),
                })
                .collect(),
            current_user_input: user_input.to_string(),
        },
        final_context: FinalContextSnapshot {
            request_body: request_body.clone(),
        },
    }
}

fn render_context_preview(entry: &ContextHistoryEntry, history_path: &Path) -> String {
    let final_request_json = serde_json::to_string_pretty(&entry.final_context.request_body)
        .unwrap_or_else(|_| entry.final_context.request_body.to_string());
    let prior_history = if entry.actual_context.prior_history.is_empty() {
        "- none".to_string()
    } else {
        entry
            .actual_context
            .prior_history
            .iter()
            .map(|message| format!("- {}: {}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "LLM context preview\n- datetime: {}\n- provider: {}\n- model: {}\n- history file: {}\n\nActual context:\nSystem prompt:\n{}\n\nPlanner prompt:\n{}\n\nPrior history:\n{}\n\nCurrent user input:\n{}\n\nFinal request JSON:\n{}",
        entry.datetime,
        entry.provider,
        entry.model,
        history_path.display(),
        entry.actual_context.system_prompt,
        entry.actual_context.planner_prompt,
        prior_history,
        entry.actual_context.current_user_input,
        final_request_json,
    )
}

fn persist_context_history(path: &Path, entry: &ContextHistoryEntry) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut entries = if path.is_file() {
        let contents = fs::read_to_string(path)?;
        if contents.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str::<Vec<ContextHistoryEntry>>(&contents)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        }
    } else {
        Vec::new()
    };

    entries.push(entry.clone());
    let serialized = serde_json::to_string_pretty(&entries)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, serialized)
}

fn emit_context_preview(root_dir: &Path, entry: &ContextHistoryEntry) {
    let history_path = context_history_path(root_dir);
    println!("{}", render_context_preview(entry, &history_path));
    match persist_context_history(&history_path, entry) {
        Ok(()) => println!("Saved LLM context snapshot to {}.", history_path.display()),
        Err(error) => crate::runtime_log::warn(
            "planner",
            format!(
                "Failed to persist LLM context snapshot to {}: {}",
                history_path.display(),
                error
            ),
        ),
    }
}

fn planner_prompt(
    state: &SessionState,
    memory_context: &str,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
    skills: &BTreeMap<String, SkillSpec>,
    active_skill: Option<&ActiveSkill>,
    task_contract: &TaskContract,
    observations: &[String],
    retry_count: u8,
) -> String {
    let tool_list = visible_tools_for_active_skill(tools, active_skill)
        .iter()
        .map(|tool| format!("- {}", tool.planner_description()))
        .collect::<Vec<_>>()
        .join("\n");
    let subagent_list = subagents
        .values()
        .map(|spec| format!("- {}: {}", spec.name, spec.description))
        .collect::<Vec<_>>()
        .join("\n");
    let skill_list = skills
        .values()
        .map(|skill| format!("- {}: {}", skill.name, skill.description))
        .collect::<Vec<_>>()
        .join("\n");
    let history_summary = planner_history_summary(state);
    let memory_summary = if memory_context.trim().is_empty() {
        "No project memory was loaded.".to_string()
    } else {
        observability::compact_text(memory_context, 2_400)
    };
    let observation_summary = if observations.is_empty() {
        "No observations recorded yet.".to_string()
    } else {
        observations
            .iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|item| observability::compact_text(item, 220))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let active_skill_layer = active_skill.and_then(|active_skill| {
        skills
            .get(&active_skill.name)
            .map(|skill| ActiveSkillLayer {
                summary: format!(
                    "{} activated_by={} task_fit={} allowed_tools={} preferred_subagents={}",
                    active_skill.name,
                    active_skill.activated_by,
                    active_skill.task_fit,
                    active_skill
                        .allowed_tools
                        .as_ref()
                        .map(|items| items.join(", "))
                        .unwrap_or_else(|| "inherit all visible tools".to_string()),
                    if active_skill.preferred_subagents.is_empty() {
                        "none".to_string()
                    } else {
                        active_skill.preferred_subagents.join(", ")
                    }
                ),
                instructions: skill
                    .instructions
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            })
    });
    PlannerPromptLayers {
        system: SystemPromptLayer {
            instructions: "You are the planner for a harness-first Rust coding agent.\nChoose exactly one next action and return JSON only.\nRules:\n- Start from the explicit task contract.\n- The harness loop permits exactly one action per iteration.\n- Prefer calling a tool when the user's request requires an available tool now.\n- Prefer using a skill when the request matches a reusable capability pack or the user explicitly asks for one.\n- Do not select the same skill again when it is already active; use the active skill to guide the next tool, delegation, stop, retry, or finish decision.\n- Prefer delegating to a subagent when the request is exploratory, planning-oriented, or file-focused.\n- Prefer `finish` when the latest observation already contains enough information to answer.\n- Use `retry` only for recoverable failures that gained new information.\n- Use action=`finish` when you can answer directly without a tool or subagent.\n- Use action=`stop` only for terminal conditions.\n- For action=`tool`, set `tool_name` exactly and encode an object in `tool_arguments_json`.\n- For action=`skill`, set `skill_name` exactly.\n- For action=`delegate`, set `subagent_name` exactly.\n- When a field does not apply, leave it empty, except `tool_arguments_json`, which must be `{}`.\n- Fill `reason` with a short explanation.".to_string(),
        },
        project_memory: ProjectMemoryLayer {
            summary: memory_summary,
        },
        task_contract: TaskContractLayer {
            goal: task_contract.goal.clone(),
            constraints: task_contract.constraints.clone(),
            acceptance_criteria: task_contract.acceptance_criteria.clone(),
            relevant_files: task_contract.relevant_files.clone(),
            stop_condition: task_contract.stop_condition.clone(),
        },
        available_skills: if skill_list.is_empty() {
            Vec::new()
        } else {
            skill_list.lines().map(str::to_string).collect()
        },
        active_skill: active_skill_layer,
        observations: ObservationLayer {
            entries: if observations.is_empty() {
                Vec::new()
            } else {
                observation_summary.lines().map(str::to_string).collect()
            },
        },
        channel_metadata: ChannelMetadataLayer {
            channel_name: "cli".to_string(),
            surface: "terminal".to_string(),
            delivery_mode: "local-session".to_string(),
        },
        tool_context: if tool_list.is_empty() {
            Vec::new()
        } else {
            tool_list.lines().map(str::to_string).collect()
        },
        subagent_context: if subagent_list.is_empty() {
            Vec::new()
        } else {
            subagent_list.lines().map(str::to_string).collect()
        },
        history_summary,
        retry_count,
    }
    .render()
}

fn planner_payload_to_decision(
    payload: &PlannerPayload,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
    skills: &BTreeMap<String, SkillSpec>,
) -> Result<Decision, String> {
    let reason = empty_to_default(&payload.reason, "planner did not provide a reason");
    match payload.action.as_str() {
        "tool" => {
            let tool_name = normalize_tool_name(&empty_to_default(&payload.tool_name, ""));
            if tool_name.is_empty() {
                return Err("Planner requested a tool without `tool_name`.".to_string());
            }
            if !tools.iter().any(|tool| tool.is_named(&tool_name)) {
                return Err(format!("Planner requested unknown tool `{tool_name}`"));
            }
            Ok(Decision::CallTool {
                tool_name: tool_name.clone(),
                arguments: parse_planner_tool_arguments(&payload.tool_arguments_json, &tool_name)?,
                reason,
            })
        }
        "skill" => {
            let skill_name = empty_to_default(&payload.skill_name, "");
            if skill_name.is_empty() {
                return Err("Planner requested a skill without `skill_name`.".to_string());
            }
            if !skills.contains_key(&skill_name) {
                return Err(format!("Planner requested unknown skill `{skill_name}`"));
            }
            Ok(Decision::UseSkill { skill_name, reason })
        }
        "delegate" => {
            let subagent_name = empty_to_default(&payload.subagent_name, "");
            if subagent_name.is_empty() {
                return Err("Planner requested delegation without `subagent_name`.".to_string());
            }
            if !subagents.contains_key(&subagent_name) {
                return Err(format!(
                    "Planner requested unknown subagent `{subagent_name}`"
                ));
            }
            Ok(Decision::DelegateSubagent {
                subagent_name,
                reason,
            })
        }
        "finish" => Ok(Decision::Finish {
            answer: empty_to_default(&payload.answer, "The model returned an empty answer."),
            reason,
        }),
        "retry" => Ok(Decision::Retry(reason)),
        "stop" => Ok(Decision::Stop(reason)),
        other => Err(format!("Planner returned unsupported action `{other}`")),
    }
}

fn parse_planner_tool_arguments(raw_arguments: &str, tool_name: &str) -> Result<Value, String> {
    let trimmed = raw_arguments.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Default::default()));
    }

    let parsed = serde_json::from_str::<Value>(trimmed).map_err(|error| {
        format!("Planner tool arguments for `{tool_name}` were not valid JSON: {error}")
    })?;
    if !parsed.is_object() {
        return Err(format!(
            "Planner tool arguments for `{tool_name}` must decode to a JSON object."
        ));
    }
    Ok(parsed)
}

fn empty_to_default(value: &str, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value.trim().to_string()
    }
}

fn reasoning_summary_for_decision(decision: &Decision) -> String {
    match decision {
        Decision::CallTool {
            tool_name, reason, ..
        } => format!("The planner chose tool `{tool_name}` because {reason}"),
        Decision::UseSkill { skill_name, reason } => {
            format!("The planner chose skill `{skill_name}` because {reason}")
        }
        Decision::DelegateSubagent {
            subagent_name,
            reason,
        } => format!("The planner delegated to `{subagent_name}` because {reason}"),
        Decision::Finish { reason, .. } => {
            format!("The planner answered directly because {reason}")
        }
        Decision::Retry(reason) => format!("The planner requested a retry because {reason}"),
        Decision::Stop(reason) => format!("The planner stopped because {reason}"),
    }
}

fn visible_tools_for_active_skill<'a>(
    tools: &'a [Tool],
    active_skill: Option<&ActiveSkill>,
) -> Vec<&'a Tool> {
    let Some(active_skill) = active_skill else {
        return tools.iter().collect();
    };
    let Some(allowed_tools) = &active_skill.allowed_tools else {
        return tools.iter().collect();
    };

    tools
        .iter()
        .filter(|tool| {
            allowed_tools
                .iter()
                .any(|tool_name| tool.is_named(tool_name))
        })
        .collect()
}

fn execute_subagent(spec: &SubagentSpec, request: DispatchRequest) -> DispatchResponse {
    match spec.name.as_str() {
        "explore" => execute_explore_subagent(spec, request),
        "plan" => execute_plan_subagent(spec, request),
        _ => execute_general_subagent(spec, request),
    }
}

pub fn run_spawned_subagent_child() -> io::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let request: DispatchRequest = serde_json::from_str(&input).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid child dispatch request JSON: {error}"),
        )
    })?;
    let root_dir = env::current_dir()?;
    let response = execute_spawned_subagent_request_in_root(&root_dir, request)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
    let payload = serde_json::to_string(&response).map_err(|error| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("failed to serialize child dispatch response: {error}"),
        )
    })?;
    println!("{payload}");
    Ok(())
}

fn execute_spawned_subagent_request_in_root(
    root_dir: &Path,
    request: DispatchRequest,
) -> Result<DispatchResponse, String> {
    let target_name = match &request.target {
        crate::dispatch::DispatchTarget::Subagent { name } => name.clone(),
        crate::dispatch::DispatchTarget::Worker { profile } => {
            return Err(format!(
                "child subagent runtime does not support worker target `{profile}`"
            ))
        }
    };

    let subagents = load_subagent_specs(&root_dir, None)
        .map_err(|error| format!("failed to load subagent specs: {error}"))?;
    let spec = subagents
        .get(&target_name)
        .ok_or_else(|| format!("Unknown subagent `{target_name}`."))?;
    Ok(execute_subagent(spec, request))
}

fn execute_explore_subagent(spec: &SubagentSpec, request: DispatchRequest) -> DispatchResponse {
    let files = if request.file_refs.is_empty() {
        vec!["No matching files were inferred.".to_string()]
    } else {
        request.file_refs.clone()
    };
    let findings = files
        .iter()
        .map(|path| format!("Potentially relevant file: {path}"))
        .collect::<Vec<_>>();
    DispatchResponse {
        dispatch_id: request.dispatch_id,
        responder: format!("subagent {}", spec.name),
        summary: format!("{} narrowed the task to likely files.", spec.name),
        findings: findings.clone(),
        artifact_refs: files.clone(),
        recommended_next_action: if files.is_empty() {
            "Clarify the task or cite exact files.".to_string()
        } else {
            "Use `/agent plan ...` once the target files are confirmed.".to_string()
        },
        final_text: format!(
            "Explore agent\nTask: {}\n\nContext packet goal: {}\n\nRecommended files:\n{}",
            request.task,
            request.context_packet.goal,
            findings
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        usage: vec![TokenUsageRecord::zero(
            format!("subagent `{}`", spec.name),
            "local synthesized subagent",
            "no model API call",
        )],
    }
}

fn execute_plan_subagent(spec: &SubagentSpec, request: DispatchRequest) -> DispatchResponse {
    let steps = vec![
        format!("Load only the files relevant to `{}`.", request.task),
        "Make the smallest coherent change set that matches project memory and commands."
            .to_string(),
        "Verify with targeted tests and finish with a review-oriented summary.".to_string(),
    ];
    let artifact_refs = request
        .file_refs
        .into_iter()
        .take(MAX_RELEVANT_FILES)
        .collect::<Vec<_>>();
    DispatchResponse {
        dispatch_id: request.dispatch_id,
        responder: format!("subagent {}", spec.name),
        summary: format!("{} produced a compact execution plan.", spec.name),
        findings: steps.clone(),
        artifact_refs,
        recommended_next_action: "Execute the plan or run `/review` after implementation."
            .to_string(),
        final_text: format!(
            "Plan agent\nTask: {}\n\nPlan:\n{}",
            request.task,
            steps
                .iter()
                .map(|step| format!("- {step}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        usage: vec![TokenUsageRecord::zero(
            format!("subagent `{}`", spec.name),
            "local synthesized subagent",
            "no model API call",
        )],
    }
}

fn execute_general_subagent(spec: &SubagentSpec, request: DispatchRequest) -> DispatchResponse {
    let findings = vec![
        format!("Active task: {}", request.task),
        format!("Loaded memory sources: {}", request.memory_refs.len()),
        format!("Relevant files inferred: {}", request.file_refs.len()),
    ];
    DispatchResponse {
        dispatch_id: request.dispatch_id,
        responder: format!("subagent {}", spec.name),
        summary: format!("{} returned a compact handoff.", spec.name),
        findings: findings.clone(),
        artifact_refs: request.file_refs.clone(),
        recommended_next_action:
            "Refine the task, delegate to `/agent plan ...`, or call an explicit tool if needed."
                .to_string(),
        final_text: format!(
            "General-purpose agent\nTask: {}\n\nKnown facts:\n{}\n\nPrompt excerpt:\n{}",
            request.task,
            request
                .context_packet
                .known_facts
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n"),
            observability::compact_text(&spec.prompt, 320)
        ),
        usage: vec![TokenUsageRecord::zero(
            format!("subagent `{}`", spec.name),
            "local synthesized subagent",
            "no model API call",
        )],
    }
}

fn usage_trace_lines(records: &[TokenUsageRecord]) -> Vec<String> {
    records
        .iter()
        .map(|record| format!("Token usage: {}", record.trace_line()))
        .collect()
}

fn render_usage_summary(records: &[TokenUsageRecord]) -> String {
    if records.is_empty() {
        return "Token usage\n- none recorded".to_string();
    }

    let mut lines = vec!["Token usage".to_string()];
    lines.extend(records.iter().map(TokenUsageRecord::summary_line));
    lines.join("\n")
}

fn record_usage_observability(records: &[TokenUsageRecord]) {
    for record in records {
        Observability::record_event(
            "token_usage",
            vec![
                KeyValue::new("usage.actor", record.actor.clone()),
                KeyValue::new("usage.execution", record.execution.clone()),
                KeyValue::new("usage.input_tokens", i64::from(record.input_tokens)),
                KeyValue::new("usage.output_tokens", i64::from(record.output_tokens)),
                KeyValue::new("usage.total_tokens", i64::from(record.total_tokens)),
                KeyValue::new("usage.note", record.note.clone()),
            ],
        );
    }
}

fn compact_history(history: &[ConversationMessage]) -> String {
    if history.is_empty() {
        return "No prior session history was available.".to_string();
    }

    history
        .iter()
        .rev()
        .take(MAX_COMPACTED_HISTORY_ITEMS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            };
            format!(
                "{role}: {}",
                observability::compact_text(&message.content, 120)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn planner_history_summary(state: &SessionState) -> String {
    let mut sections = Vec::new();
    if let Some(summary) = &state.compacted_summary {
        sections.push(format!(
            "Earlier compacted context: {}",
            observability::compact_text(summary, MAX_COMPACTED_SUMMARY_CHARS)
        ));
    }

    if !state.history.is_empty() {
        sections.push(format!("Recent turns: {}", compact_history(&state.history)));
    }

    if sections.is_empty() {
        "No prior session history was available.".to_string()
    } else {
        sections.join("\n")
    }
}

fn compact_session_state(state: &mut SessionState, force: bool) -> CompactionResult {
    let history_len = state.history.len();
    let history_chars = state
        .history
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();
    let needs_compaction =
        force || history_len > MAX_ACTIVE_HISTORY_ITEMS || history_chars > MAX_ACTIVE_HISTORY_CHARS;

    if !needs_compaction {
        return CompactionResult::no_op(
            "history is already within the active context budget",
            state,
        );
    }

    if history_len <= RETAINED_RECENT_HISTORY_ITEMS {
        return CompactionResult::no_op(
            "there are not enough older messages to summarize yet",
            state,
        );
    }

    let split_index = history_len - RETAINED_RECENT_HISTORY_ITEMS;
    let compacted_slice = state.history[..split_index].to_vec();
    let retained_history = state.history[split_index..].to_vec();
    let summary = merge_compacted_summary(state.compacted_summary.as_deref(), &compacted_slice);
    state.compacted_summary = Some(summary.clone());
    state.history = retained_history;

    let reason = if force {
        "manual compaction requested".to_string()
    } else if history_len > MAX_ACTIVE_HISTORY_ITEMS {
        format!("history exceeded {} messages", MAX_ACTIVE_HISTORY_ITEMS)
    } else {
        format!("history exceeded {} characters", MAX_ACTIVE_HISTORY_CHARS)
    };

    CompactionResult {
        compacted: true,
        reason,
        compacted_messages: compacted_slice.len(),
        retained_messages: state.history.len(),
        summary: state.compacted_summary.clone(),
    }
}

fn merge_compacted_summary(
    existing_summary: Option<&str>,
    compacted_messages: &[ConversationMessage],
) -> String {
    let mut sections = Vec::new();
    if let Some(summary) = existing_summary {
        let trimmed = summary.trim();
        if !trimmed.is_empty() {
            sections.push(trimmed.to_string());
        }
    }

    let recent_summary = compact_history(compacted_messages);
    if recent_summary != "No prior session history was available." {
        sections.push(format!("Earlier turns: {recent_summary}"));
    }

    observability::compact_text(&sections.join("\n"), MAX_COMPACTED_SUMMARY_CHARS)
}

fn agent_memory_paths(root_dir: &Path) -> AgentMemoryPaths {
    let memory_dir = root_dir.join(WORKSPACE_DIR);
    AgentMemoryPaths {
        short_term_snapshot: memory_dir.join(SHORT_TERM_MEMORY_FILE),
        long_term_notes: memory_dir.join(LONG_TERM_MEMORY_FILE),
    }
}

fn load_memory_stack(
    root_dir: &Path,
    home_dir: Option<&Path>,
    mem0: Option<&Mem0Config>,
) -> io::Result<MemoryStack> {
    load_memory_stack_with_request(root_dir, home_dir, mem0, &MemoryLoadRequest::default())
}

fn load_memory_stack_with_request(
    root_dir: &Path,
    home_dir: Option<&Path>,
    mem0: Option<&Mem0Config>,
    request: &MemoryLoadRequest,
) -> io::Result<MemoryStack> {
    let mut sources = Vec::new();
    let mut visited = BTreeSet::new();
    let normalized_focus_paths = request.normalized_focus_paths(root_dir);

    if let Some(home_dir) = home_dir {
        let user_memory = home_dir.join(CLAUDE_DIR).join(PROJECT_MEMORY_FILE);
        load_memory_source(
            root_dir,
            &user_memory,
            MemoryScope::User,
            None,
            0,
            &mut visited,
            &mut sources,
        )?;
    }

    let project_memory = root_dir.join(PROJECT_MEMORY_FILE);
    load_memory_source(
        root_dir,
        &project_memory,
        MemoryScope::Project,
        None,
        0,
        &mut visited,
        &mut sources,
    )?;

    let long_term_memory = agent_memory_paths(root_dir).long_term_notes;
    if let Some(mut source) = read_long_term_memory_via_agent(mem0, &long_term_memory)? {
        source.selector_hint = None;
        sources.push(source);
    }

    let merged_instructions = sources
        .iter()
        .map(|source| {
            format!(
                "[{}] {}\n{}",
                source.scope,
                source.path.display(),
                source.contents.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(MemoryStack {
        sources,
        merged_instructions,
        load_request: MemoryLoadRequest {
            focus_paths: normalized_focus_paths,
        },
    })
}

fn load_short_term_memory(path: &Path) -> io::Result<ShortTermMemorySnapshot> {
    if !path.is_file() {
        return Ok(ShortTermMemorySnapshot::default());
    }
    let contents = fs::read_to_string(path)?;
    if contents.trim().is_empty() {
        return Ok(ShortTermMemorySnapshot::default());
    }
    serde_json::from_str(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn save_short_term_memory(path: &Path, state: &SessionState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let snapshot = ShortTermMemorySnapshot {
        compacted_summary: state.compacted_summary.clone(),
        observations: state.observations.clone(),
        active_skill: state.active_skill.clone(),
        recent_history: state
            .history
            .iter()
            .rev()
            .take(MAX_SHORT_TERM_HISTORY_ITEMS)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    };
    let serialized = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, serialized)
}

fn normalize_repo_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }

    if normalized.as_os_str().is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn memory_selector_hint(
    root_dir: &Path,
    source_path: &Path,
    imported_from: Option<&Path>,
) -> Option<PathBuf> {
    imported_from?;
    let canonical_root = fs::canonicalize(root_dir).unwrap_or_else(|_| root_dir.to_path_buf());
    let relative = source_path.strip_prefix(&canonical_root).ok()?;
    let relative = normalize_repo_relative_path(relative)?;
    if relative == Path::new(PROJECT_MEMORY_FILE) || relative.starts_with(WORKSPACE_DIR) {
        return None;
    }

    relative
        .parent()
        .map(Path::to_path_buf)
        .filter(|path| !path.as_os_str().is_empty())
}

fn append_long_term_memory(mem0: Option<&Mem0Config>, path: &Path, note: &str) -> io::Result<()> {
    let agent = long_term_memory_agent(mem0, path);
    agent.append_long_term_note(&format!("- [{}] {}\n", iso8601ish_now(), note.trim()))
}

fn long_term_memory_agent<'a>(
    mem0: Option<&'a Mem0Config>,
    path: &'a Path,
) -> Box<dyn MemoryAgent + 'a> {
    if let Some(mem0) = mem0 {
        Box::new(Mem0MemoryAgent::new(mem0))
    } else {
        Box::new(FileMemoryAgent::new(path))
    }
}

fn read_long_term_memory_via_agent(
    mem0: Option<&Mem0Config>,
    path: &Path,
) -> io::Result<Option<MemorySource>> {
    let agent = long_term_memory_agent(mem0, path);
    let snapshot = agent.read_long_term_memory()?;
    if mem0.is_none() && !path.is_file() && snapshot.contents.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(MemorySource {
        scope: MemoryScope::Project.to_string(),
        path: snapshot.path,
        contents: snapshot.contents,
        imported_from: None,
        selector_hint: None,
    }))
}

fn load_memory_source(
    root_dir: &Path,
    path: &Path,
    scope: MemoryScope,
    imported_from: Option<PathBuf>,
    depth: usize,
    visited: &mut BTreeSet<PathBuf>,
    sources: &mut Vec<MemorySource>,
) -> io::Result<()> {
    if !path.is_file() || depth > MAX_IMPORT_DEPTH {
        return Ok(());
    }

    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }

    let contents = fs::read_to_string(path)?;
    sources.push(MemorySource {
        scope: scope.to_string(),
        path: canonical.clone(),
        contents: contents.clone(),
        imported_from: imported_from.clone(),
        selector_hint: memory_selector_hint(root_dir, &canonical, imported_from.as_deref()),
    });

    for import_path in extract_imports(&contents, path) {
        load_memory_source(
            root_dir,
            &import_path,
            MemoryScope::Imported,
            Some(canonical.clone()),
            depth + 1,
            visited,
            sources,
        )?;
    }

    Ok(())
}

fn extract_imports(contents: &str, source_path: &Path) -> Vec<PathBuf> {
    contents
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('@') {
                return None;
            }
            let raw = trimmed.trim_start_matches('@').trim();
            if raw.is_empty() {
                return None;
            }
            let candidate = PathBuf::from(raw);
            Some(if candidate.is_absolute() {
                candidate
            } else {
                source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(candidate)
            })
        })
        .collect()
}

fn load_subagent_specs(
    root_dir: &Path,
    home_dir: Option<&Path>,
) -> io::Result<BTreeMap<String, SubagentSpec>> {
    let mut entries = BTreeMap::new();
    if let Some(home_dir) = home_dir {
        load_subagent_dir(
            &home_dir.join(CLAUDE_DIR).join(AGENTS_DIR),
            DefinitionScope::User,
            &mut entries,
        )?;
    }
    load_subagent_dir(
        &root_dir.join(CLAUDE_DIR).join(AGENTS_DIR),
        DefinitionScope::Project,
        &mut entries,
    )?;
    Ok(entries)
}

fn load_skill_specs(
    root_dir: &Path,
    home_dir: Option<&Path>,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
) -> io::Result<LoadedSkills> {
    let mut loaded = LoadedSkills::default();
    if let Some(home_dir) = home_dir {
        load_skill_dir(
            &home_dir.join(CLAUDE_DIR).join(SKILLS_DIR),
            DefinitionScope::User,
            &mut loaded,
            tools,
            subagents,
        )?;
    }
    load_skill_dir(
        &root_dir.join(CLAUDE_DIR).join(SKILLS_DIR),
        DefinitionScope::Project,
        &mut loaded,
        tools,
        subagents,
    )?;
    Ok(loaded)
}

fn load_subagent_dir(
    dir: &Path,
    scope: DefinitionScope,
    entries: &mut BTreeMap<String, SubagentSpec>,
) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let mut paths = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        let contents = fs::read_to_string(&path)?;
        let (frontmatter, body) = split_frontmatter(&contents);
        let name = frontmatter
            .get("name")
            .cloned()
            .unwrap_or_else(|| file_stem_name(&path));
        let description = frontmatter
            .get("description")
            .cloned()
            .unwrap_or_else(|| "Claude-style delegated subagent.".to_string());
        let tool_allowlist = frontmatter
            .get("tools")
            .and_then(|value| parse_tools_field(value));
        entries.insert(
            name.clone(),
            SubagentSpec {
                name,
                description,
                prompt: body.trim().to_string(),
                tool_allowlist,
                scope: scope.to_string(),
                source_path: path,
            },
        );
    }

    Ok(())
}

fn load_skill_dir(
    dir: &Path,
    scope: DefinitionScope,
    loaded: &mut LoadedSkills,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let mut paths = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir() && path.join("SKILL.md").is_file())
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        let skill_file = path.join("SKILL.md");
        let contents = fs::read_to_string(&skill_file)?;
        match parse_skill_spec(
            &skill_file,
            &contents,
            &path,
            scope.clone(),
            tools,
            subagents,
        ) {
            Ok(spec) => {
                loaded.entries.insert(spec.name.clone(), spec);
            }
            Err(issues) => loaded.issues.extend(issues),
        }
    }

    Ok(())
}

fn parse_skill_spec(
    skill_file: &Path,
    contents: &str,
    skill_dir: &Path,
    scope: DefinitionScope,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
) -> Result<SkillSpec, Vec<SkillValidationIssue>> {
    let (frontmatter, body) = split_frontmatter(contents);
    let name = frontmatter
        .get("name")
        .cloned()
        .unwrap_or_else(|| file_stem_name(skill_dir));
    let description = frontmatter
        .get("description")
        .cloned()
        .unwrap_or_else(|| "Reusable skill capability pack.".to_string());
    let version = frontmatter.get("version").cloned();
    let tags = parse_list_field(frontmatter.get("tags").map(String::as_str)).unwrap_or_default();
    let when_to_use = frontmatter.get("when_to_use").cloned();
    let when_not_to_use = frontmatter.get("when_not_to_use").cloned();
    let allowed_tools = parse_list_field(frontmatter.get("allowed_tools").map(String::as_str));
    let preferred_subagents =
        parse_list_field(frontmatter.get("preferred_subagents").map(String::as_str))
            .unwrap_or_default();
    let input_contract = frontmatter.get("input_contract").cloned();
    let output_contract = frontmatter.get("output_contract").cloned();
    let instructions = body.trim().to_string();

    let mut issues = Vec::new();
    if name.trim().is_empty() {
        issues.push(skill_issue(skill_file, "Skill `name` must not be empty."));
    }
    if description.trim().is_empty() {
        issues.push(skill_issue(
            skill_file,
            "Skill `description` must not be empty.",
        ));
    }
    if instructions.is_empty() {
        issues.push(skill_issue(skill_file, "Skill body must not be empty."));
    }

    for heading in [
        "## Purpose",
        "## Use When",
        "## Do Not Use When",
        "## Workflow",
        "## Constraints",
        "## Expected Outputs",
    ] {
        if !instructions.contains(heading) {
            issues.push(skill_issue(
                skill_file,
                format!("Skill is missing required section `{heading}`."),
            ));
        }
    }

    if let Some(allowed_tools) = &allowed_tools {
        for tool_name in allowed_tools {
            if !tools.iter().any(|tool| tool.is_named(tool_name)) {
                issues.push(skill_issue(
                    skill_file,
                    format!("Skill references unknown tool `{tool_name}` in `allowed_tools`."),
                ));
            }
        }
    }

    for subagent_name in &preferred_subagents {
        if !subagents.contains_key(subagent_name) {
            issues.push(skill_issue(
                skill_file,
                format!(
                    "Skill references unknown subagent `{subagent_name}` in `preferred_subagents`."
                ),
            ));
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    Ok(SkillSpec {
        name,
        description,
        instructions,
        version,
        tags,
        when_to_use,
        when_not_to_use,
        allowed_tools: allowed_tools.filter(|items| !items.is_empty()),
        preferred_subagents,
        input_contract,
        output_contract,
        scope: scope.to_string(),
        source_path: skill_file.to_path_buf(),
    })
}

fn load_custom_commands(
    root_dir: &Path,
    home_dir: Option<&Path>,
) -> io::Result<BTreeMap<String, CustomCommand>> {
    let mut entries = BTreeMap::new();
    if let Some(home_dir) = home_dir {
        load_command_dir(
            &home_dir.join(CLAUDE_DIR).join(COMMANDS_DIR),
            DefinitionScope::User,
            &mut entries,
        )?;
    }
    load_command_dir(
        &root_dir.join(CLAUDE_DIR).join(COMMANDS_DIR),
        DefinitionScope::Project,
        &mut entries,
    )?;
    Ok(entries)
}

fn load_command_dir(
    dir: &Path,
    scope: DefinitionScope,
    entries: &mut BTreeMap<String, CustomCommand>,
) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let mut paths = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        let contents = fs::read_to_string(&path)?;
        let (frontmatter, body) = split_frontmatter(&contents);
        let name = frontmatter
            .get("name")
            .cloned()
            .unwrap_or_else(|| file_stem_name(&path));
        let description = frontmatter
            .get("description")
            .cloned()
            .unwrap_or_else(|| "Project slash command.".to_string());
        entries.insert(
            name.clone(),
            CustomCommand {
                name,
                description,
                template: body.trim().to_string(),
                source_path: path,
                scope: scope.clone(),
            },
        );
    }

    Ok(())
}

fn split_frontmatter(contents: &str) -> (BTreeMap<String, String>, String) {
    let mut frontmatter = BTreeMap::new();
    if !contents.starts_with("---\n") {
        return (frontmatter, contents.to_string());
    }

    let mut sections = contents.splitn(3, "---\n");
    let _ = sections.next();
    let Some(frontmatter_body) = sections.next() else {
        return (frontmatter, contents.to_string());
    };
    let Some(body) = sections.next() else {
        return (frontmatter, contents.to_string());
    };

    for line in frontmatter_body.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        frontmatter.insert(key.trim().to_string(), value.trim().to_string());
    }

    (frontmatter, body.to_string())
}

fn parse_tools_field(value: &str) -> Option<Vec<String>> {
    parse_list_field(Some(value)).map(|items| {
        items
            .into_iter()
            .map(|item| normalize_tool_name(&item))
            .collect()
    })
}

fn normalize_tool_name(name: &str) -> String {
    match name.trim() {
        LEGACY_WEB_SEARCH_TOOL_NAME => WEB_SEARCH_TOOL_NAME.to_string(),
        other => other.to_string(),
    }
}

fn file_stem_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unnamed")
        .to_string()
}

fn parse_list_field(value: Option<&str>) -> Option<Vec<String>> {
    let Some(value) = value else {
        return None;
    };
    let normalized = value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    if normalized.is_empty() {
        return None;
    }

    let items = normalized
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

fn skill_issue(path: &Path, message: impl Into<String>) -> SkillValidationIssue {
    SkillValidationIssue {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

fn expand_custom_command(command: &CustomCommand, arguments: &str) -> String {
    let mut output = command.template.clone();
    output = output.replace("$ARGUMENTS", arguments.trim());
    let positional = arguments.split_whitespace().collect::<Vec<_>>();
    for (index, value) in positional.iter().enumerate() {
        output = output.replace(&format!("${}", index + 1), value);
    }
    output
}

fn parse_definition_scope(raw: &str) -> io::Result<DefinitionScope> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "project" => Ok(DefinitionScope::Project),
        "user" => Ok(DefinitionScope::User),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Unsupported scope `{other}`. Use `project` or `user`."),
        )),
    }
}

fn skills_root_for_scope(
    root_dir: &Path,
    home_dir: Option<&Path>,
    scope: DefinitionScope,
) -> io::Result<PathBuf> {
    match scope {
        DefinitionScope::Project => Ok(root_dir.join(CLAUDE_DIR).join(SKILLS_DIR)),
        DefinitionScope::User => {
            let Some(home_dir) = home_dir else {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "User scope requires a home directory.",
                ));
            };
            Ok(home_dir.join(CLAUDE_DIR).join(SKILLS_DIR))
        }
    }
}

fn normalize_skill_name(raw: &str) -> io::Result<String> {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| match character {
            'a'..='z' | '0'..='9' => character,
            '-' | '_' | ' ' | '/' => '-',
            _ => '-',
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if normalized.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Skill name must contain at least one alphanumeric character.",
        ));
    }
    Ok(normalized)
}

fn create_skill_scaffold(
    root_dir: &Path,
    home_dir: Option<&Path>,
    scope: DefinitionScope,
    name: &str,
    description: &str,
) -> io::Result<PathBuf> {
    let skill_name = normalize_skill_name(name)?;
    let description = description.trim();
    if description.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Skill description must not be empty.",
        ));
    }

    let skill_root = skills_root_for_scope(root_dir, home_dir, scope)?;
    let skill_dir = skill_root.join(&skill_name);
    let skill_file = skill_dir.join("SKILL.md");
    if skill_file.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "Skill `{skill_name}` already exists at {}.",
                skill_file.display()
            ),
        ));
    }

    fs::create_dir_all(&skill_dir)?;
    fs::write(
        &skill_file,
        default_skill_template(&skill_name, description),
    )?;
    Ok(skill_file)
}

enum ParsedSkillSource {
    Local {
        root: PathBuf,
        skill_name: String,
    },
    Git {
        repo_url: String,
        skill_name: String,
    },
}

fn install_skill_from_source(
    root_dir: &Path,
    home_dir: Option<&Path>,
    scope: DefinitionScope,
    source: &str,
    explicit_skill_name: Option<&str>,
) -> io::Result<PathBuf> {
    let parsed = parse_skill_install_source(source, explicit_skill_name)?;
    let destination_root = skills_root_for_scope(root_dir, home_dir, scope)?;
    fs::create_dir_all(&destination_root)?;

    let (source_dir, skill_name) = match parsed {
        ParsedSkillSource::Local { root, skill_name } => (root, skill_name),
        ParsedSkillSource::Git {
            repo_url,
            skill_name,
        } => {
            let checkout_dir = std::env::temp_dir()
                .join(format!("agent-in-rust-skill-install-{}", now_epoch_ms()));
            let status = std::process::Command::new("git")
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg(&repo_url)
                .arg(&checkout_dir)
                .status()?;
            if !status.success() {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Failed to clone skill source `{repo_url}`."),
                ));
            }
            (checkout_dir.join(&skill_name), skill_name)
        }
    };

    let destination_dir = destination_root.join(&skill_name);
    let destination_file = destination_dir.join("SKILL.md");
    if destination_file.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "Skill `{skill_name}` already exists at {}.",
                destination_file.display()
            ),
        ));
    }

    if !source_dir.join("SKILL.md").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Skill source `{}` does not contain `SKILL.md`.",
                source_dir.display()
            ),
        ));
    }

    copy_directory_recursive(&source_dir, &destination_dir)?;
    Ok(destination_file)
}

fn parse_skill_install_source(
    source: &str,
    explicit_skill_name: Option<&str>,
) -> io::Result<ParsedSkillSource> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Skill install source must not be empty.",
        ));
    }

    let path = PathBuf::from(trimmed);
    if path.exists() {
        return resolve_local_skill_source(path, explicit_skill_name);
    }

    if let Ok(parsed_url) = url::Url::parse(trimmed) {
        let host = parsed_url.host_str().unwrap_or_default();
        let segments = parsed_url
            .path_segments()
            .map(|items| items.filter(|item| !item.is_empty()).collect::<Vec<_>>())
            .unwrap_or_default();
        if host.eq_ignore_ascii_case("skills.sh") {
            if segments.len() < 3 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "skills.sh URLs must look like `https://skills.sh/<owner>/<repo>/<skill>`.",
                ));
            }
            let skill_name = explicit_skill_name
                .map(normalize_skill_name)
                .transpose()?
                .unwrap_or_else(|| {
                    normalize_skill_name(segments[2]).unwrap_or_else(|_| segments[2].to_string())
                });
            return Ok(ParsedSkillSource::Git {
                repo_url: format!("https://github.com/{}/{}.git", segments[0], segments[1]),
                skill_name,
            });
        }
        if host.eq_ignore_ascii_case("github.com") {
            if segments.len() < 2 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "GitHub URLs must include owner and repo.",
                ));
            }
            let skill_name = explicit_skill_name
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "GitHub skill installs require `--skill <name>`.",
                    )
                })
                .and_then(normalize_skill_name)?;
            return Ok(ParsedSkillSource::Git {
                repo_url: format!("https://github.com/{}/{}.git", segments[0], segments[1]),
                skill_name,
            });
        }
    }

    let repo_parts = trimmed.split('/').collect::<Vec<_>>();
    if repo_parts.len() == 3 {
        return Ok(ParsedSkillSource::Git {
            repo_url: format!("https://github.com/{}/{}.git", repo_parts[0], repo_parts[1]),
            skill_name: normalize_skill_name(repo_parts[2])?,
        });
    }
    if repo_parts.len() == 2 {
        let skill_name = explicit_skill_name
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Repository installs require `--skill <name>` unless the source already includes it.",
                )
            })
            .and_then(normalize_skill_name)?;
        return Ok(ParsedSkillSource::Git {
            repo_url: format!("https://github.com/{}/{}.git", repo_parts[0], repo_parts[1]),
            skill_name,
        });
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "Unsupported skill source. Use a local path, `owner/repo[/skill]`, GitHub URL, or skills.sh URL.",
    ))
}

fn resolve_local_skill_source(
    root: PathBuf,
    explicit_skill_name: Option<&str>,
) -> io::Result<ParsedSkillSource> {
    if root.join("SKILL.md").is_file() {
        let skill_name = explicit_skill_name
            .map(normalize_skill_name)
            .transpose()?
            .unwrap_or_else(|| {
                normalize_skill_name(
                    root.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("skill"),
                )
                .unwrap_or_else(|_| "skill".to_string())
            });
        return Ok(ParsedSkillSource::Local { root, skill_name });
    }

    let skill_name = explicit_skill_name
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Local skill collection installs require `--skill <name>`.",
            )
        })
        .and_then(normalize_skill_name)?;
    let skill_root = root.join(&skill_name);
    if !skill_root.join("SKILL.md").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Skill `{skill_name}` was not found under local source `{}`.",
                root.display()
            ),
        ));
    }

    Ok(ParsedSkillSource::Local {
        root: skill_root,
        skill_name,
    })
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory_recursive(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn default_skill_template(name: &str, description: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nversion: 0.1.0\ntags: [workflow]\nwhen_to_use: explicit request or close match to the skill purpose\nwhen_not_to_use: when the task is better served by a tool-only answer or a different skill\nallowed_tools: []\npreferred_subagents: []\ninput_contract: task contract plus relevant files and observations\noutput_contract: concise result, artifact, or next step\n---\n\n# {title}\n\n## Purpose\n{description}\n\n## Use When\n- The user explicitly asks for this workflow.\n- The task matches the reusable capability described above.\n\n## Do Not Use When\n- A single tool call is enough.\n- Another skill or delegated subagent is a better fit.\n\n## Workflow\n1. Start from the explicit task contract.\n2. Load only the files, tools, and observations needed for this capability.\n3. Produce the smallest coherent result for the current task.\n\n## Constraints\n- Keep the working set compact.\n- Prefer deterministic execution and explicit observations.\n- Respect active tool and approval boundaries.\n\n## Expected Outputs\n- A concise result, artifact, or next step.\n\n## Examples\n- Example task: apply this skill to a bounded implementation or review task.\n",
        title = name
            .split('-')
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn suggest_relevant_files(root_dir: &Path, task: &str) -> Vec<String> {
    let tokens = task
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .filter(|token| token.len() >= 4)
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut scored = collect_candidate_files(root_dir)
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(root_dir).unwrap_or(&path).to_path_buf();
            let display = relative.display().to_string();
            let lower_path = display.to_ascii_lowercase();
            let contents = fs::read_to_string(&path).unwrap_or_default();
            let preview = contents
                .chars()
                .take(4_000)
                .collect::<String>()
                .to_ascii_lowercase();
            let mut score = 0usize;
            for token in &tokens {
                if lower_path.contains(token) {
                    score += 3;
                }
                if preview.contains(token) {
                    score += 1;
                }
            }
            if score == 0 {
                None
            } else {
                Some((score, display))
            }
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored
        .into_iter()
        .take(MAX_RELEVANT_FILES)
        .map(|(_, path)| path)
        .collect()
}

fn collect_candidate_files(root_dir: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    collect_candidate_files_recursive(root_dir, root_dir, &mut results);
    results
}

fn collect_candidate_files_recursive(root_dir: &Path, current: &Path, results: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if path.is_dir() {
            if matches!(name, ".git" | "target" | ".venv" | "graphify-out" | "logs") {
                continue;
            }
            collect_candidate_files_recursive(root_dir, &path, results);
            continue;
        }

        if path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| matches!(value, "rs" | "md" | "toml" | "sh" | "json"))
            .unwrap_or(false)
        {
            if path.starts_with(root_dir) {
                results.push(path);
            }
        }
    }
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn iso8601ish_now() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_seconds = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let days = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };

    (year, month as u32, day as u32)
}

fn configured_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sanitize_mem0_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn http_client() -> io::Result<Client> {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(mem0_io_error)
}

fn mem0_io_error(error: impl fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("Mem0 request error: {error}"))
}

fn insert_optional_json_field(payload: &mut Value, key: &str, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(key.to_string(), json!(value));
    }
}

fn render_mem0_memory_contents(config: &Mem0Config, memories: &[Mem0MemoryRecord]) -> String {
    let mut lines = vec![
        "# Long-Term Memory (Mem0)".to_string(),
        String::new(),
        format!(
            "Scope: user_id=`{}` agent_id=`{}` app_id=`{}`",
            config.user_id, config.agent_id, config.app_id
        ),
        String::new(),
    ];

    if memories.is_empty() {
        lines.push("No durable notes recorded yet.".to_string());
    } else {
        for memory in memories {
            let timestamp = memory
                .updated_at
                .as_deref()
                .or(memory.created_at.as_deref())
                .unwrap_or("unknown-time");
            lines.push(format!("- [{timestamp}] {}", memory.memory.trim()));
        }
    }

    lines.join("\n")
}

#[derive(Debug, Deserialize)]
struct Mem0MemoryRecord {
    memory: String,
    #[allow(dead_code)]
    id: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Mem0AddResult {
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    event: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Mem0ResultsEnvelope<T> {
    results: Vec<T>,
}

fn parse_mem0_list_response(body: &str) -> io::Result<Vec<Mem0MemoryRecord>> {
    if let Ok(response) = serde_json::from_str::<Vec<Mem0MemoryRecord>>(body) {
        return Ok(response);
    }
    if let Ok(response) = serde_json::from_str::<Mem0ResultsEnvelope<Mem0MemoryRecord>>(body) {
        return Ok(response.results);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Unable to parse Mem0 list response: {body}"),
    ))
}

fn parse_mem0_add_response(body: &str) -> io::Result<Vec<Mem0AddResult>> {
    if let Ok(response) = serde_json::from_str::<Vec<Mem0AddResult>>(body) {
        return Ok(response);
    }
    if let Ok(response) = serde_json::from_str::<Mem0ResultsEnvelope<Mem0AddResult>>(body) {
        return Ok(response.results);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Unable to parse Mem0 add response: {body}"),
    ))
}

fn scaffold_claude_project(root_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut created = Vec::new();
    let files = vec![
        (root_dir.join(PROJECT_MEMORY_FILE), default_claude_memory()),
        (
            root_dir.join(SYSTEM_PROMPT_DIR).join(SYSTEM_PROMPT_FILE),
            default_system_prompt_file(),
        ),
        (
            root_dir
                .join(CLAUDE_DIR)
                .join(AGENTS_DIR)
                .join("explore.md"),
            default_explore_agent(),
        ),
        (
            root_dir.join(CLAUDE_DIR).join(AGENTS_DIR).join("plan.md"),
            default_plan_agent(),
        ),
        (
            root_dir
                .join(CLAUDE_DIR)
                .join(AGENTS_DIR)
                .join("general-purpose.md"),
            default_general_agent(),
        ),
        (
            root_dir
                .join(CLAUDE_DIR)
                .join(COMMANDS_DIR)
                .join("review.md"),
            default_review_command(),
        ),
        (
            root_dir
                .join(CLAUDE_DIR)
                .join(SKILLS_DIR)
                .join("ship-small")
                .join("SKILL.md"),
            default_ship_small_skill(),
        ),
        (
            root_dir.join(WORKSPACE_DIR).join(LONG_TERM_MEMORY_FILE),
            default_long_term_memory(),
        ),
    ];

    for (path, contents) in files {
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)?;
        created.push(path);
    }

    Ok(created)
}

fn default_claude_memory() -> &'static str {
    "# CLAUDE.md\n\nProject memory for the local harness-first runtime.\n\n## Commands\n- `cargo check`\n- `cargo test`\n- `cargo fmt`\n\n## Agent Contract\n- Start every run with an explicit task contract: goal, constraints, acceptance criteria, relevant files, and stop condition.\n- Take exactly one action per loop iteration: call a tool, use a skill, delegate to a subagent, retry, stop, or finalize.\n- Keep observations explicit between iterations instead of hiding state in prose.\n- Prefer compact context packets and explicit file references over transcript sprawl.\n\n## Memory Model\n- Short-term memory lives in `Workspace/short-term.json` and resumes recent session state.\n- Long-term memory uses Mem0 when `MEM0_API_KEY` is configured and falls back to `Workspace/MEMORY.md` otherwise.\n- Retrieved long-term memory is loaded into the planner's standing memory stack.\n- Promote durable notes with `/remember <note>` instead of hiding them in chat history.\n\n## Skills\n- Project skills live under `.claude/skills/<skill-name>/SKILL.md`.\n- User skills live under `~/.claude/skills/<skill-name>/SKILL.md`.\n- Install reusable skills before duplicating behavior in prompts or chat history.\n\n## Working Style\n- Load only the files needed for the current task.\n- Prefer deterministic harness behavior over prompt-only cleverness.\n- Use `/review` after meaningful code changes.\n"
}

fn default_system_prompt_file() -> &'static str {
    "You are a harness-first coding agent. Start from an explicit task contract, load project memory before acting, take exactly one action per loop iteration, keep context compact, and make tools, delegation, retries, and stop conditions legible in the trace.\n"
}

fn default_explore_agent() -> &'static str {
    "---\nname: explore\ndescription: Inspect code paths, locate files, and return a compact handoff.\ntools: [web_search_tool]\n---\n\n# Explore Agent\n\nStart from the delegated task contract.\nNarrow the task to files, modules, constraints, and missing facts.\nReturn findings, file references, unresolved gaps, and the next practical action.\nDo not sprawl into implementation."
}

fn default_plan_agent() -> &'static str {
    "---\nname: plan\ndescription: Produce a compact, decision-complete implementation plan.\n---\n\n# Plan Agent\n\nStart from the delegated task contract.\nTurn the task into a concrete plan with the minimum coherent change set, relevant files, verification steps, risks, and an explicit stop condition."
}

fn default_general_agent() -> &'static str {
    "---\nname: general-purpose\ndescription: Handle focused delegated tasks that do not require a specialist.\n---\n\n# General-Purpose Agent\n\nStart from the delegated task contract.\nHandle one bounded delegated task at a time and return what was learned, the concrete artifact or answer, and the recommended next action."
}

fn default_review_command() -> &'static str {
    "---\nname: review\ndescription: Run a review-focused workflow over the current task or cited files.\n---\n\nReview this task from an engineering-harness perspective.\nFocus on bugs, regressions, incorrect tool or retry behavior, missing tests, unclear assumptions, and gaps between runtime behavior and durable project memory.\n\n$ARGUMENTS"
}

fn default_ship_small_skill() -> &'static str {
    "---\nname: ship-small\ndescription: Bias implementation toward the smallest coherent change set with verification.\n---\n\n# Ship Small\n\n## Purpose\nKeep implementation increments small, testable, and easy to review.\n\n## Workflow\n1. Confirm the exact goal, constraints, and stop condition.\n2. Prefer the minimum coherent code change over broad refactors.\n3. Verify with the narrowest useful test loop before expanding scope.\n4. Return what changed, what was verified, and what remains open.\n\n## Constraints\n- Do not widen scope unless the current path is blocked.\n- Preserve existing repository conventions.\n- Make risks and assumptions explicit."
}

fn default_long_term_memory() -> &'static str {
    "# Long-Term Memory\n\nFallback durable notes used when Mem0 is not configured.\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventId;
    use crate::Tools::default_tools;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{label}-{}", now_epoch_ms()));
        fs::create_dir_all(&path).expect("temp dir should exist");
        path
    }

    fn test_agent(root_dir: PathBuf) -> MainAgent {
        let memory_paths = agent_memory_paths(&root_dir);
        let mut subagents = BTreeMap::new();
        subagents.insert(
            "general-purpose".to_string(),
            SubagentSpec {
                name: "general-purpose".to_string(),
                description: "General test agent".to_string(),
                prompt: "Test prompt".to_string(),
                tool_allowlist: None,
                scope: "project".to_string(),
                source_path: root_dir.join(".claude/agents/general-purpose.md"),
            },
        );

        MainAgent {
            root_dir,
            home_dir: None,
            memory_paths,
            mem0: None,
            config: AgentConfig {
                system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
                default_model: "OpenAI GPT-5.4",
                fallback_model: FALLBACK_MODEL,
                planner_backend: "local heuristic planner".to_string(),
                max_retries: MAX_RETRIES,
                allow_heuristic_planner: true,
            },
            llm_engine: None,
            fallback_engine: None,
            tools: default_tools(),
            mcp_servers: Vec::new(),
            observability: Observability::from_env(),
            memory_stack: MemoryStack::default(),
            short_term_memory: ShortTermMemorySnapshot::default(),
            subagents,
            skills: BTreeMap::new(),
            skill_validation_issues: Vec::new(),
            commands: BTreeMap::new(),
        }
    }

    fn planner_test_agent(
        root_dir: PathBuf,
        llm_engine: Option<OpenAiEngine>,
        fallback_engine: Option<AnthropicEngine>,
        allow_heuristic_planner: bool,
    ) -> MainAgent {
        let mut agent = test_agent(root_dir);
        agent.llm_engine = llm_engine;
        agent.fallback_engine = fallback_engine;
        agent.config.planner_backend = match (&agent.llm_engine, &agent.fallback_engine) {
            (Some(primary), Some(fallback)) => format!(
                "OpenAI Responses planner ({}) -> Anthropic planner ({})",
                primary.model, fallback.model
            ),
            (Some(primary), None) => format!("OpenAI Responses planner ({})", primary.model),
            (None, Some(fallback)) => format!("Anthropic planner ({})", fallback.model),
            (None, None) => "local heuristic planner".to_string(),
        };
        agent.config.allow_heuristic_planner = allow_heuristic_planner;
        agent
    }

    fn openai_test_response(payload: &str) -> String {
        format!(
            r#"{{
                "id": "resp_test",
                "status": "completed",
                "output": [
                    {{
                        "type": "message",
                        "content": [
                            {{
                                "type": "output_text",
                                "text": {payload:?}
                            }}
                        ]
                    }}
                ],
                "usage": {{
                    "input_tokens": 11,
                    "output_tokens": 7,
                    "total_tokens": 18
                }}
            }}"#
        )
    }

    fn anthropic_test_response(payload: &str) -> String {
        format!(
            r#"{{
                "content": [
                    {{
                        "type": "text",
                        "text": {payload:?}
                    }}
                ],
                "usage": {{
                    "input_tokens": 13,
                    "output_tokens": 5
                }}
            }}"#
        )
    }

    fn valid_skill_markdown(name: &str, description: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\nversion: 0.1.0\ntags: [workflow]\nwhen_to_use: use this skill\nwhen_not_to_use: use another workflow\nallowed_tools: []\npreferred_subagents: []\ninput_contract: task contract\noutput_contract: concise output\n---\n\n# Skill\n\n## Purpose\n{description}\n\n## Use When\n- Use it intentionally.\n\n## Do Not Use When\n- Another skill is a better fit.\n\n## Workflow\n1. Read the task contract.\n2. Execute the bounded workflow.\n\n## Constraints\n- Keep the result concise.\n\n## Expected Outputs\n- A concrete result.\n\n## Examples\n- Example usage.\n"
        )
    }

    #[test]
    fn split_frontmatter_extracts_fields_and_body() {
        let (frontmatter, body) =
            split_frontmatter("---\nname: plan\ndescription: Planner\n---\n\n# Body\nPrompt");

        assert_eq!(frontmatter.get("name"), Some(&"plan".to_string()));
        assert_eq!(frontmatter.get("description"), Some(&"Planner".to_string()));
        assert!(body.contains("# Body"));
    }

    #[test]
    fn agent_memory_paths_use_workspace_directory() {
        let root = temp_root("memory-paths");
        let paths = agent_memory_paths(&root);

        assert_eq!(
            paths.short_term_snapshot,
            root.join("Workspace/short-term.json")
        );
        assert_eq!(paths.long_term_notes, root.join("Workspace/MEMORY.md"));
    }

    #[test]
    fn scaffold_claude_project_creates_system_prompt_file() {
        let root = temp_root("scaffold-system-prompt");

        let created = scaffold_claude_project(&root).expect("project scaffold should succeed");

        assert!(
            created.contains(&root.join("system_prompt/system_prompt.md")),
            "expected scaffold to create a system prompt file"
        );
        assert_eq!(
            fs::read_to_string(root.join("system_prompt/system_prompt.md"))
                .expect("system prompt file should be readable"),
            default_system_prompt_file()
        );
    }

    #[test]
    fn load_memory_stack_follows_imports() {
        let root = temp_root("memory-stack");
        let imported = root.join("docs.md");
        fs::write(&imported, "# Extra\nImported memory").expect("imported file");
        fs::write(
            root.join("CLAUDE.md"),
            format!("# Root\n@{}\n", imported.display()),
        )
        .expect("root memory");

        let stack = load_memory_stack(&root, None, None).expect("memory stack should load");

        assert_eq!(stack.sources.len(), 2);
        assert!(stack.merged_instructions.contains("Imported memory"));
    }

    #[test]
    fn load_memory_stack_records_selector_hint_for_imported_project_memory() {
        let root = temp_root("memory-selector-hint");
        let imported_dir = root.join("docs/agents");
        fs::create_dir_all(&imported_dir).expect("import dir");
        let imported = imported_dir.join("memory.md");
        fs::write(&imported, "# Scoped\nPath scoped memory").expect("imported file");
        fs::write(
            root.join("CLAUDE.md"),
            format!("# Root\n@{}\n", imported.display()),
        )
        .expect("root memory");

        let stack = load_memory_stack(&root, None, None).expect("memory stack should load");
        let imported_source = stack
            .sources
            .iter()
            .find(|source| source.path.ends_with("docs/agents/memory.md"))
            .expect("imported source should exist");

        assert_eq!(
            imported_source.selector_hint.as_deref(),
            Some(Path::new("docs/agents"))
        );
    }

    #[test]
    fn memory_load_request_normalizes_focus_paths_under_root() {
        let root = temp_root("memory-load-request");
        let request = MemoryLoadRequest {
            focus_paths: vec![
                root.join("src/main.rs"),
                PathBuf::from("docs/architecture.md"),
                PathBuf::from("../outside.md"),
            ],
        };

        let normalized = request.normalized_focus_paths(&root);

        assert_eq!(
            normalized,
            vec![
                PathBuf::from("src/main.rs"),
                PathBuf::from("docs/architecture.md")
            ]
        );
    }

    #[test]
    fn load_memory_stack_includes_long_term_memory_file() {
        let root = temp_root("memory-long-term");
        fs::write(root.join("CLAUDE.md"), "# Root\nBase memory").expect("root memory");
        let long_term_path = agent_memory_paths(&root).long_term_notes;
        append_long_term_memory(
            None,
            &long_term_path,
            "Prefer explicit approvals for risky actions.",
        )
        .expect("long-term memory should append");

        let stack = load_memory_stack(&root, None, None).expect("memory stack should load");

        assert!(stack
            .merged_instructions
            .contains("Prefer explicit approvals for risky actions."));
    }

    #[test]
    fn short_term_memory_round_trips_recent_state() {
        let root = temp_root("short-term-memory");
        let snapshot_path = agent_memory_paths(&root).short_term_snapshot;
        let mut state = SessionState::default();
        state.compacted_summary = Some("Earlier context".to_string());
        state.observations.push("Tool observation".to_string());
        state
            .history
            .push(ConversationMessage::user("First prompt"));
        state
            .history
            .push(ConversationMessage::assistant("First reply"));

        save_short_term_memory(&snapshot_path, &state).expect("snapshot should save");
        let restored = load_short_term_memory(&snapshot_path).expect("snapshot should load");

        assert_eq!(
            restored.compacted_summary,
            Some("Earlier context".to_string())
        );
        assert_eq!(restored.observations, vec!["Tool observation".to_string()]);
        assert_eq!(restored.recent_history.len(), 2);
    }

    #[test]
    fn automatic_compaction_summarizes_older_history() {
        let root = temp_root("auto-compact");
        let agent = test_agent(root);
        let mut state = SessionState::default();
        for index in 0..10 {
            if index % 2 == 0 {
                state
                    .history
                    .push(ConversationMessage::user(format!("user turn {index}")));
            } else {
                state.history.push(ConversationMessage::assistant(format!(
                    "assistant turn {index}"
                )));
            }
        }

        let result = agent.run_with_state(&mut state, "Inspect the current task");

        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Auto-compacted session context")));
        assert_eq!(state.history.len(), 6);
        let summary = state
            .compacted_summary
            .as_deref()
            .expect("summary should be present");
        assert!(summary.contains("user turn 0"));
        assert!(summary.contains("assistant turn 5"));
        assert!(!summary.contains("assistant turn 9"));
    }

    #[test]
    fn explicit_compaction_updates_short_term_snapshot() {
        let root = temp_root("explicit-compact");
        let snapshot_path = agent_memory_paths(&root).short_term_snapshot;
        let mut state = SessionState::default();
        for index in 0..10 {
            state
                .history
                .push(ConversationMessage::user(format!("history item {index}")));
        }
        save_short_term_memory(&snapshot_path, &state).expect("snapshot should save");

        let mut agent = test_agent(root);
        let result = agent.compact_context().expect("compaction should succeed");
        let restored = load_short_term_memory(&snapshot_path).expect("snapshot should load");

        assert!(result.compacted);
        assert_eq!(restored.recent_history.len(), RETAINED_RECENT_HISTORY_ITEMS);
        assert!(restored
            .compacted_summary
            .as_deref()
            .expect("summary should be present")
            .contains("history item 2"));
    }

    #[test]
    fn load_memory_stack_uses_mem0_when_configured() {
        let root = temp_root("mem0-load");
        fs::write(root.join("CLAUDE.md"), "# Root\nBase memory").expect("root memory");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_http_test_server(
            requests.clone(),
            vec![(
                "200 OK",
                r#"[{"id":"mem_1","memory":"Use Mem0-backed durable memory.","created_at":"2026-04-20T00:00:00Z","updated_at":"2026-04-20T00:00:00Z"}]"#
                    .to_string(),
            )],
        );
        let mem0 = test_mem0_config(&base_url, &root);

        let stack = load_memory_stack(&root, None, Some(&mem0)).expect("memory stack should load");

        handle.join().expect("server thread should finish");
        assert!(stack
            .merged_instructions
            .contains("Use Mem0-backed durable memory."));
        assert!(stack
            .sources
            .iter()
            .any(|source| source.path.to_string_lossy().starts_with("mem0:")));
        let request_log = requests.lock().expect("request log should lock");
        assert!(request_log
            .iter()
            .any(|request| request.contains("POST /v2/memories")));
    }

    #[test]
    fn append_long_term_memory_uses_mem0_when_configured() {
        let root = temp_root("mem0-append");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_http_test_server(
            requests.clone(),
            vec![(
                "200 OK",
                r#"{"results":[{"id":"mem_1","event":"ADD"}]}"#.to_string(),
            )],
        );
        let mem0 = test_mem0_config(&base_url, &root);
        let long_term_path = agent_memory_paths(&root).long_term_notes;

        append_long_term_memory(Some(&mem0), &long_term_path, "Persist via Mem0.")
            .expect("mem0 append should succeed");

        handle.join().expect("server thread should finish");
        assert!(!long_term_path.exists());
        let request_log = requests.lock().expect("request log should lock");
        assert!(request_log
            .iter()
            .any(|request| request.contains("POST /v1/memories")));
        assert!(request_log
            .iter()
            .any(|request| request.contains("Persist via Mem0.")));
    }

    #[test]
    fn planner_eval_preview_routes_web_queries_to_tool() {
        let root = temp_root("planner-eval-tool");
        let agent = test_agent(root);

        let decision = agent.preview_planner_decision(
            r#"use tool web_search with {"query":"latest Rust release notes"}"#,
            &[],
        );

        assert_eq!(decision.action, "tool");
        assert_eq!(decision.tool_name.as_deref(), Some("web_search_tool"));
        assert_eq!(
            decision.tool_arguments_json,
            Some(serde_json::json!({
                "query": "latest Rust release notes"
            }))
        );
        assert_eq!(decision.planner_backend, "local heuristic planner");
        assert!(decision
            .reasoning
            .iter()
            .any(|item| item.contains("web_search_tool")));
    }

    #[test]
    fn planner_eval_preview_uses_local_heuristic_path_even_when_model_planners_are_configured() {
        let root = temp_root("planner-eval-configured");
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            Some(AnthropicEngine {
                api_key: "anthropic-test-key".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                model: FALLBACK_MODEL.to_string(),
                version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            }),
            false,
        );

        let decision = agent.preview_planner_decision(
            r#"use tool web_search with {"query":"latest Rust release notes"}"#,
            &[],
        );

        assert_eq!(decision.action, "tool");
        assert_eq!(decision.tool_name.as_deref(), Some("web_search_tool"));
        assert!(decision
            .planner_backend
            .starts_with("local heuristic eval preview"));
    }

    #[test]
    fn planner_eval_preview_prefers_explicit_subagent_request_over_skill_inference() {
        let agent = MainAgent::from_env(DEFAULT_SYSTEM_PROMPT.to_string())
            .expect("agent should load for explicit subagent preview");

        let decision = agent.preview_planner_decision(
            "use agent plan to outline the implementation for a new eval runner",
            &[],
        );

        assert_eq!(decision.action, "delegate");
        assert_eq!(decision.subagent_name.as_deref(), Some("plan"));
        assert!(decision
            .reason
            .contains("Explicit subagent request detected"));
    }

    #[test]
    fn planner_eval_preview_finishes_from_latest_observation() {
        let root = temp_root("planner-eval-finish");
        let agent = test_agent(root);

        let decision =
            agent.preview_planner_decision("summarize the current state", &["Done.".to_string()]);

        assert_eq!(decision.action, "finish");
        assert_eq!(
            decision.reason,
            "the latest observation is now the best available answer"
        );
    }

    #[test]
    fn planner_eval_preview_stops_on_empty_input() {
        let root = temp_root("planner-eval-stop");
        let agent = test_agent(root);

        let decision = agent.preview_planner_decision("   ", &[]);

        assert_eq!(decision.action, "stop");
        assert_eq!(decision.reason, "user input was empty");
    }

    #[test]
    fn planner_eval_preview_retries_after_recoverable_observation() {
        let root = temp_root("planner-eval-retry");
        let agent = test_agent(root);

        let decision = agent.preview_planner_decision(
            "retry the search",
            &["Recoverable tool failure from `web_search_tool`: timeout".to_string()],
        );

        assert_eq!(decision.action, "retry");
        assert_eq!(
            decision.reason,
            "the latest observation still reflects a recoverable failure"
        );
        assert_eq!(
            decision.failure_class.as_deref(),
            Some(PlannerFailureKind::Recoverable.label())
        );
    }

    #[test]
    fn planner_failure_classifier_distinguishes_transient_and_terminal_errors() {
        assert_eq!(
            classify_planner_failure_kind("HTTP 503 Service Unavailable"),
            PlannerFailureKind::Recoverable
        );
        assert_eq!(
            classify_planner_failure_kind("Planner JSON parse failed: missing action"),
            PlannerFailureKind::Terminal
        );

        let retry_run: PlannerRun = planner_failure_run(
            "OpenAI Responses API",
            "HTTP 503 Service Unavailable".to_string(),
            PlannerFailureKind::Recoverable,
            None,
        );
        assert!(matches!(retry_run.decision, Decision::Retry(_)));
        assert_eq!(retry_run.failure_kind, Some(PlannerFailureKind::Recoverable));

        let stop_run: PlannerRun = planner_failure_run(
            "OpenAI Responses API",
            "Planner JSON parse failed: missing action".to_string(),
            PlannerFailureKind::Terminal,
            None,
        );
        assert!(matches!(stop_run.decision, Decision::Stop(_)));
        assert_eq!(stop_run.failure_kind, Some(PlannerFailureKind::Terminal));
    }

    #[test]
    fn decide_next_step_uses_primary_planner_when_openai_succeeds() {
        let root = temp_root("planner-primary-success");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let planner_payload =
            r#"{"action":"finish","tool_name":"","tool_arguments_json":"{}","skill_name":"","subagent_name":"","answer":"primary answer","reason":"primary planner answered directly"}"#;
        let (base_url, handle) = spawn_http_test_server(
            requests.clone(),
            vec![("200 OK", openai_test_response(planner_payload))],
        );
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url,
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            None,
            false,
        );
        let state = SessionState::default();
        let task_contract = agent.build_task_contract("Answer directly", &[]);

        let run = agent.decide_next_step(&state, "Answer directly", &task_contract, &[], 0);

        handle.join().expect("server thread should finish");
        assert!(matches!(
            run.decision,
            Decision::Finish {
                answer,
                reason
            } if answer == "primary answer" && reason == "primary planner answered directly"
        ));
        assert!(run.failure_kind.is_none());
        let usage = run.usage.expect("primary planner should report usage");
        assert_eq!(usage.execution, "OpenAI Responses API (gpt-5.4)");
        let request_log = requests.lock().expect("request log should lock");
        assert_eq!(request_log.len(), 1);
        assert!(request_log[0].contains("POST / HTTP/1.1"));
        assert!(request_log[0].contains("openai-test-key"));
    }

    #[test]
    fn decide_next_step_falls_back_to_anthropic_after_primary_failure() {
        let root = temp_root("planner-fallback-success");
        let openai_requests = Arc::new(Mutex::new(Vec::new()));
        let anthropic_requests = Arc::new(Mutex::new(Vec::new()));
        let fallback_payload =
            r#"{"action":"finish","tool_name":"","tool_arguments_json":"{}","skill_name":"","subagent_name":"","answer":"fallback answer","reason":"fallback planner answered directly"}"#;
        let (openai_base_url, openai_handle) = spawn_http_test_server(
            openai_requests.clone(),
            vec![("503 Service Unavailable", r#"{"error":"planner unavailable"}"#.to_string())],
        );
        let (anthropic_base_url, anthropic_handle) = spawn_http_test_server(
            anthropic_requests.clone(),
            vec![("200 OK", anthropic_test_response(fallback_payload))],
        );
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url: openai_base_url,
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            Some(AnthropicEngine {
                api_key: "anthropic-test-key".to_string(),
                base_url: anthropic_base_url,
                model: FALLBACK_MODEL.to_string(),
                version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            }),
            false,
        );
        let state = SessionState::default();
        let task_contract = agent.build_task_contract("Answer with fallback", &[]);

        let run = agent.decide_next_step(&state, "Answer with fallback", &task_contract, &[], 0);

        openai_handle.join().expect("openai server thread should finish");
        anthropic_handle
            .join()
            .expect("anthropic server thread should finish");
        assert!(matches!(
            run.decision,
            Decision::Finish {
                answer,
                reason
            } if answer == "fallback answer" && reason == "fallback planner answered directly"
        ));
        assert!(run.failure_kind.is_none());
        assert!(run
            .reasoning
            .iter()
            .any(|entry| entry.contains("Primary planner failed and the harness fell back")));
        let usage = run.usage.expect("fallback planner should report usage");
        assert_eq!(usage.execution, "Anthropic Messages API (Opus 4.6)");
        let openai_request_log = openai_requests.lock().expect("openai request log should lock");
        let anthropic_request_log = anthropic_requests
            .lock()
            .expect("anthropic request log should lock");
        assert_eq!(openai_request_log.len(), 1);
        assert_eq!(anthropic_request_log.len(), 1);
        assert!(anthropic_request_log[0].contains("x-api-key: anthropic-test-key"));
    }

    #[test]
    fn decide_next_step_classifies_dual_planner_failure_without_heuristic_fallback() {
        let root = temp_root("planner-dual-failure");
        let openai_requests = Arc::new(Mutex::new(Vec::new()));
        let anthropic_requests = Arc::new(Mutex::new(Vec::new()));
        let (openai_base_url, openai_handle) = spawn_http_test_server(
            openai_requests.clone(),
            vec![("503 Service Unavailable", r#"{"error":"planner unavailable"}"#.to_string())],
        );
        let (anthropic_base_url, anthropic_handle) = spawn_http_test_server(
            anthropic_requests.clone(),
            vec![("500 Internal Server Error", r#"{"error":"fallback unavailable"}"#.to_string())],
        );
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url: openai_base_url,
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            Some(AnthropicEngine {
                api_key: "anthropic-test-key".to_string(),
                base_url: anthropic_base_url,
                model: FALLBACK_MODEL.to_string(),
                version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            }),
            false,
        );
        let state = SessionState::default();
        let task_contract = agent.build_task_contract("Handle planner failure", &[]);

        let run = agent.decide_next_step(&state, "Handle planner failure", &task_contract, &[], 0);

        openai_handle.join().expect("openai server thread should finish");
        anthropic_handle
            .join()
            .expect("anthropic server thread should finish");
        assert_eq!(run.failure_kind, Some(PlannerFailureKind::Recoverable));
        assert!(matches!(run.decision, Decision::Retry(_)));
        assert!(run
            .reasoning
            .iter()
            .any(|entry| entry.contains("fallback planner failed")));
        let openai_request_log = openai_requests.lock().expect("openai request log should lock");
        let anthropic_request_log = anthropic_requests
            .lock()
            .expect("anthropic request log should lock");
        assert_eq!(openai_request_log.len(), 1);
        assert_eq!(anthropic_request_log.len(), 1);
    }

    #[test]
    fn run_trace_records_primary_planner_backend_and_decision() {
        let root = temp_root("planner-trace-primary-success");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let planner_payload =
            r#"{"action":"finish","tool_name":"","tool_arguments_json":"{}","skill_name":"","subagent_name":"","answer":"primary answer","reason":"primary planner answered directly"}"#;
        let (base_url, handle) = spawn_http_test_server(
            requests,
            vec![("200 OK", openai_test_response(planner_payload))],
        );
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url,
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            None,
            false,
        );
        let mut state = SessionState::default();

        let result = agent.run_with_state(&mut state, "Answer directly");

        handle.join().expect("server thread should finish");
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner attempt = primary backend `OpenAI GPT-5.4`; outcome = success; failure class = none."
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner fallback outcome = not available; no configured fallback planner."
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("planner decision = finish (primary planner answered directly; primary answer)")));
    }

    #[test]
    fn run_trace_records_fallback_success_after_primary_failure() {
        let root = temp_root("planner-trace-fallback-success");
        let openai_requests = Arc::new(Mutex::new(Vec::new()));
        let anthropic_requests = Arc::new(Mutex::new(Vec::new()));
        let fallback_payload =
            r#"{"action":"finish","tool_name":"","tool_arguments_json":"{}","skill_name":"","subagent_name":"","answer":"fallback answer","reason":"fallback planner answered directly"}"#;
        let (openai_base_url, openai_handle) = spawn_http_test_server(
            openai_requests,
            vec![("503 Service Unavailable", r#"{"error":"planner unavailable"}"#.to_string())],
        );
        let (anthropic_base_url, anthropic_handle) = spawn_http_test_server(
            anthropic_requests,
            vec![("200 OK", anthropic_test_response(fallback_payload))],
        );
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url: openai_base_url,
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            Some(AnthropicEngine {
                api_key: "anthropic-test-key".to_string(),
                base_url: anthropic_base_url,
                model: FALLBACK_MODEL.to_string(),
                version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            }),
            false,
        );
        let mut state = SessionState::default();

        let result = agent.run_with_state(&mut state, "Answer with fallback");

        openai_handle.join().expect("openai server thread should finish");
        anthropic_handle
            .join()
            .expect("anthropic server thread should finish");
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner attempt = primary backend `OpenAI GPT-5.4`; outcome = failure; failure class = recoverable;"
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner attempt = fallback backend `Opus 4.6`; outcome = success; failure class = none."
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner fallback outcome = configured fallback planner succeeded after primary failure."
            )));
    }

    #[test]
    fn run_trace_records_fallback_failure_and_overall_failure_class() {
        let root = temp_root("planner-trace-dual-failure");
        let openai_requests = Arc::new(Mutex::new(Vec::new()));
        let anthropic_requests = Arc::new(Mutex::new(Vec::new()));
        let (openai_base_url, openai_handle) = spawn_http_test_server(
            openai_requests,
            vec![("503 Service Unavailable", r#"{"error":"planner unavailable"}"#.to_string())],
        );
        let (anthropic_base_url, anthropic_handle) = spawn_http_test_server(
            anthropic_requests,
            vec![("500 Internal Server Error", r#"{"error":"fallback unavailable"}"#.to_string())],
        );
        let agent = planner_test_agent(
            root,
            Some(OpenAiEngine {
                api_key: "openai-test-key".to_string(),
                base_url: openai_base_url,
                model: DEFAULT_OPENAI_MODEL.to_string(),
            }),
            Some(AnthropicEngine {
                api_key: "anthropic-test-key".to_string(),
                base_url: anthropic_base_url,
                model: FALLBACK_MODEL.to_string(),
                version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            }),
            false,
        );
        let mut state = SessionState::default();

        let result = agent.run_with_state(&mut state, "Handle planner failure");

        openai_handle.join().expect("openai server thread should finish");
        anthropic_handle
            .join()
            .expect("anthropic server thread should finish");
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner attempt = primary backend `OpenAI GPT-5.4`; outcome = failure; failure class = recoverable;"
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner attempt = fallback backend `Opus 4.6`; outcome = failure; failure class = terminal;"
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "planner fallback outcome = configured fallback planner failed after primary failure."
            )));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("planner failure class = recoverable.")));
    }

    fn test_mem0_config(base_url: &str, root_dir: &Path) -> Mem0Config {
        Mem0Config {
            api_key: "mem0-test-key".to_string(),
            base_url: base_url.to_string(),
            user_id: "test-user".to_string(),
            agent_id: "test-agent".to_string(),
            app_id: root_dir.display().to_string(),
            org_id: None,
            project_id: None,
            workspace_label: root_dir.display().to_string(),
        }
    }

    fn spawn_http_test_server(
        requests: Arc<Mutex<Vec<String>>>,
        responses: Vec<(&'static str, String)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener address should resolve");
        let handle = thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("request should arrive");
                let request = read_http_request(&mut stream);
                requests
                    .lock()
                    .expect("request log should lock")
                    .push(request);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("response should write");
                stream.flush().expect("response should flush");
            }
        });

        (format!("http://{}", address), handle)
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout should set");
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut target_size = None;

        loop {
            let read = stream.read(&mut chunk).expect("request should read");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if target_size.is_none() {
                target_size = expected_http_request_size(&buffer);
            }
            if let Some(target_size) = target_size {
                if buffer.len() >= target_size {
                    break;
                }
            }
        }

        String::from_utf8(buffer).expect("request should be utf8")
    }

    fn expected_http_request_size(buffer: &[u8]) -> Option<usize> {
        let marker = b"\r\n\r\n";
        let header_end = buffer
            .windows(marker.len())
            .position(|window| window == marker)?;
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        Some(header_end + marker.len() + content_length)
    }

    #[test]
    fn project_subagent_overrides_user_subagent() {
        let root = temp_root("subagent-project");
        let home = temp_root("subagent-home");
        let user_dir = home.join(".claude").join("agents");
        let project_dir = root.join(".claude").join("agents");
        fs::create_dir_all(&user_dir).expect("user dir");
        fs::create_dir_all(&project_dir).expect("project dir");
        fs::write(
            user_dir.join("plan.md"),
            "---\nname: plan\ndescription: User planner\n---\nUser",
        )
        .expect("user agent");
        fs::write(
            project_dir.join("plan.md"),
            "---\nname: plan\ndescription: Project planner\n---\nProject",
        )
        .expect("project agent");

        let subagents = load_subagent_specs(&root, Some(&home)).expect("subagents should load");
        let plan = subagents.get("plan").expect("plan agent should exist");

        assert_eq!(plan.description, "Project planner");
        assert_eq!(plan.scope, "project");
    }

    #[test]
    fn spawned_subagent_request_uses_loaded_subagent_spec() {
        let root = temp_root("spawned-subagent");
        let agent_dir = root.join(".claude").join("agents");
        fs::create_dir_all(&agent_dir).expect("agent dir");
        fs::write(
            agent_dir.join("plan.md"),
            "---\nname: plan\ndescription: Project planner\n---\nPlan body",
        )
        .expect("project agent");

        let request = DispatchRequest::for_subagent(
            "main-worker",
            "plan",
            "Produce a plan",
            ContextPacket {
                goal: "Produce a plan".to_string(),
                constraints: vec!["Keep it small".to_string()],
                relevant_files: vec!["src/mainAgent.rs".to_string()],
                known_facts: vec!["Main agent exists".to_string()],
                missing_facts: vec!["Need a child execution path".to_string()],
                next_action: "Return the delegated result".to_string(),
                stop_condition: "Stop after the delegated result".to_string(),
            },
            vec!["Workspace/MEMORY.md".to_string()],
            vec!["src/mainAgent.rs".to_string()],
            vec!["user: inspect child spawn".to_string()],
        );

        let response = execute_spawned_subagent_request_in_root(&root, request)
            .expect("spawned subagent should execute");

        assert_eq!(response.responder, "subagent plan");
        assert!(response.summary.contains("compact execution plan"));
        assert!(response.final_text.contains("Plan agent"));
        assert!(response.final_text.contains("Task: Produce a plan"));
        assert!(response.dispatch_id.as_str().starts_with("evt-"));
    }

    #[test]
    fn spawned_subagent_request_rejects_unknown_subagent() {
        let root = temp_root("spawned-subagent-unknown");
        let agent_dir = root.join(".claude").join("agents");
        fs::create_dir_all(&agent_dir).expect("agent dir");
        fs::write(
            agent_dir.join("plan.md"),
            "---\nname: plan\ndescription: Project planner\n---\nPlan body",
        )
        .expect("project agent");

        let request = DispatchRequest::for_subagent(
            "main-worker",
            "missing",
            "Produce a plan",
            ContextPacket {
                goal: "Produce a plan".to_string(),
                constraints: vec!["Keep it small".to_string()],
                relevant_files: vec!["src/mainAgent.rs".to_string()],
                known_facts: vec!["Main agent exists".to_string()],
                missing_facts: vec!["Need a child execution path".to_string()],
                next_action: "Return the delegated result".to_string(),
                stop_condition: "Stop after the delegated result".to_string(),
            },
            vec![],
            vec![],
            vec![],
        );

        let error = execute_spawned_subagent_request_in_root(&root, request)
            .expect_err("unknown subagents should be rejected");

        assert_eq!(error, "Unknown subagent `missing`.");
    }

    #[test]
    fn spawned_subagent_request_rejects_worker_targets() {
        let root = temp_root("spawned-subagent-worker");
        let request = DispatchRequest {
            dispatch_id: EventId::new("evt-fixed"),
            requester: "main-worker".to_string(),
            target: crate::dispatch::DispatchTarget::Worker {
                profile: "planner".to_string(),
            },
            task: "Produce a plan".to_string(),
            context_packet: ContextPacket {
                goal: "Produce a plan".to_string(),
                constraints: vec!["Keep it small".to_string()],
                relevant_files: vec!["src/mainAgent.rs".to_string()],
                known_facts: vec!["Main agent exists".to_string()],
                missing_facts: vec!["Need a child execution path".to_string()],
                next_action: "Return the delegated result".to_string(),
                stop_condition: "Stop after the delegated result".to_string(),
            },
            memory_refs: vec![],
            file_refs: vec![],
            observations: vec![],
        };

        let error = execute_spawned_subagent_request_in_root(&root, request)
            .expect_err("worker targets should be rejected");

        assert_eq!(
            error,
            "child subagent runtime does not support worker target `planner`"
        );
    }

    #[test]
    fn project_skill_overrides_user_skill() {
        let root = temp_root("skill-project");
        let home = temp_root("skill-home");
        let user_dir = home.join(".claude").join("skills").join("developer");
        let project_dir = root.join(".claude").join("skills").join("developer");
        fs::create_dir_all(&user_dir).expect("user dir");
        fs::create_dir_all(&project_dir).expect("project dir");
        fs::write(
            user_dir.join("SKILL.md"),
            valid_skill_markdown("developer", "User skill"),
        )
        .expect("user skill");
        fs::write(
            project_dir.join("SKILL.md"),
            valid_skill_markdown("developer", "Project skill"),
        )
        .expect("project skill");

        let skills = load_skill_specs(&root, Some(&home), &default_tools(), &BTreeMap::new())
            .expect("skills should load");
        let developer = skills
            .entries
            .get("developer")
            .expect("developer skill should exist");

        assert_eq!(developer.description, "Project skill");
        assert_eq!(developer.scope, "project");
    }

    #[test]
    fn invalid_skill_files_are_reported_and_skipped() {
        let root = temp_root("skill-invalid");
        let skill_dir = root.join(".claude").join("skills").join("broken");
        fs::create_dir_all(&skill_dir).expect("skill dir");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: broken\ndescription: Broken skill\n---\nNo required sections",
        )
        .expect("skill file");

        let loaded = load_skill_specs(&root, None, &default_tools(), &BTreeMap::new())
            .expect("skills should load");

        assert!(loaded.entries.is_empty());
        assert!(!loaded.issues.is_empty());
        assert!(loaded
            .issues
            .iter()
            .any(|issue| issue.message.contains("missing required section")));
    }

    #[test]
    fn create_skill_scaffold_writes_skill_md() {
        let root = temp_root("skill-create");

        let path = create_skill_scaffold(
            &root,
            None,
            DefinitionScope::Project,
            "Developer Review",
            "Review implementation changes.",
        )
        .expect("skill scaffold should be created");

        let contents = fs::read_to_string(&path).expect("skill file should exist");
        assert!(path.ends_with(".claude/skills/developer-review/SKILL.md"));
        assert!(contents.contains("name: developer-review"));
        assert!(contents.contains("Review implementation changes."));
    }

    #[test]
    fn install_skill_from_local_collection_copies_skill_dir() {
        let root = temp_root("skill-install-root");
        let source_root = temp_root("skill-install-source");
        let source_dir = source_root.join("developer");
        fs::create_dir_all(source_dir.join("references")).expect("references dir");
        fs::write(
            source_dir.join("SKILL.md"),
            valid_skill_markdown("developer", "Developer workflow"),
        )
        .expect("skill file");
        fs::write(source_dir.join("references").join("guide.md"), "guide").expect("guide file");

        let installed = install_skill_from_source(
            &root,
            None,
            DefinitionScope::Project,
            &source_root.display().to_string(),
            Some("developer"),
        )
        .expect("skill should install");

        assert!(installed.ends_with(".claude/skills/developer/SKILL.md"));
        assert!(installed.is_file());
        assert!(installed
            .parent()
            .expect("skill dir")
            .join("references/guide.md")
            .is_file());
    }

    #[test]
    fn expand_custom_command_replaces_arguments() {
        let command = CustomCommand {
            name: "review".to_string(),
            description: "Run review".to_string(),
            template: "Review $1 with $ARGUMENTS".to_string(),
            source_path: PathBuf::from("review.md"),
            scope: DefinitionScope::Project,
        };

        let expanded = expand_custom_command(&command, "src/main.rs thoroughly");

        assert_eq!(expanded, "Review src/main.rs with src/main.rs thoroughly");
    }

    #[test]
    fn render_usage_summary_lists_main_and_subagent_requests() {
        let root = temp_root("usage-summary");
        let agent = test_agent(root);

        let result = agent.run("Inspect the task and suggest next steps");
        let summary = result.render_usage_summary();

        assert!(summary.contains("Token usage"));
        assert!(summary.contains("main agent request: 0 input, 0 output, 0 total"));
        assert!(summary.contains(
            "subagent `general-purpose`: 0 input, 0 output, 0 total [local synthesized subagent; no model API call]"
        ));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Token usage: main agent request")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Token usage: subagent `general-purpose`")));
    }

    #[test]
    fn run_trace_records_subagent_correlation_and_result_details() {
        let root = temp_root("subagent-trace-correlation");
        let agent = test_agent(root);

        let result = agent.run("Inspect the task and suggest next steps");

        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("delegating to subagent `general-purpose` because")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("subagent launch mode = `local-fallback`")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("subagent dispatch_id = `evt-")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("subagent result summary = general-purpose returned a compact handoff.")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains(
                "subagent recommended next action = Refine the task, delegate to `/agent plan ...`, or call an explicit tool if needed."
            )));
    }

    #[test]
    fn empty_input_reports_noop_usage() {
        let root = temp_root("usage-empty");
        let agent = test_agent(root);

        let result = agent.run("   ");
        let summary = result.render_usage_summary();

        assert!(result.stopped);
        assert!(summary.contains("main agent request: 0 input, 0 output, 0 total"));
        assert!(summary.contains("empty input; no model API call"));
    }

    #[test]
    fn context_history_appends_entries_to_history_file() {
        let root = temp_root("context-history");
        let path = context_history_path(&root);
        let mut state = SessionState::default();
        state
            .history
            .push(ConversationMessage::user("previous question"));

        let first = capture_context_history_entry(
            "OpenAI Responses API",
            "gpt-5.4",
            "system prompt",
            "planner prompt",
            &state,
            "current question",
            &json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "current question"}]
            }),
        );
        let second = capture_context_history_entry(
            "Anthropic Messages API",
            "Opus 4.6",
            "fallback system prompt",
            "fallback planner prompt",
            &state,
            "retry question",
            &json!({
                "model": "Opus 4.6",
                "messages": [{"role": "user", "content": "retry question"}]
            }),
        );

        persist_context_history(&path, &first).expect("first context entry should persist");
        persist_context_history(&path, &second).expect("second context entry should persist");

        let contents = fs::read_to_string(&path).expect("history file should exist");
        let entries: Vec<ContextHistoryEntry> =
            serde_json::from_str(&contents).expect("history file should parse");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].provider, "OpenAI Responses API");
        assert_eq!(
            entries[0].actual_context.current_user_input,
            "current question"
        );
        assert_eq!(entries[1].model, "Opus 4.6");
        assert_eq!(
            entries[1].final_context.request_body["messages"][0]["content"],
            "retry question"
        );
        assert!(entries[0].datetime.contains('T'));
        assert!(entries[0].datetime.ends_with('Z'));
    }

    #[test]
    fn rendered_context_preview_shows_actual_and_final_context_sections() {
        let root = temp_root("context-preview");
        let path = context_history_path(&root);
        let mut state = SessionState::default();
        state
            .history
            .push(ConversationMessage::assistant("previous answer"));

        let entry = capture_context_history_entry(
            "OpenAI Responses API",
            "gpt-5.4",
            "system prompt",
            "planner prompt",
            &state,
            "current prompt",
            &json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "current prompt"}]
            }),
        );

        let preview = render_context_preview(&entry, &path);

        assert!(preview.contains("LLM context preview"));
        assert!(preview.contains("Actual context:"));
        assert!(preview.contains("Final request JSON:"));
        assert!(preview.contains("system prompt"));
        assert!(preview.contains("planner prompt"));
        assert!(preview.contains("current prompt"));
        assert!(preview.contains(&path.display().to_string()));
    }

    #[test]
    fn infer_tool_request_routes_web_queries_to_web_search_tool() {
        let inferred =
            infer_tool_request("Find the latest OpenAI API docs updates", &default_tools())
                .expect("web search should be inferred");

        assert_eq!(inferred.0, "web_search_tool");
        assert_eq!(inferred.1, Value::Object(Default::default()));
        assert!(inferred.2.contains("Planner inferred `web_search_tool`"));
    }

    #[test]
    fn infer_tool_request_normalizes_legacy_web_search_name() {
        let inferred = infer_tool_request(
            r#"Use tool web_search with {"query":"latest Rust release notes"}"#,
            &default_tools(),
        )
        .expect("legacy tool request should be inferred");

        assert_eq!(inferred.0, "web_search_tool");
        assert_eq!(inferred.1["query"], "latest Rust release notes");
        assert!(inferred.2.contains("normalized to `web_search_tool`"));
    }

    #[test]
    fn parse_tools_field_normalizes_legacy_web_search_name() {
        let tools = parse_tools_field("[web_search, other_tool]").expect("tools should parse");

        assert_eq!(
            tools,
            vec!["web_search_tool".to_string(), "other_tool".to_string()]
        );
    }

    #[test]
    fn infer_skill_request_routes_explicit_skill_requests() {
        let mut skills = BTreeMap::new();
        skills.insert(
            "developer".to_string(),
            SkillSpec {
                name: "developer".to_string(),
                description: "Developer workflow".to_string(),
                instructions: "Do the work".to_string(),
                version: None,
                tags: Vec::new(),
                when_to_use: None,
                when_not_to_use: None,
                allowed_tools: None,
                preferred_subagents: Vec::new(),
                input_contract: None,
                output_contract: None,
                scope: "project".to_string(),
                source_path: PathBuf::from(".claude/skills/developer/SKILL.md"),
            },
        );

        let inferred = infer_skill_request("Use skill developer to implement the task", &skills)
            .expect("skill request should be inferred");

        assert_eq!(inferred.0, "developer");
        assert!(inferred.1.contains("Explicit skill request"));
    }

    #[test]
    fn planner_prompt_expands_only_the_active_skill_instructions() {
        let mut skills = BTreeMap::new();
        skills.insert(
            "developer".to_string(),
            SkillSpec {
                name: "developer".to_string(),
                description: "Developer workflow".to_string(),
                instructions: "## Purpose\nDo the work".to_string(),
                version: Some("0.1.0".to_string()),
                tags: vec!["workflow".to_string()],
                when_to_use: Some("when implementing".to_string()),
                when_not_to_use: Some("when browsing".to_string()),
                allowed_tools: Some(vec!["web_search_tool".to_string()]),
                preferred_subagents: vec!["general-purpose".to_string()],
                input_contract: Some("task contract".to_string()),
                output_contract: Some("concise result".to_string()),
                scope: "project".to_string(),
                source_path: PathBuf::from(".claude/skills/developer/SKILL.md"),
            },
        );
        let mut subagents = BTreeMap::new();
        subagents.insert(
            "general-purpose".to_string(),
            SubagentSpec {
                name: "general-purpose".to_string(),
                description: "General".to_string(),
                prompt: "Prompt".to_string(),
                tool_allowlist: None,
                scope: "project".to_string(),
                source_path: PathBuf::from(".claude/agents/general-purpose.md"),
            },
        );
        let mut state = SessionState::default();
        state.active_skill = Some(ActiveSkill {
            name: "developer".to_string(),
            source_path: PathBuf::from(".claude/skills/developer/SKILL.md"),
            activated_by: "planner".to_string(),
            task_fit: "implement the task".to_string(),
            allowed_tools: Some(vec!["web_search_tool".to_string()]),
            preferred_subagents: vec!["general-purpose".to_string()],
        });

        let prompt = planner_prompt(
            &state,
            "",
            &default_tools(),
            &subagents,
            &skills,
            state.active_skill.as_ref(),
            &TaskContract {
                goal: "Resolve task".to_string(),
                constraints: vec!["stay bounded".to_string()],
                acceptance_criteria: vec!["return next action".to_string()],
                relevant_files: vec![],
                stop_condition: "finish when done".to_string(),
            },
            &[],
            0,
        );

        assert!(prompt.contains("Available skills:"));
        assert!(prompt.contains("- developer: Developer workflow"));
        assert!(prompt.contains("Active skill:"));
        assert!(prompt.contains("implement the task"));
        assert!(prompt.contains("## Purpose"));
    }

    #[test]
    fn active_skill_allowed_tools_are_enforced() {
        let root = temp_root("skill-tool-enforcement");
        let agent = test_agent(root);
        let mut state = SessionState::default();
        state.active_skill = Some(ActiveSkill {
            name: "ship-small".to_string(),
            source_path: PathBuf::from(".claude/skills/ship-small/SKILL.md"),
            activated_by: "user".to_string(),
            task_fit: "search the web".to_string(),
            allowed_tools: Some(vec!["nonexistent".to_string()]),
            preferred_subagents: Vec::new(),
        });

        let result = agent.run_with_state(
            &mut state,
            "Search the web for the latest Rust release notes",
        );

        assert!(result.stopped);
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("does not allow tool `web_search_tool`")));
    }

    #[test]
    fn web_queries_use_web_search_tool_path_without_explicit_tool_wrapper() {
        let root = temp_root("web-search-routing");
        let agent = test_agent(root);

        let result = agent.run("Search the web for the latest Rust release notes");

        assert!(result.stopped);
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Planner inferred `web_search_tool`")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Retry limit reached")));
    }
}
