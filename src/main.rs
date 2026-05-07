#[allow(non_snake_case)]
mod Tools;

mod bus;
mod channels;
mod concurrency;
mod dispatch;
mod evals;
mod mcp;
mod memory_agent;
mod planner;
mod observability;
mod prompt_layers;
mod router;
mod runtime_log;
mod scheduler;
mod worker;

#[allow(non_snake_case)]
mod mainAgent;

use crate::bus::{Bus, InProcessBus};
use crate::channels::cli::{CliChannel, CliInvocation};
use crate::channels::ChannelAdapter;
use crate::dispatch::CHILD_SUBAGENT_COMMAND;
use crate::mainAgent::{
    run_spawned_subagent_child, MainAgent, SkillCreateRequest, SkillInstallRequest, WaitModeConfig,
    DEFAULT_SYSTEM_PROMPT,
};
use crate::router::{DefaultRouter, RouteTarget, Router};
use crate::worker::{MainAgentWorker, WorkerRequest, WorkerRuntime};
use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SYSTEM_PROMPT_DIR: &str = "system_prompt";
const PUBLIC_BINARY_NAME: &str = "agent-in-rust";
const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASE_TAG: Option<&str> = option_env!("AGENT_IN_RUST_RELEASE_TAG");

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Version,
    Session(SessionOptions),
    List,
    Init,
    Compact,
    Eval(EvalOptions),
    Skills(SkillsCommand),
    Help(HelpTopic),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SkillsCommand {
    List,
    Create(SkillCreateOptions),
    Install(SkillInstallOptions),
    Show(SkillShowOptions),
    Validate(SkillValidateOptions),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillCreateOptions {
    name: String,
    description: String,
    scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillInstallOptions {
    source: String,
    skill_name: Option<String>,
    scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillShowOptions {
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillValidateOptions {
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionOptions {
    system_prompt: Option<String>,
    user_input: Option<String>,
    show_trace: bool,
    one_shot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvalOptions {
    fixtures_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HelpTopic {
    General,
    Session,
    Run,
    List,
    Skills,
    Init,
    Compact,
    Eval,
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

fn parse_command(args: &[String]) -> Result<Command, CliError> {
    if args.is_empty() {
        return Ok(Command::Session(SessionOptions {
            system_prompt: None,
            user_input: None,
            show_trace: false,
            one_shot: false,
        }));
    }

    let mut command_name = args[0].as_str();
    let mut start_index = 1;
    if command_name.starts_with('-') {
        if matches!(command_name, "--version" | "-V") {
            return Ok(Command::Version);
        }
        if matches!(command_name, "--help" | "-h") {
            return Ok(Command::Help(HelpTopic::General));
        }
        command_name = "session";
        start_index = 0;
    }

    match command_name {
        "version" => Ok(Command::Version),
        "session" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Session));
            }
            let (system_prompt, user_input, show_trace) = parse_common_flags(&args[start_index..])?;
            Ok(Command::Session(SessionOptions {
                system_prompt,
                user_input,
                show_trace,
                one_shot: false,
            }))
        }
        "chat" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Session));
            }
            let (system_prompt, user_input, show_trace) = parse_common_flags(&args[start_index..])?;
            if user_input.is_some() {
                return Err(CliError::new(
                    "`chat` does not accept `--input`; use `run --input ...` for one-shot prompts.",
                ));
            }
            Ok(Command::Session(SessionOptions {
                system_prompt,
                user_input: None,
                show_trace,
                one_shot: false,
            }))
        }
        "run" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Run));
            }
            let (system_prompt, user_input, show_trace) = parse_common_flags(&args[start_index..])?;
            Ok(Command::Session(SessionOptions {
                system_prompt,
                user_input,
                show_trace,
                one_shot: true,
            }))
        }
        "list" => Ok(Command::List),
        "init" => Ok(Command::Init),
        "compact" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Compact));
            }
            if args[start_index..].is_empty() {
                Ok(Command::Compact)
            } else {
                Err(CliError::new(
                    "`compact` does not accept positional arguments or flags.",
                ))
            }
        }
        "eval" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Eval));
            }
            Ok(Command::Eval(parse_eval_options(&args[start_index..])?))
        }
        "skills" => {
            if args[start_index..]
                .iter()
                .any(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(Command::Help(HelpTopic::Skills));
            }
            Ok(Command::Skills(parse_skills_command(&args[start_index..])?))
        }
        "help" => Ok(Command::Help(parse_help_topic(&args[start_index..])?)),
        "--version" | "-V" => Ok(Command::Version),
        "--help" | "-h" => Ok(Command::Help(HelpTopic::General)),
        other => Err(CliError::new(format!("Unknown command: {other}"))),
    }
}

fn version_text(bin_name: &str, package_version: &str, release_tag: Option<&str>) -> String {
    match release_tag {
        Some(tag) if !tag.trim().is_empty() => format!("{bin_name} {package_version} (release tag {tag})"),
        _ => format!("{bin_name} {package_version}"),
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

fn parse_help_topic(args: &[String]) -> Result<HelpTopic, CliError> {
    match args {
        [] => Ok(HelpTopic::General),
        [topic] => match topic.as_str() {
            "session" | "chat" => Ok(HelpTopic::Session),
            "run" => Ok(HelpTopic::Run),
            "list" => Ok(HelpTopic::List),
            "skills" => Ok(HelpTopic::Skills),
            "init" => Ok(HelpTopic::Init),
            "compact" => Ok(HelpTopic::Compact),
            "eval" => Ok(HelpTopic::Eval),
            other => Err(CliError::new(format!("Unknown help topic: {other}"))),
        },
        _ => Err(CliError::new(
            "Too many arguments for `help`. Use `help`, `help session`, `help run`, `help list`, `help skills`, `help init`, `help compact`, or `help eval`.",
        )),
    }
}

fn parse_eval_options(args: &[String]) -> Result<EvalOptions, CliError> {
    let mut fixtures_dir = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--fixtures" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--fixtures`."))?;
                fixtures_dir = Some(value.clone());
                index += 2;
            }
            unexpected => {
                return Err(CliError::new(format!("Unknown argument: {unexpected}")));
            }
        }
    }

    Ok(EvalOptions { fixtures_dir })
}

fn parse_skills_command(args: &[String]) -> Result<SkillsCommand, CliError> {
    match args {
        [] => Ok(SkillsCommand::List),
        [subcommand] if subcommand == "list" => Ok(SkillsCommand::List),
        [subcommand, rest @ ..] if subcommand == "create" => {
            Ok(SkillsCommand::Create(parse_skill_create_options(rest)?))
        }
        [subcommand, rest @ ..] if subcommand == "install" => {
            Ok(SkillsCommand::Install(parse_skill_install_options(rest)?))
        }
        [subcommand, rest @ ..] if subcommand == "show" => {
            Ok(SkillsCommand::Show(parse_skill_show_options(rest)?))
        }
        [subcommand, rest @ ..] if subcommand == "validate" => {
            Ok(SkillsCommand::Validate(parse_skill_validate_options(rest)?))
        }
        [other, ..] => Err(CliError::new(format!(
            "Unknown `skills` subcommand: {other}"
        ))),
    }
}

fn parse_skill_create_options(args: &[String]) -> Result<SkillCreateOptions, CliError> {
    let mut name = None;
    let mut description = None;
    let mut scope = "project".to_string();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--name" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--name`."))?;
                name = Some(value.clone());
                index += 2;
            }
            "--description" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--description`."))?;
                description = Some(value.clone());
                index += 2;
            }
            "--scope" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--scope`."))?;
                scope = value.clone();
                index += 2;
            }
            unexpected => {
                return Err(CliError::new(format!("Unknown argument: {unexpected}")));
            }
        }
    }

    Ok(SkillCreateOptions {
        name: name.ok_or_else(|| CliError::new("`skills create` requires `--name`."))?,
        description: description
            .ok_or_else(|| CliError::new("`skills create` requires `--description`."))?,
        scope,
    })
}

fn parse_skill_install_options(args: &[String]) -> Result<SkillInstallOptions, CliError> {
    let mut source = None;
    let mut skill_name = None;
    let mut scope = "project".to_string();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--source`."))?;
                source = Some(value.clone());
                index += 2;
            }
            "--skill" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--skill`."))?;
                skill_name = Some(value.clone());
                index += 2;
            }
            "--scope" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--scope`."))?;
                scope = value.clone();
                index += 2;
            }
            unexpected => {
                return Err(CliError::new(format!("Unknown argument: {unexpected}")));
            }
        }
    }

    Ok(SkillInstallOptions {
        source: source.ok_or_else(|| CliError::new("`skills install` requires `--source`."))?,
        skill_name,
        scope,
    })
}

fn parse_skill_show_options(args: &[String]) -> Result<SkillShowOptions, CliError> {
    let mut name = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--name" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--name`."))?;
                name = Some(value.clone());
                index += 2;
            }
            unexpected => {
                return Err(CliError::new(format!("Unknown argument: {unexpected}")));
            }
        }
    }

    Ok(SkillShowOptions {
        name: name.ok_or_else(|| CliError::new("`skills show` requires `--name`."))?,
    })
}

fn parse_skill_validate_options(args: &[String]) -> Result<SkillValidateOptions, CliError> {
    let mut name = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--name" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::new("Missing value for `--name`."))?;
                name = Some(value.clone());
                index += 2;
            }
            unexpected => {
                return Err(CliError::new(format!("Unknown argument: {unexpected}")));
            }
        }
    }

    Ok(SkillValidateOptions { name })
}

fn help_text(bin_name: &str, topic: &HelpTopic) -> String {
    match topic {
        HelpTopic::General => format!(
            "{bin_name} runs a Claude-style local coding session.\n\nDefault entrypoint:\n  {bin_name}                Start chatting\n  {bin_name} --trace        Start chatting and print the execution trace\n\nOther commands:\n  {bin_name} --version\n  {bin_name} version\n  {bin_name} session [--system <prompt>] [--trace]\n  {bin_name} run --input <prompt> [--system <prompt>] [--trace]\n  {bin_name} list\n  {bin_name} compact\n  {bin_name} eval [--fixtures <dir>]\n  {bin_name} skills [list|show|validate|create|install] ...\n  {bin_name} init\n  {bin_name} help [session|run|list|skills|init|compact|eval]\n  {bin_name} help eval\n\nNotes:\n  - Running `{bin_name}` with no command starts the interactive chat session.\n  - `--version` prints a stable packaged verification string.\n  - The default system prompt loads from files in `system_prompt/` unless `--system` or `AGENT_SYSTEM_PROMPT` overrides it.\n  - Project memory loads from `CLAUDE.md` and imported `@path` files.\n  - Short-term memory resumes from `Workspace/short-term.json` when present.\n  - Final LLM request context snapshots are printed before planner calls and appended to `history/context_history.json`.\n  - Long-term memory uses Mem0 when `MEM0_API_KEY` is configured and falls back to `Workspace/MEMORY.md` otherwise.\n  - Project subagents load from `.claude/agents/*.md`.\n  - Skills load from `.claude/skills/<name>/SKILL.md` and `~/.claude/skills/<name>/SKILL.md`.\n  - Project slash commands load from `.claude/commands/*.md`.\n  - MCP tools remain available when `MCP_SERVERS` is configured.\n"
        ),
        HelpTopic::Session => format!(
            "Start an interactive Claude-style session.\n\nUsage:\n  {bin_name}\n  {bin_name} --trace\n  {bin_name} session\n  {bin_name} session --system <prompt> --trace\n  {bin_name} chat\n\nThe default system prompt is assembled from `system_prompt/` when no override is provided.\n\nIn-session commands:\n  /help\n  /agents\n  /skills\n  /memory\n  /remember <note>\n  /model\n  /clear\n  /compact\n  /mcp\n  /review [task]\n  /skill <name> [task]\n  /init\n  /agent <name> <task>\n  /trace\n  /exit\n"
        ),
        HelpTopic::Run => format!(
            "Run a one-shot prompt through the Claude-style runtime.\n\nUsage:\n  {bin_name} run --input \"Plan the refactor\"\n  {bin_name} run --input \"Use tool web_search_tool with {{\\\"query\\\":\\\"latest Rust 2026 edition updates\\\"}}\" --trace\n"
        ),
        HelpTopic::List => format!(
            "Show the currently loaded Claude-style project surface.\n\nUsage:\n  {bin_name} list\n"
        ),
        HelpTopic::Skills => format!(
            "Manage reusable skills.\n\nUsage:\n  {bin_name} skills\n  {bin_name} skills list\n  {bin_name} skills show --name <name>\n  {bin_name} skills validate [--name <name>]\n  {bin_name} skills create --name <name> --description <text> [--scope project|user]\n  {bin_name} skills install --source <path|owner/repo|owner/repo/skill|github-url|skills.sh-url> [--skill <name>] [--scope project|user]\n"
        ),
        HelpTopic::Init => format!(
            "Scaffold Claude-style project files.\n\nUsage:\n  {bin_name} init\n\nThis creates `CLAUDE.md`, `system_prompt/system_prompt.md`, `.claude/agents/*.md`, `.claude/skills/ship-small/SKILL.md`, `.claude/commands/review.md`, and the fallback file `Workspace/MEMORY.md` when missing.\n"
        ),
        HelpTopic::Compact => format!(
            "Compact stored short-term context.\n\nUsage:\n  {bin_name} compact\n\nThis loads `Workspace/short-term.json`, summarizes older session turns into the compacted summary, retains the most recent turns, and writes the compacted snapshot back to disk.\n"
        ),
        HelpTopic::Eval => format!(
            "Run planner eval fixtures against the current harness.\n\nUsage:\n  {bin_name} eval\n  {bin_name} eval --fixtures evals/fixtures\n\nThis loads JSON fixtures, asks the current planner what action it would take next, and reports pass/fail for each expected decision.\n"
        ),
    }
}

fn load_system_prompt_from_dir(system_prompt_dir: &Path) -> io::Result<String> {
    let mut files = fs::read_dir(system_prompt_dir)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|entry| {
            entry
                .file_type()
                .map(|file_type| file_type.is_file())
                .unwrap_or(false)
                && !entry.file_name().to_string_lossy().starts_with('.')
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());

    if files.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "No system prompt files found in `{}`.",
                system_prompt_dir.display()
            ),
        ));
    }

    let sections = files
        .into_iter()
        .map(|entry| fs::read_to_string(entry.path()).map(|contents| contents.trim().to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|contents| !contents.is_empty())
        .collect::<Vec<_>>();

    if sections.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "System prompt files in `{}` were empty.",
                system_prompt_dir.display()
            ),
        ));
    }

    Ok(sections.join("\n\n"))
}

fn resolve_system_prompt_from_sources(
    explicit: Option<String>,
    env_override: Option<String>,
    system_prompt_dir: &Path,
) -> io::Result<String> {
    if let Some(prompt) = explicit
        .or(env_override)
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(prompt);
    }

    match load_system_prompt_from_dir(system_prompt_dir) {
        Ok(prompt) => Ok(prompt),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(DEFAULT_SYSTEM_PROMPT.to_string())
        }
        Err(error) => Err(error),
    }
}

fn resolve_system_prompt(explicit: Option<String>) -> io::Result<String> {
    resolve_system_prompt_from_sources(
        explicit,
        env::var("AGENT_SYSTEM_PROMPT").ok(),
        Path::new(SYSTEM_PROMPT_DIR),
    )
}

fn print_catalog(agent: &MainAgent) {
    println!("Model policy:");
    println!("- default: {}", agent.default_model());
    println!("- fallback: {}", agent.fallback_model());
    println!("- planner backend: {}", agent.planner_backend_label());
    println!("- system prompt: {}", agent.system_prompt());

    println!("\nObservability:");
    println!(
        "- enabled: {}",
        if agent.observability_enabled() {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "- targets: {}",
        if agent.observability_targets().is_empty() {
            "none".to_string()
        } else {
            agent.observability_targets().join(", ")
        }
    );
    if agent.observability_warnings().is_empty() {
        println!("- warnings: none");
    } else {
        for warning in agent.observability_warnings() {
            println!("- warning: {warning}");
        }
    }

    println!("\nMemory:");
    for source in agent.memory_sources() {
        println!("- [{}] {}", source.scope, source.path.display());
    }

    println!("\nSubagents:");
    for subagent in agent.subagents() {
        println!(
            "- {} [{}]: {}",
            subagent.name, subagent.scope, subagent.description
        );
    }

    println!("\nSkills:");
    for skill in agent.skills() {
        println!(
            "- {} [{}]: {} (allowed_tools: {})",
            skill.name,
            skill.scope,
            skill.description,
            skill
                .allowed_tools
                .as_ref()
                .map(|items| items.join(", "))
                .unwrap_or_else(|| "inherit all visible tools".to_string())
        );
    }
    println!("{}", agent.validate_skills(None));

    println!("\nCommands:");
    for command in agent.command_summaries() {
        println!("- {command}");
    }

    println!("\nTools:");
    for tool in agent.tools() {
        println!(
            "- {}: {} | planner={}",
            tool.name(),
            tool.description(),
            tool.planner_description()
        );
    }

    if !agent.mcp_servers().is_empty() {
        println!("\nMCP servers:");
        for server in agent.mcp_servers() {
            println!("- {}", server);
        }
    }
}

fn log_observability_status(agent: &MainAgent) {
    if agent.observability_enabled() {
        runtime_log::info(
            "observability",
            format!(
                "exporters enabled: {}",
                agent.observability_targets().join(", ")
            ),
        );
    } else {
        runtime_log::info(
            "observability",
            "exporters disabled; no OTLP targets configured",
        );
    }

    for warning in agent.observability_warnings() {
        runtime_log::warn("observability", warning);
    }
}

fn run_session(agent: &mut MainAgent, options: SessionOptions) -> io::Result<()> {
    let channel = CliChannel::new(CliInvocation::new(
        options.user_input.clone(),
        options.show_trace,
        options.one_shot,
    ));
    let mut bus = InProcessBus::default();
    let event = channel
        .into_inbound_event()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "`run` requires `--input`."))?;
    bus.publish_inbound(event)?;
    let request_event = bus.forward_inbound().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Other,
            "CLI channel did not enqueue an inbound event.",
        )
    })?;
    let route = DefaultRouter::new().route(&request_event)?;

    match route.target {
        RouteTarget::MainWorker => {
            if options.one_shot {
                let result =
                    MainAgentWorker::new(agent).execute(WorkerRequest::new(request_event))?;
                println!("{}", result.output);
                println!("\n{}", result.usage_summary);
                if options.show_trace {
                    println!("\nTrace:");
                    for entry in &result.trace {
                        println!("- {entry}");
                    }
                }
                return Ok(());
            }

            let request = WorkerRequest::new(request_event).with_wait_mode(WaitModeConfig {
                prompt_label: "claude".to_string(),
                show_trace: options.show_trace,
                bin_name: PUBLIC_BINARY_NAME.to_string(),
            });
            MainAgentWorker::new(agent).execute(request)?;
        }
    }

    Ok(())
}

fn build_eval_suite(
    agent: &MainAgent,
    options: EvalOptions,
) -> io::Result<evals::PlannerEvalSuiteResult> {
    let fixture_dir = options
        .fixtures_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("evals/fixtures"));
    let fixtures = evals::load_eval_fixtures(&fixture_dir)?;
    if fixtures.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("No eval fixtures found in {}.", fixture_dir.display()),
        ));
    }

    let cases = fixtures
        .into_iter()
        .map(|fixture| {
            let actual = agent.preview_planner_decision(&fixture.user_input, &fixture.observations);
            let passed = fixture.expected.matches(&actual);
            evals::PlannerEvalCaseResult {
                fixture,
                actual,
                passed,
            }
        })
        .collect();
    Ok(evals::PlannerEvalSuiteResult { fixture_dir, cases })
}

fn run_evals(agent: &MainAgent, options: EvalOptions) -> io::Result<()> {
    let suite = build_eval_suite(agent, options)?;
    println!("{}", suite.render());
    if suite.failed() > 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("{} planner eval(s) failed.", suite.failed()),
        ));
    }

    Ok(())
}

fn main() -> io::Result<()> {
    let _ = dotenvy::dotenv();

    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(|value| value.as_str()) == Some(CHILD_SUBAGENT_COMMAND) {
        return run_spawned_subagent_child();
    }

    runtime_log::info("main", "process booted");
    let bin_name = env::args()
        .next()
        .unwrap_or_else(|| PUBLIC_BINARY_NAME.to_string());

    match parse_command(&args) {
        Ok(Command::Version) => {
            println!("{}", version_text(PUBLIC_BINARY_NAME, PACKAGE_VERSION, RELEASE_TAG));
            Ok(())
        }
        Ok(Command::Session(options)) => {
            let system_prompt = resolve_system_prompt(options.system_prompt.clone())?;
            let mut agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            run_session(&mut agent, options)
        }
        Ok(Command::List) => {
            let system_prompt = resolve_system_prompt(None)?;
            let agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            print_catalog(&agent);
            Ok(())
        }
        Ok(Command::Init) => {
            let system_prompt = resolve_system_prompt(None)?;
            let mut agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            let created = agent.init_project_files()?;
            if created.is_empty() {
                println!("Claude-style project files already exist.");
            } else {
                println!("Created:");
                for path in created {
                    println!("- {}", path.display());
                }
            }
            Ok(())
        }
        Ok(Command::Compact) => {
            let system_prompt = resolve_system_prompt(None)?;
            let mut agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            let result = agent.compact_context()?;
            println!("{}", result.render());
            Ok(())
        }
        Ok(Command::Eval(options)) => {
            let system_prompt = resolve_system_prompt(None)?;
            let agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            run_evals(&agent, options)
        }
        Ok(Command::Skills(command)) => {
            let system_prompt = resolve_system_prompt(None)?;
            let mut agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            match command {
                SkillsCommand::List => {
                    for skill in agent.skills() {
                        println!(
                            "- {} [{}] {} ({})",
                            skill.name,
                            skill.scope,
                            skill.description,
                            skill.source_path.display()
                        );
                    }
                    println!("{}", agent.validate_skills(None));
                    Ok(())
                }
                SkillsCommand::Create(options) => {
                    let path = agent.create_skill(SkillCreateRequest {
                        name: options.name,
                        description: options.description,
                        scope: options.scope,
                    })?;
                    println!("Created skill: {}", path.display());
                    Ok(())
                }
                SkillsCommand::Install(options) => {
                    let path = agent.install_skill(SkillInstallRequest {
                        source: options.source,
                        skill_name: options.skill_name,
                        scope: options.scope,
                    })?;
                    println!("Installed skill: {}", path.display());
                    Ok(())
                }
                SkillsCommand::Show(options) => {
                    println!("{}", agent.show_skill(&options.name)?);
                    Ok(())
                }
                SkillsCommand::Validate(options) => {
                    println!("{}", agent.validate_skills(options.name.as_deref()));
                    Ok(())
                }
            }
        }
        Ok(Command::Help(topic)) => {
            println!("{}", help_text(&bin_name, &topic));
            Ok(())
        }
        Err(error) => {
            runtime_log::error("main", format!("cli parse failed: {error}"));
            eprintln!("CLI error: {error}\n");
            eprintln!("{}", help_text(&bin_name, &HelpTopic::General));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        fs::create_dir_all(&path).expect("temp dir should exist");
        path
    }

    #[test]
    fn defaults_to_session_when_no_args_are_provided() {
        let command = parse_command(&[]).expect("command should parse");

        assert_eq!(
            command,
            Command::Session(SessionOptions {
                system_prompt: None,
                user_input: None,
                show_trace: false,
                one_shot: false,
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
            "Plan the refactor".to_string(),
            "--trace".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Session(SessionOptions {
                system_prompt: Some("planner".to_string()),
                user_input: Some("Plan the refactor".to_string()),
                show_trace: true,
                one_shot: true,
            })
        );
    }

    #[test]
    fn parses_skills_create_command() {
        let args = vec![
            "skills".to_string(),
            "create".to_string(),
            "--name".to_string(),
            "developer".to_string(),
            "--description".to_string(),
            "Developer workflow".to_string(),
            "--scope".to_string(),
            "user".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Skills(SkillsCommand::Create(SkillCreateOptions {
                name: "developer".to_string(),
                description: "Developer workflow".to_string(),
                scope: "user".to_string(),
            }))
        );
    }

    #[test]
    fn parses_skills_install_command() {
        let args = vec![
            "skills".to_string(),
            "install".to_string(),
            "--source".to_string(),
            "acme/agent-skills/dev".to_string(),
            "--scope".to_string(),
            "project".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Skills(SkillsCommand::Install(SkillInstallOptions {
                source: "acme/agent-skills/dev".to_string(),
                skill_name: None,
                scope: "project".to_string(),
            }))
        );
    }

    #[test]
    fn parses_skills_show_command() {
        let args = vec![
            "skills".to_string(),
            "show".to_string(),
            "--name".to_string(),
            "developer".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Skills(SkillsCommand::Show(SkillShowOptions {
                name: "developer".to_string(),
            }))
        );
    }

    #[test]
    fn parses_skills_validate_command() {
        let args = vec![
            "skills".to_string(),
            "validate".to_string(),
            "--name".to_string(),
            "developer".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Skills(SkillsCommand::Validate(SkillValidateOptions {
                name: Some("developer".to_string()),
            }))
        );
    }

    #[test]
    fn parses_compact_command() {
        let command = parse_command(&["compact".to_string()]).expect("command should parse");

        assert_eq!(command, Command::Compact);
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
    fn help_flag_returns_general_help() {
        let command = parse_command(&["--help".to_string()]).expect("command should parse");

        assert_eq!(command, Command::Help(HelpTopic::General));
    }

    #[test]
    fn version_flag_returns_version_command() {
        let command = parse_command(&["--version".to_string()]).expect("command should parse");

        assert_eq!(command, Command::Version);
    }

    #[test]
    fn version_subcommand_returns_version_command() {
        let command = parse_command(&["version".to_string()]).expect("command should parse");

        assert_eq!(command, Command::Version);
    }

    #[test]
    fn trace_flag_without_command_starts_session() {
        let command = parse_command(&["--trace".to_string()]).expect("command should parse");

        assert_eq!(
            command,
            Command::Session(SessionOptions {
                system_prompt: None,
                user_input: None,
                show_trace: true,
                one_shot: false,
            })
        );
    }

    #[test]
    fn help_topic_parses_compact() {
        let command = parse_command(&["help".to_string(), "compact".to_string()])
            .expect("command should parse");

        assert_eq!(command, Command::Help(HelpTopic::Compact));
    }

    #[test]
    fn parses_eval_command_with_custom_fixture_dir() {
        let args = vec![
            "eval".to_string(),
            "--fixtures".to_string(),
            "tmp/fixtures".to_string(),
        ];

        let command = parse_command(&args).expect("command should parse");

        assert_eq!(
            command,
            Command::Eval(EvalOptions {
                fixtures_dir: Some("tmp/fixtures".to_string()),
            })
        );
    }

    #[test]
    fn help_topic_parses_eval() {
        let command =
            parse_command(&["help".to_string(), "eval".to_string()]).expect("command should parse");

        assert_eq!(command, Command::Help(HelpTopic::Eval));
    }

    #[test]
    fn help_text_mentions_eval_command() {
        let help = help_text(PUBLIC_BINARY_NAME, &HelpTopic::General);

        assert!(help.contains("agent-in-rust eval"));
        assert!(help.contains("help eval"));
    }

    #[test]
    fn version_text_reports_public_binary_and_package_version() {
        let version = version_text(PUBLIC_BINARY_NAME, PACKAGE_VERSION, None);

        assert_eq!(version, format!("agent-in-rust {}", PACKAGE_VERSION));
    }

    #[test]
    fn version_text_can_include_release_tag_metadata() {
        let version = version_text("agent-in-rust", "0.1.0", Some("v0.1.0"));

        assert_eq!(version, "agent-in-rust 0.1.0 (release tag v0.1.0)");
    }

    #[test]
    fn load_system_prompt_from_dir_reads_sorted_non_hidden_files() {
        let root = temp_root("system-prompt");
        fs::write(root.join("20-style.md"), "Keep context compact.").expect("style prompt");
        fs::write(
            root.join("10-role.md"),
            "You are a harness-first coding agent.",
        )
        .expect("role prompt");
        fs::write(root.join(".ignored.md"), "hidden").expect("hidden file");
        fs::create_dir_all(root.join("nested")).expect("nested dir");
        fs::write(root.join("nested").join("30-nested.md"), "nested").expect("nested file");

        let prompt = load_system_prompt_from_dir(&root).expect("prompt should load");

        assert_eq!(
            prompt,
            "You are a harness-first coding agent.\n\nKeep context compact."
        );
    }

    #[test]
    fn load_system_prompt_from_dir_rejects_empty_directories() {
        let root = temp_root("system-prompt-empty");

        let error = load_system_prompt_from_dir(&root).expect_err("empty prompt dir should fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("No system prompt files found"));
    }

    #[test]
    fn resolve_system_prompt_prefers_explicit_then_env_then_directory() {
        let root = temp_root("system-prompt-precedence");
        fs::write(root.join("system_prompt.md"), "folder prompt").expect("folder prompt");

        let explicit = resolve_system_prompt_from_sources(
            Some("explicit prompt".to_string()),
            Some("env prompt".to_string()),
            &root,
        )
        .expect("explicit prompt should win");
        let env_prompt =
            resolve_system_prompt_from_sources(None, Some("env prompt".to_string()), &root)
                .expect("env prompt should win");
        let folder_prompt = resolve_system_prompt_from_sources(None, None, &root)
            .expect("folder prompt should load");

        assert_eq!(explicit, "explicit prompt");
        assert_eq!(env_prompt, "env prompt");
        assert_eq!(folder_prompt, "folder prompt");
    }

    #[test]
    fn resolve_system_prompt_falls_back_to_default_when_directory_is_missing() {
        let root = temp_root("system-prompt-missing");
        let missing = root.join("missing");

        let prompt = resolve_system_prompt_from_sources(None, None, &missing)
            .expect("missing dir should use built-in fallback");

        assert_eq!(prompt, DEFAULT_SYSTEM_PROMPT);
    }

    #[test]
    fn build_eval_suite_errors_when_fixture_dir_is_empty() {
        let fixture_dir = temp_root("eval-empty");
        let agent =
            MainAgent::from_env(DEFAULT_SYSTEM_PROMPT.to_string()).expect("agent should load");

        let error = build_eval_suite(
            &agent,
            EvalOptions {
                fixtures_dir: Some(fixture_dir.display().to_string()),
            },
        )
        .expect_err("empty fixture dir should fail");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("No eval fixtures found"));
    }

    #[test]
    fn build_eval_suite_reports_passing_and_failing_cases() {
        let fixture_dir = temp_root("eval-pass-fail");
        fs::write(
            fixture_dir.join("a-pass.json"),
            r#"{
                "name": "stop passes",
                "user_input": "",
                "observations": [],
                "expected": {
                    "action": "stop"
                }
            }"#,
        )
        .expect("fixture should write");
        fs::write(
            fixture_dir.join("b-fail.json"),
            r#"{
                "name": "retry fails as finish",
                "user_input": "retry the request",
                "observations": [
                    "Recoverable tool failure from `web_search_tool`: timeout"
                ],
                "expected": {
                    "action": "finish"
                }
            }"#,
        )
        .expect("fixture should write");
        let agent =
            MainAgent::from_env(DEFAULT_SYSTEM_PROMPT.to_string()).expect("agent should load");

        let suite = build_eval_suite(
            &agent,
            EvalOptions {
                fixtures_dir: Some(fixture_dir.display().to_string()),
            },
        )
        .expect("suite should build");

        assert_eq!(suite.cases.len(), 2);
        assert_eq!(suite.passed(), 1);
        assert_eq!(suite.failed(), 1);
        assert!(suite.cases[0].passed);
        assert!(!suite.cases[1].passed);
        assert_eq!(suite.cases[0].actual.action, "stop");
        assert_eq!(suite.cases[1].actual.action, "retry");
    }

    #[test]
    fn sprint_v2_scaffolding_identifies_openclaw_gap_doc_target() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let prd_path = repo_root.join("sprints/v2/PRD.md");
        let tasks_path = repo_root.join("sprints/v2/TASKS.md");
        let gap_doc_path = repo_root.join("docs/openclaw-gap.md");

        assert!(prd_path.is_file(), "expected {:?} to exist", prd_path);
        assert!(tasks_path.is_file(), "expected {:?} to exist", tasks_path);
        assert!(
            gap_doc_path.is_file(),
            "expected {:?} to exist",
            gap_doc_path
        );

        let gap_doc = fs::read_to_string(&gap_doc_path).expect("gap doc should be readable");
        assert!(
            gap_doc.contains("OpenClaw") && gap_doc.contains("gap"),
            "gap doc should identify the OpenClaw gap-analysis target"
        );
    }

    #[test]
    fn sprint_v2_gap_doc_compares_current_harness_to_target_architecture() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let gap_doc_path = repo_root.join("docs/openclaw-gap.md");
        let gap_doc = fs::read_to_string(&gap_doc_path).expect("gap doc should be readable");

        for required_phrase in [
            "Current Harness",
            "Target OpenClaw-Style Architecture",
            "Migration Seams",
            "bus",
            "channel",
            "routing",
            "scheduler",
            "dispatch",
            "prompt layer",
            "concurrency",
            "memory-agent",
        ] {
            assert!(
                gap_doc.contains(required_phrase),
                "gap doc should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v4_spawned_subagent_docs_explain_current_vs_spawned_boundary() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let architecture_path = repo_root.join("docs/architecture.md");
        let cli_path = repo_root.join("docs/cli.md");
        let drawio_path = repo_root.join("docs/diagrams/subagent-spawn.drawio");
        let excalidraw_path = repo_root.join("docs/diagrams/subagent-spawn.excalidraw");

        for path in [&architecture_path, &cli_path, &drawio_path, &excalidraw_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let architecture =
            fs::read_to_string(&architecture_path).expect("architecture doc should be readable");
        let cli = fs::read_to_string(&cli_path).expect("CLI doc should be readable");
        let drawio = fs::read_to_string(&drawio_path).expect("draw.io file should be readable");
        let excalidraw =
            fs::read_to_string(&excalidraw_path).expect("excalidraw file should be readable");

        for required_phrase in [
            "Current synthesized delegation",
            "Spawned child execution",
            "ProcessDispatcher",
            "__spawn-subagent",
        ] {
            assert!(
                architecture.contains(required_phrase),
                "architecture doc should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "/agent <name> <task>",
            "spawned child process",
            "local fallback",
        ] {
            assert!(
                cli.contains(required_phrase),
                "CLI doc should mention `{required_phrase}`"
            );
        }

        for required_phrase in ["Current repo", "Target feature", "Local synthesized subagent"] {
            assert!(
                drawio.contains(required_phrase),
                "draw.io diagram should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Current repo",
            "Spawned child worker",
            "Local synthesized",
        ] {
            assert!(
                excalidraw.contains(required_phrase),
                "excalidraw diagram should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v4_graphify_artifacts_include_spawned_subagent_boundary() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let report_path = repo_root.join("graphify-out/GRAPH_REPORT.md");
        let graph_path = repo_root.join("graphify-out/graph.json");

        assert!(report_path.is_file(), "expected {:?} to exist", report_path);
        assert!(graph_path.is_file(), "expected {:?} to exist", graph_path);

        let report = fs::read_to_string(&report_path).expect("graph report should be readable");
        let graph = fs::read_to_string(&graph_path).expect("graph json should be readable");

        for required_phrase in [
            "DispatchRequest",
            "DispatchResponse",
            "DispatchTarget",
        ] {
            assert!(
                report.contains(required_phrase),
                "graph report should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "\"label\": \"ProcessDispatcher\"",
            "\"label\": \"execute_spawned_subagent_request_in_root()\"",
            "\"label\": \"record_subagent_dispatch_event()\"",
        ] {
            assert!(
                graph.contains(required_phrase),
                "graph json should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_packaging_docs_define_release_and_install_contract() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let prd_path = repo_root.join("sprints/v5/PRD.md");
        let tasks_path = repo_root.join("sprints/v5/TASKS.md");
        let readme_path = repo_root.join("README.md");
        let install_path = repo_root.join("docs/install.md");

        for path in [&prd_path, &tasks_path, &readme_path, &install_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let prd = fs::read_to_string(&prd_path).expect("v5 PRD should be readable");
        let tasks = fs::read_to_string(&tasks_path).expect("v5 tasks should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");

        for required_phrase in [
            "cargo-dist",
            "GitHub Releases",
            "shell installer",
            "PowerShell installer",
            "manual release download",
            "source-build fallback",
        ] {
            assert!(
                prd.contains(required_phrase),
                "v5 PRD should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Task 1: Create sprint v5 packaging docs and install contract",
            "Task 3: Add `cargo-dist` metadata for release artifacts",
            "Task 4: Add tagged GitHub Actions release automation",
            "Task 6: Add shell and PowerShell installer support through the release pipeline",
            "Task 7: Add manual download and contributor fallback install docs",
        ] {
            assert!(
                tasks.contains(required_phrase),
                "v5 tasks should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Install",
            "docs/install.md",
            "GitHub Releases",
            "manual download",
            "build from source",
        ] {
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Supported Targets",
            "GitHub Releases",
            "One-Line Install",
            "Manual Download",
            "Build From Source",
            "--version",
        ] {
            assert!(
                install.contains(required_phrase),
                "install doc should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_public_binary_name_is_normalized_across_package_runtime_and_docs() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cargo_toml_path = repo_root.join("Cargo.toml");
        let readme_path = repo_root.join("README.md");
        let cli_path = repo_root.join("docs/cli.md");
        let install_path = repo_root.join("docs/install.md");

        for path in [&cargo_toml_path, &readme_path, &cli_path, &install_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let cargo_toml =
            fs::read_to_string(&cargo_toml_path).expect("Cargo.toml should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let cli = fs::read_to_string(&cli_path).expect("CLI doc should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");
        let help = help_text("agent-in-rust", &HelpTopic::General);

        assert!(
            cargo_toml.contains("name = \"agent-in-rust\""),
            "Cargo.toml should set the public package name"
        );
        assert!(
            cargo_toml.contains("[[bin]]") && cargo_toml.contains("name = \"agent-in-rust\""),
            "Cargo.toml should explicitly pin the release binary name"
        );

        for document in [&readme, &cli, &install, &help] {
            assert!(
                document.contains("agent-in-rust"),
                "binary surface should mention the normalized public name"
            );
            assert!(
                !document.contains("agent_in_rust"),
                "binary surface should not keep the old underscore name"
            );
        }
    }

    #[test]
    fn sprint_v5_cargo_dist_metadata_covers_release_artifacts_and_installers() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cargo_toml_path = repo_root.join("Cargo.toml");
        let cargo_toml =
            fs::read_to_string(&cargo_toml_path).expect("Cargo.toml should be readable");

        for required_phrase in [
            "[workspace]",
            "[workspace.metadata.dist]",
            "cargo-dist-version = \"0.31.0\"",
            "ci = [\"github\"]",
            "installers = [\"shell\", \"powershell\"]",
            "windows-archive = \".zip\"",
            "unix-archive = \".tar.gz\"",
            "checksum = \"sha256\"",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            assert!(
                cargo_toml.contains(required_phrase),
                "Cargo.toml should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_release_workflow_is_tag_triggered_and_publishes_dist_artifacts() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workflow_path = repo_root.join(".github/workflows/release.yml");

        assert!(
            workflow_path.is_file(),
            "expected {:?} to exist",
            workflow_path
        );

        let workflow =
            fs::read_to_string(&workflow_path).expect("release workflow should be readable");

        for required_phrase in [
            "name: Release",
            "push:",
            "tags:",
            "- 'v*'",
            "workflow_dispatch:",
            "contents: write",
            "cargo dist plan --output-format=json",
            "cargo dist build --artifacts=global",
            "cargo dist build --artifacts=local",
            "cargo dist host --steps=create,upload,release,announce",
            "actions/upload-artifact@v4",
            "actions/download-artifact@v4",
            "agent-in-rust-installer.sh",
            "agent-in-rust-installer.ps1",
            "releases/download/${{ github.ref_name }}",
        ] {
            assert!(
                workflow.contains(required_phrase),
                "release workflow should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_support_matrix_is_explicit_and_limited_across_config_and_docs() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cargo_toml_path = repo_root.join("Cargo.toml");
        let readme_path = repo_root.join("README.md");
        let install_path = repo_root.join("docs/install.md");

        for path in [&cargo_toml_path, &readme_path, &install_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let cargo_toml =
            fs::read_to_string(&cargo_toml_path).expect("Cargo.toml should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");

        for required_phrase in [
            "Supported release matrix for v5",
            "macOS Intel: x86_64-apple-darwin",
            "macOS Apple Silicon: aarch64-apple-darwin",
            "Linux x86_64 GNU: x86_64-unknown-linux-gnu",
            "Linux ARM64 GNU: aarch64-unknown-linux-gnu",
            "Windows x86_64 MSVC: x86_64-pc-windows-msvc",
        ] {
            assert!(
                cargo_toml.contains(required_phrase),
                "Cargo.toml should document `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Supported Targets",
            "Initial release matrix",
            "macOS Intel (`x86_64-apple-darwin`)",
            "macOS Apple Silicon (`aarch64-apple-darwin`)",
            "Linux x86_64 GNU (`x86_64-unknown-linux-gnu`)",
            "Linux ARM64 GNU (`aarch64-unknown-linux-gnu`)",
            "Windows x86_64 MSVC (`x86_64-pc-windows-msvc`)",
            "limited v5 support contract",
        ] {
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
            assert!(
                install.contains(required_phrase),
                "install doc should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_installer_paths_are_concrete_in_workflow_and_docs() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workflow_path = repo_root.join(".github/workflows/release.yml");
        let readme_path = repo_root.join("README.md");
        let install_path = repo_root.join("docs/install.md");

        for path in [&workflow_path, &readme_path, &install_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let workflow =
            fs::read_to_string(&workflow_path).expect("release workflow should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");

        for required_phrase in [
            "curl --proto '=https' --tlsv1.2 -LsSf",
            "irm ",
            "agent-in-rust-installer.sh",
            "agent-in-rust-installer.ps1",
            "releases/latest/download/agent-in-rust-installer.sh",
            "releases/latest/download/agent-in-rust-installer.ps1",
            "without requiring a Rust toolchain",
        ] {
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
            assert!(
                install.contains(required_phrase),
                "install doc should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "curl --proto '=https' --tlsv1.2 -LsSf",
            "irm ",
            "agent-in-rust-installer.sh",
            "agent-in-rust-installer.ps1",
            "releases/download/${{ github.ref_name }}/agent-in-rust-installer.sh",
            "releases/download/${{ github.ref_name }}/agent-in-rust-installer.ps1",
        ] {
            assert!(
                workflow.contains(required_phrase),
                "release workflow should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_manual_download_and_source_fallback_are_documented() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let readme_path = repo_root.join("README.md");
        let install_path = repo_root.join("docs/install.md");
        let cli_path = repo_root.join("docs/cli.md");

        for path in [&readme_path, &install_path, &cli_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");
        let cli = fs::read_to_string(&cli_path).expect("CLI doc should be readable");

        for required_phrase in [
            "Manual Download",
            "download the archive for their target",
            "unpack it locally",
            "place the binary on `PATH`",
            "Build From Source",
            "cargo build --release",
            "target/release/agent-in-rust --version",
            "cargo install --locked --path .",
            "contributor fallback",
        ] {
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
            assert!(
                install.contains(required_phrase),
                "install doc should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Packaging And Install",
            "manual download",
            "GitHub Releases",
            "cargo build --release",
            "cargo install --locked --path .",
            "agent-in-rust --version",
        ] {
            assert!(
                cli.contains(required_phrase),
                "CLI doc should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_version_output_is_stable_and_matches_package_metadata() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cargo_toml_path = repo_root.join("Cargo.toml");
        let readme_path = repo_root.join("README.md");
        let workflow_path = repo_root.join(".github/workflows/release.yml");

        for path in [&cargo_toml_path, &readme_path, &workflow_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let cargo_toml =
            fs::read_to_string(&cargo_toml_path).expect("Cargo.toml should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let workflow =
            fs::read_to_string(&workflow_path).expect("release workflow should be readable");
        let general_help = help_text(PUBLIC_BINARY_NAME, &HelpTopic::General);
        let version = version_text(PUBLIC_BINARY_NAME, PACKAGE_VERSION, Some(&format!("v{PACKAGE_VERSION}")));

        assert!(
            cargo_toml.contains(&format!("version = \"{PACKAGE_VERSION}\"")),
            "Cargo.toml should pin the package version used by the CLI"
        );
        assert_eq!(
            version,
            format!("agent-in-rust {PACKAGE_VERSION} (release tag v{PACKAGE_VERSION})")
        );

        for required_phrase in [
            "--version",
            "version",
            "stable packaged verification string",
            "agent-in-rust --version",
        ] {
            assert!(
                general_help.contains(required_phrase),
                "help text should mention `{required_phrase}`"
            );
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "AGENT_IN_RUST_RELEASE_TAG: ${{ github.ref_name }}",
            "cargo dist build --artifacts=global",
            "cargo dist build --artifacts=local",
        ] {
            assert!(
                workflow.contains(required_phrase),
                "release workflow should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_release_artifact_naming_and_docs_stay_aligned() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cargo_toml_path = repo_root.join("Cargo.toml");
        let readme_path = repo_root.join("README.md");
        let install_path = repo_root.join("docs/install.md");
        let workflow_path = repo_root.join(".github/workflows/release.yml");

        for path in [&cargo_toml_path, &readme_path, &install_path, &workflow_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let cargo_toml =
            fs::read_to_string(&cargo_toml_path).expect("Cargo.toml should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");
        let workflow =
            fs::read_to_string(&workflow_path).expect("release workflow should be readable");

        for required_phrase in [
            "unix-archive = \".tar.gz\"",
            "windows-archive = \".zip\"",
            "checksum = \"sha256\"",
        ] {
            assert!(
                cargo_toml.contains(required_phrase),
                "Cargo.toml should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "Release Artifacts",
            "agent-in-rust-installer.sh",
            "agent-in-rust-installer.ps1",
            ".tar.gz",
            ".zip",
            ".sha256",
            "sha256",
        ] {
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
            assert!(
                install.contains(required_phrase),
                "install doc should mention `{required_phrase}`"
            );
        }

        for required_phrase in [
            "agent-in-rust-installer.sh",
            "agent-in-rust-installer.ps1",
            "## Installer URLs",
            "## Installer Commands",
        ] {
            assert!(
                workflow.contains(required_phrase),
                "release workflow should mention `{required_phrase}`"
            );
        }
    }

    #[test]
    fn sprint_v5_deferred_extensions_are_documented_without_expanding_scope() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let prd_path = repo_root.join("sprints/v5/PRD.md");
        let readme_path = repo_root.join("README.md");
        let install_path = repo_root.join("docs/install.md");

        for path in [&prd_path, &readme_path, &install_path] {
            assert!(path.is_file(), "expected {:?} to exist", path);
        }

        let prd = fs::read_to_string(&prd_path).expect("v5 PRD should be readable");
        let readme = fs::read_to_string(&readme_path).expect("README should be readable");
        let install = fs::read_to_string(&install_path).expect("install doc should be readable");

        for required_phrase in [
            "Future Extensions",
            "Homebrew",
            "Scoop",
            "crates.io",
            "self-update",
            "not part of the v5 baseline",
        ] {
            assert!(
                prd.contains(required_phrase),
                "v5 PRD should mention `{required_phrase}`"
            );
            assert!(
                readme.contains(required_phrase),
                "README should mention `{required_phrase}`"
            );
            assert!(
                install.contains(required_phrase),
                "install doc should mention `{required_phrase}`"
            );
        }
    }
}
