---
name: ship-small
description: Bias implementation toward the smallest coherent change set with verification.
version: 0.1.0
tags: [workflow, implementation]
when_to_use: explicit request or when a bounded implementation should stay narrowly scoped
when_not_to_use: when the task requires a broad refactor or exploratory analysis first
allowed_tools: []
preferred_subagents: []
input_contract: explicit task contract plus relevant files and observations
output_contract: smallest coherent change set with concise validation notes
---

# Ship Small

## Purpose
Keep implementation increments small, testable, and easy to review.

## Use When
- The user explicitly asks to ship a small, bounded change.
- The task can be solved with a narrow implementation slice and focused validation.

## Do Not Use When
- The task is still exploratory and needs file discovery or planning first.
- The correct fix requires a broad architectural refactor.

## Workflow
1. Confirm the exact goal, constraints, and stop condition.
2. Prefer the minimum coherent code change over broad refactors.
3. Verify with the narrowest useful test loop before expanding scope.
4. Return what changed, what was verified, and what remains open.

## Constraints
- Do not widen scope unless the current path is blocked.
- Preserve existing repository conventions.
- Make risks and assumptions explicit.

## Expected Outputs
- A concise implementation result with the smallest coherent diff.
- Focused validation notes and any residual risk that remains open.
