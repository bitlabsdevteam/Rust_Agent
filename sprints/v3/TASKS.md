- [x] Task 1: Create sprint v3 artifact scaffolding and planner-service notes (P0)
  - Acceptance: `sprints/v3/PRD.md` and `sprints/v3/TASKS.md` exist and describe the planner-hardening direction clearly enough to implement without re-debating scope.
  - Files: `sprints/v3/PRD.md`, `sprints/v3/TASKS.md`
  - Completed: 2026-05-06 — Added the sprint v3 PRD and task tracker with planner-service extraction, failure-classification, and mocked fallback-test direction.

- [x] Task 2: Extract planner selection into a dedicated module or service boundary (P0)
  - Acceptance: Primary planner selection, fallback selection, and planner failure classification live behind a smaller interface than `MainAgent`.
  - Files: `src/mainAgent.rs`, `src/planner.rs`
  - Completed: 2026-05-06 — Added `PlannerRouter`, `Decision`, `PlannerRun`, and `PlannerFailureKind` to `src/planner.rs` and rewired `MainAgent::decide_next_step()` to use the dedicated boundary.

- [x] Task 3: Preserve typed planner decisions and explicit failure classes (P0)
  - Acceptance: Planner decisions expose action, target metadata, tool arguments, and recoverable vs terminal failure semantics in a typed form.
  - Files: `src/planner.rs`, `src/evals.rs`, `src/mainAgent.rs`, `evals/fixtures/retry-recoverable-observation.json`, `evals/fixtures/tool-web-search.json`
  - Completed: 2026-05-06 — Preserved typed planner decisions through `Decision` and `PlannerRun`, surfaced `tool_arguments_json` and `failure_class` in eval previews, and added regression coverage for argument and failure-class matching.

- [x] Task 4: Add mocked integration tests for primary and fallback planner paths (P0)
  - Acceptance: Tests cover primary success, primary failure with fallback success, and dual-failure behavior with the correct retry/stop classification.
  - Files: `src/mainAgent.rs`, test support code in `src/mainAgent.rs`
  - Completed: 2026-05-06 — Added mocked HTTP planner-path tests for OpenAI primary success, OpenAI failure with Anthropic fallback success, and dual planner failure producing the expected retry classification without heuristic fallback.

- [x] Task 5: Expand eval coverage for tool arguments and failure-aware decisions (P1)
  - Acceptance: Eval fixtures can assert concrete tool arguments, explicit skill/subagent routing, and failure classes where relevant.
  - Files: `src/evals.rs`, `src/mainAgent.rs`, `.claude/skills/ship-small/SKILL.md`, `evals/fixtures/*.json`
  - Completed: 2026-05-06 — Tightened eval fixtures with argument and reason assertions, made eval previews deterministic via the local heuristic path, fixed explicit subagent precedence over skill inference, and brought the bundled `ship-small` skill into the valid loaded catalog so the skill-route eval is real.

- [x] Task 6: Improve trace output around planner selection and fallback decisions (P1)
  - Acceptance: Each planner attempt records the chosen backend, decision, fallback outcome, and failure class in the execution trace.
  - Files: planner module, `src/mainAgent.rs`
  - Completed: 2026-05-06 — Added planner-attempt trace entries for primary, fallback, heuristic, and failure paths, plus runtime tests that assert backend selection, fallback outcomes, and overall failure classification in `result.trace`.

- [x] Task 7: Rebuild graphify and repo-local maps after planner extraction (P2)
  - Acceptance: The graph report stays current after planner code moves into a dedicated module.
  - Files: `graphify-out/GRAPH_REPORT.md`, `graphify-out/graph.json`, related cache artifacts
  - Completed: 2026-05-06 — Rebuilt graphify after the planner extraction and trace work; the refreshed graph now includes the dedicated `src/planner.rs` community, updated planner tests, and current repo-map cache artifacts.
