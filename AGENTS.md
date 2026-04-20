# AGENTS.md

## Goal

This repository is a Rust starter for building agentic systems.

The implementation should stay small and understandable, but the design standard should follow production-grade agent guidance used in modern OpenAI and Anthropic workflows:

- context engineering first
- harness engineering second
- evals and observability from the beginning
- agent legibility over prompt sprawl

This file defines the operating standard while the scaffold stays intentionally minimal.

## Operating Principle

Humans steer. Agents execute.

## Scope Of This File

`AGENTS.md` is the entry point, not the full encyclopedia.

- keep this file short, stable, and high-signal
- store durable repo knowledge in versioned files inside the repository
- prefer progressive disclosure over one giant instruction blob
- if the project grows, move detailed architecture, product, reliability, and security guidance into `docs/` and keep this file as the map to those sources

## External AI Agent Reference

For any task involving building, designing, reviewing, or extending AI agents, consult this vault before proposing architecture or making implementation decisions:

- `/Users/davidbong/Documents/my_second_brain_vault/AGENTS.md`
- `/Users/davidbong/Documents/my_second_brain_vault/index.md`
- relevant pages under `/Users/davidbong/Documents/my_second_brain_vault/wiki/`

Required workflow for AI agent work:

1. read the vault `AGENTS.md`
2. read the vault `index.md`
3. open the most relevant wiki pages for the task
4. use the vault wiki as the standing reference for agent patterns, context engineering, workflows, and design tradeoffs
5. then apply repository-local constraints and implementation details from this repo

Priority rule:

- repo-local files remain the source of truth for this repository's code, behavior, and constraints
- the vault is the required reference for general AI agent building guidance and reusable design patterns

## Production Standard

### 1. Context Engineering Standard

- start every run with an explicit task contract:
  goal, constraints, acceptance criteria, relevant files, and stop condition
- separate stable context from changing context:
  stable instructions, tool definitions, and invariants first; task-specific input and runtime state later
- keep static instructions and reusable examples near the front of the prompt/input stack
- keep role, policy, and non-negotiable behavior in the system prompt
- keep task-specific state in structured runtime context, not buried in prose
- prefer retrieval and selective loading of repo documents over dumping everything into one prompt
- for AI agent tasks, load the external vault entrypoints and relevant wiki pages before finalizing the working context
- summarize or compact old context when the working set grows, but never drop active constraints silently
- keep tool results, observations, retries, and open questions explicit between loop iterations
- treat repository-local, versioned knowledge as the source of truth; knowledge in chat threads or human memory does not count until written into the repo

Every context packet should make these items obvious:

- what the agent is trying to achieve
- what the agent is allowed to do
- what the agent must not do
- what facts are known
- what facts are missing
- what action is next
- what condition ends the run

### 2. Harness Engineering Standard

The harness must own:

- model selection and fallback
- tool and skill registration
- decision routing
- retries and retry classification
- timeouts and failure handling
- stop conditions
- structured outputs
- execution trace logging
- eval hooks and regression checks

Each step should resolve to one of these actions:

1. call a tool
2. call a skill
3. retry
4. stop
5. return a final answer

## Core Agent Loop

The agent starts with:

- a system prompt
- a user input
- optional prior conversation history
- the registered tool and skill catalog
- the active model policy

The runtime loop should be:

1. load system prompt, user input, relevant history, and active constraints
2. select the primary model according to policy
3. decide the next action
4. if needed, call exactly one tool or one skill
5. record the observation
6. retry if the failure is recoverable and the retry budget allows it
7. stop when a stop condition is reached
8. emit the final result plus a usable trace for debugging

## Model Policy

The model order for this project is:

1. default to `OpenAI GPT-5.4`
2. if the primary model is unavailable or the planner call fails, fallback to `Opus 4.6`

Apply this policy before tool or skill execution so the planner uses the best available model first.

## Required Behaviors

The template must explicitly support all of the following:

- system prompt plus user input as the starting context
- default model selection and fallback model selection
- a planner or decision step that chooses whether to call a tool, call a skill, retry, stop, or finalize
- tool calling
- skill calling
- retry logic
- stop logic
- final output generation
- execution tracing for every loop iteration

## Tool And Skill Contract

- every tool and skill needs a clear name, purpose, input contract, and output contract
- tool and skill failures must be classified as recoverable or terminal
- tool arguments should be structured whenever possible
- tool execution should be bounded by timeout and validation
- tool side effects should be intentional and visible in the trace
- the planner should reason over tool descriptions, not hidden behavior

Production direction:

- tools should move toward structured schemas and validated arguments
- skill selection should be explicit and traceable
- observations from tool and skill results should feed back into the next decision step

## Retry Policy

Retry behavior is capped at **3 maximum retries**.

- if a tool or skill returns a recoverable failure, the agent retries
- if the planner explicitly requests a retry, the retry counter also increases
- when the retry count reaches 3, the agent stops instead of looping forever
- repeated retries without new information are a harness failure, not useful persistence

## Stop Conditions

The agent must stop when any of these occur:

- a final answer is produced
- the retry limit is reached
- the planner decides to stop
- the user input is empty
- the harness detects a terminal failure that should not be retried

## Evaluation Standard

The project should be developed with eval-driven iteration:

- define success criteria before tuning prompts
- create task-specific evals instead of relying on vague impressions
- test not only answer quality, but also tool choice, tool arguments, retries, and stop behavior
- log failures and convert real failures into regression cases
- rerun evals when prompts, tools, models, or routing logic change

Minimum eval categories for this template:

- planner chooses the correct action
- tool selection is correct
- tool arguments are correct
- recoverable failures trigger retries
- retry limit stops the loop
- empty input stops immediately
- final answers are returned when available

## Observability Standard

- every loop iteration should append to an execution trace
- record selected model, planner decisions, tool or skill calls, observations, retries, and stop reason
- prefer structured logs over ad hoc print statements when the system evolves
- make traces useful for both debugging and future eval creation

## Repository Standard

- important decisions should live in repo-local markdown or code, not only in conversation
- design rules should be explicit enough that an agent can discover and follow them
- for AI agent architecture and workflow guidance, the external vault at `/Users/davidbong/Documents/my_second_brain_vault` is a required reference and should be checked first
- if the repo grows, split durable knowledge into focused files such as architecture, product, reliability, and security references
- keep documentation fresh enough that it can be trusted by an agent

## Current Implementation

The current Rust scaffold implements:

- a minimal CLI in `src/main.rs`
- a documented model policy with default `OpenAI GPT-5.4` and fallback `Opus 4.6`
- a Claude-style interactive session runtime instead of the old ingress queue demo
- project memory loading from `CLAUDE.md` plus imported `@path` files
- project and user subagent discovery from `.claude/agents/*.md`
- project and user slash-command discovery from `.claude/commands/*.md`
- isolated subagent delegation with compact context packets and structured handoffs
- MCP tool loading plus grounded `web_search` support
- execution trace output and OpenTelemetry-based observability for session, tool, and subagent behavior

## Near-Term Production Upgrades

Good next steps from this template:

- replace heuristic delegation with a fully model-backed planner interface
- add richer memory precedence and deeper path-scoped project memory
- move command and subagent frontmatter parsing from simple strings to structured schemas
- add stronger context compaction and longer-session history management
- add external tool adapters with validation and timeout policies
- add an eval harness with regression fixtures
- split durable design knowledge into a repo-local `docs/` structure once the project outgrows this file

## Non-Goal

This repository is not yet a full production agent platform. It is a starter template designed with production standards so the architecture can scale without a rewrite later.

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- After modifying code files in this session, run `python3 -c "from graphify.watch import _rebuild_code; from pathlib import Path; _rebuild_code(Path('.'))"` to keep the graph current
