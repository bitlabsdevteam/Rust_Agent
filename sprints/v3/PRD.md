## Sprint Overview

Sprint v3 hardens the agent decision loop by extracting planner selection into a dedicated harness boundary and validating the real fallback path with mocked integration tests. v2 gave the repo the right seams; v3 makes the most important one more robust so the system can distinguish normal planning, recoverable planner failure, terminal failure, and stop conditions without relying on an oversized `MainAgent` method.

The sprint goal is reliability, not expansion. The current CLI, bus, router, worker, dispatch, scheduler, concurrency, and memory seams stay intact while planner routing becomes a smaller, testable module with clearer failure semantics.

## Goals

- Extract planner selection and fallback behavior into a dedicated planner service or module.
- Keep model-backed planning the default path and make fallback handling explicit and testable.
- Preserve the current runtime behavior for CLI, tools, skills, subagents, and memory.
- Strengthen evals and tests so tool arguments, retry transitions, and stop behavior are validated in addition to action choice.
- Keep trace output and planner context previews aligned with the new planner boundary.

## User Stories

- As the framework author, I want planner selection isolated from the main runtime so the harness is easier to reason about and evolve.
- As the framework author, I want explicit failure classification so recoverable planner issues do not look like ordinary stop conditions.
- As a developer extending the starter, I want mocked fallback tests so model routing changes do not silently regress.
- As a developer using the repo, I want evals that validate tool arguments and retry behavior so planning changes stay trustworthy.

## Technical Architecture

### Stack

- Language: Rust
- Runtime surface: local CLI
- Planner policy: OpenAI GPT-5.4 primary, Anthropic fallback, local heuristic only when no model-backed planner is available
- Existing execution core: `MainAgent` loop, tools, skills, subagents, traces, Mem0/file memory
- New architectural direction: dedicated planner service boundary with explicit failure classification and integration tests

### Target Architecture

```text
CLI / eval fixture
        |
        v
  MainAgent runtime
        |
        v
 Planner service/module
   |        |         \
   |        |          +--> failure classification
   |        +--> fallback planner
   +--> primary planner
        |
        v
  typed planner decision
        |
        v
  tool / skill / delegate / retry / stop / finish
```

### Data Flow

1. The runtime builds the task contract, prompt layers, memory context, and observations.
2. The planner module chooses the primary planner and, if needed, the configured fallback planner.
3. Model responses are parsed into a typed planner decision.
4. Planner failures are classified as recoverable or terminal before the runtime acts on them.
5. The main loop executes one bounded action and records trace output.
6. Eval fixtures and mocked integration tests validate both the action and the surrounding failure semantics.

### Architectural Principles For V3

- Keep planner policy explicit and harness-owned.
- Prefer typed planner outputs over prompt-adjacent string parsing.
- Use mocked HTTP tests for planner behavior instead of relying only on heuristic unit tests.
- Preserve the current CLI and architecture seams from v2.
- Keep trace and eval reporting legible enough to debug fallback behavior quickly.

### Planned Module Direction

- `src/mainAgent.rs`: call into the planner boundary and preserve trace/failure semantics
- `src/planner.rs` or equivalent: primary/fallback planner selection, parsing, and failure classification
- `src/evals.rs`: richer planner decision expectations, including tool args and failure class
- `evals/fixtures/*.json`: regression cases for tool args, retry, empty input, and fallback behavior
- `tests/` or focused integration tests: mocked planner HTTP coverage

## Out of Scope

- New channels or transport surfaces
- Distributed queueing or scheduler execution
- Memory model redesign beyond the existing boundary
- Prompt optimization beyond what the planner boundary needs for stability
- General multi-agent autonomy or new subagent capabilities

## Dependencies

- Existing runtime core in `src/main.rs` and `src/mainAgent.rs`
- Current v2 seams in `src/bus.rs`, `src/channels/`, `src/router.rs`, `src/worker.rs`, `src/dispatch.rs`, `src/prompt_layers.rs`, `src/scheduler.rs`, `src/concurrency.rs`, and `src/memory_agent.rs`
- Existing eval harness in `src/evals.rs`
- Repo-local standards in `AGENTS.md`

