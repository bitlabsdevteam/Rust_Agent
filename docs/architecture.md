# Architecture

## Purpose

This repository is a CLI-first Rust starter for harness-first agent systems. The core design goal is to keep the implementation small while making the operational subsystems legible: planner routing, memory loading, compaction, tools, skills, subagents, retries, stop conditions, and trace output.

The current architecture centers on `MainAgent` in `src/mainAgent.rs`. The CLI in `src/main.rs` is a thin entrypoint that parses commands, resolves the system prompt, and hands control to the runtime.

## Top-Level Structure

```text
src/main.rs
  -> CLI parsing and command dispatch
  -> MainAgent construction

src/mainAgent.rs
  -> runtime loop
  -> planner integration and heuristic fallback
  -> memory loading and compaction
  -> slash command handling
  -> tool / skill / subagent execution

src/mcp.rs
  -> MCP catalog loading and tool registration

src/observability.rs
  -> OpenTelemetry setup and trace/event helpers
```

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
3. local heuristic router if model-backed planning fails or is unavailable

Both model-backed planners receive:

- system prompt
- merged project memory
- visible tools
- visible subagents
- visible skills
- task contract
- recent observations
- compacted and recent history summary

The planner prompt is assembled in `planner_prompt()`. It explicitly instructs the model to choose one action and return strict JSON. This keeps decision-making inspectable and bounded rather than embedding hidden behavior in free-form responses.

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

Skills are loaded from project and user `.claude/skills/` directories. `apply_skill()` treats a skill as a reusable capability pack that returns its instructions and task fit as an observation.

### Subagents

Subagents are loaded from project and user `.claude/agents/*.md`. `delegate_to_subagent()` constructs a compact context packet with:

- goal
- constraints
- relevant files
- known facts
- missing facts
- next action
- stop condition

The current implementation executes local synthesized subagent behaviors for `explore`, `plan`, and general-purpose handoffs.

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

- CLI-first only
- no event bus
- no remote API/server surface
- no dedicated eval runner yet
- no path-scoped memory precedence yet
- subagent execution is still local synthesized behavior rather than a deeper agent runtime

That is acceptable for v1. The next meaningful step is not more transport layers; it is making planner behavior regression-testable and better documented.
