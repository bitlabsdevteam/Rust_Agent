# Skills

## Purpose

Skills are reusable, file-backed capability packs for the local harness. In this project, a skill shapes planning and execution, but it does not register new runtime code paths or arbitrary executable code.

## Locations

- project scope: `.claude/skills/<skill-name>/SKILL.md`
- user scope: `~/.claude/skills/<skill-name>/SKILL.md`

User skills load first. Project skills override user skills when they share the same normalized name.

## Required structure

Each skill directory must contain `SKILL.md`.

Supported frontmatter fields:

- `name`
- `description`
- `version`
- `tags`
- `when_to_use`
- `when_not_to_use`
- `allowed_tools`
- `preferred_subagents`
- `input_contract`
- `output_contract`

Required body sections:

- `## Purpose`
- `## Use When`
- `## Do Not Use When`
- `## Workflow`
- `## Constraints`
- `## Expected Outputs`

`## Examples` is recommended and included in the scaffold template.

## Harness behavior

- The planner always sees the available skill catalog.
- Only the currently active skill body is injected into the planner prompt.
- Selecting a skill activates it in session state; it does not count as task completion.
- If `allowed_tools` is set, the harness narrows visible tools and blocks disallowed tool execution.
- Invalid skill files are skipped and reported by `agent-in-rust skills validate`.

## CLI

```bash
agent-in-rust skills list
agent-in-rust skills show --name ship-small
agent-in-rust skills validate
agent-in-rust skills create --name ship-small --description "Bias toward the smallest coherent change set"
agent-in-rust skills install --source owner/repo/skill-name
```

## Authoring guidance

- Keep instructions compact and behavior-level.
- Prefer skill-specific workflow guidance over generic agent rules already covered by the system prompt.
- Use `allowed_tools` only to narrow execution, not to imply new capabilities.
- Prefer `preferred_subagents` for delegation hints, not mandatory routing.
