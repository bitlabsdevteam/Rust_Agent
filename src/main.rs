#[allow(non_snake_case)]
mod Tools;

mod mcp;
mod observability;
mod runtime_log;
mod evals;

#[allow(non_snake_case)]
mod mainAgent;

use crate::mainAgent::{
    MainAgent, SkillCreateRequest, SkillInstallRequest, WaitModeConfig, DEFAULT_SYSTEM_PROMPT,
};
use std::env;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
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
        if matches!(command_name, "--help" | "-h") {
            return Ok(Command::Help(HelpTopic::General));
        }
        command_name = "session";
        start_index = 0;
    }

    match command_name {
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
        "--help" | "-h" => Ok(Command::Help(HelpTopic::General)),
        other => Err(CliError::new(format!("Unknown command: {other}"))),
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

fn help_text(bin_name: &str, topic: &HelpTopic) -> String {
    match topic {
        HelpTopic::General => format!(
            "{bin_name} runs a Claude-style local coding session.\n\nDefault entrypoint:\n  {bin_name}                Start chatting\n  {bin_name} --trace        Start chatting and print the execution trace\n\nOther commands:\n  {bin_name} session [--system <prompt>] [--trace]\n  {bin_name} run --input <prompt> [--system <prompt>] [--trace]\n  {bin_name} list\n  {bin_name} compact\n  {bin_name} eval [--fixtures <dir>]\n  {bin_name} skills [list|create|install] ...\n  {bin_name} init\n  {bin_name} help [session|run|list|skills|init|compact|eval]\n  {bin_name} help eval\n\nNotes:\n  - Running `{bin_name}` with no command starts the interactive chat session.\n  - Project memory loads from `CLAUDE.md` and imported `@path` files.\n  - Short-term memory resumes from `Workspace/short-term.json` when present.\n  - Long-term memory uses Mem0 when `MEM0_API_KEY` is configured and falls back to `Workspace/MEMORY.md` otherwise.\n  - Project subagents load from `.claude/agents/*.md`.\n  - Skills load from `.claude/skills/<name>/SKILL.md` and `~/.claude/skills/<name>/SKILL.md`.\n  - Project slash commands load from `.claude/commands/*.md`.\n  - MCP tools remain available when `MCP_SERVERS` is configured.\n"
        ),
        HelpTopic::Session => format!(
            "Start an interactive Claude-style session.\n\nUsage:\n  {bin_name}\n  {bin_name} --trace\n  {bin_name} session\n  {bin_name} session --system <prompt> --trace\n  {bin_name} chat\n\nIn-session commands:\n  /help\n  /agents\n  /skills\n  /memory\n  /remember <note>\n  /model\n  /clear\n  /compact\n  /mcp\n  /review [task]\n  /skill <name> [task]\n  /init\n  /agent <name> <task>\n  /trace\n  /exit\n"
        ),
        HelpTopic::Run => format!(
            "Run a one-shot prompt through the Claude-style runtime.\n\nUsage:\n  {bin_name} run --input \"Plan the refactor\"\n  {bin_name} run --input \"Use tool web_search with {{\\\"query\\\":\\\"latest Rust 2026 edition updates\\\"}}\" --trace\n"
        ),
        HelpTopic::List => format!(
            "Show the currently loaded Claude-style project surface.\n\nUsage:\n  {bin_name} list\n"
        ),
        HelpTopic::Skills => format!(
            "Manage reusable skills.\n\nUsage:\n  {bin_name} skills\n  {bin_name} skills list\n  {bin_name} skills create --name <name> --description <text> [--scope project|user]\n  {bin_name} skills install --source <path|owner/repo|owner/repo/skill|github-url|skills.sh-url> [--skill <name>] [--scope project|user]\n"
        ),
        HelpTopic::Init => format!(
            "Scaffold Claude-style project files.\n\nUsage:\n  {bin_name} init\n\nThis creates `CLAUDE.md`, `.claude/agents/*.md`, `.claude/skills/ship-small/SKILL.md`, `.claude/commands/review.md`, and the fallback file `Workspace/MEMORY.md` when missing.\n"
        ),
        HelpTopic::Compact => format!(
            "Compact stored short-term context.\n\nUsage:\n  {bin_name} compact\n\nThis loads `Workspace/short-term.json`, summarizes older session turns into the compacted summary, retains the most recent turns, and writes the compacted snapshot back to disk.\n"
        ),
        HelpTopic::Eval => format!(
            "Run planner eval fixtures against the current harness.\n\nUsage:\n  {bin_name} eval\n  {bin_name} eval --fixtures evals/fixtures\n\nThis loads JSON fixtures, asks the current planner what action it would take next, and reports pass/fail for each expected decision.\n"
        ),
    }
}

fn resolve_system_prompt(explicit: Option<String>) -> String {
    explicit
        .or_else(|| env::var("AGENT_SYSTEM_PROMPT").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string())
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
        println!("- {} [{}]: {}", skill.name, skill.scope, skill.description);
    }

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
    if options.one_shot {
        let input = options
            .user_input
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "`run` requires `--input`.")
            })?;
        let result = agent.run(&input);
        println!("{}", result.output);
        println!("\n{}", result.render_usage_summary());
        if options.show_trace {
            println!("\nTrace:");
            for entry in &result.trace {
                println!("- {entry}");
            }
        }
        return Ok(());
    }

    agent.wait_for_context(WaitModeConfig {
        prompt_label: "claude".to_string(),
        show_trace: options.show_trace,
        bin_name: "agent_in_rust".to_string(),
    })
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
    runtime_log::info("main", "process booted");

    let args: Vec<String> = env::args().skip(1).collect();
    let bin_name = env::args()
        .next()
        .unwrap_or_else(|| "agent_in_rust".to_string());

    match parse_command(&args) {
        Ok(Command::Session(options)) => {
            let system_prompt = resolve_system_prompt(options.system_prompt.clone());
            let mut agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            run_session(&mut agent, options)
        }
        Ok(Command::List) => {
            let system_prompt = resolve_system_prompt(None);
            let agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            print_catalog(&agent);
            Ok(())
        }
        Ok(Command::Init) => {
            let system_prompt = resolve_system_prompt(None);
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
            let system_prompt = resolve_system_prompt(None);
            let mut agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            let result = agent.compact_context()?;
            println!("{}", result.render());
            Ok(())
        }
        Ok(Command::Eval(options)) => {
            let system_prompt = resolve_system_prompt(None);
            let agent = MainAgent::from_env(system_prompt)?;
            log_observability_status(&agent);
            run_evals(&agent, options)
        }
        Ok(Command::Skills(command)) => {
            let system_prompt = resolve_system_prompt(None);
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
        let help = help_text("agent_in_rust", &HelpTopic::General);

        assert!(help.contains("agent_in_rust eval"));
        assert!(help.contains("help eval"));
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
                    "Recoverable tool failure from `web_search`: timeout"
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
}
