# Sprint v2 - Walkthrough

## Summary
Sprint v2 turns the repository from a CLI-first harness into a legible control-plane skeleton. The current `MainAgent` loop still runs locally, but it now sits behind explicit bus, channel, router, worker, dispatch, scheduler, concurrency, prompt-layer, and memory-agent seams so the architecture can grow without a rewrite.

The sprint also tightens the repo's documentation and validation surfaces. The result is still small and understandable, but the major orchestration pieces are now named, testable, and documented in repo-local files instead of being implied by one large runtime loop.

## Architecture Overview

```text
CLI / one-shot input
        |
        v
  Channel adapter
        |
        v
  In-process bus  <------------------+
        |                            |
        v                            |
      Router                         |
        |                            |
        v                            |
   Main worker runtime                |
        |                            |
        +--> Prompt layers           |
        +--> Memory agent            |
        +--> Tools / skills          |
        +--> Dispatch to subagents   |
        +--> Scheduler events        |
        +--> Concurrency gate        |
                                     |
        +----------------------------+
                   Outbound delivery
```

## Files Created/Modified

### `sprints/v2/PRD.md`
**Purpose**: Defines the target OpenClaw-style architecture for the sprint.

**Key functions/components**:
- The event-driven control plane goals.
- The bus/channel/router/worker/dispatch/scheduler/concurrency/memory seams.

**How it works**:
This file sets the sprint contract. It makes the architectural direction explicit: keep the existing CLI runtime alive, but surround it with named boundaries so later work can move from a single loop to a control plane without breaking the current harness. The PRD also defines the migration order, which is important because this sprint is about structure, not feature expansion.

### `sprints/v2/TASKS.md`
**Purpose**: Tracks the implemented sprint backlog and completion notes.

**Key functions/components**:
- Completed task list from scaffolding through graphify alignment.
- File ownership for each seam.

**How it works**:
The task list records what was actually landed and when. It is the quickest way to see that v2 covered not just docs, but the full set of architecture seams: bus, channel, worker, router, dispatch, prompt layers, memory-agent, scheduler, concurrency, outbound delivery, and the supporting documentation.

### `docs/openclaw-gap.md`
**Purpose**: Explains the gap between the current harness and the sprint v2 target architecture.

**Key functions/components**:
- Current harness summary.
- Migration seams for bus, channels, routing, scheduler, dispatch, prompt layers, concurrency, and memory-agent boundaries.

**How it works**:
This doc is the bridge between the existing `MainAgent` design and the new control-plane shape. It is intentionally honest about the current state: the repo already has planning, memory, tools, skills, and traces, but those capabilities are still owned by a single runtime. The document explains where each future seam belongs and why.

### `docs/architecture.md`
**Purpose**: Documents the new control-plane skeleton and the local runtime path.

**Key functions/components**:
- `MainAgent` as the current worker core.
- Bus, channel, router, worker, dispatch, scheduler, concurrency, and prompt-layer sections.

**How it works**:
The architecture doc was updated to describe the local control-plane path in concrete terms. It now explains that the CLI is just one channel, that scheduled work and delivery use explicit seams, and that dispatch is a dedicated contract instead of an implicit call path inside the main loop.

### `docs/cli.md`
**Purpose**: Documents the operator-facing command surface.

**Key functions/components**:
- Session, run, list, compact, eval, skills, and init commands.
- Slash commands and memory behavior.

**How it works**:
The CLI guide now matches the runtime path more closely. It explains how the session command enters the same control-plane flow as future channels, how compaction and memory behave, and how the skill system and review workflow are supposed to be used.

### `docs/memory.md`
**Purpose**: Documents standing memory precedence and the narrow seam for future path-scoped loading.

**Key functions/components**:
- Project memory precedence.
- `MemoryLoadRequest` and selector hints.
- Long-term memory boundary through `MemoryAgent`.

**How it works**:
The memory doc makes two things explicit: current behavior and intended future behavior. Current behavior still loads the full standing memory stack, but the doc describes selector hints and focus paths as the seam for future path-aware memory retrieval without silently dropping core repo instructions.

### `docs/observability.md`
**Purpose**: Describes tracing and exporter behavior.

**Key functions/components**:
- LangSmith and Langfuse OTLP/HTTP configuration.
- Runtime trace spans and events.

**How it works**:
This doc records what is traced and how the OTLP exporters are configured. The important detail is that observability is treated as part of the harness, not as an afterthought: planner decisions, tool execution, subagent spans, and context previews are all visible.

### `docs/skills.md`
**Purpose**: Defines the local skill contract and authoring expectations.

**Key functions/components**:
- Skill file locations and required frontmatter.
- Allowed tools, preferred subagents, and validation behavior.

**How it works**:
The skills doc turns skills into a real repo surface instead of an informal prompt convention. It explains how skills are loaded, when they activate, and how the harness narrows tool access when a skill declares an allowlist.

### `docs/tooling-audit.md`
**Purpose**: Documents the Node audit findings and the repo boundary around them.

**Key functions/components**:
- Audit summary.
- Why the findings are global-tooling related, not repo-local.

**How it works**:
This doc keeps the audit scope bounded. It clarifies that the current vulnerabilities come from the user-level Node toolchain, so the repository does not pretend to fix them with an invented lockfile.

### `src/main.rs`
**Purpose**: CLI entrypoint and command dispatcher.

**Key functions/components**:
- `parse_command`
- `resolve_system_prompt`
- `run_session`
- `run_evals`
- `print_catalog`

**How it works**:
`main.rs` stays thin. It parses the top-level commands, resolves the system prompt from explicit input or `system_prompt/`, and then hands work to `MainAgent` through the new channel/worker/router seam.

The important runtime path is the session runner:

```rust
fn run_session(agent: &mut MainAgent, options: SessionOptions) -> io::Result<()> {
    let channel = CliChannel::new(CliInvocation::new(
        options.user_input.clone(),
        options.show_trace,
        options.one_shot,
    ));
    let mut bus = InProcessBus::default();
    let event = channel.into_inbound_event().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "`run` requires `--input`.")
    })?;
    bus.publish_inbound(event)?;
    let request_event = bus.forward_inbound().ok_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "CLI channel did not enqueue an inbound event.")
    })?;
    let route = DefaultRouter::new().route(&request_event)?;
    ...
}
```

This is the key structural change: CLI input now becomes an event, goes through a bus, gets routed, and then reaches the worker runtime.

### `src/mainAgent.rs`
**Purpose**: Core runtime, planner loop, memory loading, skill/subagent handling, and eval preview logic.

**Key functions/components**:
- `MainAgent::from_env`
- `MainAgent::run`
- `MainAgent::wait_for_context`
- `MainAgent::preview_planner_decision`
- `MainAgent::delegate_to_subagent`
- `MainAgent::build_context_packet`
- `MainAgent::build_task_contract`
- `planner_prompt`
- `decide_next_step`

**How it works**:
`MainAgent` remains the execution core, but it now has explicit boundaries around it. It loads memory, tools, skills, subagents, and observability from a single initialization path, then runs a one-action-per-iteration planner loop with explicit retry and stop rules.

The `run_with_state` loop is the heart of the harness:

```rust
loop {
    if step_count >= MAX_LOOP_STEPS {
        ...
        return AgentResult::stopped(reason, trace, usage);
    }

    step_count += 1;
    let task_contract = self.build_task_contract(input, &observations);
    let planner_run =
        self.decide_next_step(state, input, &task_contract, &observations, retry_count);

    match planner_run.decision {
        Decision::CallTool { .. } => { ... }
        Decision::UseSkill { .. } => { ... }
        Decision::DelegateSubagent { .. } => { ... }
        Decision::Finish { .. } => { ... }
        Decision::Retry(reason) => { ... }
        Decision::Stop(reason) => { ... }
    }
}
```

That loop enforces the sprint standard: exactly one action per iteration, trace everything, and stop cleanly when the task is done or the retry budget is exhausted.

The planner input is also structured through explicit layers:

```rust
PlannerPromptLayers {
    system: SystemPromptLayer { ... },
    project_memory: ProjectMemoryLayer { ... },
    task_contract: TaskContractLayer { ... },
    available_skills: ...,
    active_skill: ...,
    observations: ObservationLayer { ... },
    channel_metadata: ChannelMetadataLayer { ... },
    tool_context: ...,
    subagent_context: ...,
    history_summary,
    retry_count,
}
```

That is the main prompt-engineering improvement in v2: prompt assembly is now inspectable data, not an opaque concatenated block.

### `src/bus.rs`
**Purpose**: Event envelope types and the in-process bus skeleton.

**Key functions/components**:
- `EventId`
- `EventSource`
- `InboundEvent`
- `OutboundEvent`
- `Bus`
- `InProcessBus`

**How it works**:
The bus introduces explicit event envelopes for inbound and outbound traffic. Each event carries source metadata, a timestamp, and a typed body so the control plane can reason about requests and delivery separately.

```rust
pub trait Bus {
    fn publish_inbound(&mut self, event: InboundEvent) -> io::Result<()>;
    fn publish_outbound(&mut self, event: OutboundEvent) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct InProcessBus {
    inbound_queue: VecDeque<InboundEvent>,
    outbound_queue: VecDeque<OutboundEvent>,
}
```

The local implementation is intentionally simple FIFO storage. That keeps the seam real without introducing distributed infrastructure too early.

### `src/channels/mod.rs`
**Purpose**: Channel adapter and delivery contracts.

**Key functions/components**:
- `ChannelAdapter`
- `ChannelWorker`
- `DeliveryReceipt`
- `summarize_outbound_event`

**How it works**:
This module separates channel mapping from channel delivery. Adapters turn transport-specific input into inbound events, while workers accept outbound events and produce delivery receipts.

That split matters because it keeps the worker runtime transport-agnostic. The CLI is just one channel implementation, not the whole architecture.

### `src/channels/cli.rs`
**Purpose**: CLI channel adapter and CLI delivery worker.

**Key functions/components**:
- `CliInvocation`
- `CliChannel`
- `CliDeliveryWorker`

**How it works**:
`CliChannel` converts interactive or one-shot terminal input into an inbound event. One-shot input becomes a `UserMessage`; interactive mode becomes a `session` command with trace flags preserved.

```rust
impl ChannelAdapter for CliChannel {
    fn into_inbound_event(&self) -> Option<InboundEvent> {
        let source = self.event_source();

        if self.invocation.one_shot {
            return self.invocation.user_input.as_ref().map(|input| {
                InboundEvent::user_message(EventId::generate(), source, input.clone())
            });
        }

        Some(InboundEvent::command(
            EventId::generate(),
            source,
            "session",
            self.command_args(),
        ))
    }
}
```

The delivery worker stores a summarized form of outbound messages so the CLI path can be reused as a real channel worker later.

### `src/router.rs`
**Purpose**: Ownership routing for inbound events.

**Key functions/components**:
- `RouteTarget`
- `RouteDecision`
- `Router`
- `DefaultRouter`

**How it works**:
The router is intentionally minimal right now. It always routes to the main worker, but the existence of the seam means ownership decisions are now explicit before execution.

This is an important separation from the old design, where the runtime itself implicitly owned every inbound request.

### `src/worker.rs`
**Purpose**: Worker-facing runtime contract around `MainAgent`.

**Key functions/components**:
- `WorkerRequest`
- `WorkerResult`
- `WorkerRuntime`
- `WorkerAgentRuntime`
- `MainAgentWorker`

**How it works**:
The worker module wraps the agent loop behind request/result contracts. A user message becomes a `run` call, while a session command becomes a `wait_for_context` call using explicit wait-mode configuration.

That makes the current agent loop behave like a worker instead of the whole system.

### `src/dispatch.rs`
**Purpose**: Delegated-work request and response contracts.

**Key functions/components**:
- `DispatchTarget`
- `DispatchRequest`
- `DispatchResponse`
- `Dispatcher`
- `LocalDispatcher`

**How it works**:
Dispatch makes sub-work an explicit contract. The request bundles the goal, context packet, memory references, file references, and observations, while the response returns findings, artifacts, a recommendation, and final text.

This keeps delegation legible and makes it easy to move from direct in-loop behavior to worker-to-worker handoffs later.

### `src/prompt_layers.rs`
**Purpose**: Structured planner prompt assembly.

**Key functions/components**:
- `SystemPromptLayer`
- `ProjectMemoryLayer`
- `TaskContractLayer`
- `ActiveSkillLayer`
- `ObservationLayer`
- `ChannelMetadataLayer`
- `PlannerPromptLayers`

**How it works**:
Prompt construction is now a data structure with a stable render order. The planner sees the system instructions, channel metadata, memory, tools, subagents, skills, active skill, task contract, observations, and compacted history as separate sections.

That is the main defense against prompt sprawl in this sprint.

### `src/memory_agent.rs`
**Purpose**: Dedicated long-term memory boundary.

**Key functions/components**:
- `MemoryAgent`
- `MemoryAgentSnapshot`
- `FileMemoryAgent`
- `StubMemoryAgent`

**How it works**:
The memory-agent seam keeps Mem0 and the file fallback behind one interface. The runtime can read and append durable notes without knowing which backend is active.

The stub agent exists so future worker-shaped memory routing can be tested without depending on a real backend.

### `src/scheduler.rs`
**Purpose**: Explicit scheduled-event contracts and a local scheduler skeleton.

**Key functions/components**:
- `ScheduledEvent`
- `Scheduler`
- `LocalScheduler`

**How it works**:
The scheduler is deliberately manual. It only emits events that were explicitly enqueued, which means there is no hidden background work. That is the right shape for a skeleton: the seam is real, but it stays deterministic.

### `src/concurrency.rs`
**Purpose**: Local admission control for worker execution.

**Key functions/components**:
- `WorkerLease`
- `ConcurrencyGate`
- `LocalConcurrencyGate`
- `GatedWorkerRuntime`

**How it works**:
Concurrency is modeled as leases and release rules. The gate prevents execution when no slots are available and ensures slots are released after the worker finishes.

This keeps admission control in the harness instead of hiding it inside the worker.

### `src/evals.rs`
**Purpose**: Planner eval fixture parsing and suite rendering.

**Key functions/components**:
- `PlannerEvalFixture`
- `PlannerEvalExpectedDecision`
- `PlannerEvalActualDecision`
- `PlannerEvalSuiteResult`
- `parse_eval_fixture`
- `load_eval_fixtures`

**How it works**:
The eval module lets the harness compare expected and actual planner decisions from JSON fixtures. It validates the fixture shape, loads sorted JSON files, and renders pass/fail output with enough context to debug a mismatch.

### `src/Tools/mod.rs`
**Purpose**: Built-in tool registration and MCP tool wrapping.

**Key functions/components**:
- `Tool`
- `default_tools`
- `WEB_SEARCH_TOOL_NAME`
- `LEGACY_WEB_SEARCH_TOOL_NAME`

**How it works**:
The tool module now exposes a grounded web-search tool and the MCP tool wrapper under one registry. Built-in tools return `StepOutcome`, while MCP tools are wrapped so they can participate in the same planner flow.

### `src/observability.rs`
**Purpose**: OpenTelemetry tracing and exporter setup.

**Key functions/components**:
- `Observability`
- `from_env`
- `with_span`
- `record_event`
- `record_log`

**How it works**:
This module turns tracing into a harness service instead of ad hoc logging. It configures LangSmith and Langfuse OTLP exporters from environment variables, records events on the active span, and falls back cleanly when exporters are unavailable or misconfigured.

The important part is that the runtime can attach span metadata to planner calls, tool calls, subagent work, and runtime log lines without making those details part of the business logic.

### `system_prompt/system_prompt.md`
**Purpose**: Default system prompt content for the CLI runtime.

**Key functions/components**:
- The harness-first operating instructions.

**How it works**:
This prompt now aligns with the sprint standard: start from a task contract, preserve explicit context, take one action per iteration, and keep the trace legible. It serves as the default behavior scaffold when no custom system prompt is provided.

### `Workspace/MEMORY.md`
**Purpose**: File-backed long-term memory fallback.

**Key functions/components**:
- Durable notes promoted by the runtime.

**How it works**:
This file is the non-Mem0 fallback for long-term memory. It stays intentionally simple because the real behavior lives in the memory-agent boundary; this file is just the persistent storage target.

### `.claude/agents/explore.md`
**Purpose**: Project subagent definition for inspection and handoff work.

**Key functions/components**:
- `web_search_tool` access for exploration.

**How it works**:
The explore agent is designed to return a compact handoff instead of implementation. That matches the repo's subagent strategy: use specialized agents for bounded discovery, not for sprawl.

### `.claude/skills/ship-small/SKILL.md`
**Purpose**: Small-change implementation skill.

**Key functions/components**:
- Goal and workflow for minimal coherent changes.

**How it works**:
This skill reinforces the sprint's execution style. It biases work toward the smallest testable change set and makes assumptions explicit, which is aligned with the control-plane design.

### `.env.example`
**Purpose**: Example environment variables for local setup.

**Key functions/components**:
- OpenAI, Anthropic, Perplexity, and agent prompt variables.

**How it works**:
The example environment file documents the minimum inputs required for model access and web-search wiring. It is a setup aid, not runtime logic, but it helps keep the repo operable.

### `.gitignore`
**Purpose**: Excludes local secrets and generated state.

**Key functions/components**:
- `.env`
- `target/`
- `.venv/`
- `Workspace/short-term.json`

**How it works**:
The ignore rules keep local secrets and generated session state out of version control. That matters more now that the runtime persists short-term context and context previews.

### `graphify-out/GRAPH_REPORT.md`
**Purpose**: Updated knowledge-graph summary for the repo.

**Key functions/components**:
- God nodes and community breakdown.
- Cross-community bridge analysis.

**How it works**:
The graph report was rebuilt to reflect the new architecture. It now highlights the new module seams, shows the high-connectivity nodes, and provides a quick map of where the repo's conceptual clusters live.

### `graphify-out/graph.json`
**Purpose**: Machine-readable graph data behind the report.

**Key functions/components**:
- Graph nodes, edges, and community metadata.

**How it works**:
This is the backing artifact for the graph report. It matters because future agents can use it to navigate the repo structure without re-deriving module relationships from scratch.

### `history/context_history.json`
**Purpose**: Append-only planner context snapshots.

**Key functions/components**:
- Stored LLM context previews.

**How it works**:
This file is the audit trail for prompt assembly. Each snapshot captures the system prompt, planner prompt, prior history, current input, and final request JSON so context changes are visible after the fact.

### `evals/fixtures/finish-observation.json`
**Purpose**: Verifies that the planner finishes when the latest observation already answers the task.

**How it works**:
The fixture expects `finish`, which matches the harness rule that an already-sufficient observation should stop the loop instead of forcing another action.

### `evals/fixtures/retry-recoverable-observation.json`
**Purpose**: Verifies recoverable retry handling.

**How it works**:
The fixture feeds a recoverable tool failure observation and expects `retry`, which checks that the harness preserves retry semantics instead of treating the failure as terminal.

### `evals/fixtures/skill-ship-small.json`
**Purpose**: Verifies explicit skill routing.

**How it works**:
The fixture expects the `ship-small` skill when the user asks to keep a change scoped. That checks that reusable skill packs can be selected intentionally.

### `evals/fixtures/stop-empty-input.json`
**Purpose**: Verifies immediate stop on empty input.

**How it works**:
The fixture expects `stop` for an empty request. That aligns with the harness rule that empty input is a stop condition, not a planner task.

### `evals/fixtures/subagent-plan.json`
**Purpose**: Verifies delegation to the plan subagent.

**How it works**:
The fixture expects `delegate` with subagent `plan` for a planning-oriented request. That checks the subagent seam and the planner's ability to route exploratory or planning work outward.

### `evals/fixtures/tool-web-search.json`
**Purpose**: Verifies web-tool routing.

**How it works**:
The fixture expects `tool` with `web_search_tool` when the user asks for web research. That confirms the grounded web-search tool remains visible to planner selection.

### `AGENTS.md`
**Purpose**: Repo operating standard and map to durable docs.

**Key functions/components**:
- Context engineering standard.
- Harness engineering standard.
- Graphify guidance.

**How it works**:
The repository instructions were kept short and high-signal, with pointers to the durable docs for architecture, CLI, memory, and tooling. That is consistent with the sprint's goal of keeping the repo understandable without burying policy in one giant instruction blob.

## Data Flow

1. The user starts a CLI session or runs a one-shot command.
2. `src/main.rs` resolves the system prompt, creates `MainAgent`, and adapts the CLI into a `CliChannel`.
3. The channel converts the terminal interaction into an `InboundEvent`.
4. `InProcessBus` stores the event and forwards it to `DefaultRouter`.
5. The router chooses the `MainWorker` target.
6. `MainAgentWorker` translates the inbound event into either `run` or `wait_for_context`.
7. `MainAgent` loads memory, builds a task contract, assembles prompt layers, and selects exactly one action.
8. The planner can call a tool, activate a skill, delegate to a subagent, retry, stop, or finish.
9. Tool and subagent results are recorded as observations and appended to trace and short-term memory.
10. Outbound results can be summarized for channel delivery through the outbound seam.

## Test Coverage

- Unit: `src/bus.rs`, `src/channels/cli.rs`, `src/router.rs`, `src/worker.rs`, `src/dispatch.rs`, `src/prompt_layers.rs`, `src/memory_agent.rs`, `src/scheduler.rs`, `src/concurrency.rs`, `src/evals.rs`, and `src/Tools/mod.rs` all carry focused tests for the seam they define.
- Integration: `src/main.rs` exercises CLI parsing, session routing, eval command wiring, and project scaffolding through the live harness.
- E2E: The seeded eval fixtures cover tool routing, skill routing, delegation, retry, finish, and empty-input stop behavior, but there is no separate external browser or distributed end-to-end suite in this sprint.

## Security Measures

- Long-term memory writes go through a dedicated `MemoryAgent` boundary instead of ad hoc file writes in the main loop.
- Skill validation can reject malformed or missing skill files instead of loading them silently.
- Active skills can narrow the visible tool set with `allowed_tools`.
- The scheduler emits only explicitly enqueued events, which avoids hidden background execution.
- The concurrency gate models worker slots and refuses execution when capacity is exhausted.
- Observability config is opt-in and warns on misconfiguration instead of crashing the harness.

## Known Limitations

- The router still defaults everything to the main worker.
- Subagent execution is still locally synthesized rather than a true distributed dispatch target.
- The scheduler is a skeleton with manual enqueueing only.
- The concurrency layer is local admission control, not a distributed lock or queue.
- Planner fallback still includes a heuristic path when model-backed planning is unavailable.
- The sprint updated graphify outputs, but the graph caches themselves are generated artifacts and not useful to read as narrative code changes.

## What's Next

- Replace heuristic delegation with a model-backed planner interface.
- Add richer path-scoped memory selection on top of `MemoryLoadRequest` and selector hints.
- Move skill and command frontmatter parsing to stronger structured schemas.
- Add external tool adapters with clearer validation and timeout policies.
- Expand eval coverage so tool choice, tool arguments, retries, and stop behavior are all regression-checked at the harness level.
