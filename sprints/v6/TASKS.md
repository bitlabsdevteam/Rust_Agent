- [ ] Task 1: Create sprint v6 compaction artifacts and lock the design contract (P0)
  - Acceptance: `sprints/v6/PRD.md` and `sprints/v6/TASKS.md` define the Step 05 compaction upgrade clearly enough to implement without reopening major design decisions.
  - Files: `sprints/v6/PRD.md`, `sprints/v6/TASKS.md`

- [ ] Task 2: Add a dedicated `ContextGuard` policy seam (P0)
  - Acceptance: The codebase has a dedicated module or equivalent seam for context estimation, reporting, truncation, and compaction policy instead of keeping the full policy embedded in `MainAgent`.
  - Files: `src/context_guard.rs`, `src/mainAgent.rs`

- [ ] Task 3: Add deterministic planner-context token estimation (P0)
  - Acceptance: The runtime can estimate planner-context size from rendered planner inputs, current input, observations, and history summary using a deterministic local heuristic with a `160_000` token threshold.
  - Files: `src/context_guard.rs`, `src/mainAgent.rs`, tests

- [ ] Task 4: Implement context reporting for operators (P0)
  - Acceptance: The runtime can build a context report with estimated tokens, threshold, percent used, history/observation counts, compacted-summary state, and truncated-entry status.
  - Files: `src/context_guard.rs`, `src/mainAgent.rs`, tests

- [ ] Task 5: Add truncation-first handling for oversized observations (P0)
  - Acceptance: Oversized observation entries from tools, MCP calls, or subagents are truncated with a visible marker before history compaction is attempted.
  - Files: `src/context_guard.rs`, `src/mainAgent.rs`, tests

- [ ] Task 6: Rewire auto compaction to run before planner decisions (P0)
  - Acceptance: `run_with_state()` invokes the new context guard before planner decisions, persists updated short-term state when mutations occur, and records explicit trace entries for estimate, truncation, and compaction activity.
  - Files: `src/mainAgent.rs`, `src/context_guard.rs`, tests

- [ ] Task 7: Route manual `/compact` through the shared context-guard pipeline (P1)
  - Acceptance: Manual compaction uses the same estimation and truncation pipeline as auto compaction, with richer before/after reporting and no duplicated policy logic.
  - Files: `src/mainAgent.rs`, `src/context_guard.rs`, tests

- [ ] Task 8: Add the `/context` slash command (P1)
  - Acceptance: Interactive sessions support `/context`, and the command returns a compact operator-facing report without mutating session state.
  - Files: `src/mainAgent.rs`, help text/tests as needed

- [ ] Task 9: Extend compaction result and trace reporting (P1)
  - Acceptance: Compaction results and trace output include estimate-before, estimate-after, truncation count, compaction reason, and retained/compacted message counts.
  - Files: `src/mainAgent.rs`, `src/context_guard.rs`, tests

- [ ] Task 10: Add regression coverage for token-budget-driven compaction (P1)
  - Acceptance: Tests cover threshold-triggered auto compaction, truncation-before-summary behavior, `/context` reporting, and persistence of updated short-term state.
  - Files: `src/mainAgent.rs`, `src/context_guard.rs`, test modules

- [ ] Task 11: Verify fallback safety rails remain active (P2)
  - Acceptance: Existing message-count and character-count limits remain as secondary guards, and tests confirm they still protect the runtime when the estimator is inconclusive.
  - Files: `src/context_guard.rs`, `src/mainAgent.rs`, tests

- [ ] Task 12: Document deferred follow-ons without implementing them (P2)
  - Acceptance: Sprint v6 artifacts explicitly record that session rollover, provider-specific tokenizers, and broader context-management redesign are deferred to later sprints.
  - Files: `sprints/v6/PRD.md`, `sprints/v6/TASKS.md`
