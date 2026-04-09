#[allow(non_snake_case)]
mod Tools;

mod agents;
mod mcp;
mod observability;
mod runtime_log;

#[allow(non_snake_case)]
mod mainAgent;

use crate::mainAgent::{
    MainAgent, QueueModeConfig, WaitModeConfig, DEFAULT_OPENAI_MODEL, DEFAULT_PERPLEXITY_MODEL,
    DEFAULT_SYSTEM_PROMPT,
};
use crate::agents::ingress_agent::{DEFAULT_CONVERSATION_HISTORY_FILE, DEFAULT_QUEUE_FILE};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const WORKSPACE_DIR: &str = "Workspace";
const IDENTITY_FILE_NAME: &str = "IDENTITY.md";
const DEFAULT_AGENT_NAME: &str = "Rust Agent";
const DEFAULT_AGENT_ICON: &str = "🤖";
const DEFAULT_AGENT_PURPOSE: &str = "Help the user as a simple Rust agent.";
const DEFAULT_USER_NAME: &str = "User";

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityProfile {
    agent_name: String,
    agent_icon: String,
    agent_purpose: String,
    user_name: String,
}

impl IdentityProfile {
    fn default_profile() -> Self {
        Self {
            agent_name: DEFAULT_AGENT_NAME.to_string(),
            agent_icon: DEFAULT_AGENT_ICON.to_string(),
            agent_purpose: DEFAULT_AGENT_PURPOSE.to_string(),
            user_name: DEFAULT_USER_NAME.to_string(),
        }
    }

    fn is_complete(&self) -> bool {
        !self.agent_name.trim().is_empty()
            && !self.agent_icon.trim().is_empty()
            && !self.agent_purpose.trim().is_empty()
            && !self.user_name.trim().is_empty()
    }

    fn with_fallbacks(&self) -> Self {
        let defaults = Self::default_profile();

        Self {
            agent_name: first_non_empty(&self.agent_name, &defaults.agent_name),
            agent_icon: first_non_empty(&self.agent_icon, &defaults.agent_icon),
            agent_purpose: first_non_empty(&self.agent_purpose, &defaults.agent_purpose),
            user_name: first_non_empty(&self.user_name, &defaults.user_name),
        }
    }

    fn prompt_label(&self) -> &str {
        if self.agent_name.trim().is_empty() {
            "agent"
        } else {
            self.agent_name.trim()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Run(RunOptions),
    Chat(ChatOptions),
    List,
    Help(HelpTopic),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunOptions {
    system_prompt: Option<String>,
    user_input: Option<String>,
    show_trace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatOptions {
    system_prompt: Option<String>,
    show_trace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HelpTopic {
    General,
    Run,
    Chat,
    List,
    Examples,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn first_non_empty(primary: &str, fallback: &str) -> String {
    if primary.trim().is_empty() {
        fallback.to_string()
    } else {
        primary.trim().to_string()
    }
}

fn sanitize_identity_value(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn identity_file_path() -> PathBuf {
    Path::new(WORKSPACE_DIR).join(IDENTITY_FILE_NAME)
}

fn parse_identity_markdown(contents: &str) -> Option<IdentityProfile> {
    let mut profile = IdentityProfile {
        agent_name: String::new(),
        agent_icon: String::new(),
        agent_purpose: String::new(),
        user_name: String::new(),
    };
    let mut found_any = false;

    for line in contents.lines() {
        let trimmed = line.trim();
        let Some(entry) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let Some((key, value)) = entry.split_once(':') else {
            continue;
        };

        let value = sanitize_identity_value(value);
        match key.trim() {
            "agent_name" => {
                profile.agent_name = value;
                found_any = true;
            }
            "agent_icon" => {
                profile.agent_icon = value;
                found_any = true;
            }
            "agent_purpose" => {
                profile.agent_purpose = value;
                found_any = true;
            }
            "user_name" => {
                profile.user_name = value;
                found_any = true;
            }
            _ => {}
        }
    }

    if found_any {
        Some(profile)
    } else {
        None
    }
}

fn render_identity_markdown(profile: &IdentityProfile) -> String {
    format!(
        "# IDENTITY\n\n\
This file stores the first-run identity settings for the agent workspace.\n\
The CLI reads these values on startup and reuses them on later runs.\n\n\
- agent_name: {}\n\
- agent_icon: {}\n\
- agent_purpose: {}\n\
- user_name: {}\n",
        sanitize_identity_value(&profile.agent_name),
        sanitize_identity_value(&profile.agent_icon),
        sanitize_identity_value(&profile.agent_purpose),
        sanitize_identity_value(&profile.user_name)
    )
}

fn load_identity_profile(path: &Path) -> io::Result<Option<IdentityProfile>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(parse_identity_markdown(&contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn save_identity_profile(path: &Path, profile: &IdentityProfile) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, render_identity_markdown(profile))
}

fn prompt_identity_profile(defaults: &IdentityProfile) -> io::Result<IdentityProfile> {
    Ok(IdentityProfile {
        agent_name: sanitize_identity_value(&read_prompt("Agent name", &defaults.agent_name)?),
        agent_icon: sanitize_identity_value(&read_prompt(
            "Agent icon/emoji",
            &defaults.agent_icon,
        )?),
        agent_purpose: sanitize_identity_value(&read_prompt(
            "Agent purpose",
            &defaults.agent_purpose,
        )?),
        user_name: sanitize_identity_value(&read_prompt(
            "What should the agent call you",
            &defaults.user_name,
        )?),
    })
}

fn ensure_identity_profile() -> io::Result<IdentityProfile> {
    let path = identity_file_path();
    let existing = load_identity_profile(&path)?;

    if let Some(profile) = existing.as_ref() {
        if profile.is_complete() {
            return Ok(profile.clone());
        }
    }

    let defaults = existing
        .unwrap_or_else(IdentityProfile::default_profile)
        .with_fallbacks();

    println!("Identity setup");
    println!(
        "Create the first-run profile for this agent. The answers will be saved to {}.",
        path.display()
    );

    let profile = prompt_identity_profile(&defaults)?;
    save_identity_profile(&path, &profile)?;
    println!("Saved identity profile to {}.", path.display());

    Ok(profile)
}

fn build_system_prompt_with_identity(base_prompt: &str, identity: &IdentityProfile) -> String {
    format!(
        "{base_prompt}\n\n\
Identity profile:\n\
- Agent name: {}\n\
- Agent icon: {}\n\
- Agent purpose: {}\n\
- Address the user as: {}\n",
        identity.agent_name, identity.agent_icon, identity.agent_purpose, identity.user_name
    )
}

fn parse_command(args: &[String]) -> Result<Command, CliError> {
    if args.is_empty() {
        return Ok(Command::Chat(ChatOptions {
            system_prompt: None,
            show_trace: false,
        }));
    }

    let mut command_name = args[0].as_str();
    let mut start_index = 1;

    if command_name.starts_with('-') {
        command_name = "run";
        start_index = 0;
    }

    match command_name {
        "run" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Run));
            }
            let (system_prompt, user_input, show_trace) = parse_common_flags(&args[start_index..])?;
            Ok(Command::Run(RunOptions {
                system_prompt,
                user_input,
                show_trace,
            }))
        }
        "chat" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Chat));
            }
            let (system_prompt, user_input, show_trace) = parse_common_flags(&args[start_index..])?;
            if user_input.is_some() {
                return Err(CliError::new(
                    "`chat` does not accept `--input`; type messages interactively instead.",
                ));
            }

            Ok(Command::Chat(ChatOptions {
                system_prompt,
                show_trace,
            }))
        }
        "list" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::List));
            }
            if !args[start_index..].is_empty() {
                return Err(CliError::new(
                    "`list` does not accept additional arguments.",
                ));
            }
            Ok(Command::List)
        }
        "help" => Ok(Command::Help(parse_help_topic(&args[start_index..])?)),
        "--help" | "-h" => Ok(Command::Help(HelpTopic::General)),
        other => Err(CliError::new(format!("Unknown command: {other}"))),
    }
}

fn parse_help_topic(args: &[String]) -> Result<HelpTopic, CliError> {
    match args {
        [] => Ok(HelpTopic::General),
        [topic] => match topic.as_str() {
            "run" => Ok(HelpTopic::Run),
            "chat" => Ok(HelpTopic::Chat),
            "list" => Ok(HelpTopic::List),
            "examples" => Ok(HelpTopic::Examples),
            other => Err(CliError::new(format!("Unknown help topic: {other}"))),
        },
        _ => Err(CliError::new(
            "Too many arguments for `help`. Use `help`, `help run`, `help chat`, `help list`, or `help examples`.",
        )),
    }
}

fn parse_common_flags(args: &[String]) -> Result<(Option<String>, Option<String>, bool), CliError> {
    let mut system_prompt = None;
    let mut user_input = None;
    let mut show_trace = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--system" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--system`."))?;
                system_prompt = Some(value.clone());
                index += 2;
            }
            "--input" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--input`."))?;
                user_input = Some(value.clone());
                index += 2;
            }
            "--trace" => {
                show_trace = true;
                index += 1;
            }
            unexpected => {
                return Err(CliError::new(format!("Unknown argument: {unexpected}")));
            }
        }
    }

    Ok((system_prompt, user_input, show_trace))
}

fn general_help_text(bin_name: &str) -> String {
    format!(
        "{bin_name} provides a simple CLI for interacting with the Rust ingress agent template.\n\n\
Usage:\n  {bin_name} chat [--system <prompt>] [--trace]\n  {bin_name} run [--system <prompt>] [--input <message>] [--trace]\n  {bin_name} list\n  {bin_name} help [run|chat|list|examples]\n\n\
Commands:\n  chat   Start an interactive session for developers or end users.\n  run    Start the ingress runner and optionally bootstrap one ingress payload.\n  list   Show available tools, skills, and model policy.\n  help   Show detailed command help and developer examples.\n\n\
Flags:\n  --system <prompt>  Override the system prompt.\n  --input <message>   Submit one bootstrap payload before waiting for more user input.\n  --trace             Print planner-agent trace output.\n\n\
Environment:\n  OPENAI_API_KEY           Enables the real OpenAI planner.\n  OPENAI_MODEL             Overrides the OpenAI model ID (default: {DEFAULT_OPENAI_MODEL}).\n  OPENAI_BASE_URL          Overrides the Responses API URL.\n  PERPLEXITY_API_KEY       Enables the Perplexity web_search tool.\n  PERPLEXITY_MODEL         Overrides the Perplexity model ID (default: {DEFAULT_PERPLEXITY_MODEL}).\n  PERPLEXITY_BASE_URL      Overrides the Perplexity API URL.\n  MCP_SERVERS              JSON array describing stdio MCP servers to load at startup.\n  AGENT_QUEUE_FILE         Overrides the ingress queue file path (default: {DEFAULT_QUEUE_FILE}).\n  AGENT_QUEUE_POLL_INTERVAL_MS Overrides how often `run` checks for unplanned queue work (default: 1000).\n  AGENT_WORKSPACE_DIR      Overrides the workspace markdown directory used for planner and installed skill context (default: Workspace).\n  AGENT_SYSTEM_PROMPT      Default system prompt when --system is omitted.\n  AGENT_USER_INPUT         Default input for `run` when --input is omitted.\n\n\
Skills:\n  - Installed skills are auto-discovered from `skills/<skill-name>/SKILL.md`.\n  - Repo-local installed skills appear in `list` and `/skills` automatically.\n\n\
Help topics:\n  {bin_name} help run\n  {bin_name} help chat\n  {bin_name} help list\n  {bin_name} help examples\n"
    )
}

fn run_help_text(bin_name: &str) -> String {
    format!(
        "Start the ingress runner.\n\n\
Usage:\n  {bin_name} run --input <message>\n  {bin_name} run --system <prompt> --input <message> --trace\n\n\
Developer notes:\n  - `run` accepts direct user ingress and also polls `ingress_queue.jsonl` for unplanned queued work.\n  - If `--input` or `AGENT_USER_INPUT` is provided, `run` first queues that payload as a bootstrap ingress submission.\n  - If `OPENAI_API_KEY` is set, the planner uses the OpenAI Responses API.\n  - The planner agent also picks up pre-existing and externally appended queue items, then writes next steps into `planner_queue.jsonl`.\n  - If `PERPLEXITY_API_KEY` is set, explicit `web_search` tool requests can still run grounded web research.\n  - If `MCP_SERVERS` is set, stdio MCP tools are discovered at startup and added to the catalog.\n  - `AGENT_QUEUE_FILE` overrides where queued ingress records are written.\n  - `AGENT_QUEUE_POLL_INTERVAL_MS` overrides how often the planner checks for unplanned queue work.\n  - `AGENT_WORKSPACE_DIR` overrides which workspace markdown files are injected into installed skills and the planner skill.\n  - `{DEFAULT_CONVERSATION_HISTORY_FILE}` is created on demand and reused for ingress context compaction.\n  - If `--system` is omitted, the CLI uses `AGENT_SYSTEM_PROMPT` or prompts for one.\n  - Installed skills under `skills/` are discovered automatically at startup and shown in the waiting roster.\n  - `--trace` prints planner-agent trace output for ingress events.\n  - Stop the ingress runner with `exit`, `quit`, `:q`, or EOF.\n\n\
Examples:\n  {bin_name} run\n  {bin_name} run --input \"Simple inbound text to queue.\" --trace\n  {bin_name} run --input '{{\"source\":\"whatsapp\",\"items\":[{{\"type\":\"text\",\"text\":\"Review this invoice\"}},{{\"type\":\"image\",\"path\":\"inbox/invoice.jpg\",\"notes\":\"customer upload\"}}]}}' --trace\n  AGENT_QUEUE_FILE=/tmp/ingress.jsonl {bin_name} run --input '{{\"source\":\"voice\",\"items\":[{{\"type\":\"audio\",\"path\":\"calls/voicemail.wav\",\"transcript\":\"Call me back about the order\"}}]}}'\n  OPENAI_MODEL=gpt-5.4 {bin_name} run --input '{{\"source\":\"api\",\"items\":[{{\"type\":\"text\",\"text\":\"Classify and queue this support escalation\"}}]}}' --trace\n"
    )
}

fn list_help_text(bin_name: &str) -> String {
    format!(
        "Show the current ingress-agent catalog.\n\n\
Usage:\n  {bin_name} list\n\n\
Developer notes:\n  - Prints the model policy plus registered tools and skills.\n  - The built-in `queue_ingress` tool is the default operational path for inbound payloads.\n  - Includes repo-local installed skills discovered from `skills/`.\n  - Shows whether the OpenAI planner is enabled from the current environment.\n"
    )
}

fn examples_help_text(bin_name: &str) -> String {
    format!(
        "Common ingress-agent workflows.\n\n\
Inspect the template surface:\n  {bin_name} list\n\n\
Queue a plain text inbound message:\n  {bin_name} run --input \"A new customer message arrived.\" --trace\n\n\
Trigger ingress plus planner queue generation:\n  {bin_name} run --input \"A new customer message arrived.\" --trace\n\n\
Queue a multimodal inbound message:\n  {bin_name} run --input '{{\"source\":\"mobile-app\",\"items\":[{{\"type\":\"text\",\"text\":\"Please inspect this damaged package\"}},{{\"type\":\"image\",\"path\":\"uploads/package.jpg\"}}]}}' --trace\n\n\
Run an OpenAI-backed ingress submission:\n  OPENAI_API_KEY=<your-key> {bin_name} run --input '{{\"source\":\"api\",\"items\":[{{\"type\":\"text\",\"text\":\"Classify and queue this escalated support case\"}}]}}' --trace\n\n\
Run grounded web research through Perplexity:\n  PERPLEXITY_API_KEY=<your-key> {bin_name} run --input \"Use tool web_search to research the current AI chip export rules in the US\" --trace\n\n\
Inspect MCP-discovered tools:\n  MCP_SERVERS='[{{\"name\":\"demo\",\"command\":\"/path/to/server\",\"args\":[]}}]' {bin_name} list\n\n\
Open an interactive session:\n  {bin_name} chat --trace\n\n\
Install a repo-local skill and run it:\n  mkdir -p skills/my-skill && $EDITOR skills/my-skill/SKILL.md\n  {bin_name} list\n  {bin_name} run --input \"Use skill my-skill to help with this task.\" --trace\n\n\
Use environment defaults:\n  AGENT_SYSTEM_PROMPT=\"You are an ingress triage agent.\" AGENT_USER_INPUT='{{\"source\":\"cli\",\"items\":[{{\"type\":\"text\",\"text\":\"Summarize the template.\"}}]}}' {bin_name} run\n"
    )
}

fn help_text(bin_name: &str, topic: &HelpTopic) -> String {
    match topic {
        HelpTopic::General => general_help_text(bin_name),
        HelpTopic::Run => run_help_text(bin_name),
        HelpTopic::Chat => format!(
            "Start an interactive ingress CLI session.\n\n\
Usage:\n  {bin_name} chat\n  {bin_name} chat --system <prompt> --trace\n\n\
Developer notes:\n  - This delegates to the persistent wait loop owned by `mainAgent`.\n  - If `OPENAI_API_KEY` is set, the agent keeps multi-turn history locally and sends it to the OpenAI planner on each turn.\n  - Repo and workspace markdown plus persisted conversation history are compacted into the planner context.\n  - Installed skills under `skills/` appear in `/skills` automatically.\n  - `/trace` toggles execution trace output while the session is running.\n  - `/system`, `/tools`, and `/skills` inspect the active agent configuration.\n"
        ),
        HelpTopic::List => list_help_text(bin_name),
        HelpTopic::Examples => examples_help_text(bin_name),
    }
}

fn print_catalog(agent: &MainAgent) {
    println!("Model policy:");
    println!("- default: {}", agent.default_model());
    println!("- fallback: {}", agent.fallback_model());
    println!("- planner backend: {}", agent.planner_backend_label());
    println!(
        "- ingress queue file: {}",
        crate::agents::ingress_agent::resolve_queue_path().display()
    );

    if !agent.mcp_servers().is_empty() {
        println!("\nLoaded MCP servers:");
        for server in agent.mcp_servers() {
            println!("- {}", server);
        }
    }

    println!("\nAvailable tools:");
    for tool in agent.tools() {
        println!("- {}: {}", tool.name(), tool.description());
    }

    println!("\nAvailable skills:");
    for skill in agent.skills() {
        println!("- {}: {}", skill.name(), skill.description());
    }
}

fn read_prompt(label: &str, default: &str) -> io::Result<String> {
    print!("{label} [{default}]: ");
    io::stdout().flush()?;

    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer)?;
    let trimmed = buffer.trim();

    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn resolve_system_prompt(explicit: Option<String>) -> io::Result<String> {
    if let Some(value) = explicit {
        return Ok(value);
    }

    match env::var("AGENT_SYSTEM_PROMPT") {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => read_prompt("System prompt", DEFAULT_SYSTEM_PROMPT),
    }
}

fn resolve_bootstrap_input(explicit: Option<String>) -> Option<String> {
    if let Some(value) = explicit {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    match env::var("AGENT_USER_INPUT") {
        Ok(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => None,
    }
}

fn run_once(options: RunOptions) -> io::Result<()> {
    let identity = ensure_identity_profile()?;
    let system_prompt = build_system_prompt_with_identity(
        &resolve_system_prompt(options.system_prompt)?,
        &identity,
    );
    runtime_log::info("main", "starting ingress runner mode");
    let agent = MainAgent::from_env(system_prompt)?;
    let bootstrap_input = resolve_bootstrap_input(options.user_input);
    agent.wait_for_queue(
        QueueModeConfig {
            agent_name: identity.agent_name.clone(),
            agent_icon: identity.agent_icon.clone(),
            show_trace: options.show_trace,
        },
        bootstrap_input.as_deref(),
    )
}

fn run_chat(options: ChatOptions) -> io::Result<()> {
    let identity = ensure_identity_profile()?;
    let system_prompt = build_system_prompt_with_identity(
        &resolve_system_prompt(options.system_prompt)?,
        &identity,
    );
    runtime_log::info("main", "starting interactive chat mode");
    let agent = MainAgent::from_env(system_prompt)?;
    agent.wait_for_context(WaitModeConfig {
        prompt_label: identity.prompt_label().to_string(),
        agent_name: identity.agent_name.clone(),
        agent_icon: identity.agent_icon.clone(),
        user_name: identity.user_name.clone(),
        show_trace: options.show_trace,
        bin_name: "agent_in_rust".to_string(),
    })
}

fn main() -> io::Result<()> {
    let _ = dotenvy::dotenv();
    runtime_log::info("main", "process booted");

    let args: Vec<String> = env::args().skip(1).collect();
    let bin_name = env::args()
        .next()
        .unwrap_or_else(|| "agent_in_rust".to_string());

    match parse_command(&args) {
        Ok(Command::Run(options)) => run_once(options),
        Ok(Command::Chat(options)) => run_chat(options),
        Ok(Command::List) => {
            let system_prompt = env::var("AGENT_SYSTEM_PROMPT")
                .unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.to_string());
            let agent = MainAgent::from_env(system_prompt)?;
            print_catalog(&agent);
            Ok(())
        }
        Ok(Command::Help(topic)) => {
            println!("{}", help_text(&bin_name, &topic));
            Ok(())
        }
        Err(error) => {
            eprintln!("CLI error: {error}\n");
            eprintln!("{}", help_text(&bin_name, &HelpTopic::General));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_chat_when_no_args_are_provided() {
        let command = parse_command(&[]).expect("command should parse");

        assert_eq!(
            command,
            Command::Chat(ChatOptions {
                system_prompt: None,
                show_trace: false,
            })
        );
    }

    #[test]
    fn parses_run_command_flags() {
        let args = vec![
            "run".to_string(),
            "--system".to_string(),
            "planner".to_string(),
            "--input".to_string(),
            "Use tool echo".to_string(),
            "--trace".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Run(RunOptions {
                system_prompt: Some("planner".to_string()),
                user_input: Some("Use tool echo".to_string()),
                show_trace: true,
            })
        );
    }

    #[test]
    fn rejects_chat_input_flag() {
        let args = vec![
            "chat".to_string(),
            "--input".to_string(),
            "unexpected".to_string(),
        ];

        let error = parse_command(&args).expect_err("chat with --input should fail");
        assert!(error.message.contains("`chat` does not accept `--input`"));
    }

    #[test]
    fn parses_help_topic_for_run() {
        let args = vec!["help".to_string(), "run".to_string()];

        let command = parse_command(&args).expect("help run should parse");
        assert_eq!(command, Command::Help(HelpTopic::Run));
    }

    #[test]
    fn parses_run_help_flag_as_run_help_topic() {
        let args = vec!["run".to_string(), "--help".to_string()];

        let command = parse_command(&args).expect("run --help should parse");
        assert_eq!(command, Command::Help(HelpTopic::Run));
    }

    #[test]
    fn parses_identity_markdown_entries() {
        let profile = parse_identity_markdown(
            "# IDENTITY\n\
\n\
- agent_name: Aya\n\
- agent_icon: 🤖\n\
- agent_purpose: Help with setup.\n\
- user_name: David\n",
        )
        .expect("identity file should parse");

        assert_eq!(
            profile,
            IdentityProfile {
                agent_name: "Aya".to_string(),
                agent_icon: "🤖".to_string(),
                agent_purpose: "Help with setup.".to_string(),
                user_name: "David".to_string(),
            }
        );
    }

    #[test]
    fn renders_identity_markdown_with_saved_values() {
        let profile = IdentityProfile {
            agent_name: "Aya".to_string(),
            agent_icon: "🛠️".to_string(),
            agent_purpose: "Build Rust agents.".to_string(),
            user_name: "David".to_string(),
        };

        let rendered = render_identity_markdown(&profile);

        assert!(rendered.contains("- agent_name: Aya"));
        assert!(rendered.contains("- agent_icon: 🛠️"));
        assert!(rendered.contains("- agent_purpose: Build Rust agents."));
        assert!(rendered.contains("- user_name: David"));
    }

    #[test]
    fn appends_identity_to_system_prompt() {
        let profile = IdentityProfile {
            agent_name: "Aya".to_string(),
            agent_icon: "🤖".to_string(),
            agent_purpose: "Guide the user.".to_string(),
            user_name: "David".to_string(),
        };

        let prompt = build_system_prompt_with_identity("Base prompt.", &profile);

        assert!(prompt.contains("Base prompt."));
        assert!(prompt.contains("Agent name: Aya"));
        assert!(prompt.contains("Address the user as: David"));
    }
}
