# MEMORY

This file stores durable workspace memory that is worth reusing across
runs. Transient reasoning should stay in the execution trace, not here.

## What Belongs Here

- stable project facts discovered during implementation
- recurring constraints that affect future runs
- decisions that should survive session boundaries

## What Does Not Belong Here

- scratch notes
- chain-of-thought style reasoning
- temporary failures that do not change future work

## Current Durable Memory

- workspace identity is defined in `Workspace/IDENTITY.md`
- repository-wide operating guidance is defined in `AGENTS.md`
- memory-aware technical design guidance is defined in `TDD.md`
- the current scaffold is a Rust starter for agentic systems
