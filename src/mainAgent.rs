use crate::agents::{ingress_agent, planner_agent, retry_once_agent, summarize_agent};
use crate::mcp::{load_mcp_catalog_from_env, McpServerSummary};
use crate::observability::{self, Observability};
use crate::runtime_log;
use crate::Tools::{default_tools, Tool};
use opentelemetry::KeyValue;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const EXIT_COMMANDS: &[&str] = &["exit", "quit", ":q"];
const MAX_RETRIES: u8 = 3;
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are the ingress and data ingestion agent. Accept incoming text, image, video, and audio payloads, normalize them, classify them, and queue them for downstream processing.";
const DEFAULT_MODEL: &str = "OpenAI GPT-5.4";
const FALLBACK_MODEL: &str = "Opus 4.6";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-5.4";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1/responses";
pub const DEFAULT_PERPLEXITY_MODEL: &str = "sonar-pro";
const SKILLS_DIR: &str = "skills";
const SKILL_FILE_NAME: &str = "SKILL.md";
const WORKSPACE_CONTEXT_DIR: &str = "Workspace";
const REPO_CONTEXT_FILES: &[&str] = &["AGENTS.md", "TDD.md"];
const MAX_COMPACT_CONTEXT_DOC_CHARS: usize = 280;
const MAX_COMPACT_CONTEXT_HISTORY_MESSAGES: usize = 8;
const DEFAULT_QUEUE_POLL_INTERVAL_MS: u64 = 1000;

#[derive(Debug, Clone)]
struct AgentConfig {
    system_prompt: String,
    max_retries: u8,
    default_model: &'static str,
    fallback_model: &'static str,
    planner_model: Option<String>,
}

#[derive(Debug, Clone)]
struct AgentState {
    retries: u8,
    trace: Vec<String>,
}

impl AgentState {
    fn new() -> Self {
        Self {
            retries: 0,
            trace: Vec::new(),
        }
    }

    fn record(&mut self, message: impl Into<String>) {
        self.trace.push(message.into());
    }

    fn recent_trace(&self, max_entries: usize) -> String {
        let total = self.trace.len();
        let start = total.saturating_sub(max_entries);
        self.trace[start..].join("\n")
    }
}

pub struct MainAgent {
    config: AgentConfig,
    tools: Vec<Tool>,
    skills: Vec<Skill>,
    skill_catalog_warnings: Vec<String>,
    mcp_servers: Vec<McpServerSummary>,
    llm_engine: Option<OpenAiEngine>,
    observability: Observability,
}

pub struct WaitModeConfig {
    pub prompt_label: String,
    pub agent_name: String,
    pub agent_icon: String,
    pub user_name: String,
    pub show_trace: bool,
    pub bin_name: String,
}

pub struct QueueModeConfig {
    pub agent_name: String,
    pub agent_icon: String,
    pub show_trace: bool,
}

enum QueueInputEvent {
    Line(String),
    Eof,
    Error(String),
}

pub trait StreamObserver {
    fn on_reasoning_delta(&mut self, _delta: &str) {}

    fn on_reasoning_done(&mut self, _text: &str) {}
}

pub struct TerminalReasoningStreamer {
    line_open: bool,
    emitted_any: bool,
}

impl TerminalReasoningStreamer {
    pub fn new() -> Self {
        Self {
            line_open: false,
            emitted_any: false,
        }
    }

    pub fn emitted_any(&self) -> bool {
        self.emitted_any
    }

    pub fn finish(&mut self) {
        if self.line_open {
            println!();
            self.line_open = false;
        }
    }
}

impl StreamObserver for TerminalReasoningStreamer {
    fn on_reasoning_delta(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }

        if !self.line_open {
            print!("Reasoning: ");
            self.line_open = true;
            self.emitted_any = true;
        }

        print!("{delta}");
        let _ = io::stdout().flush();
    }

    fn on_reasoning_done(&mut self, _text: &str) {
        self.finish();
    }
}

impl MainAgent {
    #[cfg(test)]
    fn new(system_prompt: String) -> Self {
        Self::with_engine(system_prompt, None, default_tools(), Vec::new())
    }

    pub fn from_env(system_prompt: String) -> io::Result<Self> {
        let catalog = load_mcp_catalog_from_env()
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
        let mut tools = default_tools();
        tools.extend(catalog.tools.into_iter().map(Tool::from_mcp));
        Ok(Self::with_engine(
            system_prompt,
            OpenAiEngine::from_env(),
            tools,
            catalog.servers,
        ))
    }

    fn with_engine(
        system_prompt: String,
        llm_engine: Option<OpenAiEngine>,
        tools: Vec<Tool>,
        mcp_servers: Vec<McpServerSummary>,
    ) -> Self {
        let (skills, skill_catalog_warnings) = load_skill_catalog_from_dir(Path::new(SKILLS_DIR));
        Self {
            config: AgentConfig {
                system_prompt,
                max_retries: MAX_RETRIES,
                default_model: DEFAULT_MODEL,
                fallback_model: FALLBACK_MODEL,
                planner_model: llm_engine.as_ref().map(|engine| engine.model.clone()),
            },
            tools,
            skills,
            skill_catalog_warnings,
            mcp_servers,
            llm_engine,
            observability: Observability::from_env(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn run(&self, user_input: &str) -> AgentResult {
        self.run_with_history(&[], user_input)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn run_with_history(
        &self,
        history: &[ConversationMessage],
        user_input: &str,
    ) -> AgentResult {
        self.run_with_history_and_observer(history, user_input, None)
    }

    pub fn run_with_history_and_observer(
        &self,
        history: &[ConversationMessage],
        user_input: &str,
        mut observer: Option<&mut dyn StreamObserver>,
    ) -> AgentResult {
        self.observability.with_span(
            "main_agent.run",
            build_root_span_attributes(&self.config, history, user_input, self.observability.enabled_targets()),
            || {
                let mut state = AgentState::new();
                state.record(format!(
                    "Model policy loaded: default=`{}`, fallback=`{}`",
                    self.config.default_model, self.config.fallback_model
                ));
                if let Some(engine) = &self.llm_engine {
                    state.record(format!(
                        "Planner backend: OpenAI Responses API model=`{}`",
                        engine.model
                    ));
                } else {
                    state.record("Planner backend: local heuristic (no OpenAI API key loaded)");
                }
                state.record(format!(
                    "System prompt loaded: {}",
                    self.config.system_prompt
                ));
                state.record(format!("Registered tools: {}", self.tools.len()));
                state.record(format!("Registered skills: {}", self.skills.len()));
                if self.observability.is_enabled() {
                    state.record(format!(
                        "Observability exporters: {}",
                        self.observability.enabled_targets().join(", ")
                    ));
                    if let Some(trace_id) = Observability::active_trace_id() {
                        state.record(format!("Trace ID: {trace_id}"));
                    }
                }
                for warning in &self.skill_catalog_warnings {
                    state.record(format!("Skill catalog warning: {warning}"));
                }
                for warning in self.observability.warnings() {
                    state.record(format!("Observability warning: {warning}"));
                }
                if !self.mcp_servers.is_empty() {
                    state.record(format!("Loaded MCP servers: {}", self.mcp_servers.len()));
                }
                state.record(format!("History items loaded: {}", history.len()));
                state.record(format!("User input received: {user_input}"));

                loop {
                    let planner_run = self.decide_next_step(
                        history,
                        &state,
                        user_input,
                        reborrow_observer(&mut observer),
                    );
                    let decision = planner_run.decision;
                    state.record(format!("Decision: {}", decision));
                    record_reasoning_summary(
                        &mut state,
                        &self.config,
                        &decision,
                        &planner_run.reasoning,
                    );
                    record_decision_event(&decision, &planner_run.reasoning);

                    match decision {
                        Decision::CallTool {
                            tool_name,
                            arguments,
                            ..
                        } => {
                            let outcome = self.call_tool(&tool_name, user_input, &arguments);
                            state.record(format!("Tool outcome: {}", outcome));
                            match outcome {
                                StepOutcome::Success(output) => {
                                    Observability::set_attributes(vec![KeyValue::new(
                                        "output.value",
                                        observability::compact_text(&output, 4_000),
                                    )]);
                                    Observability::set_status_ok();
                                    return AgentResult::completed(output, state.trace);
                                }
                                StepOutcome::Retry(reason) => {
                                    Observability::record_event(
                                        "agent.retry",
                                        vec![KeyValue::new("retry.reason", reason.clone())],
                                    );
                                    if should_stop_after_retry(
                                        &mut state,
                                        self.config.max_retries,
                                        &reason,
                                    ) {
                                        Observability::set_status_error(reason);
                                        return AgentResult::stopped(state.trace);
                                    }
                                }
                            }
                        }
                        Decision::CallSkill(skill_name, _) => {
                            let outcome = self.call_skill(&skill_name, history, user_input, &mut state);
                            state.record(format!("Skill outcome: {}", outcome));
                            match outcome {
                                StepOutcome::Success(output) => {
                                    Observability::set_attributes(vec![KeyValue::new(
                                        "output.value",
                                        observability::compact_text(&output, 4_000),
                                    )]);
                                    Observability::set_status_ok();
                                    return AgentResult::completed(output, state.trace);
                                }
                                StepOutcome::Retry(reason) => {
                                    Observability::record_event(
                                        "agent.retry",
                                        vec![KeyValue::new("retry.reason", reason.clone())],
                                    );
                                    if should_stop_after_retry(
                                        &mut state,
                                        self.config.max_retries,
                                        &reason,
                                    ) {
                                        Observability::set_status_error(reason);
                                        return AgentResult::stopped(state.trace);
                                    }
                                }
                            }
                        }
                        Decision::Retry(reason) => {
                            state.record(format!("Planner requested retry: {reason}"));
                            Observability::record_event(
                                "planner.retry",
                                vec![KeyValue::new("retry.reason", reason.clone())],
                            );
                            if should_stop_after_retry(&mut state, self.config.max_retries, &reason) {
                                Observability::set_status_error(reason);
                                return AgentResult::stopped(state.trace);
                            }
                        }
                        Decision::Finish(answer, _) => {
                            state.record("Stop condition met: planner produced final answer.");
                            Observability::set_attributes(vec![KeyValue::new(
                                "output.value",
                                observability::compact_text(&answer, 4_000),
                            )]);
                            Observability::set_status_ok();
                            return AgentResult::completed(answer, state.trace);
                        }
                        Decision::Stop(reason) => {
                            state.record(format!("Stop condition met: {reason}"));
                            Observability::set_status_error(reason);
                            return AgentResult::stopped(state.trace);
                        }
                    }
                }
            },
        )
    }

    pub fn system_prompt(&self) -> &str {
        &self.config.system_prompt
    }

    pub fn default_model(&self) -> &str {
        self.config.default_model
    }

    pub fn fallback_model(&self) -> &str {
        self.config.fallback_model
    }

    pub fn planner_backend_label(&self) -> String {
        match &self.llm_engine {
            Some(engine) => format!("OpenAI Responses API ({})", engine.model),
            None => "local heuristic".to_string(),
        }
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    pub fn mcp_servers(&self) -> &[McpServerSummary] {
        &self.mcp_servers
    }

    pub fn wait_for_context(&self, config: WaitModeConfig) -> io::Result<()> {
        let mut show_trace = config.show_trace;
        let mut history: Vec<ConversationMessage> = Vec::new();
        if let Err(reason) = ingress_agent::ensure_conversation_history_file() {
            eprintln!("Conversation history setup failed: {reason}");
        }

        println!("{} {} CLI", config.agent_icon, config.agent_name);
        println!("Welcome, {}.", config.user_name);
        println!(
            "Default model: {} | Fallback model: {}",
            self.default_model(),
            self.fallback_model()
        );
        println!("Planner backend: {}", self.planner_backend_label());
        println!("{}", chat_help_text(&config.bin_name));

        let stdin = io::stdin();
        loop {
            print!("\n{} {}> ", config.agent_icon, config.prompt_label);
            io::stdout().flush()?;

            let mut buffer = String::new();
            let bytes_read = stdin.read_line(&mut buffer)?;
            if bytes_read == 0 {
                println!("\nSession ended.");
                break;
            }

            let input = buffer.trim();
            if input.is_empty() {
                println!("Waiting for context. Enter a message or use /help.");
                continue;
            }

            match input {
                "/help" => println!("{}", chat_help_text(&config.bin_name)),
                "/system" => println!("System prompt: {}", self.system_prompt()),
                "/tools" => {
                    println!("Tools:");
                    for tool in self.tools() {
                        println!("- {}: {}", tool.name(), tool.description());
                    }
                }
                "/skills" => {
                    println!("Skills:");
                    for skill in self.skills() {
                        println!("- {}: {}", skill.name(), skill.description());
                    }
                }
                "/trace" => {
                    show_trace = !show_trace;
                    println!(
                        "Trace output is now {}.",
                        if show_trace { "on" } else { "off" }
                    );
                }
                "/exit" => {
                    println!("Session ended.");
                    break;
                }
                other if EXIT_COMMANDS.contains(&other) => {
                    println!("Session ended.");
                    break;
                }
                message => {
                    let mut streamer = TerminalReasoningStreamer::new();
                    let result =
                        self.run_with_history_and_observer(&history, message, Some(&mut streamer));
                    streamer.finish();
                    print_run_result(&result, show_trace, !streamer.emitted_any());

                    history.push(ConversationMessage::user(message));
                    history.push(ConversationMessage::assistant(result.output.clone()));
                    if let Err(reason) = ingress_agent::append_conversation_history("user", message)
                    {
                        eprintln!("Conversation history update failed: {reason}");
                    }
                    if let Err(reason) =
                        ingress_agent::append_conversation_history("assistant", &result.output)
                    {
                        eprintln!("Conversation history update failed: {reason}");
                    }
                }
            }
        }

        Ok(())
    }

    pub fn wait_for_queue(
        &self,
        config: QueueModeConfig,
        bootstrap_input: Option<&str>,
    ) -> io::Result<()> {
        let stdin_is_terminal = io::stdin().is_terminal();
        let queue_path = ingress_agent::resolve_queue_path();
        let planner_queue_path = planner_agent::resolve_planner_queue_path();
        if let Err(reason) = ingress_agent::ensure_conversation_history_file() {
            eprintln!("Conversation history setup failed: {reason}");
        }

        println!("{} {} Ingress Runner", config.agent_icon, config.agent_name);
        println!(
            "Default model: {} | Fallback model: {}",
            self.default_model(),
            self.fallback_model()
        );
        println!("Planner backend: {}", self.planner_backend_label());
        println!("Ingress queue file: {}", queue_path.display());
        println!("Planner queue file: {}", planner_queue_path.display());
        println!("Active agents:");
        for label in self.registered_queue_agent_labels() {
            let status = if label == "planner_agent" {
                "waiting for ingress queue"
            } else {
                "waiting for user input"
            };
            println!("- {label}: {status}");
        }
        println!("Enter inbound payloads. Type `exit`, `quit`, or `:q` to stop.");
        println!(
            "Planner poll interval: {} ms",
            resolve_queue_poll_interval().as_millis()
        );
        runtime_log::info(
            "main_agent",
            format!(
                "started agents: {}",
                self.registered_queue_agent_labels().join(", ")
            ),
        );
        runtime_log::info(
            "planner_agent",
            format!(
                "watching ingress queue {} with planner queue {}",
                queue_path.display(),
                planner_queue_path.display()
            ),
        );

        if let Some(input) = bootstrap_input
            .map(str::trim)
            .filter(|input| !input.is_empty())
        {
            self.process_ingress_submission(input, config.show_trace, true);
        }
        let input_events = Some(spawn_queue_input_reader());
        self.watch_ingress_queue_forever(&config, stdin_is_terminal, input_events)
    }

    fn watch_ingress_queue_forever(
        &self,
        config: &QueueModeConfig,
        stdin_is_terminal: bool,
        mut input_events: Option<mpsc::Receiver<QueueInputEvent>>,
    ) -> io::Result<()> {
        let poll_interval = resolve_queue_poll_interval();
        let mut prompt_visible = false;
        let _ = self.process_pending_queue_work(config.show_trace);

        loop {
            if stdin_is_terminal && !prompt_visible {
                print!("\n{} ingress> ", config.agent_icon);
                io::stdout().flush()?;
                prompt_visible = true;
            }

            let queue_event = match &input_events {
                Some(receiver) => receiver.recv_timeout(poll_interval),
                None => {
                    thread::sleep(poll_interval);
                    Err(mpsc::RecvTimeoutError::Timeout)
                }
            };

            match queue_event {
                Ok(QueueInputEvent::Line(buffer)) => {
                    prompt_visible = false;
                    let input = buffer.trim();
                    if input.is_empty() {
                        println!("Waiting for user input.");
                        continue;
                    }
                    if EXIT_COMMANDS.contains(&input) {
                        println!("Ingress runner ended.");
                        break;
                    }

                    self.process_ingress_submission(input, config.show_trace, false);
                }
                Ok(QueueInputEvent::Eof) => {
                    if should_continue_polling_after_stdin_close(stdin_is_terminal) {
                        input_events = None;
                        prompt_visible = false;
                        println!("\nStdin closed. Continuing planner queue polling.");
                        runtime_log::warn(
                            "main_agent",
                            "stdin closed; continuing planner queue watcher",
                        );
                    } else {
                        println!("\nIngress runner ended.");
                        runtime_log::info("main_agent", "ingress runner stopped after stdin close");
                        break;
                    }
                }
                Ok(QueueInputEvent::Error(reason)) => {
                    runtime_log::error(
                        "main_agent",
                        format!("stdin reader failed while watching ingress queue: {reason}"),
                    );
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("failed to read stdin: {reason}"),
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.process_pending_queue_work(config.show_trace) {
                        prompt_visible = false;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if should_continue_polling_after_stdin_close(stdin_is_terminal) {
                        input_events = None;
                        prompt_visible = false;
                        println!("\nInput reader disconnected. Continuing planner queue polling.");
                        runtime_log::warn(
                            "main_agent",
                            "input reader disconnected; continuing planner queue watcher",
                        );
                    } else {
                        println!("\nIngress runner ended.");
                        runtime_log::info(
                            "main_agent",
                            "ingress runner stopped after input reader disconnect",
                        );
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    fn decide_next_step(
        &self,
        history: &[ConversationMessage],
        state: &AgentState,
        user_input: &str,
        observer: Option<&mut dyn StreamObserver>,
    ) -> PlannerRun {
        self.observability.with_span(
            "planner.decide_next_step",
            vec![
                KeyValue::new(
                    "planner.backend",
                    if self.llm_engine.is_some() {
                        "openai"
                    } else {
                        "heuristic"
                    },
                ),
                KeyValue::new("agent.retry_count", state.retries as i64),
                KeyValue::new(
                    "input.value",
                    observability::compact_text(user_input, 1_000),
                ),
            ],
            || {
                if let Some(engine) = &self.llm_engine {
                    match engine.plan(
                        &self.config,
                        &self.tools,
                        &self.skills,
                        history,
                        state,
                        user_input,
                        observer,
                        &self.observability,
                    ) {
                        Ok(plan) => plan,
                        Err(reason) => PlannerRun::new(Decision::Retry(format!(
                            "OpenAI planner call failed: {reason}"
                        ))),
                    }
                } else {
                    PlannerRun::new(decide_next_step_heuristic(
                        &self.config,
                        &self.tools,
                        &self.skills,
                        state,
                        user_input,
                    ))
                }
            },
        )
    }

    fn call_tool(&self, tool_name: &str, user_input: &str, arguments: &Value) -> StepOutcome {
        self.observability.with_span(
            format!("tool.{tool_name}"),
            vec![
                KeyValue::new("langsmith.span.kind", "tool"),
                KeyValue::new("tool.name", tool_name.to_string()),
                observability::kv_json("tool.arguments", arguments),
                KeyValue::new(
                    "input.value",
                    observability::compact_text(user_input, 1_000),
                ),
            ],
            || {
                let outcome = match self.tools.iter().find(|tool| tool.is_named(tool_name)) {
                    Some(tool) => tool.run(user_input, arguments),
                    None => StepOutcome::Retry(format!("Unknown tool requested: {tool_name}")),
                };
                record_step_outcome("tool", &outcome);
                outcome
            },
        )
    }

    fn call_skill(
        &self,
        skill_name: &str,
        history: &[ConversationMessage],
        user_input: &str,
        state: &mut AgentState,
    ) -> StepOutcome {
        self.observability.with_span(
            format!("skill.{skill_name}"),
            vec![
                KeyValue::new("langsmith.span.kind", "chain"),
                KeyValue::new("skill.name", skill_name.to_string()),
                KeyValue::new(
                    "input.value",
                    observability::compact_text(user_input, 1_000),
                ),
            ],
            || {
                let outcome = match self.skills.iter().find(|skill| skill.is_named(skill_name)) {
                    Some(skill) => skill.run(
                        user_input,
                        history,
                        state,
                        self.llm_engine.as_ref(),
                        &self.config.system_prompt,
                        &self.observability,
                    ),
                    None => StepOutcome::Retry(format!("Unknown skill requested: {skill_name}")),
                };
                record_step_outcome("skill", &outcome);
                outcome
            },
        )
    }

    fn registered_queue_agent_labels(&self) -> Vec<String> {
        let mut labels = vec!["ingress_agent".to_string()];
        for skill in &self.skills {
            let label = match skill.name() {
                "planner" => "planner_agent".to_string(),
                "summarize" => "summarize_agent".to_string(),
                "retry_once" => "retry_once_agent".to_string(),
                other => format!("skill::{other}"),
            };
            labels.push(label);
        }
        labels
    }

    fn process_ingress_submission(&self, input: &str, show_trace: bool, bootstrap: bool) {
        self.observability.with_span(
            "ingress_agent.process_submission",
            vec![
                KeyValue::new("langsmith.trace.name", "ingress_agent.process_submission"),
                KeyValue::new("langsmith.span.kind", "tool"),
                KeyValue::new(
                    "input.value",
                    observability::compact_text(input, 2_000),
                ),
                KeyValue::new("ingress.bootstrap", bootstrap),
            ],
            || {
                if let Err(reason) = ingress_agent::append_conversation_history("user", input) {
                    eprintln!("Conversation history update failed: {reason}");
                }
                runtime_log::info(
                    "ingress_agent",
                    format!(
                        "received {}submission for queue ingestion",
                        if bootstrap { "bootstrap " } else { "" }
                    ),
                );

                let ack = match ingress_agent::queue_ingress(input, &Value::Null) {
                    Ok(ack) => ack,
                    Err(reason) => {
                        Observability::set_status_error(reason.clone());
                        if bootstrap {
                            eprintln!("Bootstrap ingress submission failed: {reason}");
                        } else {
                            eprintln!("Ingress submission failed: {reason}");
                        }
                        return;
                    }
                };
                Observability::set_attributes(vec![
                    KeyValue::new("ingress.queue_id", ack.queue_id.clone()),
                    KeyValue::new("ingress.queue_path", ack.queue_path.clone()),
                ]);
                Observability::set_status_ok();
                runtime_log::info(
                    "ingress_agent",
                    format!(
                        "queued ingress item {} into {}",
                        ack.queue_id, ack.queue_path
                    ),
                );

                if bootstrap {
                    println!("\nBootstrap ingress submission:");
                } else {
                    println!("\nIngress submission:");
                }
                println!("{}", ack.render());

                let _ = self.process_pending_queue_work(show_trace);
            },
        );
    }

    fn process_pending_queue_work(&self, show_trace: bool) -> bool {
        self.observability.with_span(
            "planner_agent.queue_cycle",
            vec![
                KeyValue::new("langsmith.trace.name", "planner_agent.queue_cycle"),
                KeyValue::new("langsmith.span.kind", "chain"),
            ],
            || {
                let pending_entries = match planner_agent::pending_queue_entries() {
                    Ok(entries) => entries,
                    Err(reason) => {
                        eprintln!("planner_agent queue inspection failed: {reason}");
                        Observability::set_status_error(reason.clone());
                        runtime_log::error(
                            "planner_agent",
                            format!("queue inspection failed: {reason}"),
                        );
                        return true;
                    }
                };

                if pending_entries.is_empty() {
                    Observability::record_event("planner_agent.noop", Vec::new());
                    Observability::set_status_ok();
                    return false;
                }

                Observability::set_attributes(vec![KeyValue::new(
                    "planner.pending_entries",
                    pending_entries.len() as i64,
                )]);
                runtime_log::info(
                    "planner_agent",
                    format!(
                        "picked up {} unplanned ingress item(s)",
                        pending_entries.len()
                    ),
                );

                println!(
                    "\nplanner_agent picked up {} queued item(s) from {}.",
                    pending_entries.len(),
                    ingress_agent::resolve_queue_path().display()
                );

                let planner_prompt = format!(
                    "Read ingress queue items and write the next explicit action to {}.",
                    planner_agent::resolve_planner_queue_path().display()
                );
                let mut state = AgentState::new();
                let outcome = self.call_skill("planner", &[], &planner_prompt, &mut state);
                println!("planner_agent:");
                match outcome {
                    StepOutcome::Success(output) => {
                        println!("{output}");
                        Observability::set_status_ok();
                        runtime_log::info(
                            "planner_agent",
                            format!(
                                "planner run completed; planner queue path {}",
                                planner_agent::resolve_planner_queue_path().display()
                            ),
                        );
                        if let Err(reason) =
                            ingress_agent::append_conversation_history("assistant", &output)
                        {
                            eprintln!("Conversation history update failed: {reason}");
                        }
                    }
                    StepOutcome::Retry(reason) => {
                        Observability::set_status_error(reason.clone());
                        runtime_log::warn(
                            "planner_agent",
                            format!("planner requested retry: {reason}"),
                        );
                        eprintln!("planner_agent retry: {reason}");
                    }
                }

                if show_trace && !state.trace.is_empty() {
                    println!("planner_agent trace:");
                    for step in &state.trace {
                        println!("- {step}");
                    }
                }

                true
            },
        )
    }
}

fn resolve_queue_poll_interval() -> Duration {
    env::var("AGENT_QUEUE_POLL_INTERVAL_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_QUEUE_POLL_INTERVAL_MS))
}

fn should_continue_polling_after_stdin_close(stdin_is_terminal: bool) -> bool {
    !stdin_is_terminal
}

fn spawn_queue_input_reader() -> mpsc::Receiver<QueueInputEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let event = match line {
                Ok(line) => QueueInputEvent::Line(line),
                Err(error) => QueueInputEvent::Error(error.to_string()),
            };

            if sender.send(event).is_err() {
                return;
            }
        }

        let _ = sender.send(QueueInputEvent::Eof);
    });
    receiver
}

#[derive(Debug, Clone)]
pub struct Skill {
    name: String,
    description: String,
    source: SkillSource,
}

impl Skill {
    fn built_in(
        name: impl Into<String>,
        description: impl Into<String>,
        kind: BuiltInSkill,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            source: SkillSource::BuiltIn(kind),
        }
    }

    fn installed(manifest: InstalledSkillManifest) -> Self {
        Self {
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            source: SkillSource::Installed(manifest),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    fn is_named(&self, name: &str) -> bool {
        self.name == name
    }

    fn run(
        &self,
        user_input: &str,
        history: &[ConversationMessage],
        state: &mut AgentState,
        llm_engine: Option<&OpenAiEngine>,
        system_prompt: &str,
        observability: &Observability,
    ) -> StepOutcome {
        match &self.source {
            SkillSource::BuiltIn(kind) => match kind {
                BuiltInSkill::Planner => planner_agent::run(user_input),
                BuiltInSkill::Summarize => summarize_agent::run(user_input),
                BuiltInSkill::RetryOnce => retry_once_agent::run(state.retries),
            },
            SkillSource::Installed(manifest) => {
                state.record(format!(
                    "Loaded installed skill `{}` from {}",
                    self.name,
                    manifest.skill_file.display()
                ));
                match manifest.load_definition() {
                    Ok(loaded) => {
                        state.record(format!(
                            "Installed skill `{}` resources: {}",
                            self.name,
                            loaded.resource_summary()
                        ));
                        if let Some(engine) = llm_engine {
                            match engine.execute_skill(
                                &self.name,
                                &self.description,
                                &loaded,
                                &render_workspace_markdown_context(),
                                history,
                                user_input,
                                system_prompt,
                                &state.recent_trace(10),
                                observability,
                            ) {
                                Ok(output) => StepOutcome::Success(output),
                                Err(reason) => StepOutcome::Retry(format!(
                                    "Installed skill `{}` execution failed: {}",
                                    self.name, reason
                                )),
                            }
                        } else {
                            StepOutcome::Success(render_skill_fallback_output(
                                &self.name,
                                &self.description,
                                &loaded,
                                user_input,
                            ))
                        }
                    }
                    Err(reason) => StepOutcome::Retry(format!(
                        "Installed skill `{}` could not be loaded: {}",
                        self.name, reason
                    )),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
enum SkillSource {
    BuiltIn(BuiltInSkill),
    Installed(InstalledSkillManifest),
}

#[derive(Debug, Clone)]
enum BuiltInSkill {
    Planner,
    Summarize,
    RetryOnce,
}

#[derive(Debug, Clone)]
struct InstalledSkillManifest {
    name: String,
    description: String,
    directory: PathBuf,
    skill_file: PathBuf,
}

#[derive(Debug, Clone)]
struct LoadedInstalledSkill {
    directory: PathBuf,
    markdown: String,
    body: String,
    resource_files: Vec<PathBuf>,
}

impl LoadedInstalledSkill {
    fn resource_summary(&self) -> String {
        if self.resource_files.is_empty() {
            "no bundled resource files".to_string()
        } else {
            self.resource_files
                .iter()
                .map(|path| path.strip_prefix(&self.directory).unwrap_or(path))
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

impl InstalledSkillManifest {
    fn load_definition(&self) -> Result<LoadedInstalledSkill, String> {
        let markdown = fs::read_to_string(&self.skill_file)
            .map_err(|error| format!("read {} failed: {error}", self.skill_file.display()))?;
        Ok(LoadedInstalledSkill {
            directory: self.directory.clone(),
            body: strip_skill_frontmatter(&markdown).trim().to_string(),
            markdown,
            resource_files: list_skill_resource_files(&self.directory),
        })
    }
}

fn built_in_skills() -> Vec<Skill> {
    vec![
        Skill::built_in(
            "planner",
            "Use when the request should inspect ingress queue items, read markdown workspace context, and write the next-step plan into logs/planner_queue.jsonl.",
            BuiltInSkill::Planner,
        ),
        Skill::built_in(
            "summarize",
            "Use when the request should be compressed into a short summary.",
            BuiltInSkill::Summarize,
        ),
        Skill::built_in(
            "retry_once",
            "Use when you need to exercise the retry path with one recoverable failure.",
            BuiltInSkill::RetryOnce,
        ),
    ]
}

fn load_skill_catalog_from_dir(skills_dir: &Path) -> (Vec<Skill>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut catalog = BTreeMap::new();

    for skill in built_in_skills() {
        catalog.insert(skill.name().to_string(), skill);
    }

    match load_installed_skills_from_dir(skills_dir) {
        Ok(installed_skills) => {
            for skill in installed_skills {
                let name = skill.name().to_string();
                if catalog.insert(name.clone(), skill).is_some() {
                    warnings.push(format!(
                        "Installed skill `{name}` overrides an existing skill with the same name."
                    ));
                }
            }
        }
        Err(reason) => warnings.push(reason),
    }

    (catalog.into_values().collect(), warnings)
}

fn load_installed_skills_from_dir(skills_dir: &Path) -> Result<Vec<Skill>, String> {
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(skills_dir)
        .map_err(|error| format!("could not read {}: {error}", skills_dir.display()))?;
    let mut skills = Vec::new();

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to read skills directory entry: {error}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_file = path.join(SKILL_FILE_NAME);
        if !skill_file.is_file() {
            continue;
        }

        match parse_installed_skill_manifest(&path, &skill_file) {
            Ok(manifest) => skills.push(Skill::installed(manifest)),
            Err(reason) => {
                return Err(format!("failed to load {}: {}", path.display(), reason));
            }
        }
    }

    skills.sort_by(|left, right| left.name().cmp(right.name()));
    Ok(skills)
}

fn parse_installed_skill_manifest(
    directory: &Path,
    skill_file: &Path,
) -> Result<InstalledSkillManifest, String> {
    let markdown = fs::read_to_string(skill_file)
        .map_err(|error| format!("read {} failed: {error}", skill_file.display()))?;
    let frontmatter = parse_skill_frontmatter(&markdown);
    let fallback_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unnamed-skill")
        .trim()
        .to_string();
    let name = frontmatter
        .get("name")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_name);
    let description = frontmatter
        .get("description")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            "Use when the request explicitly asks for this installed skill.".to_string()
        });

    Ok(InstalledSkillManifest {
        name,
        description,
        directory: directory.to_path_buf(),
        skill_file: skill_file.to_path_buf(),
    })
}

fn parse_skill_frontmatter(markdown: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return result;
    };
    let Some((frontmatter, _)) = rest.split_once("\n---\n") else {
        return result;
    };

    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        result.insert(
            key.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }

    result
}

fn strip_skill_frontmatter(markdown: &str) -> String {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return markdown.to_string();
    };
    match rest.split_once("\n---\n") {
        Some((_, body)) => body.to_string(),
        None => markdown.to_string(),
    }
}

fn list_skill_resource_files(directory: &Path) -> Vec<PathBuf> {
    let mut resources = Vec::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return resources;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.file_name().and_then(|name| name.to_str()) != Some(SKILL_FILE_NAME)
        {
            resources.push(path);
            continue;
        }

        if path.is_dir() {
            collect_nested_files(&path, &mut resources);
        }
    }

    resources.sort();
    resources
}

fn collect_nested_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_nested_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn render_skill_fallback_output(
    skill_name: &str,
    description: &str,
    loaded: &LoadedInstalledSkill,
    user_input: &str,
) -> String {
    let preview = loaded
        .body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Installed skill `{skill_name}` was loaded from `{}`.\nDescription: {description}\nUser request: {user_input}\nBundled resources: {}\n\nSKILL.md preview:\n{}",
        loaded.directory.display(),
        loaded.resource_summary(),
        preview
    )
}

fn resolve_workspace_context_dir() -> PathBuf {
    env::var("AGENT_WORKSPACE_DIR")
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

fn load_agent_markdown_documents() -> Result<Vec<(PathBuf, String)>, String> {
    load_markdown_documents(&resolve_workspace_context_dir())
}

fn render_workspace_markdown_context() -> String {
    match load_agent_markdown_documents() {
        Ok(documents) if documents.is_empty() => {
            "No repo or workspace markdown context files were found.".to_string()
        }
        Ok(documents) => documents
            .into_iter()
            .map(|(path, contents)| format!("## {}\n{}", path.display(), contents.trim()))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Err(reason) => format!("Markdown context could not be loaded: {reason}"),
    }
}

fn render_compact_runtime_context(history: &[ConversationMessage]) -> String {
    let markdown_context = match load_agent_markdown_documents() {
        Ok(documents) if documents.is_empty() => "Markdown context: none".to_string(),
        Ok(documents) => {
            let digest = documents
                .into_iter()
                .map(|(path, contents)| {
                    format!(
                        "- {} => {}",
                        path.display(),
                        compact_text_preview(&contents, MAX_COMPACT_CONTEXT_DOC_CHARS)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("Markdown context digest:\n{digest}")
        }
        Err(reason) => format!("Markdown context unavailable: {reason}"),
    };

    let persisted_history =
        match ingress_agent::ensure_conversation_history_file().and_then(|path| {
            fs::read_to_string(&path)
                .map_err(|error| format!("read {} failed: {error}", path.display()))
        }) {
            Ok(contents) => compact_text_preview(&contents, 700),
            Err(reason) => format!("Conversation history unavailable: {reason}"),
        };

    let recent_history = if history.is_empty() {
        "In-memory history: none".to_string()
    } else {
        let start = history
            .len()
            .saturating_sub(MAX_COMPACT_CONTEXT_HISTORY_MESSAGES);
        let digest = history[start..]
            .iter()
            .map(|message| {
                let label = match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                };
                format!("{label}: {}", compact_text_preview(&message.content, 180))
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Recent conversation digest:\n{digest}")
    };

    format!(
        "Task contract\n- goal: compact repo markdown, memory, and conversation context before the planner chooses its next action\n- constraints: keep only the highest-signal context, preserve active conversation state, and prefer repo-local markdown as source of truth\n- acceptance_criteria: the planner sees a concise context packet grounded in markdown files plus conversation history\n- stop_condition: stop after providing the compact context packet for the current decision\n\n{}\n\nPersisted conversation history:\n{}\n\n{}",
        markdown_context, persisted_history, recent_history
    )
}

fn compact_text_preview(value: &str, limit: usize) -> String {
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let shortened = normalized.chars().take(limit).collect::<String>();
    if normalized.chars().count() > limit {
        format!("{shortened}...")
    } else {
        shortened
    }
}

fn chat_help_text(bin_name: &str) -> String {
    format!(
        "Start an interactive ingress CLI session.\n\n\
Usage:\n  {bin_name} chat\n  {bin_name} chat --system <prompt> --trace\n\n\
Developer notes:\n  - This mode keeps the main agent in a persistent wait loop until new context arrives.\n  - Repo and workspace markdown plus persisted conversation history are compacted into the planner context.\n  - If `OPENAI_API_KEY` is set, the agent keeps multi-turn history locally and sends it to the OpenAI planner on each turn.\n  - `/trace` toggles execution trace output while the session is running.\n  - `/system`, `/tools`, and `/skills` inspect the active agent configuration.\n\n\
In-session commands:\n  /help    Show chat help.\n  /system  Show the active system prompt.\n  /tools   Show available tools.\n  /skills  Show available skills.\n  /trace   Toggle execution trace output.\n  /exit    Leave the chat session.\n"
    )
}

fn print_run_result(result: &AgentResult, show_trace: bool, show_reasoning: bool) {
    println!("Status: {:?}", result.status);
    if show_reasoning && !result.reasoning.is_empty() {
        println!("Reasoning:");
        for step in &result.reasoning {
            println!("- {step}");
        }
    }
    println!("Output: {}", result.output);

    if show_trace {
        println!("Trace:");
        for step in &result.trace {
            println!("- {step}");
        }
    }
}

#[derive(Debug, Clone)]
enum Decision {
    CallTool {
        tool_name: String,
        arguments: Value,
        reason: String,
    },
    CallSkill(String, String),
    Retry(String),
    Finish(String, String),
    Stop(String),
}

struct PlannerRun {
    decision: Decision,
    reasoning: Vec<String>,
}

impl PlannerRun {
    fn new(decision: Decision) -> Self {
        Self {
            decision,
            reasoning: Vec::new(),
        }
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Decision::CallTool {
                tool_name, reason, ..
            } => write!(f, "call tool `{tool_name}` ({reason})"),
            Decision::CallSkill(name, reason) => write!(f, "call skill `{name}` ({reason})"),
            Decision::Retry(reason) => write!(f, "retry ({reason})"),
            Decision::Finish(answer, reason) => write!(f, "finish ({reason}; {answer})"),
            Decision::Stop(reason) => write!(f, "stop ({reason})"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum StepOutcome {
    Success(String),
    Retry(String),
}

impl fmt::Display for StepOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepOutcome::Success(output) => write!(f, "success ({output})"),
            StepOutcome::Retry(reason) => write!(f, "retry ({reason})"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub status: RunStatus,
    pub output: String,
    pub trace: Vec<String>,
    pub reasoning: Vec<String>,
}

impl AgentResult {
    fn completed(output: String, trace: Vec<String>) -> Self {
        let reasoning = collect_reasoning_messages(&trace);
        Self {
            status: RunStatus::Completed,
            output,
            trace,
            reasoning,
        }
    }

    fn stopped(trace: Vec<String>) -> Self {
        let reasoning = collect_reasoning_messages(&trace);
        Self {
            status: RunStatus::Stopped,
            output: "Stopped after reaching retry limit or explicit stop condition.".to_string(),
            trace,
            reasoning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Completed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    fn as_api_role(&self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    role: MessageRole,
    content: String,
}

impl ConversationMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
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
        skills: &[Skill],
        history: &[ConversationMessage],
        state: &AgentState,
        user_input: &str,
        observer: Option<&mut dyn StreamObserver>,
        observability: &Observability,
    ) -> Result<PlannerRun, String> {
        observability.with_span(
            "openai.plan",
            vec![
                KeyValue::new("langsmith.span.kind", "llm"),
                KeyValue::new("gen_ai.system", "OpenAI"),
                KeyValue::new("gen_ai.operation.name", "chat"),
                KeyValue::new("gen_ai.request.model", self.model.clone()),
                KeyValue::new(
                    "input.value",
                    observability::compact_text(user_input, 1_500),
                ),
            ],
            || {
                let request = json!({
                    "model": self.model,
                    "instructions": config.system_prompt,
                    "input": build_planner_input(history, user_input, state, tools, skills),
                    "stream": true,
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

                let response = self.send_streaming_request(request, observer)?;
                let raw_text = response
                    .response
                    .output_text()
                    .ok_or_else(|| "OpenAI response did not contain text output.".to_string())?;
                let payload: PlannerPayload = serde_json::from_str(&raw_text)
                    .map_err(|error| format!("Planner JSON parse failed: {error}; raw={raw_text}"))?;
                Observability::set_attributes(vec![
                    KeyValue::new("gen_ai.response.model", self.model.clone()),
                    KeyValue::new(
                        "output.value",
                        observability::compact_text(&raw_text, 2_000),
                    ),
                ]);
                Observability::set_status_ok();

                Ok(PlannerRun {
                    decision: planner_payload_to_decision(&payload, tools, skills)?,
                    reasoning: response.reasoning,
                })
            },
        )
    }

    fn execute_skill(
        &self,
        skill_name: &str,
        skill_description: &str,
        loaded_skill: &LoadedInstalledSkill,
        workspace_markdown_context: &str,
        history: &[ConversationMessage],
        user_input: &str,
        system_prompt: &str,
        recent_trace: &str,
        observability: &Observability,
    ) -> Result<String, String> {
        observability.with_span(
            format!("openai.skill.{skill_name}"),
            vec![
                KeyValue::new("langsmith.span.kind", "llm"),
                KeyValue::new("gen_ai.system", "OpenAI"),
                KeyValue::new("gen_ai.operation.name", "chat"),
                KeyValue::new("gen_ai.request.model", self.model.clone()),
                KeyValue::new("skill.name", skill_name.to_string()),
                KeyValue::new(
                    "input.value",
                    observability::compact_text(user_input, 1_500),
                ),
            ],
            || {
                let mut input = vec![ApiInputMessage {
                    role: "developer".to_string(),
                    content: format!(
                        "You are executing an installed Claude-style skill for a Rust agent.\n\
Follow the skill instructions closely and answer the user's request directly.\n\
Do not mention internal planner mechanics unless the user asks.\n\
\n\
Skill name: {skill_name}\n\
Skill description: {skill_description}\n\
Skill directory: {}\n\
Bundled resources: {}\n\
\n\
Recent trace:\n{recent_trace}\n\
\n\
Repo and workspace markdown context:\n{workspace_markdown_context}\n\
\n\
SKILL.md:\n{}",
                        loaded_skill.directory.display(),
                        loaded_skill.resource_summary(),
                        loaded_skill.markdown
                    ),
                }];

                input.extend(history.iter().map(|message| ApiInputMessage {
                    role: message.role.as_api_role().to_string(),
                    content: message.content.clone(),
                }));

                input.push(ApiInputMessage {
                    role: "user".to_string(),
                    content: user_input.to_string(),
                });

                let request = json!({
                    "model": self.model,
                    "instructions": system_prompt,
                    "input": input,
                });

                let response = self.send_json_request(request)?;
                let output = response.output_text().ok_or_else(|| {
                    format!("OpenAI skill execution for `{skill_name}` returned no text output.")
                })?;
                Observability::set_attributes(vec![
                    KeyValue::new("gen_ai.response.model", self.model.clone()),
                    KeyValue::new(
                        "output.value",
                        observability::compact_text(&output, 2_000),
                    ),
                ]);
                Observability::set_status_ok();
                Ok(output)
            },
        )
    }

    fn send_streaming_request(
        &self,
        request: Value,
        observer: Option<&mut dyn StreamObserver>,
    ) -> Result<StreamedOpenAiResponse, String> {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| format!("client build error: {error}"))?;

        let response = client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .map_err(|error| format!("request error: {error}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .map_err(|error| format!("response read error: {error}"))?;
            return Err(format!("HTTP {}: {}", status.as_u16(), body));
        }

        parse_streaming_openai_response(BufReader::new(response), observer)
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

struct StreamedOpenAiResponse {
    response: OpenAiResponse,
    reasoning: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    status: Option<String>,
    error: Option<OpenAiErrorBody>,
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
struct OpenAiOutputItem {
    #[allow(dead_code)]
    #[serde(rename = "type")]
    item_type: String,
    #[allow(dead_code)]
    role: Option<String>,
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
struct PlannerPayload {
    action: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_arguments_json: String,
    #[serde(default)]
    skill_name: String,
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
                "enum": ["tool", "skill", "retry", "finish", "stop"]
            },
            "tool_name": {
                "type": "string"
            },
            "tool_arguments_json": {
                "type": "string"
            },
            "skill_name": {
                "type": "string"
            },
            "answer": {
                "type": "string"
            },
            "reason": {
                "type": "string"
            }
        },
        "required": ["action", "tool_name", "tool_arguments_json", "skill_name", "answer", "reason"]
    })
}

fn build_planner_input(
    history: &[ConversationMessage],
    user_input: &str,
    state: &AgentState,
    tools: &[Tool],
    skills: &[Skill],
) -> Vec<ApiInputMessage> {
    let mut messages = vec![ApiInputMessage {
        role: "developer".to_string(),
        content: planner_prompt(state, tools, skills),
    }];
    messages.push(ApiInputMessage {
        role: "developer".to_string(),
        content: render_compact_runtime_context(history),
    });

    messages.extend(history.iter().map(|message| ApiInputMessage {
        role: message.role.as_api_role().to_string(),
        content: message.content.clone(),
    }));

    messages.push(ApiInputMessage {
        role: "user".to_string(),
        content: user_input.to_string(),
    });

    messages
}

fn planner_prompt(state: &AgentState, tools: &[Tool], skills: &[Skill]) -> String {
    let tool_list = tools
        .iter()
        .map(|tool| tool.planner_description().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let skill_list = skills
        .iter()
        .map(|skill| format!("{}: {}", skill.name(), skill.description()))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are the planner for a Rust ingress and data-ingestion agent. Choose exactly one next action.\n\
Return JSON only matching the required schema.\n\
Rules:\n\
- The default operational path is ingress: for any non-empty inbound message that is not an explicit developer request to use another tool or skill, choose action=`tool` with `tool_name`=`queue_ingress`.\n\
- Choose action=`tool` only when one of the listed tools should be executed now.\n\
- For action=`tool`, set `tool_name` to the exact tool identifier from the list and set `tool_arguments_json` to a compact JSON string encoding an object that matches the described input shape. Use `{{}}` when no arguments are needed.\n\
- Choose action=`skill` only when one of the listed skills should be executed now.\n\
- Choose action=`retry` only for recoverable errors or when another attempt is required.\n\
- Choose action=`stop` for explicit stop conditions such as empty input or retry exhaustion.\n\
- Otherwise choose action=`finish` and put the full assistant reply in `answer`.\n\
- When action is not `tool`, leave `tool_name` as an empty string.\n\
- When action is not `tool`, leave `tool_arguments_json` as `\"{{}}\"`.\n\
- When action is not `skill`, leave `skill_name` as an empty string.\n\
- When action is not `finish`, leave `answer` as an empty string.\n\
- Always fill `reason` with a short explanation.\n\
- Available tools:\n{tool_list}\n\
- Available skills:\n{skill_list}\n\
- Current retry count: {}/{}\n\
- Recent trace:\n{}\n",
        state.retries,
        MAX_RETRIES,
        state.recent_trace(10)
    )
}

fn planner_payload_to_decision(
    payload: &PlannerPayload,
    tools: &[Tool],
    skills: &[Skill],
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

            let arguments = parse_planner_tool_arguments(&payload.tool_arguments_json, &tool_name)?;

            Ok(Decision::CallTool {
                tool_name,
                arguments,
                reason,
            })
        }
        "skill" => {
            let skill_name = empty_to_default(&payload.skill_name, "");
            if skill_name.is_empty() {
                return Err("Planner requested a skill without `skill_name`.".to_string());
            }
            if !skills.iter().any(|skill| skill.is_named(&skill_name)) {
                return Err(format!("Planner requested unknown skill `{skill_name}`"));
            }
            Ok(Decision::CallSkill(skill_name, reason))
        }
        "retry" => Ok(Decision::Retry(reason)),
        "finish" => Ok(Decision::Finish(
            empty_to_default(&payload.answer, "The model returned an empty answer."),
            reason,
        )),
        "stop" => Ok(Decision::Stop(reason)),
        other => Err(format!("Planner returned unsupported action `{other}`")),
    }
}

fn empty_to_default(value: &str, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value.trim().to_string()
    }
}

fn is_reasoning_model(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.starts_with('o') || normalized.contains("reason") || normalized.contains("gpt-5")
}

fn reasoning_summary_for_decision(decision: &Decision) -> Option<String> {
    match decision {
        Decision::CallTool {
            tool_name, reason, ..
        } => Some(format!("Using tool `{tool_name}` because {reason}")),
        Decision::CallSkill(name, reason) => Some(format!("Using skill `{name}` because {reason}")),
        Decision::Retry(reason) => Some(format!("Retry requested because {reason}")),
        Decision::Finish(_, reason) => Some(format!("Answering directly because {reason}")),
        Decision::Stop(reason) => Some(format!("Stopping because {reason}")),
    }
}

fn record_reasoning_summary(
    state: &mut AgentState,
    config: &AgentConfig,
    decision: &Decision,
    streamed_reasoning: &[String],
) {
    if !streamed_reasoning.is_empty() {
        for summary in streamed_reasoning {
            state.record(format!("Reasoning: {summary}"));
        }
        return;
    }

    let Some(model) = config.planner_model.as_deref() else {
        return;
    };

    if !is_reasoning_model(model) {
        return;
    }

    if let Some(summary) = reasoning_summary_for_decision(decision) {
        state.record(format!("Reasoning: {summary}"));
    }
}

fn collect_reasoning_messages(trace: &[String]) -> Vec<String> {
    trace
        .iter()
        .filter_map(|entry| entry.strip_prefix("Reasoning: ").map(str::to_string))
        .collect()
}

fn reborrow_observer<'a>(
    observer: &'a mut Option<&mut dyn StreamObserver>,
) -> Option<&'a mut dyn StreamObserver> {
    match observer {
        Some(observer) => Some(&mut **observer),
        None => None,
    }
}

fn build_root_span_attributes(
    config: &AgentConfig,
    history: &[ConversationMessage],
    user_input: &str,
    enabled_targets: &[&'static str],
) -> Vec<KeyValue> {
    vec![
        KeyValue::new("langsmith.trace.name", "main_agent.run"),
        KeyValue::new("langsmith.span.kind", "chain"),
        KeyValue::new("langsmith.trace.session_name", "agent_in_rust"),
        KeyValue::new("langfuse.trace.name", "main_agent.run"),
        KeyValue::new(
            "agent.observability.targets",
            enabled_targets.join(","),
        ),
        KeyValue::new("agent.default_model", config.default_model.to_string()),
        KeyValue::new("agent.fallback_model", config.fallback_model.to_string()),
        KeyValue::new("agent.history_count", history.len() as i64),
        KeyValue::new(
            "input.value",
            observability::compact_text(user_input, 2_000),
        ),
    ]
}

fn record_decision_event(decision: &Decision, reasoning: &[String]) {
    let mut attributes = vec![
        KeyValue::new("planner.decision", decision.to_string()),
        KeyValue::new("planner.reasoning_count", reasoning.len() as i64),
    ];
    if !reasoning.is_empty() {
        attributes.push(KeyValue::new(
            "planner.reasoning",
            observability::compact_text(&reasoning.join(" | "), 2_000),
        ));
    }
    Observability::record_event("planner.decision", attributes);
}

fn record_step_outcome(kind: &str, outcome: &StepOutcome) {
    match outcome {
        StepOutcome::Success(output) => {
            Observability::set_attributes(vec![KeyValue::new(
                "output.value",
                observability::compact_text(output, 2_000),
            )]);
            Observability::record_event(
                format!("{kind}.success"),
                vec![KeyValue::new(
                    format!("{kind}.output"),
                    observability::compact_text(output, 2_000),
                )],
            );
            Observability::set_status_ok();
        }
        StepOutcome::Retry(reason) => {
            Observability::record_event(
                format!("{kind}.retry"),
                vec![KeyValue::new(
                    format!("{kind}.retry_reason"),
                    observability::compact_text(reason, 2_000),
                )],
            );
            Observability::set_status_error(reason.clone());
        }
    }
}

fn parse_streaming_openai_response<R: BufRead>(
    mut reader: R,
    mut observer: Option<&mut dyn StreamObserver>,
) -> Result<StreamedOpenAiResponse, String> {
    let mut line = String::new();
    let mut event_name: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();
    let mut reasoning_summaries: BTreeMap<usize, String> = BTreeMap::new();
    let mut completed_response: Option<OpenAiResponse> = None;

    loop {
        line.clear();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|error| format!("response stream read error: {error}"))?;

        if bytes_read == 0 {
            if !data_lines.is_empty() {
                process_stream_event(
                    &event_name,
                    &data_lines,
                    &mut reasoning_summaries,
                    reborrow_observer(&mut observer),
                    &mut completed_response,
                )?;
            }
            break;
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if !data_lines.is_empty() {
                process_stream_event(
                    &event_name,
                    &data_lines,
                    &mut reasoning_summaries,
                    reborrow_observer(&mut observer),
                    &mut completed_response,
                )?;
                event_name = None;
                data_lines.clear();
            }
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }

    let response = completed_response.ok_or_else(|| {
        "OpenAI response stream ended before a `response.completed` event arrived.".to_string()
    })?;

    if let Some(error) = &response.error {
        return Err(format!(
            "{}: {}",
            error.code.as_deref().unwrap_or("api_error"),
            error.message
        ));
    }

    Ok(StreamedOpenAiResponse {
        response,
        reasoning: reasoning_summaries.into_values().collect(),
    })
}

fn process_stream_event(
    event_name: &Option<String>,
    data_lines: &[String],
    reasoning_summaries: &mut BTreeMap<usize, String>,
    observer: Option<&mut dyn StreamObserver>,
    completed_response: &mut Option<OpenAiResponse>,
) -> Result<(), String> {
    let data = data_lines.join("\n");
    if data == "[DONE]" {
        return Ok(());
    }

    let payload: Value = serde_json::from_str(&data)
        .map_err(|error| format!("stream event JSON parse error: {error}; data={data}"))?;
    let event_type = event_name
        .as_deref()
        .or_else(|| payload.get("type").and_then(Value::as_str))
        .unwrap_or_default();

    match event_type {
        "response.reasoning_summary_text.delta" => {
            let index = payload
                .get("summary_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let delta = payload
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default();
            reasoning_summaries
                .entry(index)
                .or_default()
                .push_str(delta);
            if let Some(observer) = observer {
                observer.on_reasoning_delta(delta);
            }
        }
        "response.reasoning_summary_text.done" => {
            let index = payload
                .get("summary_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            reasoning_summaries.insert(index, text.clone());
            if let Some(observer) = observer {
                observer.on_reasoning_done(&text);
            }
        }
        "response.reasoning_summary_part.done" => {
            let index = payload
                .get("summary_index")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let text = payload
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !text.is_empty() {
                reasoning_summaries.insert(index, text.clone());
                if let Some(observer) = observer {
                    observer.on_reasoning_done(&text);
                }
            }
        }
        "response.completed" => {
            let response_value = payload.get("response").cloned().unwrap_or(payload);
            let parsed: OpenAiResponse = serde_json::from_value(response_value)
                .map_err(|error| format!("response JSON parse error: {error}"))?;
            *completed_response = Some(parsed);
        }
        "response.failed" => {
            let response_value = payload.get("response").cloned().unwrap_or(payload.clone());
            let parsed: OpenAiResponse = serde_json::from_value(response_value)
                .map_err(|error| format!("failed response JSON parse error: {error}"))?;
            if let Some(error) = parsed.error {
                return Err(format!(
                    "{}: {}",
                    error.code.as_deref().unwrap_or("api_error"),
                    error.message
                ));
            }
            return Err("OpenAI response stream failed.".to_string());
        }
        "error" => {
            let message = payload
                .get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .or_else(|| payload.get("message").and_then(Value::as_str))
                .unwrap_or("OpenAI stream returned an error event.");
            return Err(message.to_string());
        }
        _ => {}
    }

    Ok(())
}

fn should_stop_after_retry(state: &mut AgentState, max_retries: u8, reason: &str) -> bool {
    state.retries += 1;
    state.record(format!(
        "Retry attempt {}/{} triggered because: {}",
        state.retries, max_retries, reason
    ));
    state.retries >= max_retries
}

fn decide_next_step_heuristic(
    config: &AgentConfig,
    tools: &[Tool],
    skills: &[Skill],
    state: &AgentState,
    user_input: &str,
) -> Decision {
    let input = user_input.trim();
    let lower = input.to_ascii_lowercase();

    if input.is_empty() {
        return Decision::Stop("empty user input".to_string());
    }

    if state.retries >= config.max_retries {
        return Decision::Stop("retry limit reached".to_string());
    }

    if let Some((tool_name, arguments)) = parse_explicit_tool_request(input) {
        if tools.iter().any(|tool| tool.is_named(&tool_name)) {
            return Decision::CallTool {
                tool_name,
                arguments,
                reason: "the request explicitly named a tool".to_string(),
            };
        }
    }

    if lower.contains("use skill") || lower.starts_with("skill:") {
        if let Some(skill_name) = find_skill_name_in_input(skills, &lower) {
            return Decision::CallSkill(
                skill_name,
                "the request explicitly named an installed skill".to_string(),
            );
        }
        if let Some(skill_name) = default_heuristic_skill(skills) {
            return Decision::CallSkill(
                skill_name,
                "the request explicitly asks to use a skill".to_string(),
            );
        }
    }

    if let Some(skill_name) = find_skill_name_in_input(skills, &lower) {
        return Decision::CallSkill(
            skill_name,
            "the request mentions a registered skill by name".to_string(),
        );
    }

    if lower.contains("retry") && state.retries < config.max_retries {
        return Decision::Retry("input explicitly requested a retry".to_string());
    }

    if tools.iter().any(|tool| tool.is_named("queue_ingress")) {
        return Decision::CallTool {
            tool_name: "queue_ingress".to_string(),
            arguments: json!({}),
            reason: "the main agent acts as the ingress layer and should queue inbound payloads"
                .to_string(),
        };
    }

    Decision::Finish(
        format!(
            "Ingress agent received `{}` but no queue_ingress tool is available.",
            input
        ),
        "the ingress queue tool was unavailable".to_string(),
    )
}

fn default_heuristic_skill(skills: &[Skill]) -> Option<String> {
    for preferred in ["planner", "summarize", "retry_once"] {
        if let Some(skill) = skills.iter().find(|skill| skill.is_named(preferred)) {
            return Some(skill.name().to_string());
        }
    }

    skills.first().map(|skill| skill.name().to_string())
}

fn find_skill_name_in_input(skills: &[Skill], lower_input: &str) -> Option<String> {
    let mut matches = skills
        .iter()
        .filter_map(|skill| {
            let candidate = skill.name().to_ascii_lowercase();
            lower_input
                .contains(&candidate)
                .then_some((candidate.len(), skill.name().to_string()))
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| right.0.cmp(&left.0));
    matches.into_iter().next().map(|(_, name)| name)
}

fn parse_explicit_tool_request(user_input: &str) -> Option<(String, Value)> {
    let trimmed = user_input.trim();
    let prefixes = ["use tool ", "tool: "];
    let remainder = prefixes
        .iter()
        .find_map(|prefix| {
            trimmed
                .to_ascii_lowercase()
                .strip_prefix(prefix)
                .map(|_| &trimmed[prefix.len()..])
        })?
        .trim();

    if remainder.is_empty() {
        return None;
    }

    if let Some((tool_name, raw_arguments)) = remainder.split_once(" with ") {
        let tool_name = tool_name.trim();
        if tool_name.is_empty() {
            return None;
        }
        let arguments = serde_json::from_str::<Value>(raw_arguments.trim()).ok()?;
        return Some((tool_name.to_string(), normalize_tool_arguments(arguments)));
    }

    let tool_name = remainder
        .split_whitespace()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    Some((tool_name.to_string(), json!({})))
}

fn normalize_tool_arguments(arguments: Value) -> Value {
    match arguments {
        Value::Object(_) => arguments,
        Value::Null => json!({}),
        other => json!({ "value": other }),
    }
}

fn parse_planner_tool_arguments(raw: &str, tool_name: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }

    let parsed = serde_json::from_str::<Value>(trimmed).map_err(|error| {
        format!(
            "Planner returned invalid JSON in `tool_arguments_json` for `{tool_name}`: {error}; raw={trimmed}"
        )
    })?;

    match parsed {
        Value::Object(_) => Ok(parsed),
        Value::Null => Ok(json!({})),
        other => Err(format!(
            "Planner returned non-object JSON in `tool_arguments_json` for `{tool_name}`: {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tools::{
        default_tools, extract_web_search_query, format_perplexity_response, PerplexityResponse,
    };
    use std::fs;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn heuristic_queues_plain_inbound_messages_by_default() {
        let decision = decide_next_step_heuristic(
            &AgentConfig {
                system_prompt: "system".to_string(),
                max_retries: MAX_RETRIES,
                default_model: DEFAULT_MODEL,
                fallback_model: FALLBACK_MODEL,
                planner_model: None,
            },
            &default_tools(),
            &built_in_skills(),
            &AgentState::new(),
            "Answer directly.",
        );

        assert!(matches!(
            decision,
            Decision::CallTool {
                tool_name,
                arguments,
                ..
            } if tool_name == "queue_ingress" && arguments == json!({})
        ));
    }

    #[test]
    fn calls_tool_when_requested() {
        let agent = MainAgent::new("system".to_string());
        let result = agent.run("Use tool word_count for this sentence.");

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.output, "Word count tool output: 6");
    }

    #[test]
    fn heuristic_explicit_tool_request_supports_json_arguments() {
        let agent = MainAgent::new("system".to_string());
        let result = agent.run("Use tool echo with {\"text\":\"from args\"}");

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.output, "Echo tool output: from args");
    }

    #[test]
    fn retries_once_then_recovers_for_retry_once_skill() {
        let agent = MainAgent::new("system".to_string());
        let result = agent.run("Use skill retry_once.");

        assert_eq!(result.status, RunStatus::Completed);
        assert_eq!(result.output, "Retry-once skill recovered successfully.");
        assert!(result
            .trace
            .iter()
            .any(|line| line.contains("Retry attempt 1/3")));
    }

    #[test]
    fn planner_skill_is_available_and_selected_by_name() {
        let decision = decide_next_step_heuristic(
            &AgentConfig {
                system_prompt: "system".to_string(),
                max_retries: MAX_RETRIES,
                default_model: DEFAULT_MODEL,
                fallback_model: FALLBACK_MODEL,
                planner_model: None,
            },
            &default_tools(),
            &built_in_skills(),
            &AgentState::new(),
            "Use skill planner to analyze the queue.",
        );

        assert!(matches!(
            decision,
            Decision::CallSkill(name, _) if name == "planner"
        ));
    }

    #[test]
    fn stops_after_three_retries() {
        let agent = MainAgent::new("system".to_string());
        let result = agent.run("retry please");

        assert_eq!(result.status, RunStatus::Stopped);
        assert!(result
            .trace
            .iter()
            .any(|line| line.contains("Retry attempt 3/3")));
    }

    #[test]
    fn converts_valid_planner_payload_to_tool_decision() {
        let payload = PlannerPayload {
            action: "tool".to_string(),
            tool_name: "echo".to_string(),
            tool_arguments_json: "{}".to_string(),
            skill_name: String::new(),
            answer: String::new(),
            reason: "tool needed".to_string(),
        };

        let decision = planner_payload_to_decision(&payload, &default_tools(), &built_in_skills())
            .expect("payload should convert");
        assert!(matches!(
            decision,
            Decision::CallTool {
                tool_name,
                arguments,
                ..
            } if tool_name == "echo" && arguments == json!({})
        ));
    }

    #[test]
    fn converts_valid_planner_payload_to_web_search_decision() {
        let payload = PlannerPayload {
            action: "tool".to_string(),
            tool_name: "web_search".to_string(),
            tool_arguments_json: "{\"query\":\"rust news\"}".to_string(),
            skill_name: String::new(),
            answer: String::new(),
            reason: "research needed".to_string(),
        };

        let decision = planner_payload_to_decision(&payload, &default_tools(), &built_in_skills())
            .expect("payload should convert");
        assert!(matches!(
            decision,
            Decision::CallTool {
                tool_name,
                arguments,
                ..
            } if tool_name == "web_search" && arguments == json!({ "query": "rust news" })
        ));
    }

    #[test]
    fn planner_schema_uses_stringified_tool_arguments_for_strict_json_schema() {
        let schema = planner_schema();
        let tool_arguments = schema
            .get("properties")
            .and_then(|properties| properties.get("tool_arguments_json"))
            .expect("planner schema should define tool_arguments_json");

        assert_eq!(tool_arguments.get("type"), Some(&json!("string")));
        assert!(tool_arguments.get("additionalProperties").is_none());
        assert!(tool_arguments.get("oneOf").is_none());
    }

    #[test]
    fn reasoning_messages_are_extracted_from_trace() {
        let trace = vec![
            "Decision: call tool `word_count` (count words)".to_string(),
            "Reasoning: Using tool `word_count` because count words".to_string(),
        ];

        assert_eq!(
            collect_reasoning_messages(&trace),
            vec!["Using tool `word_count` because count words".to_string()]
        );
    }

    #[test]
    fn extracts_output_text_from_openai_response() {
        let response: OpenAiResponse = serde_json::from_value(json!({
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Hello from the model."
                        }
                    ]
                }
            ]
        }))
        .expect("response should parse");

        assert_eq!(
            response.output_text().as_deref(),
            Some("Hello from the model.")
        );
    }

    #[test]
    fn parses_streamed_reasoning_summary_and_completed_response() {
        let stream = concat!(
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"summary_index\":0,\"delta\":\"Thinking\"}\n",
            "\n",
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"summary_index\":0,\"delta\":\" aloud\"}\n",
            "\n",
            "event: response.reasoning_summary_text.done\n",
            "data: {\"type\":\"response.reasoning_summary_text.done\",\"summary_index\":0,\"text\":\"Thinking aloud\"}\n",
            "\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"{\\\"action\\\":\\\"finish\\\",\\\"tool_name\\\":\\\"\\\",\\\"skill_name\\\":\\\"\\\",\\\"answer\\\":\\\"Hello\\\",\\\"reason\\\":\\\"direct\\\"}\"}]}]}\n",
            "\n"
        );

        let parsed = parse_streaming_openai_response(Cursor::new(stream.as_bytes()), None)
            .expect("stream should parse");

        assert_eq!(parsed.reasoning, vec!["Thinking aloud".to_string()]);
        assert_eq!(
            parsed.response.output_text().as_deref(),
            Some("{\"action\":\"finish\",\"tool_name\":\"\",\"skill_name\":\"\",\"answer\":\"Hello\",\"reason\":\"direct\"}")
        );
    }

    #[test]
    fn extracts_web_search_query_from_common_prefixes() {
        assert_eq!(
            extract_web_search_query("Use tool web_search to research the Rust 2024 roadmap."),
            "research the Rust 2024 roadmap."
        );
        assert_eq!(
            extract_web_search_query("Research: current TypeScript 6 roadmap"),
            "current TypeScript 6 roadmap"
        );
    }

    #[test]
    fn formats_perplexity_response_with_citations_and_results() {
        let response: PerplexityResponse = serde_json::from_value(json!({
            "model": "sonar-pro",
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "Rust remains popular for systems programming and performance-sensitive services."
                    }
                }
            ],
            "citations": [
                "https://example.com/rust-report",
                "https://example.com/rust-benchmarks"
            ],
            "search_results": [
                {
                    "title": "Rust Report",
                    "url": "https://example.com/rust-report",
                    "snippet": "Survey data about Rust adoption.",
                    "date": "2026-01-15"
                }
            ],
            "related_questions": [
                "How does Rust compare with Go for backend services?"
            ]
        }))
        .expect("response should parse");

        let formatted = format_perplexity_response("Rust adoption trends", &response);

        assert!(formatted.contains("Perplexity web research"));
        assert!(formatted.contains("Rust remains popular"));
        assert!(formatted.contains("Citations:"));
        assert!(formatted.contains("Related follow-up questions:"));
    }

    #[test]
    fn loads_installed_skills_from_repo_style_directory() {
        let temp_dir = unique_test_directory("skills-loader");
        let skills_root = temp_dir.join("skills");
        let skill_dir = skills_root.join("release-notes");
        fs::create_dir_all(&skill_dir).expect("skill directory should be created");
        fs::write(
            skill_dir.join(SKILL_FILE_NAME),
            "---\nname: release-notes\ndescription: Use when the user wants release notes.\n---\n\n# Release Notes\n\nWrite release notes.",
        )
        .expect("skill file should be written");

        let (skills, warnings) = load_skill_catalog_from_dir(&skills_root);

        let installed = skills
            .iter()
            .find(|skill| skill.is_named("release-notes"))
            .expect("installed skill should be discovered");
        assert_eq!(
            installed.description(),
            "Use when the user wants release notes."
        );
        assert!(warnings.is_empty());

        fs::remove_dir_all(&temp_dir).expect("temp directory should be removed");
    }

    #[test]
    fn heuristic_can_select_installed_skill_by_name() {
        let temp_dir = unique_test_directory("skills-heuristic");
        let skills_root = temp_dir.join("skills");
        let skill_dir = skills_root.join("incident-report");
        fs::create_dir_all(&skill_dir).expect("skill directory should be created");
        fs::write(
            skill_dir.join(SKILL_FILE_NAME),
            "---\nname: incident-report\ndescription: Use when the user wants an incident report.\n---\n\n# Incident Report\n",
        )
        .expect("skill file should be written");

        let (skills, warnings) = load_skill_catalog_from_dir(&skills_root);
        assert!(warnings.is_empty());

        let decision = decide_next_step_heuristic(
            &AgentConfig {
                system_prompt: "system".to_string(),
                max_retries: MAX_RETRIES,
                default_model: DEFAULT_MODEL,
                fallback_model: FALLBACK_MODEL,
                planner_model: None,
            },
            &default_tools(),
            &skills,
            &AgentState::new(),
            "Use skill incident-report for this postmortem.",
        );

        assert!(matches!(
            decision,
            Decision::CallSkill(name, _) if name == "incident-report"
        ));

        fs::remove_dir_all(&temp_dir).expect("temp directory should be removed");
    }

    #[test]
    fn loads_workspace_markdown_documents_from_uppercase_extension() {
        let temp_dir = unique_test_directory("workspace-markdown");
        let workspace_dir = temp_dir.join("Workspace");
        fs::create_dir_all(&workspace_dir).expect("workspace directory should be created");
        fs::write(
            workspace_dir.join("Agent.MD"),
            "# AGENT\n\n- goal: plan the latest queue item\n",
        )
        .expect("workspace markdown should be written");

        let documents = load_markdown_documents(&workspace_dir)
            .expect("workspace markdown documents should load");

        assert!(documents.iter().any(|(path, contents)| {
            path.file_name().and_then(|name| name.to_str()) == Some("Agent.MD")
                && contents.contains("plan the latest queue item")
        }));

        fs::remove_dir_all(&temp_dir).expect("temp directory should be removed");
    }

    #[test]
    fn queue_runner_continues_polling_after_non_interactive_stdin_closes() {
        assert!(should_continue_polling_after_stdin_close(false));
    }

    #[test]
    fn queue_runner_stops_after_interactive_stdin_closes() {
        assert!(!should_continue_polling_after_stdin_close(true));
    }

    fn unique_test_directory(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "agent-in-rust-{prefix}-{}-{}",
            std::process::id(),
            nanos
        ))
    }
}
