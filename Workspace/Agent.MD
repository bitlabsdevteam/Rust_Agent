# AGENT

This file defines the workspace-level contract for the active agent.
Use it as the short operational brief for runs that need local context
in addition to the repository-wide guidance in `AGENTS.md`.

## Purpose

- keep the current agent's goal explicit
- make workspace constraints visible
- reduce prompt sprawl by storing stable local context here

## Task Contract Template

- goal:
- constraints:
- acceptance_criteria:
- relevant_files:
- stop_condition:

## Working Rules

- follow the repository model policy: default `OpenAI GPT-5.4`, fallback `Opus 4.6`
- prefer small, legible actions over large opaque prompts
- call one tool or one skill per decision step
- record observations and retries explicitly
- stop when the task is complete, input is empty, or retry budget is exhausted

## Notes

- use this file for the current workspace contract
- keep durable repository-wide rules in `AGENTS.md`
