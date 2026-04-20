# CLAUDE.md

Project memory for the local harness-first runtime.

## Commands
- `cargo check`
- `cargo test`
- `cargo fmt`

## Agent Contract
- Start every run with an explicit task contract: goal, constraints, acceptance criteria, relevant files, and stop condition.
- Take exactly one action per loop iteration: call a tool, delegate to a subagent, retry, stop, or finalize.
- Keep observations explicit between iterations instead of hiding state in prose.
- Prefer compact context packets and explicit file references over transcript sprawl.

## Model Policy
- Use `OpenAI GPT-5.4` as the primary planner when available.
- If the primary planner is unavailable or fails, try the fallback planner path before dropping to heuristic routing.

## Working Style
- Load only the files needed for the current task.
- Prefer deterministic harness behavior over prompt-only cleverness.
- Keep durable project guidance here instead of in transient chat history.
- Use `/review` after meaningful code changes.

## Trace Standard
- Record planner choice, tool or subagent action, observation, retry count, and stop reason for each iteration.
