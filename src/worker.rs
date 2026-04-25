#![allow(dead_code)]

use crate::bus::{EventId, InboundEvent, InboundEventBody};
use crate::mainAgent::{AgentResult, MainAgent, WaitModeConfig};
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerMode {
    OneShot,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRequest {
    pub event: InboundEvent,
    pub wait_mode: Option<WaitModeConfig>,
}

impl WorkerRequest {
    pub fn new(event: InboundEvent) -> Self {
        Self {
            event,
            wait_mode: None,
        }
    }

    pub fn with_wait_mode(mut self, wait_mode: WaitModeConfig) -> Self {
        self.wait_mode = Some(wait_mode);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerResult {
    pub event_id: EventId,
    pub output: String,
    pub trace: Vec<String>,
    pub usage_summary: String,
    pub stopped: bool,
    pub mode: WorkerMode,
}

pub trait WorkerRuntime {
    fn execute(&mut self, request: WorkerRequest) -> io::Result<WorkerResult>;
}

pub trait WorkerAgentRuntime {
    fn run(&mut self, input: &str) -> AgentResult;
    fn wait_for_context(&mut self, config: WaitModeConfig) -> io::Result<()>;
}

impl WorkerAgentRuntime for MainAgent {
    fn run(&mut self, input: &str) -> AgentResult {
        MainAgent::run(self, input)
    }

    fn wait_for_context(&mut self, config: WaitModeConfig) -> io::Result<()> {
        MainAgent::wait_for_context(self, config)
    }
}

pub struct MainAgentWorker<'a, A: WorkerAgentRuntime> {
    agent: &'a mut A,
}

impl<'a, A: WorkerAgentRuntime> MainAgentWorker<'a, A> {
    pub fn new(agent: &'a mut A) -> Self {
        Self { agent }
    }
}

impl<A: WorkerAgentRuntime> WorkerRuntime for MainAgentWorker<'_, A> {
    fn execute(&mut self, request: WorkerRequest) -> io::Result<WorkerResult> {
        let event_id = request.event.id.clone();

        match request.event.body {
            InboundEventBody::UserMessage { content } => {
                let result = self.agent.run(&content);
                Ok(WorkerResult {
                    event_id,
                    output: result.output.clone(),
                    trace: result.trace.clone(),
                    usage_summary: result.render_usage_summary(),
                    stopped: result.stopped,
                    mode: WorkerMode::OneShot,
                })
            }
            InboundEventBody::Command { name, .. } if name == "session" => {
                let wait_mode = request.wait_mode.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "interactive worker request requires wait-mode config",
                    )
                })?;
                self.agent.wait_for_context(wait_mode)?;
                Ok(WorkerResult {
                    event_id,
                    output: "interactive session completed".to_string(),
                    trace: Vec::new(),
                    usage_summary: String::new(),
                    stopped: false,
                    mode: WorkerMode::Interactive,
                })
            }
            InboundEventBody::Command { name, .. } => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported worker command: {name}"),
            )),
            InboundEventBody::ScheduledTick { .. } => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "scheduled events are not supported by the main worker yet",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{EventSource, EventSourceKind, InboundEvent};
    use crate::mainAgent::TokenUsageRecord;

    struct FakeWorkerAgent {
        last_run_input: Option<String>,
        last_wait_mode: Option<WaitModeConfig>,
        run_result: AgentResult,
    }

    impl FakeWorkerAgent {
        fn new(run_result: AgentResult) -> Self {
            Self {
                last_run_input: None,
                last_wait_mode: None,
                run_result,
            }
        }
    }

    impl WorkerAgentRuntime for FakeWorkerAgent {
        fn run(&mut self, input: &str) -> AgentResult {
            self.last_run_input = Some(input.to_string());
            self.run_result.clone()
        }

        fn wait_for_context(&mut self, config: WaitModeConfig) -> io::Result<()> {
            self.last_wait_mode = Some(config);
            Ok(())
        }
    }

    #[test]
    fn worker_executes_user_message_requests_through_agent_run() {
        let run_result = AgentResult {
            output: "worker output".to_string(),
            trace: vec!["step 1".to_string()],
            usage: vec![TokenUsageRecord {
                actor: "main-agent".to_string(),
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
                execution: "planner".to_string(),
                note: "test".to_string(),
            }],
            stopped: false,
        };
        let mut agent = FakeWorkerAgent::new(run_result);
        let source = EventSource::new(EventSourceKind::Cli, "terminal");
        let request = WorkerRequest::new(InboundEvent::user_message(
            EventId::new("evt-worker-1"),
            source,
            "Inspect the repo",
        ));

        let result = MainAgentWorker::new(&mut agent)
            .execute(request)
            .expect("worker execution should succeed");

        assert_eq!(agent.last_run_input.as_deref(), Some("Inspect the repo"));
        assert_eq!(result.event_id.as_str(), "evt-worker-1");
        assert_eq!(result.output, "worker output");
        assert_eq!(result.trace, vec!["step 1".to_string()]);
        assert_eq!(
            result.usage_summary,
            "Token usage\n- main-agent: 1 input, 2 output, 3 total [planner; test]"
        );
        assert_eq!(result.mode, WorkerMode::OneShot);
        assert!(!result.stopped);
    }

    #[test]
    fn worker_executes_session_commands_through_wait_mode() {
        let mut agent = FakeWorkerAgent::new(AgentResult {
            output: String::new(),
            trace: Vec::new(),
            usage: Vec::new(),
            stopped: false,
        });
        let wait_mode = WaitModeConfig {
            prompt_label: "claude".to_string(),
            show_trace: true,
            bin_name: "agent_in_rust".to_string(),
        };
        let source = EventSource::new(EventSourceKind::Cli, "terminal");
        let request = WorkerRequest::new(InboundEvent::command(
            EventId::new("evt-worker-2"),
            source,
            "session",
            vec!["--trace".to_string()],
        ))
        .with_wait_mode(wait_mode.clone());

        let result = MainAgentWorker::new(&mut agent)
            .execute(request)
            .expect("interactive worker execution should succeed");

        assert_eq!(agent.last_wait_mode, Some(wait_mode));
        assert_eq!(result.event_id.as_str(), "evt-worker-2");
        assert_eq!(result.mode, WorkerMode::Interactive);
        assert_eq!(result.output, "interactive session completed");
    }
}
