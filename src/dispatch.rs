#![allow(dead_code)]

use crate::bus::EventId;
use crate::mainAgent::{ContextPacket, TokenUsageRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchTarget {
    Worker { profile: String },
    Subagent { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
}
