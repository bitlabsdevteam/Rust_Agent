#![allow(dead_code)]

use crate::bus::EventId;
use crate::mainAgent::{ContextPacket, TokenUsageRecord};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const CHILD_SUBAGENT_COMMAND: &str = "__spawn-subagent";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchTarget {
    Worker { profile: String },
    Subagent { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchRequest {
    pub dispatch_id: EventId,
    pub requester: String,
    pub target: DispatchTarget,
    pub task: String,
    pub context_packet: ContextPacket,
    pub memory_refs: Vec<String>,
    pub file_refs: Vec<String>,
    pub observations: Vec<String>,
}

impl DispatchRequest {
    pub fn for_subagent(
        requester: impl Into<String>,
        subagent: impl Into<String>,
        task: impl Into<String>,
        context_packet: ContextPacket,
        memory_refs: Vec<String>,
        file_refs: Vec<String>,
        observations: Vec<String>,
    ) -> Self {
        Self {
            dispatch_id: EventId::generate(),
            requester: requester.into(),
            target: DispatchTarget::Subagent {
                name: subagent.into(),
            },
            task: task.into(),
            context_packet,
            memory_refs,
            file_refs,
            observations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchResponse {
    pub dispatch_id: EventId,
    pub responder: String,
    pub summary: String,
    pub findings: Vec<String>,
    pub artifact_refs: Vec<String>,
    pub recommended_next_action: String,
    pub final_text: String,
    pub usage: Vec<TokenUsageRecord>,
}

impl DispatchResponse {
    pub fn render(&self) -> String {
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

pub trait Dispatcher {
    fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResponse, String>;
}

#[derive(Debug, Clone)]
pub struct ProcessDispatcher {
    executable: PathBuf,
    current_dir: PathBuf,
    timeout: Duration,
}

impl ProcessDispatcher {
    pub fn with_executable(
        executable: impl Into<PathBuf>,
        current_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            executable: executable.into(),
            current_dir: current_dir.into(),
            timeout,
        }
    }

    pub fn from_current_exe(
        current_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> io::Result<Self> {
        Ok(Self::with_executable(
            resolve_spawn_executable()?,
            current_dir,
            timeout,
        ))
    }

    fn dispatch_spawned_subagent(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchResponse, String> {
        Self::validate_spawn_request(&request)?;

        let request_json = serde_json::to_string(&request)
            .map_err(|error| format!("failed to serialize dispatch request: {error}"))?;

        let mut child = Command::new(&self.executable)
            .arg(CHILD_SUBAGENT_COMMAND)
            .current_dir(&self.current_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                format!(
                    "failed to spawn child subagent `{}`: {error}",
                    self.executable.display()
                )
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(request_json.as_bytes())
                .and_then(|_| stdin.flush())
                .map_err(|error| format!("failed to write child request: {error}"))?;
        }

        let started_at = Instant::now();
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("child subagent wait failed: {error}"))?
            {
                let mut stdout = String::new();
                if let Some(mut handle) = child.stdout.take() {
                    handle
                        .read_to_string(&mut stdout)
                        .map_err(|error| format!("failed to read child stdout: {error}"))?;
                }
                let mut stderr = String::new();
                if let Some(mut handle) = child.stderr.take() {
                    handle
                        .read_to_string(&mut stderr)
                        .map_err(|error| format!("failed to read child stderr: {error}"))?;
                }

                if !status.success() {
                    let stderr = stderr.trim();
                    let detail = if stderr.is_empty() {
                        String::new()
                    } else {
                        format!("; stderr={stderr}")
                    };
                    return Err(format!(
                        "child subagent exited with status {}{detail}",
                        status
                    ));
                }

                let response: DispatchResponse = serde_json::from_str(stdout.trim()).map_err(
                    |error| {
                        let stderr = stderr.trim();
                        let detail = if stderr.is_empty() {
                            String::new()
                        } else {
                            format!("; stderr={stderr}")
                        };
                        format!(
                            "child subagent returned invalid JSON: {error}; stdout={stdout:?}{detail}"
                        )
                    },
                )?;

                if response.dispatch_id != request.dispatch_id {
                    return Err(format!(
                        "child subagent response dispatch id mismatch: expected {}, got {}",
                        request.dispatch_id.as_str(),
                        response.dispatch_id.as_str()
                    ));
                }

                return Ok(response);
            }

            if started_at.elapsed() >= self.timeout {
                let _ = child.kill();
                let mut stdout = String::new();
                if let Some(mut handle) = child.stdout.take() {
                    let _ = handle.read_to_string(&mut stdout);
                }
                let mut stderr = String::new();
                if let Some(mut handle) = child.stderr.take() {
                    let _ = handle.read_to_string(&mut stderr);
                }
                let _ = child.wait();
                let stderr = stderr.trim();
                let detail = if stderr.is_empty() {
                    String::new()
                } else {
                    format!("; stderr={stderr}")
                };
                return Err(format!(
                    "child subagent timed out after {:?}{detail}",
                    self.timeout
                ));
            }

            thread::sleep(Duration::from_millis(10));
        }
    }

    fn validate_spawn_request(request: &DispatchRequest) -> Result<(), String> {
        match &request.target {
            DispatchTarget::Subagent { name } => {
                if name.trim().is_empty() {
                    return Err(
                        "process dispatcher requires a non-empty subagent target name"
                            .to_string(),
                    );
                }
                Ok(())
            }
            DispatchTarget::Worker { profile } => Err(format!(
                "process dispatcher only supports subagent targets; worker target `{profile}` is unsupported"
            )),
        }
    }
}

fn resolve_spawn_executable() -> io::Result<PathBuf> {
    if cfg!(test) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "child spawning is disabled in unit tests",
        ));
    }

    if let Ok(path) = std::env::var("CARGO_BIN_EXE_agent_in_rust") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let current_exe = std::env::current_exe()?;
    if let Some(candidate) = current_exe
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("agent_in_rust"))
        .filter(|path| path.is_file())
    {
        return Ok(candidate);
    }

    if current_exe
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value == "agent_in_rust")
        .unwrap_or(false)
    {
        return Ok(current_exe);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "agent_in_rust binary was not found for child spawning",
    ))
}

impl Dispatcher for ProcessDispatcher {
    fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResponse, String> {
        self.dispatch_spawned_subagent(request)
    }
}

pub struct LocalDispatcher<H> {
    handler: H,
}

impl<H> LocalDispatcher<H> {
    pub fn new(handler: H) -> Self {
        Self { handler }
    }
}

impl<H> Dispatcher for LocalDispatcher<H>
where
    H: Fn(DispatchRequest) -> Result<DispatchResponse, String>,
{
    fn dispatch(&self, request: DispatchRequest) -> Result<DispatchResponse, String> {
        (self.handler)(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mainAgent::ContextPacket;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn sample_context_packet() -> ContextPacket {
        ContextPacket {
            goal: "Inspect the repo".to_string(),
            constraints: vec!["Keep changes small".to_string()],
            relevant_files: vec!["src/main.rs".to_string()],
            known_facts: vec!["CLI flow already exists".to_string()],
            missing_facts: vec!["Which files own delegation".to_string()],
            next_action: "Return a compact handoff.".to_string(),
            stop_condition: "Return a delegated result.".to_string(),
        }
    }

    #[test]
    fn dispatch_request_targets_a_subagent_without_direct_loop_coupling() {
        let request = DispatchRequest::for_subagent(
            "main-worker",
            "plan",
            "Produce a plan",
            sample_context_packet(),
            vec!["Workspace/MEMORY.md".to_string()],
            vec!["src/mainAgent.rs".to_string()],
            vec!["user: inspect dispatch".to_string()],
        );

        assert_eq!(request.requester, "main-worker");
        assert_eq!(
            request.target,
            DispatchTarget::Subagent {
                name: "plan".to_string(),
            }
        );
        assert_eq!(request.task, "Produce a plan");
        assert_eq!(request.file_refs, vec!["src/mainAgent.rs".to_string()]);
    }

    #[test]
    fn local_dispatcher_returns_structured_dispatch_response() {
        let dispatcher = LocalDispatcher::new(|request: DispatchRequest| {
            Ok(DispatchResponse {
                dispatch_id: request.dispatch_id.clone(),
                responder: "subagent plan".to_string(),
                summary: "planned the work".to_string(),
                findings: vec!["Load the relevant files".to_string()],
                artifact_refs: vec!["src/mainAgent.rs".to_string()],
                recommended_next_action: "Implement the change.".to_string(),
                final_text: "Plan agent output".to_string(),
                usage: Vec::new(),
            })
        });
        let request = DispatchRequest::for_subagent(
            "main-worker",
            "plan",
            "Produce a plan",
            sample_context_packet(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let response = dispatcher
            .dispatch(request.clone())
            .expect("local dispatch should succeed");

        assert_eq!(response.dispatch_id, request.dispatch_id);
        assert_eq!(response.responder, "subagent plan");
        assert_eq!(response.summary, "planned the work");
        assert_eq!(response.final_text, "Plan agent output");
    }

    #[cfg(unix)]
    #[test]
    fn process_dispatcher_parses_child_stdout() {
        let temp_dir = std::env::temp_dir().join(format!(
            "agent-in-rust-dispatch-{}",
            EventId::generate().as_str()
        ));
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let script = temp_dir.join("child.sh");
        fs::write(
            &script,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"dispatch_id\":\"evt-fixed\",\"responder\":\"subagent plan\",\"summary\":\"planned the work\",\"findings\":[\"Loaded the relevant files\"],\"artifact_refs\":[\"src/mainAgent.rs\"],\"recommended_next_action\":\"Implement the change.\",\"final_text\":\"Plan agent output\",\"usage\":[]}'\n",
        )
        .expect("script should write");
        let mut permissions = fs::metadata(&script)
            .expect("script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("script should be executable");

        let dispatcher = ProcessDispatcher::with_executable(
            &script,
            &temp_dir,
            Duration::from_secs(2),
        );
        let request = DispatchRequest {
            dispatch_id: EventId::new("evt-fixed"),
            requester: "main-worker".to_string(),
            target: DispatchTarget::Subagent {
                name: "plan".to_string(),
            },
            task: "Produce a plan".to_string(),
            context_packet: sample_context_packet(),
            memory_refs: Vec::new(),
            file_refs: Vec::new(),
            observations: Vec::new(),
        };

        let response = dispatcher
            .dispatch(request.clone())
            .expect("process dispatch should succeed");

        assert_eq!(response.dispatch_id, request.dispatch_id);
        assert_eq!(response.responder, "subagent plan");
        assert_eq!(response.summary, "planned the work");
    }

    #[cfg(unix)]
    #[test]
    fn process_dispatcher_times_out_when_child_does_not_exit() {
        let temp_dir = std::env::temp_dir().join(format!(
            "agent-in-rust-dispatch-timeout-{}",
            EventId::generate().as_str()
        ));
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let script = temp_dir.join("child.sh");
        fs::write(
            &script,
            "#!/bin/sh\nsleep 2\nprintf '%s' '{\"dispatch_id\":\"evt-fixed\",\"responder\":\"subagent plan\",\"summary\":\"planned the work\",\"findings\":[],\"artifact_refs\":[],\"recommended_next_action\":\"Implement the change.\",\"final_text\":\"Plan agent output\",\"usage\":[]}'\n",
        )
        .expect("script should write");
        let mut permissions = fs::metadata(&script)
            .expect("script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("script should be executable");

        let dispatcher = ProcessDispatcher::with_executable(
            &script,
            &temp_dir,
            Duration::from_millis(100),
        );
        let request = DispatchRequest {
            dispatch_id: EventId::new("evt-fixed"),
            requester: "main-worker".to_string(),
            target: DispatchTarget::Subagent {
                name: "plan".to_string(),
            },
            task: "Produce a plan".to_string(),
            context_packet: sample_context_packet(),
            memory_refs: Vec::new(),
            file_refs: Vec::new(),
            observations: Vec::new(),
        };

        let error = dispatcher
            .dispatch(request)
            .expect_err("process dispatch should time out");

        assert!(error.contains("timed out"));
    }

    #[test]
    fn process_dispatcher_rejects_worker_targets_before_spawning() {
        let dispatcher = ProcessDispatcher::with_executable(
            "/path/that/should/not/run",
            std::env::temp_dir(),
            Duration::from_secs(1),
        );
        let request = DispatchRequest {
            dispatch_id: EventId::new("evt-fixed"),
            requester: "main-worker".to_string(),
            target: DispatchTarget::Worker {
                profile: "planner".to_string(),
            },
            task: "Produce a plan".to_string(),
            context_packet: sample_context_packet(),
            memory_refs: Vec::new(),
            file_refs: Vec::new(),
            observations: Vec::new(),
        };

        let error = dispatcher
            .dispatch(request)
            .expect_err("worker targets should be rejected before spawn");

        assert!(error.contains("only supports subagent targets"));
        assert!(error.contains("planner"));
    }
}
