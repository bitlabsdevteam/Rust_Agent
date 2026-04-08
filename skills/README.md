# Skills

This directory is where repo-local skills live.

The agent auto-discovers any skill package that matches this layout:

```text
skills/
  my-skill/
    SKILL.md
```

Notes:

- `SKILL.md` is required.
- The skill folder name is used as a fallback skill name when `SKILL.md` does not provide one in frontmatter.
- Optional supporting files can live beside `SKILL.md` or inside subfolders like `examples/`, `references/`, or `scripts/`.
- Installed skills appear automatically in `agent_in_rust list` and in the chat `/skills` command.
- When the planner selects an installed skill, the agent loads the skill lazily from disk and injects its `SKILL.md` into the model execution step.

Minimal example:

```md
---
name: my-skill
description: Use when the user wants help with a specific workflow.
---

# My Skill

Explain what the skill should do, when to use it, and any constraints.
```
