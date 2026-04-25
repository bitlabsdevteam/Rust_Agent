- [x] Task 1: Create sprint v2 artifact scaffolding and architecture stub docs (P0)
  - Acceptance: `sprints/v2/PRD.md` and `sprints/v2/TASKS.md` exist, and a repo-local gap doc target is identified for this sprint.
  - Files: `sprints/v2/PRD.md`, `sprints/v2/TASKS.md`, `docs/openclaw-gap.md`, `src/main.rs`
  - Completed: 2026-04-20 — Added the initial `docs/openclaw-gap.md` stub and a regression test that enforces the sprint v2 scaffolding target.

- [x] Task 2: Add a repo-local OpenClaw gap analysis document (P0)
  - Acceptance: A focused doc compares the current harness to the target OpenClaw-style architecture and names the migration seams for bus, channels, routing, scheduler, dispatch, prompt layers, concurrency, and memory-agent boundaries.
  - Files: `docs/openclaw-gap.md`, `src/main.rs`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-20 — Replaced the stub with a focused current-vs-target gap analysis and added a regression test that enforces the required seam coverage.

- [x] Task 3: Add core event envelope types and bus traits (P0)
  - Acceptance: The repo defines inbound/outbound event structs, event IDs, event source metadata, and a `Bus` trait or equivalent local interface without requiring a distributed backend.
  - Files: `src/bus.rs`, `src/main.rs`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-20 — Added typed inbound and outbound event envelopes, event IDs, source metadata, a local `Bus` trait contract, and unit tests for the new seam.

- [x] Task 4: Add a local in-process event bus implementation skeleton (P0)
  - Acceptance: A minimal local bus implementation can accept, store, and forward event envelopes in-process, even if the execution path is still stubbed.
  - Files: `src/bus.rs`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-20 — Added a FIFO in-process bus implementation with queue inspection and forwarding methods, plus regression tests for local storage and forwarding behavior.

- [x] Task 5: Introduce a channel abstraction and adapt the current CLI into it (P0)
  - Acceptance: CLI input is represented through a channel adapter contract so the current terminal path is one channel implementation rather than the whole architecture.
  - Files: `src/channels/mod.rs`, `src/channels/cli.rs`, `src/main.rs`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-20 — Added a channel adapter contract, implemented the CLI channel mapping to inbound events, and routed the session path through the CLI adapter plus the local bus seam.

- [x] Task 6: Add a worker runtime boundary around the current agent loop (P0)
  - Acceptance: The current loop is wrapped behind a worker-facing interface that accepts an event/request object and returns a structured worker result.
  - Files: `src/worker.rs`, `src/main.rs`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-21 — Added worker request and result contracts, wrapped `MainAgent` behind a worker runtime adapter, and routed session execution through the worker boundary.

- [x] Task 7: Add a router contract for event ownership (P0)
  - Acceptance: A router interface exists that can choose a target worker or agent profile for an inbound event, even if the initial implementation routes everything to the current default path.
  - Files: `src/router.rs`, `src/main.rs`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-21 — Added a router contract, shipped a default main-worker route, and made session execution resolve route ownership before worker dispatch.

- [x] Task 8: Add dispatch request and response contracts for delegated work (P0)
  - Acceptance: A dedicated dispatch module defines how one worker requests sub-work from another without coupling dispatch to direct in-loop method calls.
  - Files: `src/dispatch.rs`, `src/main.rs`, `src/mainAgent.rs`, `docs/architecture.md`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-21 — Added dispatch request and response contracts, introduced a local dispatcher seam for subagent work, and routed main-agent delegation through the dispatch boundary.

- [x] Task 9: Add prompt-layer assembly structures (P0)
  - Acceptance: The codebase separates prompt layers such as system, project memory, task contract, skill context, observations, and channel metadata into explicit structures or builders.
  - Files: `src/prompt_layers.rs`, `src/main.rs`, `src/mainAgent.rs`, `docs/architecture.md`, `sprints/v2/TASKS.md`
  - Completed: 2026-04-21 — Added explicit prompt-layer structures and a planner prompt builder, then routed planner prompt assembly through those layers with updated architecture notes.

- [x] Task 10: Add a memory-agent boundary and stub implementation (P0)
  - Acceptance: Long-term memory reads and writes cross a dedicated interface that can be backed by Mem0 or file storage now and by a future memory agent later.
  - Files: `src/memory_agent.rs`, `src/mainAgent.rs`, `docs/memory.md`
  - Completed: 2026-04-21 — Added a dedicated memory-agent interface with file, Mem0, and stub backends, then routed long-term memory reads and writes through the new boundary.

- [x] Task 11: Add scheduler event contracts and a local no-op scheduler skeleton (P1)
  - Acceptance: The repo defines scheduled-event types and a scheduler interface, with a placeholder implementation that produces no background work unless explicitly invoked.
  - Files: `src/scheduler.rs`, `docs/architecture.md`
  - Completed: 2026-04-25 — Added scheduled-event contracts, a local explicit-invocation scheduler, module wiring, architecture notes, and regression tests.

- [x] Task 12: Add concurrency gate interfaces for worker admission (P1)
  - Acceptance: A concurrency module defines worker lease/token contracts and a local single-process guard implementation, even if limits remain conservative.
  - Files: `src/concurrency.rs`, `src/worker.rs`, tests as needed
  - Completed: 2026-04-25 — Added worker lease contracts, a local concurrency gate, a gated worker wrapper, and tests for limits, release behavior, and worker-slot cleanup.

- [x] Task 13: Add placeholder outbound channel worker contracts (P1)
  - Acceptance: Outbound delivery is represented as a channel-worker interface with a CLI delivery implementation and placeholders for future non-CLI surfaces.
  - Files: `src/channels/mod.rs`, `src/channels/cli.rs`, `docs/architecture.md`
  - Completed: 2026-04-25 — Added channel-worker delivery contracts, a delivery receipt type, outbound summarization, a local CLI delivery worker, and CLI delivery tests.

- [x] Task 14: Update architecture docs to describe the new control plane (P1)
  - Acceptance: Repo-local docs explain the event bus, channel adapter, router, worker, dispatch, scheduler, concurrency, and memory-agent seams using diagrams and concise data flow notes.
  - Files: `docs/architecture.md`, `docs/cli.md`, `docs/memory.md`
  - Completed: 2026-04-25 — Updated architecture and CLI docs to describe the local control-plane path plus scheduler, concurrency, and outbound delivery seams.

- [x] Task 15: Add regression tests for the new architectural seams (P1)
  - Acceptance: Tests cover event envelope creation, default router behavior, prompt-layer assembly, memory-agent stub behavior, and worker entrypoint contracts.
  - Files: `src/bus.rs`, `src/router.rs`, `src/prompt_layers.rs`, `src/memory_agent.rs`, `src/worker.rs`
  - Completed: 2026-04-25 — Existing seam tests remain green, and new tests cover scheduler polling, concurrency admission, gated worker cleanup, and CLI outbound delivery.

- [x] Task 16: Keep graphify and repo-local maps aligned with the new architecture (P2)
  - Acceptance: After the code skeleton lands, graphify is rebuilt and any high-signal repo-local references stay accurate enough for future agents to navigate the new modules.
  - Files: `graphify-out/GRAPH_REPORT.md`, `graphify-out/graph.json`, `AGENTS.md` if pointers change
  - Completed: 2026-04-25 — Rebuilt graphify after code changes; the graph now reports 599 nodes, 1114 edges, and 34 communities.
