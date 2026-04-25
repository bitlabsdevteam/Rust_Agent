## Sprint Overview

Sprint v2 shifts this repository from a strong CLI-first single-runtime harness toward an OpenClaw-style architecture skeleton. The purpose of this sprint is not to deeply implement every subsystem, but to introduce the structural seams that OpenClaw-like systems need: event-driven flow, multi-surface channels, routing, scheduling, prompt layering, dispatch, concurrency control, and a dedicated memory-agent boundary.

The outcome of this sprint is a legible architectural backbone that keeps the current Rust harness small while making future multi-agent and multi-surface expansion possible without a rewrite. Done correctly, v2 should leave the repo still understandable, but no longer trapped in a single terminal-loop architecture.

## Goals

- Introduce an explicit event-bus skeleton that separates inbound events, planner work, and outbound delivery.
- Add channel, router, scheduler, dispatch, prompt-layer, and concurrency abstractions as repo-local architecture seams.
- Preserve the current CLI runtime as one channel and one worker path inside the new architecture rather than replacing it.
- Define a dedicated memory-agent interface so long-term memory can evolve beyond direct in-loop reads and writes.
- Document the new architecture clearly enough that later sprints can implement depth on top of stable interfaces.

## User Stories

- As the framework author, I want an event-driven control plane skeleton so that this starter can grow toward OpenClaw without a major redesign.
- As the framework author, I want channel and routing abstractions so that CLI is no longer the only first-class surface.
- As the framework author, I want prompt layering and dispatch seams so that multi-agent coordination can be added incrementally instead of by prompt sprawl.
- As a developer extending the starter, I want clearly documented interfaces for router, scheduler, channel workers, and memory services so that I can implement one subsystem at a time.
- As a developer using this repo as an agent foundation, I want the current CLI behavior preserved while the architecture evolves so that new infrastructure does not break the existing harness.

## Technical Architecture

### Stack

- Language: Rust
- Existing runtime surface: local CLI
- Planner policy: OpenAI GPT-5.4 primary, Opus 4.6 fallback
- Existing execution core: `MainAgent` loop, tools, skills, subagents, trace, Mem0/file memory
- New architectural direction: event-driven orchestration skeleton with channel and worker boundaries
- Persistence direction: session/event records remain file-friendly first, with seams for future external adapters

### Target Architecture Skeleton

```text
                          +----------------------+
                          |   Control Plane      |
                          | bootstrap + wiring   |
                          +----------+-----------+
                                     |
                                     v
 +-------------+      +--------------------------+      +------------------+
 | CLI Channel | ---> | Inbound Event Bus        | ---> | Router           |
 +-------------+      | event envelopes + queue  |      | ownership choice |
                      +------------+-------------+      +--------+---------+
                                   |                             |
                                   v                             v
                      +--------------------------+      +------------------+
                      | Worker Runtime           | ---> | Dispatch         |
                      | planner + tool loop      |      | subagent handoff |
                      +------------+-------------+      +--------+---------+
                                   |                             |
               +-------------------+-------------------+         |
               |                   |                   |         |
               v                   v                   v         v
      +----------------+   +----------------+   +----------------+   +------------------+
      | Prompt Layers  |   | Memory Agent   |   | Scheduler      |   | Concurrency Gate |
      | system/task/   |   | long-term mem  |   | cron/queued    |   | limits + lease   |
      | skills/context |   | boundary       |   | work producer  |   | checks           |
      +----------------+   +----------------+   +----------------+   +------------------+
                                   |
                                   v
                      +--------------------------+
                      | Outbound Event Bus       |
                      | delivery envelopes       |
                      +------------+-------------+
                                   |
                                   v
                          +----------------------+
                          | Channel Workers      |
                          | CLI now, others later|
                          +----------------------+
```

### Data Flow

1. A channel adapter converts an external interaction into an inbound event envelope.
2. The event bus records and forwards the envelope to the router.
3. The router selects which worker or agent profile should own the event.
4. The worker runtime assembles prompt layers, memory context, visible tools, visible skills, and task state.
5. The worker executes one bounded action cycle and may dispatch delegated work through the dispatch interface.
6. Scheduler-produced events enter the same event bus path instead of bypassing the runtime.
7. Memory reads and durable writes cross a memory-agent boundary rather than being treated as an unstructured side effect.
8. Outbound results become delivery events handled by channel workers.
9. Concurrency controls guard worker admission, dispatch fan-out, and scheduled job execution.

### Architectural Principles For V2

- Keep the current CLI path working by adapting it into the new event model rather than deleting `MainAgent`.
- Prefer traits, structs, event types, and module boundaries over feature-complete implementations.
- Keep worker contracts explicit: input envelope, planner context, observation output, stop reason.
- Keep channel logic transport-specific and worker logic transport-agnostic.
- Keep prompt layering structured and inspectable instead of burying architecture in concatenated strings.
- Keep memory access behind one boundary so future dedicated memory agents or services fit naturally.

### Planned Module Direction

- `src/bus.rs`: inbound/outbound event types, bus trait, local in-process implementation
- `src/channels/`: CLI channel adapter now, placeholders for future surfaces
- `src/router.rs`: event ownership and agent-profile routing contracts
- `src/worker.rs`: worker execution boundary around the current planner loop
- `src/dispatch.rs`: delegated-work request/response contracts
- `src/prompt_layers.rs`: system, project memory, task, skills, observations, and channel metadata assembly
- `src/scheduler.rs`: scheduled-event trait and local no-op or manual tick implementation
- `src/concurrency.rs`: lease/token guard abstractions for worker execution
- `src/memory_agent.rs`: dedicated interface for long-term memory reads/writes
- `docs/openclaw-gap.md`: repo-local explanation of current gaps, target shape, and migration path

## Out of Scope

- Full Slack, Discord, email, or web channel implementations
- Production-grade WebSocket server or REST gateway
- Real distributed queue infrastructure
- Full cron/job persistence engine
- Deep multi-agent planning logic or autonomous agent societies
- Full memory-agent intelligence beyond interface and stub boundary
- Exhaustive prompt optimization or full template system
- Advanced security and tenancy hardening beyond architecture seams

## Dependencies

- Existing CLI harness in `src/main.rs` and `src/mainAgent.rs`
- Current docs in `docs/architecture.md`, `docs/cli.md`, and `docs/memory.md`
- Current tool, skill, subagent, trace, and Mem0/file memory foundations
- Repo-local standards in `AGENTS.md`
- Vault guidance on OpenClaw-style build order, coding-agent design space, context management, and memory architecture
