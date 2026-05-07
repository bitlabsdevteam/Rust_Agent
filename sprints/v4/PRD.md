# Sprint v4 - Spawned Sub-Agents

## Sprint Overview

Sprint v4 upgrades delegation from a local synthesized handoff into a real spawned-subagent boundary. The main agent still makes the delegation decision, but the delegated work now executes in a separate child process with typed request/response envelopes, bounded lifecycle rules, and traceable outcomes.

The goal is to keep the repository small and understandable while making subagent launches feel closer to Codex or Claude Code. This sprint focuses on the execution boundary, not on distributed orchestration or new agent autonomy.

## Goals

- Launch delegated subagent work in a separate process instead of a local callback.
- Preserve the current `explore`, `plan`, and `general-purpose` subagent definitions as the first child targets.
- Keep the parent agent as the only component that decides when delegation happens.
- Add explicit request/response IPC contracts with timeout and failure classification.
- Update docs, diagrams, and tests so the spawned boundary is observable and easy to reason about.

## User Stories

- As the main agent, I want to launch a child subagent process so delegated work is isolated from the parent loop.
- As a developer, I want the child request to carry a compact context packet so delegated work stays bounded.
- As a developer, I want the child response to be structured so the parent can record it in trace output and usage summaries.
- As an operator, I want delegation failures to be explicit so timeouts and bad child output do not look like normal planning.
- As a maintainer, I want the process boundary documented so future changes do not reintroduce hidden in-process coupling.

## Technical Architecture

### Stack

- Language: Rust
- Parent runtime: CLI-first `MainAgent`
- Child runtime: same binary invoked in a private subagent mode
- IPC: JSON over stdin/stdout
- Guardrails: timeout, target validation, and explicit failure classification
- Observability: parent/child trace correlation plus structured usage reporting

### Target Architecture

```text
User
  |
  v
CLI / session input
  |
  v
MainAgent planner
  |
  +--> tool
  +--> skill
  +--> delegate
               |
               v
      Process dispatcher
               |
               v
      Child subagent mode
               |
               v
     Structured child result
               |
               v
         Parent trace
```

### Data Flow

1. The parent builds the task contract, compact context packet, and relevant file references.
2. The planner selects `delegate` with a concrete subagent name.
3. The parent launches the same binary in a private child mode and sends the serialized dispatch request over stdin.
4. The child loads the subagent spec from the repo, executes the delegated subagent logic, and returns a structured response over stdout.
5. The parent enforces timeout and output validation before accepting the child result.
6. Parent trace and usage summaries record the child launch, result, and any failure classification.

### Architectural Principles

- Keep delegation decision-making in the parent.
- Keep child execution bounded and explicit.
- Prefer typed request/response payloads over ad hoc text parsing.
- Treat stderr as diagnostics, not as the structured response channel.
- Preserve the current subagent naming and file layout so the feature feels like an extension, not a rewrite.

## Out of Scope

- Distributed queues or remote execution
- Persistent child agent pools
- Multi-child fan-out in a single parent turn
- New planning models or autonomous agent societies
- New transport surfaces beyond CLI child mode
- Large refactors to the planner decision vocabulary

## Dependencies

- Existing `MainAgent` planner and trace loop
- Existing `.claude/agents/*.md` subagent definitions
- Dispatch, observability, and eval infrastructure from earlier sprints
- Repo-local architecture and CLI docs
- The current Rust binary entrypoint

