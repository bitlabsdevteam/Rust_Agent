# Architecture

## Purpose

This repository is a CLI-first Rust starter for harness-first agent systems. The core design goal is to keep the implementation small while making the operational subsystems legible: planner routing, memory loading, compaction, tools, skills, subagents, retries, stop conditions, and trace output.

The current architecture centers on `MainAgent` in `src/mainAgent.rs`. The CLI in `src/main.rs` is a thin entrypoint that parses commands, resolves the system prompt from `system_prompt/` or explicit overrides, and hands control to the runtime.

## Top-Level Structure

```text
src/main.rs
  -> CLI parsing and command dispatch
  -> MainAgent construction

src/mainAgent.rs
  -> runtime loop
  -> planner integration and failure classification
  -> memory loading and compaction
  -> slash command handling
  -> tool / skill / subagent execution

src/mcp.rs
  -> MCP catalog loading and tool registration

src/observability.rs
  -> OpenTelemetry setup and trace/event helpers

src/dispatch.rs
  -> delegated-work request/response contracts
  -> process dispatcher seam between worker ownership and spawned subagent execution

src/prompt_layers.rs
  -> explicit planner prompt-layer structures
  -> builder for system, memory, task, skills, observations, and channel metadata

src/scheduler.rs
  -> scheduled-event contracts
  -> local explicit-invocation scheduler skeleton

src/concurrency.rs
  -> worker lease and admission-control contracts
  -> local single-process concurrency gate
```

## Emerging Control-Plane Seams

The repository is in transition toward the sprint v2 control plane. Several seams now exist even though the overall runtime is still centered on `MainAgent`.

- `src/bus.rs` defines event envelopes plus the local in-process bus skeleton.
- `src/channels/` adapts CLI input into inbound events instead of treating the terminal path as the whole runtime.
- `src/channels/` also defines outbound channel-worker delivery contracts, with a local CLI delivery worker.
- `src/worker.rs` wraps the current agent loop behind a worker-facing request/result contract.
- `src/router.rs` makes event ownership explicit before worker execution.
- `src/dispatch.rs` defines delegated-work contracts so sub-work can cross a dispatch boundary instead of being treated as an implicit direct call.
- `src/prompt_layers.rs` separates prompt assembly into explicit layers instead of building planner context as one opaque string.
- `src/scheduler.rs` defines scheduled events and a local scheduler that produces no background work unless a caller explicitly enqueues an event.
- `src/concurrency.rs` defines worker leases and a local concurrency gate so admission control can wrap worker execution before distributed scheduling exists.

These seams are intentionally small. The current implementation still keeps the child subagent logic lightweight, but dispatch now crosses a real process boundary so later tasks can route through richer worker ownership and scheduling paths.

## Control Plane Skeleton

The current control-plane path is local and single-process:

```text
CLI invocation
  -> CliChannel
  -> InProcessBus
  -> DefaultRouter
  -> MainAgentWorker
  -> MainAgent loop
  -> WorkerResult / OutboundEvent
  -> ChannelWorker delivery seam
```

Scheduled work and concurrency are represented as contracts rather than background automation. `LocalScheduler` only emits events that were explicitly enqueued by the caller, and `LocalConcurrencyGate` provides conservative worker-slot leases for future worker admission wrappers.

## Runtime Flow

### 1. Startup

`MainAgent::from_env()` builds the runtime state:

- loads OpenAI and Anthropic planner clients if configured
- loads MCP tool definitions
- detects Mem0 configuration for long-term memory
- initializes model policy metadata
- computes workspace memory paths
- refreshes project state

The refresh step is the main state-loading boundary. `refresh_project_state()` reloads:

- merged memory stack
- short-term memory snapshot
- subagent definitions
- skill definitions
- custom slash commands

This keeps the runtime deterministic and lets file-backed project changes become visible without hidden state.

## Main Loop

The main execution path is `MainAgent::run_with_state()`.

At a high level:

1. load current session state
2. auto-compact when history exceeds the active budget
3. build a task contract from user input and latest observations
4. decide exactly one next action
5. execute that action
6. record the observation in trace and short-term memory
7. stop on final answer, explicit stop, empty input, retry exhaustion, or loop limit

The loop enforces the repo’s “one action per iteration” rule. Actions are:

- call a tool
- use a skill
- delegate to a subagent
- retry
- stop
- finalize with an answer

## Planner Flow

`decide_next_step()` owns planner routing.

Planner order:

1. OpenAI planner if configured
2. Anthropic fallback planner if configured
3. local heuristic router only when no model-backed planner is configured

Both model-backed planners receive explicit prompt layers for:

- system instructions
- project memory
- visible tools
- visible subagents
- skill context
- task contract
- recent observations
- channel metadata
- compacted and recent history summary

The planner prompt is assembled in `planner_prompt()` through `src/prompt_layers.rs`. It still instructs the model to choose one action and return strict JSON, but the input is now organized as explicit sections rather than one hand-built formatter block. If the model-backed planner fails, the runtime now classifies that failure as recoverable or terminal instead of silently dropping to the heuristic router, which keeps stop and retry behavior explicit.

## Memory Surfaces

The runtime has three memory layers.

### Project memory

Loaded by `load_memory_stack()`:

- `CLAUDE.md`
- imported `@path` markdown files
- long-term memory source

The merged output becomes planner memory context.

### Short-term memory

Stored in `Workspace/short-term.json`:

- recent conversation history
- compacted summary
- accumulated observations

This is loaded on session start and updated after runtime actions and compaction.

### Long-term memory

Long-term memory is loaded through one boundary:

- Mem0 if `MEM0_API_KEY` is configured
- `Workspace/MEMORY.md` fallback otherwise

`/remember <note>` promotes durable notes through the same boundary. `render_memory()` exposes the active backend and the current long-term preview.

## Context Compaction

Compaction is explicit and automatic.

- automatic compaction runs inside `run_with_state()` when active history exceeds configured message or character limits
- manual compaction is exposed through `/compact` and `compact_context()`

`compact_session_state()` summarizes older turns into `compacted_summary` and retains only the recent suffix. `planner_history_summary()` then feeds both compacted and recent context into the planner prompt.

This follows the repo’s context-management standard: do not silently drop active context, summarize it and keep the result visible.

## Execution Surfaces

### Tools

Tools are loaded from:

- built-in tool definitions
- MCP tool catalog

`call_tool()` is the execution boundary. Tool results are converted into explicit observations, and recoverable failures consume retry budget.

### Skills

Skills are loaded from project and user `.claude/skills/` directories. Each skill is a file-backed capability pack with metadata, workflow guidance, and optional tool/subagent preferences.

The runtime keeps two separate skill surfaces:

- visible skill catalog for planner selection
- one active skill for the current session step

`apply_skill()` activates a skill in session state instead of treating the skill body as a terminal answer. The next planner iteration receives the active skill instructions in a dedicated prompt layer. If the active skill declares `allowed_tools`, the harness narrows the visible tool set and enforces that allowlist at tool-execution time.

### Subagents

Subagents are loaded from project and user `.claude/agents/*.md`. `delegate_to_subagent()` constructs a compact context packet with:

- goal
- constraints
- relevant files
- known facts
- missing facts
- next action
- stop condition

The current implementation launches the child subagent as a separate process, passes the compact context packet over stdin, and reads the structured response back from stdout. The child process still uses the repo's lightweight `explore`, `plan`, and general-purpose handlers, but the execution boundary is now explicit.

### Delegation Boundary

Current synthesized delegation:

- the older local path delegated directly to an in-process synthesized subagent handler
- the parent and child shared one runtime boundary
- delegation results came back as a local callback rather than a spawned process result

Spawned child execution:

- `ProcessDispatcher` in `src/dispatch.rs` launches the same binary as a child worker
- the hidden `__spawn-subagent` command in `src/main.rs` switches the binary into child-subagent mode
- the parent writes a serialized `DispatchRequest` to stdin and reads a structured `DispatchResponse` from stdout
- the parent trace now records launch reason, launch mode, dispatch ID, responder, summary, and recommended next action
- unit tests still use a local fallback when child spawning is disabled, but that is now a test/runtime guardrail rather than the primary delegation path

### Slash Commands

`handle_slash_command()` provides operator-facing control surfaces for:

- help
- agents
- skills
- memory
- remember
- model
- clear
- compact
- mcp
- review
- skill
- init
- agent
- exit
- trace toggle

These commands matter architecturally because they expose human control over state, memory, and routing rather than leaving everything to the planner.

## Observability

The runtime records:

- planner backend
- model policy
- user input preview
- per-iteration planner decisions
- planner reasoning summaries
- tool, skill, and subagent observations
- retry usage
- stop reasons
- token usage summaries

`src/observability.rs` provides OpenTelemetry-backed spans and event recording. The user-facing trace printed in session mode is the compact debugging view; observability hooks are the structured evolution path.

## Key Extension Points

The safest extension seams in the current codebase are:

- `planner_prompt()` and `planner_payload_to_decision()` for planner behavior
- `load_memory_stack()` for richer memory precedence and path-scoped loading
- `call_tool()` and MCP loading for tool expansion
- `apply_skill()` and skill loaders for reusable capability packs
- `delegate_to_subagent()` and context packet construction for orchestration changes
- `compact_session_state()` for context-management policy

These are the main places to evolve the harness without turning the whole runtime into one opaque loop.

## Current Limits

The current architecture is intentionally narrow:

- CLI-first execution path
- no remote API/server surface
- no distributed queue or durable event log
- scheduler exists as an explicit local seam, not a background cron engine
- concurrency exists as a local worker-admission seam, not distributed locking
- no path-scoped memory precedence yet
- child subagents are process-spawned locally, not distributed or persistent workers
- local synthesized delegation still exists only as a fallback path when child spawning is unavailable

That is acceptable for v1. The next meaningful step is not more transport layers; it is making planner behavior regression-testable and better documented.
