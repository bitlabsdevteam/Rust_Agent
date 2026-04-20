## Sprint Overview

Sprint v1 formalizes this repository as a usable CLI-first agent starter for two audiences: the framework author evolving the harness and developers who want to build on top of it. This sprint focuses on turning the current scaffold into a coherent, documented, eval-aware baseline with clear runtime surfaces, durable memory behavior, and a first-class planner loop that is easier to inspect, test, and extend.

The outcome of this sprint is not a full production platform. It is a stable first sprint artifact set that makes the current harness legible, runnable, and ready for follow-on work in memory precedence, external adapters, and richer orchestration.

## Goals

- Establish a clear CLI-first agent starter with documented runtime behaviors and operator commands.
- Make planner behavior inspectable and regression-testable with a minimal eval fixture flow.
- Keep context, compaction, memory, tools, skills, and subagents visible in the harness rather than hidden in prompts.
- Document the current architecture and sprint direction in repo-local files so future agents can follow the intended design.
- Preserve small-repo ergonomics while improving readiness for future production-oriented upgrades.

## User Stories

- As a framework author, I want a documented baseline architecture so that I can evolve the harness without rediscovering design intent.
- As a framework author, I want planner decisions to be regression-testable so that routing changes do not silently break the loop.
- As a developer using the starter, I want clear CLI commands and memory behavior so that I can understand how to operate the agent locally.
- As a developer using the starter, I want visible trace and evaluation surfaces so that I can debug tool, skill, retry, and stop behavior.
- As a developer using the starter, I want scaffolded project artifacts and documentation so that I can adapt the starter without reading the whole codebase first.

## Technical Architecture

### Stack

- Language: Rust
- Runtime surface: local CLI
- Model policy: OpenAI GPT-5.4 primary, Opus 4.6 fallback
- Memory: `CLAUDE.md`, imported memory files, short-term JSON snapshot, Mem0-backed or file-backed long-term memory
- Extensibility: tools, MCP tools, skills, subagents, slash commands
- Observability: execution trace plus OpenTelemetry hooks

### Component Diagram

```text
+-------------+        +------------------+        +------------------+
| CLI Session | -----> | MainAgent Loop   | -----> | Planner Decision |
+-------------+        +------------------+        +------------------+
        |                       |                            |
        |                       |                            |
        v                       v                            v
+-------------+        +------------------+        +------------------+
| Slash Cmds  |        | Context Builder  |        | Tool / Skill /   |
| /memory etc |        | Task Contract    |        | Subagent Action  |
+-------------+        +------------------+        +------------------+
                                |                            |
                                v                            v
                       +------------------+        +------------------+
                       | Memory Sources   |        | Trace / Eval     |
                       | CLAUDE / JSON /  |        | Assertions       |
                       | Mem0 / imports   |        +------------------+
                       +------------------+
```

### Data Flow

1. User starts the CLI session or runs a one-shot command.
2. The runtime loads system prompt, user input, project memory, short-term memory, tools, skills, subagents, and commands.
3. The harness builds a task contract and planner context.
4. The planner selects exactly one action: tool, skill, subagent, retry, stop, or final answer.
5. The selected action produces an observation that is recorded in trace and short-term memory.
6. The loop continues until a final answer or stop condition is reached.
7. Sprint v1 adds a fixture-based eval path that runs the same planner and verifies expected decisions and stop behavior.

## Out of Scope

- GUI, web UI, API server, WebSocket surface, or event bus architecture
- Multi-channel delivery such as Slack, Discord, or email
- Advanced scheduler, cron, or background worker orchestration
- Deep security hardening beyond the existing local harness constraints
- Fully general external tool adapter framework with rich schema validation for every tool
- Long-session distributed storage or multi-user tenancy beyond basic Mem0 scoping

## Dependencies

- Existing CLI runtime in `src/main.rs` and `src/mainAgent.rs`
- Existing observability and MCP loading paths
- Current memory and compaction behavior already present in the scaffold
- Repo-local architecture guidance in `AGENTS.md`
- Vault guidance on context management, memory and persistence, and coding-agent design patterns
