- [x] Task 1: Create sprint v4 scaffolding and spawned-subagent docs (P0)
  - Acceptance: `sprints/v4/PRD.md` and `sprints/v4/TASKS.md` exist and describe the child-process delegation direction clearly enough to implement without a fresh architecture debate.
  - Files: `sprints/v4/PRD.md`, `sprints/v4/TASKS.md`, `docs/architecture.md`, `docs/cli.md`
  - Completed: 2026-05-06 — Added the v4 PRD/task tracker and updated the repo-local architecture and CLI docs to describe spawned subagents.

- [x] Task 2: Define typed child IPC contracts for delegated work (P0)
  - Acceptance: The repo has serializable request/response contracts for delegated subagent launches, including correlation IDs, task context, references, and usage reporting.
  - Files: `src/dispatch.rs`, `src/bus.rs`, `src/mainAgent.rs`
  - Completed: 2026-05-06 — Added serde-backed dispatch request/response types and serialized event IDs/context/usage records for child IPC.

- [x] Task 3: Add a process dispatcher that launches child subagents (P0)
  - Acceptance: Delegated work can be sent to a child process, with stdout parsing, stderr capture, exit-status handling, and timeout classification.
  - Files: `src/dispatch.rs`, `src/mainAgent.rs`
  - Completed: 2026-05-06 — Added `ProcessDispatcher` with current-exe launch support, timeout handling, and structured response validation.

- [x] Task 4: Add a private child-subagent CLI mode (P0)
  - Acceptance: The binary can run in a private child mode that reads a dispatch request from stdin and writes a structured child response to stdout.
  - Files: `src/main.rs`, `src/mainAgent.rs`
  - Completed: 2026-05-06 — Added hidden `__spawn-subagent` handling and a child entrypoint that loads the target subagent from the repo root.

- [x] Task 5: Wire main-agent delegation to the process dispatcher (P0)
  - Acceptance: `delegate` launches a real child process instead of the local synthesized callback path, while preserving the parent planner flow.
  - Files: `src/mainAgent.rs`
  - Completed: 2026-05-06 — Updated `delegate_to_subagent()` to launch the child binary and record the child launch mode in trace attributes.

- [x] Task 6: Add launch guardrails for child execution (P1)
  - Acceptance: Child launches enforce explicit timeout, target validation, and a documented failure path for unknown or unsupported targets.
  - Files: `src/dispatch.rs`, `src/mainAgent.rs`, tests
  - Completed: 2026-05-06 — Added pre-spawn target validation in `ProcessDispatcher` and regression tests for timeout, unknown subagents, and unsupported worker targets.

- [x] Task 7: Thread parent/child trace and usage correlation (P1)
  - Acceptance: Parent trace entries show the child launch reason, child result, and correlation ID, and usage summaries still roll up cleanly.
  - Files: `src/observability.rs`, `src/mainAgent.rs`, `src/runtime_log.rs`
  - Completed: 2026-05-07 — Added correlated subagent lifecycle events plus parent-trace lines for launch mode, dispatch ID, responder, summary, and next action without breaking usage rollups.

- [x] Task 8: Add regression tests for the spawned-subagent path (P1)
  - Acceptance: Tests cover process dispatch success, timeout, and child execution against a real subagent spec.
  - Files: `src/dispatch.rs`, `src/mainAgent.rs`
  - Completed: 2026-05-06 — Added unit coverage for process dispatch success/timeout and child execution against a temp subagent tree.

- [x] Task 9: Update repo diagrams and architecture notes for the spawned boundary (P1)
  - Acceptance: Repository docs and diagram assets explain current synthesized delegation versus spawned child execution.
  - Files: `docs/architecture.md`, `docs/cli.md`, `docs/diagrams/*`
  - Completed: 2026-05-07 — Added explicit current-vs-spawned delegation notes to the architecture and CLI docs, normalized the diagram labels, and added a regression test that checks the documentation assets.

- [x] Task 10: Rebuild graphify and keep repo-local maps aligned (P2)
  - Acceptance: The graph report reflects the new child-process boundary and related module relationships.
  - Files: `graphify-out/GRAPH_REPORT.md`, `graphify-out/graph.json`, cache artifacts
  - Completed: 2026-05-07 — Added a regression that checks the graph artifacts for the spawned-subagent seam and rebuilt graphify so the report, graph JSON, and cache reflect the current codebase.
