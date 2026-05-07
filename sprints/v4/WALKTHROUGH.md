# Sprint v4 — Walkthrough

## Summary
Sprint v4 changed subagent delegation from an in-process synthesized callback into a real spawned child-process boundary. The parent `MainAgent` still decides when to delegate, but delegated work now crosses a typed IPC seam, runs in a private child mode of the same binary, returns a structured result, and is recorded with explicit trace and observability metadata.

The sprint also tightened the surrounding harness: dispatch IDs are serialized end to end, child launches are validated and time-bounded, the parent trace records child correlation details, the architecture and CLI docs now explain the spawned boundary, and graphify artifacts plus regression tests keep the new runtime shape visible.

## Architecture Overview
```text
User / CLI
   |
   v
MainAgent
   |
   +--> planner chooses delegate(subagent_name)
   |
   v
delegate_to_subagent()
   |
   v
DispatchRequest { dispatch_id, target, context_packet, refs, observations }
   |
   v
ProcessDispatcher
   |
   +--> validates target + timeout policy
   +--> launches same binary with "__spawn-subagent"
   |
   v
Child runtime
   |
   +--> loads subagent spec
   +--> runs lightweight explore / plan / general-purpose handler
   +--> returns DispatchResponse JSON
   |
   v
Parent trace + usage summary + observability events
```

## Files Created/Modified
### `sprints/v4/PRD.md`
**Purpose**: Defines Sprint v4 as the spawned-subagent sprint and sets the architectural contract.

**Key Functions/Components**:
- Sprint goals
- Target architecture
- Data flow
- Out-of-scope boundaries

**How it works**:
This document is the sprint contract. It states that delegation should move from a local synthesized path to a real child process while preserving `MainAgent` as the sole delegation decision-maker.

It also narrows scope correctly. The sprint is about a local child-process boundary with typed IPC, timeout handling, and observability. It explicitly avoids distributed queues, long-lived worker pools, and planner vocabulary rewrites, which kept the implementation focused and prevented the sprint from becoming a platform rewrite.

### `sprints/v4/TASKS.md`
**Purpose**: Tracks the v4 backlog from scaffolding through graph alignment.

**Key Functions/Components**:
- Ten sprint tasks
- Acceptance criteria per task
- Completion notes per file group

**How it works**:
The task tracker is the file-level evidence for what landed in the sprint. It records the implementation order: contracts first, process dispatch second, child runtime third, then guardrails, trace correlation, docs, and graph alignment.

Because every task has completion notes, this file also acts as a release changelog. A new developer can read it and understand not just what the sprint intended to do, but what the code actually ended up shipping.

### `src/bus.rs`
**Purpose**: Provides the serializable event ID used as the dispatch correlation key.

**Key Functions/Components**:
- `EventId`
- `EventId::generate()`

**How it works**:
Sprint v4 relies on `dispatch_id` crossing process boundaries. That only works cleanly because `EventId` is now serializable and deserializable through `serde`, so the same identifier can be created in the parent, transmitted in JSON, and validated in the child response.

That is small but important harness work. The child process does not invent its own correlation handle; it reuses the parent-generated event identity, which is the basis for trace legibility and response matching.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);
```

### `src/dispatch.rs`
**Purpose**: Defines the typed IPC contract and the process-backed dispatcher.

**Key Functions/Components**:
- `CHILD_SUBAGENT_COMMAND`
- `DispatchTarget`
- `DispatchRequest`
- `DispatchResponse`
- `ProcessDispatcher`
- `validate_spawn_request()`
- `resolve_spawn_executable()`

**How it works**:
This file is the main execution seam added by the sprint. `DispatchRequest` packages everything a child subagent needs: task, compact context packet, memory references, file references, recent observations, and a `dispatch_id`. `DispatchResponse` packages the child result in a way the parent can render, trace, and roll up into usage reporting.

`ProcessDispatcher` is the operational shift. It serializes the request, launches the same binary in child mode, writes the request over stdin, waits for stdout JSON, enforces timeout, captures stderr for diagnostics, and rejects mismatched `dispatch_id` values. That turns delegation from “call a local helper” into “execute a bounded child runtime.”

Guardrails also live here. Before spawning, the dispatcher rejects unsupported worker targets and empty subagent names. That prevents invalid requests from crossing the process boundary at all.

```rust
fn dispatch_spawned_subagent(
    &self,
    request: DispatchRequest,
) -> Result<DispatchResponse, String> {
    Self::validate_spawn_request(&request)?;
    let request_json = serde_json::to_string(&request)?;
    let mut child = Command::new(&self.executable)
        .arg(CHILD_SUBAGENT_COMMAND)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    ...
}
```

### `src/main.rs`
**Purpose**: Adds the hidden child-mode entrypoint and regression tests for docs and graph artifacts.

**Key Functions/Components**:
- hidden `__spawn-subagent` branch in `main()`
- `run_spawned_subagent_child()` wiring
- `sprint_v4_spawned_subagent_docs_explain_current_vs_spawned_boundary()`
- `sprint_v4_graphify_artifacts_include_spawned_subagent_boundary()`

**How it works**:
The CLI surface for humans did not change, but the binary behavior did. At process start, `main()` now checks whether the first argument is `__spawn-subagent`. If so, it bypasses the normal CLI command parser and enters the dedicated child runtime path.

This file also became the home of two sprint-regression tests. One verifies that the docs and diagram assets explicitly describe the spawned boundary. The other verifies that graphify artifacts mention the key spawned-subagent nodes. That keeps the repo’s explanatory surfaces in lockstep with the runtime.

```rust
let args: Vec<String> = env::args().skip(1).collect();
if args.first().map(|value| value.as_str()) == Some(CHILD_SUBAGENT_COMMAND) {
    return run_spawned_subagent_child();
}
```

### `src/mainAgent.rs`
**Purpose**: Moves parent delegation onto the process dispatcher and records child lifecycle correlation in the run trace.

**Key Functions/Components**:
- `DelegationOutcome`
- `delegate_to_subagent()`
- `run_spawned_subagent_child()`
- `execute_spawned_subagent_request_in_root()`
- delegation branch inside `run_with_state()`
- subagent regression tests

**How it works**:
This is the file where the sprint becomes user-visible. `delegate_to_subagent()` now builds a compact context packet, collects file and memory refs, creates a typed `DispatchRequest`, and chooses between a spawned-process path and a local fallback path when spawning is unavailable. It also emits launch observability events and runtime logs before dispatch.

The parent loop now records structured child-correlation details after a successful delegation: launch mode, `dispatch_id`, responder, summary, and recommended next action. That matters because the parent trace is the main debugging surface for operators. Without those lines, child work would still feel opaque even though the process boundary exists.

The same file also hosts the child runtime implementation. `run_spawned_subagent_child()` reads stdin JSON, loads the repo-root subagent specs, validates the target, executes the lightweight child handler, and prints a structured JSON response. That keeps the child runtime explicit while still reusing the existing `explore`, `plan`, and `general-purpose` subagent definitions.

```rust
match self.delegate_to_subagent(&subagent_name, state, input) {
    Ok(outcome) => {
        trace.push(format!(
            "Iteration {step_count}: subagent launch mode = `{}`.",
            outcome.launch_mode
        ));
        trace.push(format!(
            "Iteration {step_count}: subagent dispatch_id = `{}`.",
            outcome.response.dispatch_id.as_str()
        ));
        trace.push(format!(
            "Iteration {step_count}: subagent result summary = {}",
            outcome.response.summary
        ));
        usage.extend(outcome.response.usage.clone());
    }
    Err(reason) => { ... }
}
```

```rust
pub fn run_spawned_subagent_child() -> io::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let request: DispatchRequest = serde_json::from_str(&input)?;
    let root_dir = env::current_dir()?;
    let response = execute_spawned_subagent_request_in_root(&root_dir, request)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}
```

### `src/observability.rs`
**Purpose**: Adds a first-class event helper for subagent lifecycle correlation.

**Key Functions/Components**:
- `record_subagent_dispatch_event()`

**How it works**:
The repo already had general span and log recording. Sprint v4 added a dedicated event helper for subagent lifecycle transitions so launches, results, and failures can be recorded with consistent metadata: stage, dispatch ID, subagent name, launch mode, and optional error.

This is important because subagent orchestration is a harness feature, not just a prompt feature. The trace shown to users is only one surface; OpenTelemetry events are the structured surface that lets the implementation evolve without losing debuggability.

```rust
pub fn record_subagent_dispatch_event(
    stage: &str,
    dispatch_id: &str,
    subagent_name: &str,
    launch_mode: &str,
    error: Option<String>,
) {
    let mut attributes = vec![
        KeyValue::new("subagent.stage", stage.to_string()),
        KeyValue::new("subagent.dispatch_id", dispatch_id.to_string()),
        KeyValue::new("subagent.name", subagent_name.to_string()),
        KeyValue::new("subagent.launch_mode", launch_mode.to_string()),
    ];
    ...
}
```

### `docs/architecture.md`
**Purpose**: Documents the new delegation boundary as a real process seam rather than an implied local callback.

**Key Functions/Components**:
- Subagent section
- `Delegation Boundary` section
- updated current limits

**How it works**:
The architecture doc now distinguishes the old and new worlds explicitly. It explains “Current synthesized delegation” versus “Spawned child execution,” names `ProcessDispatcher`, names the hidden `__spawn-subagent` mode, and describes exactly what crosses the boundary.

It also corrects the repo narrative. Earlier documentation still implied synthesized delegation as the runtime truth. After this sprint, that is only a fallback path, so the doc now says so directly.

### `docs/cli.md`
**Purpose**: Explains how `/agent <name> <task>` behaves after the spawned-boundary change.

**Key Functions/Components**:
- `/agent` operator workflow
- local fallback note
- hidden child-mode note

**How it works**:
The user-facing command did not become more complex, but the underlying behavior did. This doc now explains that `/agent` launches a spawned child process, passes a compact context packet, and records the result in the trace.

It also documents the operational truth that the child process is internal and that the runtime may use a local fallback when spawning is unavailable. That distinction matters for interpreting trace output and test behavior.

### `docs/diagrams/subagent-spawn.drawio`
**Purpose**: Editable architecture diagram for the current-vs-target subagent boundary.

**Key Functions/Components**:
- runtime sequence page
- current-vs-target page
- normalized “Spawned child worker” label

**How it works**:
The draw.io asset shows the runtime flow visually: user input enters `MainAgent`, delegation crosses `Dispatch`, and the target architecture uses a spawned child worker rather than a local synthesized subagent. It is the editable diagram most useful for future architecture reviews.

The important change in this sprint is not just that the file exists, but that its labels now match the code and docs. The target state is explicitly named as a spawned worker.

### `docs/diagrams/subagent-spawn.excalidraw`
**Purpose**: Editable sketch-style diagram for the same spawned-boundary explanation.

**Key Functions/Components**:
- current repo frame
- target feature frame
- normalized “Spawned child worker” label

**How it works**:
This file mirrors the draw.io explanation in a looser Excalidraw format. It gives the repo a second editable artifact for architecture discussions without locking the team into one diagramming tool.

The sprint normalized its labels as part of the documentation regression work so the diagram text matches the current runtime shape.

### `graphify-out/GRAPH_REPORT.md`
**Purpose**: Summarizes the current knowledge graph after the spawned-subagent changes.

**Key Functions/Components**:
- graph summary
- community listing
- dispatch community visibility

**How it works**:
The report confirms that the graph was rebuilt after the sprint. It shows the current corpus size, connected-node counts, and community structure, including the dispatch-oriented community containing `DispatchRequest`, `DispatchResponse`, and `DispatchTarget`.

This file is not implementation logic, but it is still part of the sprint output. The repo instructions require graphify to stay aligned after code changes, so this report is part of the operational definition of done.

### `graphify-out/graph.json`
**Purpose**: Machine-readable graph output that captures the spawned-subagent nodes and relationships.

**Key Functions/Components**:
- `ProcessDispatcher`
- `execute_spawned_subagent_request_in_root()`
- `record_subagent_dispatch_event()`

**How it works**:
The graph JSON is the detailed artifact that backs the report. After the final rebuild, it contains the core spawned-boundary nodes the sprint introduced or made operationally significant.

Task 10 added a regression test that checks for these exact labels. That means graph alignment is no longer just manual hygiene; the code now fails tests if the graph artifact falls behind the sprint’s architectural seams.

### `graphify-out/cache/*.json`
**Purpose**: Cached graphify extraction artifacts for the rebuilt code graph.

**Key Functions/Components**:
- per-file cache entries
- rebuilt extraction state

**How it works**:
These cache files are the byproduct of the required graph rebuilds. They are not hand-edited, but they are part of the sprint artifact set because Task 10 explicitly included cache alignment.

In practice, they let graphify avoid rebuilding everything from scratch on every run. Their presence is evidence that the repo-local graph state was refreshed after the spawned-boundary changes.

## Data Flow
1. The user enters a task through the CLI session or one-shot `run` path.
2. `MainAgent` builds a task contract and planner context.
3. The planner decides `delegate(subagent_name)`.
4. `delegate_to_subagent()` builds a compact `DispatchRequest` with `dispatch_id`, context packet, file refs, memory refs, and recent observations.
5. `ProcessDispatcher` validates the request, resolves the current executable, launches the same binary with `__spawn-subagent`, and writes the request JSON to stdin.
6. The child runtime reads stdin, loads the target subagent spec from `.claude/agents/*.md`, validates the target, and executes the lightweight subagent handler.
7. The child prints a `DispatchResponse` JSON payload to stdout.
8. The parent validates the response, records subagent lifecycle events and trace lines, appends child usage, and converts the response into a parent observation.
9. The main loop continues until it finishes, stops, or retries according to the normal harness policy.

## Test Coverage
- Unit:
  - `process_dispatcher_parses_child_stdout()`
  - `process_dispatcher_times_out_when_child_does_not_exit()`
  - `process_dispatcher_rejects_worker_targets_before_spawning()`
  - `spawned_subagent_request_uses_loaded_subagent_spec()`
  - `spawned_subagent_request_rejects_unknown_subagent()`
  - `spawned_subagent_request_rejects_worker_targets()`
  - `run_trace_records_subagent_correlation_and_result_details()`
  - `render_usage_summary_lists_main_and_subagent_requests()`
  - `sprint_v4_spawned_subagent_docs_explain_current_vs_spawned_boundary()`
  - `sprint_v4_graphify_artifacts_include_spawned_subagent_boundary()`
- Integration:
  - none beyond the process-dispatch unit coverage against temporary child scripts and temporary subagent trees
- E2E:
  - none

Final validation state for the sprint:
- `cargo test` — 113 passed
- `npx semgrep --config auto src/ --quiet` — clean
- `npm audit` — 0 vulnerabilities

## Security Measures
- Child launches are bounded by an explicit timeout in `ProcessDispatcher`.
- Unsupported dispatch targets are rejected before spawning.
- Empty subagent names are rejected at the dispatcher boundary.
- Child stdout must parse as structured JSON before the parent accepts the result.
- Parent and child `dispatch_id` values must match, which prevents accepting the wrong child response.
- Stderr is treated as diagnostics, not as the result channel.
- Security scanning was run as part of the sprint closeout: semgrep clean and npm audit clean.

## Known Limitations
- The child runtime still uses lightweight local `explore`, `plan`, and `general-purpose` handlers rather than a deeper independent agent loop.
- The fallback path is still local and synthesized when child spawning is unavailable, especially in unit-test environments.
- The spawned child is local-only; there is no remote execution, durable worker pool, or persistent background agent.
- Delegation remains one-child-at-a-time from the parent loop; there is no parallel subagent fan-out.
- The graph report is aligned, but the graph communities remain broad and somewhat noisy, which limits how much architecture insight it can provide automatically.

## What's Next
- Move beyond lightweight synthesized child handlers toward a richer child runtime if the project needs deeper delegated reasoning.
- Tighten planner-side delegation policy so the parent can decide more precisely when delegation is worth the overhead.
- Expand eval coverage from unit-style process tests into more end-to-end delegated task fixtures.
- If the repo continues growing, split more of `MainAgent` into smaller orchestration modules so the spawned-boundary code is less centralized.
