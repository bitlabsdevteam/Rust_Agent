# CLI Guide

## Purpose

This repository is operated as a local CLI agent. The command surface is intentionally small:

- start or resume an interactive session
- run a one-shot prompt
- inspect loaded project state
- compact stored session context
- manage reusable skills
- scaffold missing project files

The examples below use `agent_in_rust` as the binary name.

## Top-Level Commands

### Start an interactive session

```bash
agent_in_rust
agent_in_rust --trace
agent_in_rust session
agent_in_rust session --system "custom system prompt" --trace
agent_in_rust chat
```

Use this mode when you want a persistent local session with slash commands, short-term memory, and trace visibility.

By default, the runtime assembles the system prompt from the files in `system_prompt/`. `--system` overrides that folder for a single invocation.

Internally, CLI input now enters the same control-plane seam used by future channels: CLI adapter, in-process bus, router, worker runtime, and outbound delivery contracts. The user-facing command remains the same.

### Run a one-shot prompt

```bash
agent_in_rust run --input "Plan the refactor"
agent_in_rust run --input "Use tool web_search_tool with {\"query\":\"latest Rust 2026 edition updates\"}" --trace
```

Use this when you want a single request without entering the interactive REPL.

### Inspect loaded runtime state

```bash
agent_in_rust list
```

This prints:

- model policy
- observability status
- loaded memory sources
- loaded subagents
- loaded skills
- custom commands
- visible tools
- MCP servers

### Compact stored short-term context

```bash
agent_in_rust compact
```

This loads `Workspace/short-term.json`, summarizes older turns into the compacted summary, retains the recent suffix, and writes the updated snapshot back to disk.

### Manage skills

```bash
agent_in_rust skills
agent_in_rust skills list
agent_in_rust skills show --name ship-small
agent_in_rust skills validate
agent_in_rust skills validate --name ship-small
agent_in_rust skills create --name ship-small --description "Bias toward the smallest coherent change set"
agent_in_rust skills install --source owner/repo/skill-name
agent_in_rust skills install --source /absolute/path/to/skill --scope user
```

Skill sources are loaded from:

- project: `.claude/skills/<name>/SKILL.md`
- user: `~/.claude/skills/<name>/SKILL.md`

Behavior notes:

- `skills show` renders the loaded metadata contract plus the full instruction body for one skill.
- `skills validate` reports invalid or skipped skill files and confirms which skills loaded successfully.
- user-scoped skills load first, but project-scoped skills override them on name collision.
- invalid skill files are skipped instead of crashing the whole harness.

### Scaffold missing project files

```bash
agent_in_rust init
```

This creates missing starter artifacts such as:

- `CLAUDE.md`
- `system_prompt/system_prompt.md`
- `.claude/agents/*.md`
- `.claude/skills/ship-small/SKILL.md`
- `.claude/commands/review.md`
- fallback long-term memory file at `Workspace/MEMORY.md`

### Get help

```bash
agent_in_rust help
agent_in_rust help session
agent_in_rust help run
agent_in_rust help list
agent_in_rust help skills
agent_in_rust help init
agent_in_rust help compact
```

## Interactive Session Commands

Inside `session` or default REPL mode, the built-in slash commands are:

- `/help`
- `/agents`
- `/skills`
- `/memory`
- `/remember <note>`
- `/model`
- `/clear`
- `/compact`
- `/mcp`
- `/review [task]`
- `/skill <name> [task]`
- `/init`
- `/agent <name> <task>`
- `/trace`
- `/exit`

Project-defined slash commands from `.claude/commands/*.md` are also available.

## Memory Behavior

The runtime exposes three operator-relevant memory layers.

### Project memory

Loaded into the planner context from:

- `CLAUDE.md`
- imported `@path` files referenced from memory markdown
- long-term memory source

This is standing memory for the harness.

### Short-term memory

Stored in `Workspace/short-term.json`:

- recent history
- accumulated observations
- compacted summary

This snapshot is resumed across local sessions.

### Long-term memory

Long-term memory uses one of two backends:

- Mem0 when `MEM0_API_KEY` is configured
- file fallback at `Workspace/MEMORY.md` otherwise

`/remember <note>` writes durable memory through the active backend. `/memory` shows:

- loaded memory sources
- short-term snapshot status
- active long-term backend
- long-term source path
- long-term preview
- merged memory preview

## Compaction Behavior

There are two ways compaction happens.

### Manual compaction

Use:

```bash
/compact
```

or

```bash
agent_in_rust compact
```

Manual compaction summarizes older turns into the compacted summary and retains the recent turns.

### Automatic compaction

During normal runtime execution, the agent may auto-compact when active history exceeds the configured budget. When this happens, the execution trace records that compaction occurred.

## Operator Workflows

### Inspect the current runtime surface

```bash
agent_in_rust list
```

Use this first when you want to see what the agent has loaded before making changes.

### Start a debugging session with trace output

```bash
agent_in_rust --trace
```

Then use `/trace` inside the session to toggle trace printing on or off.

### Save durable knowledge

```text
/remember Prefer explicit approvals for risky filesystem actions.
```

Use this for guidance that should survive beyond the current session.

### Shrink context without losing it

```text
/compact
```

Use this when the session has grown and you want to preserve summary context while dropping older detailed turns from the active working set.

### Run a focused review

```text
/review
/review memory loading path
```

This uses the review workflow rather than a hidden background validator.

### Load a named skill for the current task

```text
/skill ship-small implement the next CLI task
```

Use this when a reusable skill should shape the next action.

### Delegate to a subagent

```text
/agent explore find where long-term memory is loaded
/agent plan add a minimal eval fixture runner
```

Use this when you want a bounded exploratory or planning handoff.

## Operational Notes

- The default model policy is OpenAI GPT-5.4 with Opus 4.6 fallback.
- The planner may fall back to a local heuristic router if model-backed planning is unavailable.
- MCP tools are available only when `MCP_SERVERS` is configured.
- `session` mode preserves local state; `run` mode is better for scripted or one-off use.
- `clear` resets short-term session state but does not remove project memory.
