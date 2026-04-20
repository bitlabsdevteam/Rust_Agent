---
name: ship-small
description: Bias implementation toward the smallest coherent change set with verification.
---

# Ship Small

## Purpose
Keep implementation increments small, testable, and easy to review.

## Workflow
1. Confirm the exact goal, constraints, and stop condition.
2. Prefer the minimum coherent code change over broad refactors.
3. Verify with the narrowest useful test loop before expanding scope.
4. Return what changed, what was verified, and what remains open.

## Constraints
- Do not widen scope unless the current path is blocked.
- Preserve existing repository conventions.
- Make risks and assumptions explicit.
