# OpenClaw Gap Analysis

## Purpose

This document compares the current harness in this repository to the Target OpenClaw-Style Architecture described in sprint v2. The goal is not to overstate what the repo already has. The goal is to name the architectural gaps clearly enough that later tasks can land as small, legible seams instead of one large rewrite.

## Current Harness

The current harness is a strong single-runtime design centered on `MainAgent` in `src/mainAgent.rs`.

- `src/main.rs` parses CLI commands and hands control directly to `MainAgent`.
- `MainAgent::run_with_state()` owns the main loop, planner selection, tool calls, skill application, subagent delegation, retries, and stop behavior.
- Memory loading, compaction, and long-term memory access already exist, but they are still consumed from inside the same runtime.
- Observability exists through `src/observability.rs`, and tool integration exists through built-in tools plus `src/mcp.rs`.

This means the repo already has a useful agent worker core, but it does not yet have the control-plane boundaries that an OpenClaw-style system expects.

## Target OpenClaw-Style Architecture

The target shape for sprint v2 is an event-driven control plane around the existing agent loop rather than a replacement for it.

- inbound requests arrive as explicit events
- a bus records and forwards those events
- channels adapt external surfaces into the same event format
- routing decides which worker or agent profile owns the event
- the worker runtime executes one bounded cycle
- dispatch handles delegated work without direct in-loop coupling
- prompt layers are assembled as explicit inputs rather than one opaque string
- scheduler and concurrency boundaries control background work and admission
- long-term memory moves behind a dedicated memory-agent interface

In short: the current repo already contains a worker-like core, but OpenClaw-style growth requires moving surrounding orchestration concerns out of `MainAgent` and into named seams.

## Gap Summary

The main gap is architectural placement, not raw capability.

- The repo already has planning, tools, skills, memory, retries, and traces.
- The repo does not yet expose those capabilities through a bus-centered, channel-agnostic control plane.
- The current CLI path is the whole runtime entrypoint, while the target architecture treats CLI as one channel among several.
- The current runtime can delegate to subagents, but that delegation is still expressed as direct runtime behavior rather than explicit dispatch contracts between workers.

## Migration Seams

### bus

Current state:
- No event envelope types exist yet.
- No local bus trait or in-process bus implementation exists.
- Requests enter through direct function calls from the CLI layer.

Target seam:
- Introduce inbound and outbound event structs plus a `Bus` interface.
- Keep the first implementation local and in-process.
- Use the bus to record and forward requests before worker execution.

Why it matters:
- The bus is the break point between a chat loop and a reusable control plane.

### channels

Current state:
- The CLI is a direct entrypoint, not a channel adapter.
- There is no transport-neutral contract for inbound or outbound delivery.

Target seam:
- Move CLI handling behind a channel adapter interface.
- Treat CLI delivery as the first channel worker.
- Keep the contract generic enough for future chat, HTTP, or background surfaces.

Why it matters:
- Channels should describe transport details; workers should stay transport-agnostic.

### routing

Current state:
- Ownership decisions are implicit in the fact that `MainAgent` handles everything.
- Subagent selection exists, but it happens inside the runtime after control is already assigned.

Target seam:
- Add a router contract that chooses a target worker or agent profile for each inbound event.
- Start with a default route that sends everything to the current main path.

Why it matters:
- Routing should decide ownership before execution so later multi-agent or multi-surface expansion stays legible.

### scheduler

Current state:
- No scheduler contracts exist.
- Background or timed work has no shared event path.

Target seam:
- Add scheduled-event types and a local no-op scheduler skeleton.
- Route scheduler output through the same bus as human-triggered events.

Why it matters:
- Scheduled work should join the main orchestration path, not bypass it.

### dispatch

Current state:
- Delegation exists as runtime-local subagent behavior.
- There is no worker-to-worker request or response contract.

Target seam:
- Define dispatch request and response types for delegated work.
- Keep dispatch separate from direct method calls inside `MainAgent`.

Why it matters:
- OpenClaw-style systems distinguish routing from delegation: routing selects an owner, dispatch requests sub-work.

### prompt layers

Current state:
- `planner_prompt()` already assembles rich context, but the structure is still largely embedded in one prompt-building path.
- System prompt, memory, task contract, tools, skills, observations, and history are present but not modeled as reusable prompt-layer objects.

Target seam:
- Introduce explicit prompt layer structures for system, project memory, task contract, skill context, observations, and channel metadata.
- Keep prompt assembly inspectable and testable.

Why it matters:
- Prompt layer infrastructure prevents prompt sprawl and makes worker inputs easier to evolve safely.

### concurrency

Current state:
- The runtime is effectively single-process and single-admission.
- Retry and loop limits exist, but there is no explicit concurrency gate.

Target seam:
- Add worker admission interfaces such as leases, tokens, or guards.
- Start with a conservative local implementation.

Why it matters:
- Concurrency limits are a control-plane responsibility, not a hidden emergent property of the current CLI loop.

### memory-agent boundaries

Current state:
- Long-term memory already has a single usage boundary in practice: Mem0 when configured, file fallback otherwise.
- That boundary is still invoked from the main runtime rather than through a dedicated memory-agent interface.

Target seam:
- Introduce a memory-agent contract for long-term reads and writes.
- Keep Mem0 and file storage as initial backends behind that interface.

Why it matters:
- This preserves the current memory behavior while making future specialized memory workers possible without invasive runtime edits.

## Recommended Build Order

Based on the current harness and the OpenClaw-style reference material, the safest migration order is:

1. bus
2. local in-process bus implementation
3. channels
4. worker boundary
5. routing
6. dispatch
7. prompt layers
8. memory-agent boundaries
9. scheduler
10. concurrency

This order keeps the existing CLI runtime alive while moving architecture outward from the current loop.

## Design Constraints For This Repo

- Preserve the current CLI behavior while the control plane is introduced.
- Prefer traits, contracts, and small structs over premature distributed infrastructure.
- Keep the current `MainAgent` loop as the initial worker implementation instead of rewriting it.
- Keep docs aligned with code so later agents can navigate the new seams without re-deriving the architecture from chat history.

## Exit Condition For Task 2

Task 2 is complete when this document is specific enough to guide the implementation of bus, channels, routing, scheduler, dispatch, prompt layer, concurrency, and memory-agent work in the remaining sprint items without requiring a fresh architecture debate.
