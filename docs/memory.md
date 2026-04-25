# Memory

## Purpose

This document defines the intended memory precedence for the current CLI harness and the narrow seam reserved for future path-scoped memory loading.

The current runtime still loads the full standing memory stack. Sprint v1 does not introduce selective retrieval yet. Instead, it documents the ordering rules and adds source metadata so future path-aware loading can evolve without rewriting the memory subsystem.

## Current Memory Layers

The harness exposes three distinct memory surfaces:

- project memory loaded from `CLAUDE.md`
- short-term session state loaded from `Workspace/short-term.json`
- long-term memory loaded from Mem0 or `Workspace/MEMORY.md`

Long-term memory now crosses a dedicated memory-agent boundary in `src/memory_agent.rs`.

- `FileMemoryAgent` backs the local file path in `Workspace/MEMORY.md`
- the runtime wraps Mem0 behind the same interface before loading or appending durable notes
- a stub memory-agent implementation exists so future worker-based memory routing can be exercised without depending on Mem0 or the file backend

Short-term memory is execution state, not standing instructions. The precedence rules below apply to the standing memory stack loaded into the planner.

## Standing Memory Precedence

When the harness builds planner memory today, the effective order is:

1. user-level `~/.claude/CLAUDE.md` when present
2. repo-level `CLAUDE.md`
3. imported `@path` memory files discovered from those root files
4. long-term memory from Mem0 or `Workspace/MEMORY.md`

The merged instruction string preserves this source order.

The intent behind that order is:

- user memory provides portable operator preferences
- project memory defines repo-local rules and defaults
- imported memory files let a project break durable guidance into smaller files
- long-term memory appends durable learned notes without replacing repo-owned guidance

## Intended Path-Scoped Behavior

Future path-scoped memory should refine loading, not replace the existing memory model.

The intended future rule is:

- root memory files remain globally loaded
- imported memory files may become selectively emphasized or filtered based on the files relevant to the current task
- long-term memory remains globally available unless a later design explicitly splits it into scoped vs global durable memory

In other words, path scoping is meant to narrow imported project memory first. It should not silently drop core repo rules from `CLAUDE.md`.

## Selector Hints

Imported project memory files now carry a `selector_hint` in `MemorySource`.

Current behavior:

- root `CLAUDE.md` files have no selector hint
- long-term memory has no selector hint
- imported project files receive a relative directory hint such as `docs/agents`
- the current runtime records this metadata but does not filter on it yet

This metadata is the seam for future work. A later loader can compare requested focus paths against `selector_hint` without changing how memory files are discovered or parsed.

## Memory Load Request Hook

The current code introduces `MemoryLoadRequest`.

Right now it only normalizes `focus_paths` and records them in the returned `MemoryStack`. The default runtime path still uses an empty request, which keeps current behavior unchanged.

This hook exists so future changes can:

- pass relevant task files into memory loading
- score or filter imported memory by path affinity
- expose path-scoped loading in evals before enabling it broadly in the runtime

## Memory-Agent Boundary

Long-term memory reads and writes no longer reach directly into Mem0 or `Workspace/MEMORY.md` from the main runtime loop.

Current behavior:

- planner memory loading calls the memory-agent interface to fetch the long-term memory snapshot
- `/remember <note>` calls the same interface to append durable notes
- the current runtime still chooses the backend locally: Mem0 when configured, file storage otherwise

This is intentionally a narrow seam. The current implementation is still local-process and deterministic, but later work can replace the local backend choice with a dedicated memory worker or service without changing the rest of the runtime contract.

## Non-Goals For V1

This sprint does not add:

- selective memory retrieval at runtime
- model-driven memory search
- path-scoped long-term memory
- per-subagent memory partitions
- automatic pruning of imported memory based on file relevance

Those remain follow-on changes. The v1 goal is a documented precedence model plus a stable implementation seam.
