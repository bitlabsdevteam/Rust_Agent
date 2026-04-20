- [x] Task 1: Create sprint scaffolding and repo-local sprint docs (P0)
  - Acceptance: `sprints/v1/PRD.md` and `sprints/v1/TASKS.md` exist with goals, architecture, scope, and ordered tasks.
  - Files: `sprints/v1/PRD.md`, `sprints/v1/TASKS.md`
  - Completed: 2026-04-20 — Verified the sprint artifacts exist and include the required PRD structure and ordered backlog.

- [x] Task 2: Add repo-local architecture docs for the current CLI harness (P0)
  - Acceptance: A focused doc explains runtime loop, planner flow, memory surfaces, and extension points without requiring a full source dive.
  - Files: `docs/architecture.md` or equivalent repo-local markdown, `AGENTS.md` if a pointer is needed
  - Completed: 2026-04-20 — Added `docs/architecture.md` and linked it from `AGENTS.md` as the repo-local architecture reference.

- [x] Task 3: Add repo-local operator docs for CLI usage and memory behavior (P0)
  - Acceptance: A doc explains session/run/list/init usage, slash commands, compaction, and Mem0-vs-file long-term memory behavior.
  - Files: `docs/cli.md` or equivalent repo-local markdown
  - Completed: 2026-04-20 — Added `docs/cli.md` covering top-level commands, session commands, compaction, and active memory backends, and linked it from `AGENTS.md`.

- [x] Task 4: Introduce a minimal eval fixture format for planner decisions (P0)
  - Acceptance: The repo contains a simple fixture schema that can express user input, observations, and expected planner action.
  - Files: `evals/fixtures/*.json` or `*.jsonl`, `src/mainAgent.rs` or a new eval module
  - Completed: 2026-04-20 — Added a minimal JSON fixture schema, parser and loader in `src/evals.rs`, plus seed fixtures under `evals/fixtures/`.

- [x] Task 5: Add a CLI entrypoint to run eval fixtures (P0)
  - Acceptance: A new CLI command runs the eval fixtures and reports pass/fail for expected planner decisions.
  - Files: `src/main.rs`, `src/mainAgent.rs`, optional new eval module
  - Completed: 2026-04-20 — Added `agent_in_rust eval`, a shared planner-preview eval path, and repo-local `ship-small` skill scaffolding so the seeded fixtures pass end-to-end.

- [x] Task 6: Add initial regression fixtures for core harness behavior (P0)
  - Acceptance: Fixtures cover tool selection, skill selection, subagent delegation, retry behavior, empty-input stop, and finish-on-observation.
  - Files: `evals/fixtures/*`
  - Completed: 2026-04-20 — Added regression fixtures for finish, retry, and empty-input stop, and tightened eval preview handling so harness-owned stop/retry outcomes stay deterministic.

- [ ] Task 7: Make planner and trace outputs easier to inspect in eval results (P1)
  - Acceptance: Eval failures show enough structured detail to understand expected vs actual action and the key reasoning context.
  - Files: `src/mainAgent.rs`, optional new eval/reporting module

- [ ] Task 8: Add path-scoped memory precedence design notes and a narrow implementation hook (P1)
  - Acceptance: The repo documents intended memory precedence and introduces a small code seam for future path-scoped loading without widening runtime complexity.
  - Files: `docs/memory.md` or equivalent repo-local markdown, `src/mainAgent.rs`

- [ ] Task 9: Add tests for the eval harness and fixture parsing path (P1)
  - Acceptance: Automated tests cover fixture loading, eval command behavior, and at least one failing-vs-passing planner case.
  - Files: `src/main.rs`, `src/mainAgent.rs`, optional new test module

- [ ] Task 10: Triage and remediate existing Node dependency audit findings used by local tooling (P2)
  - Acceptance: Current `npm audit` findings are either fixed or explicitly documented with bounded rationale and next action.
  - Files: `package.json`, `package-lock.json`, repo-local docs as needed
