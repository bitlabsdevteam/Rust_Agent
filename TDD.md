# TDD.md

## Purpose

Define the technical design standard for this Rust agent scaffold with
memory optimization as a first-class concern.

This repository should not treat memory behavior as an afterthought.
Rust gives explicit ownership, borrowing, and lifetime control, so the
agent design should use those properties to keep the runtime small,
predictable, and legible.

## Design Goal

Build an agent runtime that is:

- memory-aware by default
- explicit about ownership and object lifetimes
- bounded in context growth and retry behavior
- observable enough to explain where memory is retained or copied
- simple enough to stay understandable as the scaffold evolves

## Design Position

For this project, Rust is not only a language choice. It is a systems
constraint.

Because Rust is a low-level language with explicit ownership and resource
management, the agent architecture should prefer:

- stack-first thinking where practical
- borrowed data over cloned data when safe and readable
- bounded collections over unbounded accumulation
- short-lived allocations over hidden long-lived heap growth
- explicit lifecycle transitions for agent state, tool results, and trace data

The design should avoid importing high-level agent patterns that assume
cheap copies, hidden garbage collection, or infinite prompt growth.

## Core Memory Principles

### 1. Keep The Working Set Small

The active loop should only carry the data needed for the next decision:

- current user input
- stable system prompt
- relevant compacted history
- registered tool and skill metadata
- current retry state
- current trace entry

Inactive or historical data should be summarized, compacted, or written
to durable storage instead of being retained in the hot path.

### 2. Prefer Borrowing Before Cloning

When designing internal APIs:

- accept `&str`, `&[T]`, and references where ownership transfer is not needed
- return owned values only when the caller must retain data independently
- use cloning as a deliberate boundary, not as a convenience default

Clones are acceptable when they simplify correctness at a clear boundary,
but they should be visible in the design and justified by the call path.

### 3. Bound All Growth

The harness should define explicit upper bounds for:

- retained conversation history
- trace length kept in memory
- retry count
- tool output stored in the loop state
- queued ingress items

If growth is unbounded, memory usage is unbounded. The scaffold should
prefer hard limits and deterministic truncation rules over best-effort cleanup.

### 4. Separate Hot State From Cold State

Hot state:

- planner inputs
- current observation
- immediate tool arguments
- current stop and retry counters

Cold state:

- archived traces
- historical conversations
- long-form memory documents
- prior tool payloads no longer needed for the next step

Cold state should live on disk or in compact summaries, not in always-live
Rust objects.

### 5. Make Allocation Visible

Important runtime paths should make it obvious when allocation happens:

- prompt assembly
- tool result normalization
- trace recording
- serialization and deserialization
- context compaction

Opaque allocation-heavy helper layers should be avoided.

## Runtime Shape

The agent loop should continue to follow the repository contract in
`AGENTS.md`, but each step should also preserve memory discipline:

1. Load stable instructions and task input.
2. Build the smallest valid planner context.
3. Decide one next action.
4. Execute one tool or one skill.
5. Record a compact observation.
6. Retry only when the failure is recoverable and new information exists.
7. Stop as soon as the stop condition is met.
8. Emit the final answer and a usable execution trace.

Memory implication:

- each loop iteration should be able to release temporary buffers from the
  previous step
- the loop should not retain full intermediate payloads unless they are
  required for correctness or debugging

## Ownership And State Design

### Agent State

Agent state should be modeled as a small number of explicit structs with
clear ownership boundaries:

- immutable configuration owned once at startup
- mutable loop state owned by the harness
- tool and skill observations owned only as long as they are needed

Avoid designing a single large state object that accumulates prompts,
results, traces, and historical artifacts forever.

### Strings And Text Buffers

Text is the main memory pressure source in agent systems. The design should:

- borrow static prompt fragments where possible
- assemble prompts from small pieces instead of repeated full copies
- truncate or compact verbose tool output before storing it
- avoid retaining multiple full-string versions of nearly identical context

If future growth requires it, use more allocation-aware text strategies
such as `Cow<'a, str>`, shared immutable configuration, or streaming assembly.

### Collections

Collections should be sized intentionally:

- prefer small vectors with known bounds
- clear or replace buffers when a phase completes
- avoid hidden fan-out where one observation is copied into multiple caches

Any collection that can grow across loop iterations should document:

- owner
- maximum size
- eviction or truncation rule
- reason it must remain in memory

## Tool And Skill Design

Tool and skill contracts should support memory-aware execution:

- structured inputs to reduce reparsing and string churn
- compact outputs with explicit success and failure envelopes
- timeout-bounded execution
- failure classification without retaining unnecessary payloads

Tool results should be normalized into small observation objects for the
planner. Raw payloads should be dropped unless explicitly needed for
follow-up actions or debugging.

## Context And Memory Strategy

The agent should treat context as a managed resource.

### Stable Context

Keep these items stable and reusable:

- system prompt
- durable repository rules
- tool and skill catalog
- model policy

These should be loaded once, referenced consistently, and not rebuilt
from scratch every iteration unless the source changes.

### Changing Context

Keep these items minimal and per-run:

- user request
- current task contract
- recent observations
- retry state
- stop condition state

Changing context should be compacted aggressively as the run proceeds.

### Durable Memory

Durable knowledge belongs in repository-local files such as:

- `AGENTS.md`
- `Workspace/MEMORY.md`
- future `docs/` references

Runtime memory should not silently become long-term memory. If a fact must
survive the session, write it to a repo-local document intentionally.

## Observability For Memory Behavior

Execution traces should remain useful without becoming a memory leak.

Record:

- selected model
- chosen action
- tool or skill name
- compact observation summary
- retry count
- stop reason

Do not record:

- repeated full prompt bodies unless debugging explicitly requires it
- large raw payloads by default
- duplicate copies of tool output in multiple trace fields

As the scaffold evolves, observability should make it possible to answer:

- what data is retained across iterations
- where copies are introduced
- which step increases memory pressure
- whether retries are reusing stale large context

## Validation Standard

The design should be verified with memory-aware checks, not only functional checks.

Minimum validation areas:

- planner path does not retain unnecessary prior observations
- retry logic does not duplicate large context blocks on each retry
- tool failures are compacted into small structured observations
- empty input stops without building unnecessary runtime state
- trace output remains bounded as iterations increase
- context compaction actually reduces retained text volume

Future hardening should add benchmarks and regression checks for:

- allocation count on common loop paths
- peak resident memory during repeated runs
- large-input handling
- long-history compaction behavior

## Design Tradeoffs

This project should prefer predictable memory behavior over convenience.

That means it is acceptable to choose:

- slightly more explicit state plumbing
- smaller and more specialized structs
- stricter truncation rules
- extra compaction steps before planner calls

instead of designs that look simpler but silently retain large heap-backed state.

## Non-Goals

This document does not require premature micro-optimization.

It does require:

- ownership-aware API design
- bounded runtime state
- explicit retention policy
- memory-conscious context engineering

The goal is not maximum cleverness. The goal is an agent scaffold whose
memory behavior stays understandable as it grows.
