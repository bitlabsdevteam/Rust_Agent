use crate::mcp::{load_mcp_catalog_from_env, McpServerSummary};
use crate::observability::{self, Observability};
use crate::Tools::{default_tools, Tool};
use opentelemetry::KeyValue;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are a harness-first coding agent. Start from an explicit task contract, load project memory before acting, take exactly one action per loop iteration, keep context compact, and make tools, delegation, retries, and stop conditions legible in the trace.";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4";
pub const DEFAULT_PERPLEXITY_MODEL: &str = "sonar-pro";

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1/responses";
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
const FALLBACK_MODEL: &str = "Opus 4.6";
const MAX_RETRIES: u8 = 3;
const MAX_LOOP_STEPS: u8 = 8;
const MAX_IMPORT_DEPTH: usize = 5;
const MAX_TRACE_ENTRIES: usize = 24;
const MAX_COMPACTED_HISTORY_ITEMS: usize = 6;
const MAX_RELEVANT_FILES: usize = 5;
const PROJECT_MEMORY_FILE: &str = "CLAUDE.md";
const CLAUDE_DIR: &str = ".claude";
const AGENTS_DIR: &str = "agents";
const COMMANDS_DIR: &str = "commands";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Success(String),
    Retry(String),
}

impl fmt::Display for StepOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success(output) => write!(f, "success: {output}"),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryStack {
    pub sources: Vec<MemorySource>,
    pub merged_instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct DelegationRequest {
    pub subagent: String,
    pub context_packet: ContextPacket,
    pub memory_refs: Vec<String>,
    pub file_refs: Vec<String>,
    pub observations: Vec<String>,
    pub task: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationResult {
    pub summary: String,
    pub findings: Vec<String>,
    pub artifact_refs: Vec<String>,
    pub recommended_next_action: String,
    pub final_text: String,
    pub usage: Vec<TokenUsageRecord>,
}

impl DelegationResult {
    fn render(&self) -> String {
        let findings = if self.findings.is_empty() {
            "- none".to_string()
        } else {
            self.findings
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let artifacts = if self.artifact_refs.is_empty() {
            "- none".to_string()
        } else {
            self.artifact_refs
                .iter()
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "{}\n\nFindings:\n{}\n\nArtifacts:\n{}\n\nNext action: {}",
            self.final_text, findings, artifacts, self.recommended_next_action
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
struct CustomCommand {
    name: String,
    description: String,
    template: String,
    source_path: PathBuf,
    scope: DefinitionScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    CallTool {
        tool_name: String,
        arguments: Value,
        reason: String,
    },
    DelegateSubagent {
        subagent_name: String,
        reason: String,
    },
    Finish {
        answer: String,
        reason: String,
    },
    Retry(String),
    Stop(String),
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallTool {
                tool_name, reason, ..
            } => write!(f, "call tool `{tool_name}` ({reason})"),
            Self::DelegateSubagent {
                subagent_name,
                reason,
            } => write!(f, "delegate to subagent `{subagent_name}` ({reason})"),
            Self::Finish { answer, reason } => write!(f, "finish ({reason}; {answer})"),
            Self::Retry(reason) => write!(f, "retry ({reason})"),
            Self::Stop(reason) => write!(f, "stop ({reason})"),
        }
    }
}

#[derive(Debug, Clone)]
struct PlannerRun {
    decision: Decision,
    reasoning: Vec<String>,
    usage: Option<TokenUsageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SlashCommand {
    Help,
    Agents,
    Memory,
    Model,
    Clear,
    Compact,
    Mcp,
    Review(String),
    Init,
    Agent { name: String, task: String },
    Exit,
    Custom { name: String, arguments: String },
    Unknown(String),
}

pub struct MainAgent {
    root_dir: PathBuf,
    home_dir: Option<PathBuf>,
    config: AgentConfig,
    llm_engine: Option<OpenAiEngine>,
    fallback_engine: Option<AnthropicEngine>,
    tools: Vec<Tool>,
    mcp_servers: Vec<McpServerSummary>,
    observability: Observability,
    memory_stack: MemoryStack,
    subagents: BTreeMap<String, SubagentSpec>,
    commands: BTreeMap<String, CustomCommand>,
}

impl MainAgent {
    pub fn from_env(system_prompt: String) -> io::Result<Self> {
        let catalog = load_mcp_catalog_from_env()
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
        let mut tools = default_tools();
        tools.extend(catalog.tools.into_iter().map(Tool::from_mcp));
        let llm_engine = OpenAiEngine::from_env();
        let fallback_engine = AnthropicEngine::from_env();

        let planner_backend = match (&llm_engine, &fallback_engine) {
            (Some(primary), Some(fallback)) => format!(
                "OpenAI Responses planner ({}) -> Anthropic planner ({}) -> local heuristic router",
                primary.model, fallback.model
            ),
            (Some(primary), None) => format!(
                "OpenAI Responses planner ({}) -> local heuristic router",
                primary.model
            ),
            (None, Some(fallback)) => format!(
                "Anthropic planner ({}) -> local heuristic router",
                fallback.model
            ),
            (None, None) => "local heuristic planner".to_string(),
        };

        let mut agent = Self {
            root_dir: env::current_dir()?,
            home_dir: env::var("HOME").ok().map(PathBuf::from),
            config: AgentConfig {
                system_prompt,
                default_model: "OpenAI GPT-5.4",
                fallback_model: FALLBACK_MODEL,
                planner_backend,
                max_retries: MAX_RETRIES,
            },
            llm_engine,
            fallback_engine,
            tools,
            mcp_servers: catalog.servers,
            observability: Observability::from_env(),
            memory_stack: MemoryStack::default(),
            subagents: BTreeMap::new(),
            commands: BTreeMap::new(),
        };
        agent.refresh_project_state()?;
        Ok(agent)
    }

    pub fn init_project_files(&mut self) -> io::Result<Vec<PathBuf>> {
        let created = scaffold_claude_project(&self.root_dir)?;
        self.refresh_project_state()?;
        Ok(created)
    }

    fn refresh_project_state(&mut self) -> io::Result<()> {
        self.memory_stack = load_memory_stack(&self.root_dir, self.home_dir.as_deref())?;
        self.subagents = load_subagent_specs(&self.root_dir, self.home_dir.as_deref())?;
        self.commands = load_custom_commands(&self.root_dir, self.home_dir.as_deref())?;
        Ok(())
    }

    pub fn wait_for_context(&mut self, config: WaitModeConfig) -> io::Result<()> {
        let mut show_trace = config.show_trace;
        let mut state = SessionState::default();

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
        let mut state = SessionState::default();
        self.run_with_state(&mut state, input)
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
                    format!("Loaded {} custom command(s).", self.commands.len()),
                ];
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
                    trace.extend(planner_run.reasoning.iter().map(|item| {
                        format!("Iteration {step_count}: planner reasoning = {item}")
                    }));
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
                            match self.call_tool(&tool_name, input, &arguments) {
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
                                        return AgentResult::stopped(stop_reason, trace, usage);
                                    }
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
                                Ok(result) => {
                                    usage.extend(result.usage.clone());
                                    observations.push(format!(
                                        "Subagent `{subagent_name}` observation:\n{}",
                                        result.render()
                                    ));
                                    trace.push(format!(
                                        "Iteration {step_count}: subagent observation recorded."
                                    ));
                                }
                                Err(reason) => {
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
        if let Some(engine) = &self.llm_engine {
            match engine.plan(
                &self.config,
                &self.tools,
                &self.subagents,
                state,
                input,
                task_contract,
                observations,
                retry_count,
            ) {
                Ok(plan) => return plan,
                Err(reason) => {
                    if let Some(engine) = &self.fallback_engine {
                        match engine.plan(
                            &self.config,
                            &self.tools,
                            &self.subagents,
                            state,
                            input,
                            task_contract,
                            observations,
                            retry_count,
                        ) {
                            Ok(mut plan) => {
                                plan.reasoning.push(format!(
                                    "Primary planner failed and the harness fell back to `{}`: {}",
                                    self.config.fallback_model, reason
                                ));
                                return plan;
                            }
                            Err(fallback_reason) => {
                                let mut fallback = heuristic_plan(
                                    input,
                                    &self.tools,
                                    &self.subagents,
                                    observations,
                                );
                                fallback.reasoning.push(format!(
                                    "OpenAI planner failed: {reason}"
                                ));
                                fallback.reasoning.push(format!(
                                    "Fallback planner `{}` failed: {}",
                                    self.config.fallback_model, fallback_reason
                                ));
                                fallback.reasoning.push(
                                    "The harness fell back to the local heuristic router."
                                        .to_string(),
                                );
                                return fallback;
                            }
                        }
                    }

                    let mut fallback =
                        heuristic_plan(input, &self.tools, &self.subagents, observations);
                    fallback.reasoning.push(format!(
                        "OpenAI planner failed; fell back to the local heuristic router: {reason}"
                    ));
                    return fallback;
                }
            }
        }

        if let Some(engine) = &self.fallback_engine {
            match engine.plan(
                &self.config,
                &self.tools,
                &self.subagents,
                state,
                input,
                task_contract,
                observations,
                retry_count,
            ) {
                Ok(plan) => return plan,
                Err(reason) => {
                    let mut fallback =
                        heuristic_plan(input, &self.tools, &self.subagents, observations);
                    fallback.reasoning.push(format!(
                        "Fallback planner `{}` failed; fell back to the local heuristic router: {reason}",
                        self.config.fallback_model
                    ));
                    return fallback;
                }
            }
        }

        heuristic_plan(input, &self.tools, &self.subagents, observations)
    }

    fn call_tool(&self, name: &str, user_input: &str, arguments: &Value) -> StepOutcome {
        match self.tools.iter().find(|tool| tool.is_named(name)) {
            Some(tool) => tool.run(user_input, arguments),
            None => StepOutcome::Retry(format!("Unknown tool `{name}`.")),
        }
    }

    fn delegate_to_subagent(
        &self,
        subagent_name: &str,
        state: &SessionState,
        task: &str,
    ) -> Result<DelegationResult, String> {
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
        let request = DelegationRequest {
            subagent: spec.name.clone(),
            context_packet,
            memory_refs,
            file_refs,
            observations,
            task: task.to_string(),
        };
        self.observability.with_span(
            format!("subagent.{}", spec.name),
            vec![
                KeyValue::new("subagent.name", spec.name.clone()),
                KeyValue::new("subagent.scope", spec.scope.clone()),
            ],
            || Ok(execute_subagent(spec, request)),
        )
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

    fn handle_slash_command(
        &mut self,
        state: &mut SessionState,
        input: &str,
        bin_name: &str,
    ) -> io::Result<CommandOutcome> {
        match parse_slash_command(input, &self.commands) {
            SlashCommand::Help => Ok(CommandOutcome::Continue(self.chat_help_text(bin_name))),
            SlashCommand::Agents => Ok(CommandOutcome::Continue(self.render_agents())),
            SlashCommand::Memory => Ok(CommandOutcome::Continue(self.render_memory())),
            SlashCommand::Model => Ok(CommandOutcome::Continue(self.render_model())),
            SlashCommand::Clear => {
                state.clear();
                Ok(CommandOutcome::Continue(
                    "Cleared session history and trace. Project memory remains loaded.".to_string(),
                ))
            }
            SlashCommand::Compact => {
                let summary = compact_history(&state.history);
                state.history.clear();
                state.compacted_summary = Some(summary.clone());
                Ok(CommandOutcome::Continue(format!(
                    "Compacted session history.\nSummary: {summary}"
                )))
            }
            SlashCommand::Mcp => Ok(CommandOutcome::Continue(self.render_mcp())),
            SlashCommand::Review(task) => {
                Ok(CommandOutcome::Continue(self.run_review(state, &task)))
            }
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
                    Ok(result) => Ok(CommandOutcome::Continue(result.render())),
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

    fn render_memory(&self) -> String {
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
        lines.push("Merged memory preview:".to_string());
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
            format!("Use `{bin_name} list` to inspect loaded memory, subagents, commands, and tools."),
            "Built-in commands:".to_string(),
            "/help, /agents, /memory, /model, /clear, /compact, /mcp, /review [task], /init, /agent <name> <task>, /trace, /exit".to_string(),
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
        "memory" => SlashCommand::Memory,
        "model" => SlashCommand::Model,
        "clear" => SlashCommand::Clear,
        "compact" => SlashCommand::Compact,
        "mcp" => SlashCommand::Mcp,
        "review" => SlashCommand::Review(remainder.to_string()),
        "init" => SlashCommand::Init,
        "trace" => SlashCommand::Unknown("trace".to_string()),
        "exit" | "quit" => SlashCommand::Exit,
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
        return Some((
            tool_name.clone(),
            arguments,
            format!("Explicit tool request detected for `{tool_name}`."),
        ));
    }

    if tools.iter().any(|tool| tool.is_named("web_search")) && should_use_web_search(input) {
        return Some((
            "web_search".to_string(),
            Value::Object(Default::default()),
            "Planner inferred `web_search` for an external or time-sensitive query.".to_string(),
        ));
    }

    None
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

fn heuristic_plan(
    input: &str,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
    observations: &[String],
) -> PlannerRun {
    if let Some(observation) = observations.last() {
        if observation.starts_with("Recoverable ") || observation.starts_with("Planner retry request:") {
            return PlannerRun {
                decision: Decision::Retry(
                    "the latest observation still reflects a recoverable failure".to_string(),
                ),
                reasoning: vec![
                    "The heuristic router preserves retry semantics when the latest observation is a recoverable failure."
                        .to_string(),
                ],
                usage: None,
            };
        }

        return PlannerRun {
            decision: Decision::Finish {
                answer: observation.clone(),
                reason: "the latest observation is now the best available answer".to_string(),
            },
            reasoning: vec![
                "The heuristic router finishes after a successful observation instead of taking another action."
                    .to_string(),
            ],
            usage: None,
        };
    }

    if let Some((tool_name, arguments, route_reason)) = infer_tool_request(input, tools) {
        return PlannerRun {
            decision: Decision::CallTool {
                tool_name,
                arguments,
                reason: route_reason.clone(),
            },
            reasoning: vec![route_reason],
            usage: None,
        };
    }

    let subagent_name =
        infer_subagent_name(input, subagents).unwrap_or_else(|| "general-purpose".to_string());
    PlannerRun {
        decision: Decision::DelegateSubagent {
            subagent_name,
            reason: "No explicit tool request was inferred.".to_string(),
        },
        reasoning: vec![
            "No model-backed planner succeeded; using the local heuristic router.".to_string()
        ],
        usage: None,
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
        config: &AgentConfig,
        tools: &[Tool],
        subagents: &BTreeMap<String, SubagentSpec>,
        state: &SessionState,
        user_input: &str,
        task_contract: &TaskContract,
        observations: &[String],
        retry_count: u8,
    ) -> Result<PlannerRun, String> {
        let request = json!({
            "model": self.model,
            "instructions": config.system_prompt,
            "input": build_planner_input(
                state,
                user_input,
                tools,
                subagents,
                task_contract,
                observations,
                retry_count,
            ),
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

        let response = self.send_json_request(request)?;
        let raw_text = response
            .output_text()
            .ok_or_else(|| "OpenAI response did not contain text output.".to_string())?;
        let payload: PlannerPayload = serde_json::from_str(&raw_text)
            .map_err(|error| format!("Planner JSON parse failed: {error}; raw={raw_text}"))?;
        let decision = planner_payload_to_decision(&payload, tools, subagents)?;
        let reasoning = vec![reasoning_summary_for_decision(&decision)];

        Ok(PlannerRun {
            decision,
            reasoning,
            usage: response
                .usage
                .as_ref()
                .map(|usage| usage.as_token_record(&self.model)),
        })
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
        config: &AgentConfig,
        tools: &[Tool],
        subagents: &BTreeMap<String, SubagentSpec>,
        state: &SessionState,
        user_input: &str,
        task_contract: &TaskContract,
        observations: &[String],
        retry_count: u8,
    ) -> Result<PlannerRun, String> {
        let prompt = planner_prompt(
            state,
            tools,
            subagents,
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

        let response = self.send_json_request(request)?;
        let raw_text = response
            .output_text()
            .ok_or_else(|| "Anthropic response did not contain text output.".to_string())?;
        let payload: PlannerPayload = serde_json::from_str(&raw_text)
            .map_err(|error| format!("Planner JSON parse failed: {error}; raw={raw_text}"))?;
        let decision = planner_payload_to_decision(&payload, tools, subagents)?;
        let reasoning = vec![reasoning_summary_for_decision(&decision)];

        Ok(PlannerRun {
            decision,
            reasoning,
            usage: response
                .usage
                .as_ref()
                .map(|usage| usage.as_token_record(&self.model)),
        })
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
    subagent_name: String,
    #[serde(default)]
    answer: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Serialize)]
struct ApiInputMessage {
    role: String,
    content: String,
}

fn planner_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "action": {
                "type": "string",
                "enum": ["tool", "delegate", "finish", "retry", "stop"]
            },
            "tool_name": { "type": "string" },
            "tool_arguments_json": { "type": "string" },
            "subagent_name": { "type": "string" },
            "answer": { "type": "string" },
            "reason": { "type": "string" }
        },
        "required": ["action", "tool_name", "tool_arguments_json", "subagent_name", "answer", "reason"]
    })
}

fn build_planner_input(
    state: &SessionState,
    user_input: &str,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
    task_contract: &TaskContract,
    observations: &[String],
    retry_count: u8,
) -> Vec<ApiInputMessage> {
    let mut messages = vec![ApiInputMessage {
        role: "developer".to_string(),
        content: planner_prompt(
            state,
            tools,
            subagents,
            task_contract,
            observations,
            retry_count,
        ),
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

fn planner_prompt(
    state: &SessionState,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
    task_contract: &TaskContract,
    observations: &[String],
    retry_count: u8,
) -> String {
    let tool_list = tools
        .iter()
        .map(|tool| format!("- {}", tool.planner_description()))
        .collect::<Vec<_>>()
        .join("\n");
    let subagent_list = subagents
        .values()
        .map(|spec| format!("- {}: {}", spec.name, spec.description))
        .collect::<Vec<_>>()
        .join("\n");
    let history_summary = compact_history(&state.history);
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

    format!(
        "You are the planner for a harness-first Rust coding agent.\n\
Choose exactly one next action and return JSON only.\n\
Rules:\n\
- Start from the explicit task contract.\n\
- The harness loop permits exactly one action per iteration.\n\
- Prefer calling a tool when the user's request requires an available tool now.\n\
- Prefer delegating to a subagent when the request is exploratory, planning-oriented, or file-focused.\n\
- Prefer `finish` when the latest observation already contains enough information to answer.\n\
- Use `retry` only for recoverable failures that gained new information.\n\
- Use action=`finish` when you can answer directly without a tool or subagent.\n\
- Use action=`stop` only for terminal conditions.\n\
- For action=`tool`, set `tool_name` exactly and encode an object in `tool_arguments_json`.\n\
- For action=`delegate`, set `subagent_name` exactly.\n\
- When a field does not apply, leave it empty, except `tool_arguments_json`, which must be `{{}}`.\n\
- Fill `reason` with a short explanation.\n\
\n\
Available tools:\n{tool_list}\n\
\n\
Available subagents:\n{subagent_list}\n\
\n\
Task contract:\n\
- goal: {}\n\
- constraints: {}\n\
- acceptance criteria: {}\n\
- relevant files: {}\n\
- stop condition: {}\n\
\n\
Retry count: {retry_count}\n\
\n\
Recorded observations:\n{observation_summary}\n\
\n\
Compacted prior history:\n{history_summary}\n"
        ,
        task_contract.goal,
        task_contract.constraints.join(" | "),
        task_contract.acceptance_criteria.join(" | "),
        if task_contract.relevant_files.is_empty() {
            "none inferred".to_string()
        } else {
            task_contract.relevant_files.join(", ")
        },
        task_contract.stop_condition,
    )
}

fn planner_payload_to_decision(
    payload: &PlannerPayload,
    tools: &[Tool],
    subagents: &BTreeMap<String, SubagentSpec>,
) -> Result<Decision, String> {
    let reason = empty_to_default(&payload.reason, "planner did not provide a reason");
    match payload.action.as_str() {
        "tool" => {
            let tool_name = empty_to_default(&payload.tool_name, "");
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

fn execute_subagent(spec: &SubagentSpec, request: DelegationRequest) -> DelegationResult {
    match spec.name.as_str() {
        "explore" => execute_explore_subagent(spec, request),
        "plan" => execute_plan_subagent(spec, request),
        _ => execute_general_subagent(spec, request),
    }
}

fn execute_explore_subagent(spec: &SubagentSpec, request: DelegationRequest) -> DelegationResult {
    let files = if request.file_refs.is_empty() {
        vec!["No matching files were inferred.".to_string()]
    } else {
        request.file_refs.clone()
    };
    let findings = files
        .iter()
        .map(|path| format!("Potentially relevant file: {path}"))
        .collect::<Vec<_>>();
    DelegationResult {
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

fn execute_plan_subagent(spec: &SubagentSpec, request: DelegationRequest) -> DelegationResult {
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
    DelegationResult {
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

fn execute_general_subagent(spec: &SubagentSpec, request: DelegationRequest) -> DelegationResult {
    let findings = vec![
        format!("Active task: {}", request.task),
        format!("Loaded memory sources: {}", request.memory_refs.len()),
        format!("Relevant files inferred: {}", request.file_refs.len()),
    ];
    DelegationResult {
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

fn load_memory_stack(root_dir: &Path, home_dir: Option<&Path>) -> io::Result<MemoryStack> {
    let mut sources = Vec::new();
    let mut visited = BTreeSet::new();

    if let Some(home_dir) = home_dir {
        let user_memory = home_dir.join(CLAUDE_DIR).join(PROJECT_MEMORY_FILE);
        load_memory_source(
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
        &project_memory,
        MemoryScope::Project,
        None,
        0,
        &mut visited,
        &mut sources,
    )?;

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
    })
}

fn load_memory_source(
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
    });

    for import_path in extract_imports(&contents, path) {
        load_memory_source(
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

fn file_stem_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unnamed")
        .to_string()
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

fn scaffold_claude_project(root_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut created = Vec::new();
    let files = vec![
        (root_dir.join(PROJECT_MEMORY_FILE), default_claude_memory()),
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
    "# CLAUDE.md\n\nProject memory for the local harness-first runtime.\n\n## Commands\n- `cargo check`\n- `cargo test`\n- `cargo fmt`\n\n## Agent Contract\n- Start every run with an explicit task contract: goal, constraints, acceptance criteria, relevant files, and stop condition.\n- Take exactly one action per loop iteration: call a tool, delegate to a subagent, retry, stop, or finalize.\n- Keep observations explicit between iterations instead of hiding state in prose.\n- Prefer compact context packets and explicit file references over transcript sprawl.\n\n## Working Style\n- Load only the files needed for the current task.\n- Prefer deterministic harness behavior over prompt-only cleverness.\n- Use `/review` after meaningful code changes.\n"
}

fn default_explore_agent() -> &'static str {
    "---\nname: explore\ndescription: Inspect code paths, locate files, and return a compact handoff.\ntools: [web_search]\n---\n\n# Explore Agent\n\nStart from the delegated task contract.\nNarrow the task to files, modules, constraints, and missing facts.\nReturn findings, file references, unresolved gaps, and the next practical action.\nDo not sprawl into implementation."
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tools::default_tools;

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{label}-{}", now_epoch_ms()));
        fs::create_dir_all(&path).expect("temp dir should exist");
        path
    }

    fn test_agent(root_dir: PathBuf) -> MainAgent {
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
            config: AgentConfig {
                system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
                default_model: "OpenAI GPT-5.4",
                fallback_model: FALLBACK_MODEL,
                planner_backend: "local heuristic planner".to_string(),
                max_retries: MAX_RETRIES,
            },
            llm_engine: None,
            fallback_engine: None,
            tools: default_tools(),
            mcp_servers: Vec::new(),
            observability: Observability::from_env(),
            memory_stack: MemoryStack::default(),
            subagents,
            commands: BTreeMap::new(),
        }
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
    fn load_memory_stack_follows_imports() {
        let root = temp_root("memory-stack");
        let imported = root.join("docs.md");
        fs::write(&imported, "# Extra\nImported memory").expect("imported file");
        fs::write(
            root.join("CLAUDE.md"),
            format!("# Root\n@{}\n", imported.display()),
        )
        .expect("root memory");

        let stack = load_memory_stack(&root, None).expect("memory stack should load");

        assert_eq!(stack.sources.len(), 2);
        assert!(stack.merged_instructions.contains("Imported memory"));
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
        assert!(summary.contains("subagent `general-purpose`: 0 input, 0 output, 0 total"));
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
    fn infer_tool_request_routes_web_queries_to_web_search() {
        let inferred =
            infer_tool_request("Find the latest OpenAI API docs updates", &default_tools())
                .expect("web search should be inferred");

        assert_eq!(inferred.0, "web_search");
        assert_eq!(inferred.1, Value::Object(Default::default()));
        assert!(inferred.2.contains("Planner inferred `web_search`"));
    }

    #[test]
    fn web_queries_use_web_search_path_without_explicit_tool_wrapper() {
        let root = temp_root("web-search-routing");
        let agent = test_agent(root);

        let result = agent.run("Search the web for the latest Rust release notes");

        assert!(result.stopped);
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Planner inferred `web_search`")));
        assert!(result
            .trace
            .iter()
            .any(|entry| entry.contains("Retry limit reached")));
    }
}
