# Sprint v6 - Step 05 Context Guard And Auto Compaction

## Sprint Overview

Sprint v6 upgrades the repository's current compaction behavior into a Step 05-style context guard. The runtime already performs manual and automatic history compaction, but the policy is still a simple message/character heuristic and does not yet expose operator-facing context usage or truncation-first handling for oversized observations.

The goal of this sprint is to make compaction explicit, budget-driven, and easier to operate. The sprint adds a dedicated context-management seam, token-budget estimation, truncation-before-summary compaction, and an in-session `/context` command, while preserving the current CLI-first runtime and short-term memory model.

## Goals

- Replace the current compaction trigger with a token-budget-driven context guard, while keeping the existing heuristic limits as secondary safety rails.
- Truncate oversized observations before compacting older history so large tool and subagent outputs do not force unnecessary summary compaction.
- Run auto compaction at the planner boundary using the actual rendered planner context, not just raw history size.
- Add an operator-facing `/context` command that reports current usage, threshold, and compaction state.
- Keep manual `/compact` available, but route it through the same context-guard pipeline and return richer before/after reporting.

## User Stories

- As the runtime, I want to estimate planner context size before each planner call so compaction happens at the right boundary.
- As an operator, I want `/context` to show current usage and budget pressure so I can understand when compaction is likely to happen.
- As a developer, I want oversized tool and subagent observations truncated before history compaction so the working set stays useful.
- As a maintainer, I want compaction policy moved into a dedicated seam so future context-management changes do not sprawl through `MainAgent`.
- As a future implementer, I want Sprint v6 to stop short of session rollover so Step 05 can land cleanly without mixing in later OpenClaw steps.

## Technical Architecture

### Stack

- Language: Rust
- Runtime surface: existing CLI-first `MainAgent`
- New policy seam: `ContextGuard`
- Planner input boundary: rendered planner prompt plus current user input and session state
- Persistence surface: existing `Workspace/short-term.json`
- Operator surface: in-session slash commands

### Target Architecture

```text
User input
  |
  v
MainAgent loop
  |
  v
ContextGuard
  |
  +--> estimate rendered planner-context budget
  +--> truncate oversized observations first
  +--> compact older history if still over threshold
  |
  v
planner decision
  |
  v
short-term memory persistence + trace
```

### Data Flow

1. The runtime loads current session state from short-term memory.
2. Before each planner decision, the runtime renders the current planner-context inputs and asks `ContextGuard` for a budget check.
3. `ContextGuard` estimates context tokens using the rendered planner inputs and current user input.
4. If the context is over budget, `ContextGuard` truncates oversized observation entries first.
5. If the context is still over budget, `ContextGuard` compacts older history into `compacted_summary` and retains the recent suffix.
6. The updated state is persisted before continuing with the planner call.
7. `/context` uses the same estimator to report current usage and budget state without mutating runtime state.
8. `/compact` reuses the same guard with `force=true` and returns richer before/after reporting.

### Architectural Principles

- Use the planner boundary as the source of truth for compaction decisions.
- Prefer deterministic local estimation over provider-specific tokenizer dependencies in the first pass.
- Truncate large observations before summarizing history.
- Reuse one compaction pipeline for auto and manual paths.
- Preserve the current short-term memory file and planner history summary model unless a concrete need requires additive metadata.

### Planned Module Direction

- `src/context_guard.rs`: new context estimation, reporting, truncation, and compaction policy seam.
- `src/mainAgent.rs`: integrate the new guard into the runtime loop, slash command surface, and manual compaction entrypoint.
- `docs/cli.md` and architecture docs: update only after implementation lands, not as part of this planning sprint.

## Scope Boundaries

### In Scope

- Token-budget-driven compaction policy
- Truncation of oversized observation entries
- Auto compaction before planner decisions
- Richer manual `/compact` reporting
- `/context` command and status output
- Tests covering estimation, truncation, compaction triggers, and reporting

### Out of Scope

- Session rollover to a fresh session ID
- New top-level CLI commands outside the interactive slash-command surface
- Provider-specific tokenizer libraries
- Changes to tool behavior beyond observation truncation in memory/context handling
- Memory-agent redesign, retrieval changes, or long-term memory partitioning
- Changes to later OpenClaw steps such as channels, websocket, cron, or routing

## Acceptance Criteria

- Auto compaction is driven primarily by an estimated planner-context token budget, not only by message or character count.
- The runtime truncates oversized observation entries before compacting older history.
- Auto compaction runs before planner decisions and persists any resulting short-term memory updates.
- `/context` reports estimated usage, threshold, percent used, short-term state, and whether truncated entries are present.
- `/compact` reuses the same policy pipeline and reports before/after estimates plus truncation activity.
- Existing compaction behavior remains legible in trace output with explicit reasons and post-compaction state.
- The sprint does not implement session rollover or fresh-session handoff behavior.

## Dependencies

- Existing `MainAgent::run_with_state()` loop and planner prompt construction
- Existing `compact_session_state()` and `planner_history_summary()` behavior as the baseline compaction model
- Existing short-term memory persistence in `Workspace/short-term.json`
- Existing tool/subagent observation storage inside session observations
- Existing sprint and documentation structure under `sprints/` and `docs/`

## Design Decisions Locked For V6

- Compaction policy will follow Step 05 parity closely but remain Rust-native and deterministic.
- The primary threshold is `160_000` estimated tokens.
- Estimation will be local and approximate using rendered planner inputs and a conservative char-based heuristic.
- Oversized context will be reduced in this order: estimate -> truncate oversized observations -> compact older history.
- `/context` is included in this sprint.
- Session rollover is explicitly deferred to a later sprint.
