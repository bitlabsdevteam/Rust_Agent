#[allow(non_snake_case)]
mod Tools;

mod mcp;
mod observability;
mod runtime_log;
mod evals;

#[allow(non_snake_case)]
mod mainAgent;

use crate::mainAgent::{MainAgent, WaitModeConfig, DEFAULT_SYSTEM_PROMPT};
use std::env;
use std::fmt;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Session(SessionOptions),
    List,
    Init,
    Help(HelpTopic),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionOptions {
    system_prompt: Option<String>,
    user_input: Option<String>,
    show_trace: bool,
    one_shot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HelpTopic {
    General,
    Session,
    Run,
    List,
    Init,
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
            "init" => Ok(HelpTopic::Init),
            other => Err(CliError::new(format!("Unknown help topic: {other}"))),
        },
        _ => Err(CliError::new(
            "Too many arguments for `help`. Use `help`, `help session`, `help run`, `help list`, or `help init`.",
        )),
    }
}

fn help_text(bin_name: &str, topic: &HelpTopic) -> String {
    match topic {
        HelpTopic::General => format!(
            "{bin_name} runs a Claude-style local coding session.\n\nDefault entrypoint:\n  {bin_name}                Start chatting\n  {bin_name} --trace        Start chatting and print the execution trace\n\nOther commands:\n  {bin_name} session [--system <prompt>] [--trace]\n  {bin_name} run --input <prompt> [--system <prompt>] [--trace]\n  {bin_name} list\n  {bin_name} init\n  {bin_name} help [session|run|list|init]\n\nNotes:\n  - Running `{bin_name}` with no command starts the interactive chat session.\n  - Project memory loads from `CLAUDE.md` and imported `@path` files.\n  - Project subagents load from `.claude/agents/*.md`.\n  - Project slash commands load from `.claude/commands/*.md`.\n  - MCP tools remain available when `MCP_SERVERS` is configured.\n"
        ),
        HelpTopic::Session => format!(
            "Start an interactive Claude-style session.\n\nUsage:\n  {bin_name}\n  {bin_name} --trace\n  {bin_name} session\n  {bin_name} session --system <prompt> --trace\n  {bin_name} chat\n\nIn-session commands:\n  /help\n  /agents\n  /memory\n  /model\n  /clear\n  /compact\n  /mcp\n  /review [task]\n  /init\n  /agent <name> <task>\n  /trace\n  /exit\n"
        ),
        HelpTopic::Run => format!(
            "Run a one-shot prompt through the Claude-style runtime.\n\nUsage:\n  {bin_name} run --input \"Plan the refactor\"\n  {bin_name} run --input \"Use tool web_search with {{\\\"query\\\":\\\"latest Rust 2026 edition updates\\\"}}\" --trace\n"
        ),
        HelpTopic::List => format!(
            "Show the currently loaded Claude-style project surface.\n\nUsage:\n  {bin_name} list\n"
        ),
        HelpTopic::Init => format!(
            "Scaffold Claude-style project files.\n\nUsage:\n  {bin_name} init\n\nThis creates `CLAUDE.md`, `.claude/agents/*.md`, and `.claude/commands/review.md` when missing.\n"
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
            run_session(&mut agent, options)
        }
        Ok(Command::List) => {
            let system_prompt = resolve_system_prompt(None);
            let agent = MainAgent::from_env(system_prompt)?;
            print_catalog(&agent);
            Ok(())
        }
        Ok(Command::Init) => {
            let system_prompt = resolve_system_prompt(None);
            let mut agent = MainAgent::from_env(system_prompt)?;
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
}
