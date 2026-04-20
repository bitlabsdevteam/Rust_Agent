# Tooling Audit

## Purpose

This note records the current `npm audit` findings observed while working in this repository and explains why they are not remediated through a repo-local `package.json` or `package-lock.json`.

## Current State

Running `npm audit` from this repository reports 12 vulnerabilities:

- 5 high
- 7 moderate

The affected transitive packages include:

- `@babel/runtime`
- `ajv`
- `brace-expansion`
- `cross-spawn`
- `flatted`
- `glob`
- `js-yaml`
- `micromatch`
- `minimatch`
- `nanoid`
- `picomatch`
- `yaml`

## Why This Is Not Fixed In-Repo

This repository does not contain a versioned `package.json` or `package-lock.json`.

Observed behavior:

- `npm root` resolves to `/Users/davidbong/node_modules`
- `npm ls --depth=0` reports globally installed tooling from the user environment
- the audit findings therefore apply to global Node tooling, not to a repo-owned dependency manifest

Because there is no repo-local Node manifest, there is no bounded lockfile update to commit here without inventing a new Node package surface for the project.

## Current Global Tooling Surface

The current global packages relevant to the audit include:

- `eslint@8.57.0`
- `eslint-plugin-import@2.29.1`
- `eslint-plugin-jsx-a11y@6.9.0`
- `eslint-plugin-react@7.34.3`
- `@typescript-eslint/parser@7.14.1`
- `postcss@8.4.38`
- `tailwindcss@3.4.4`
- `@emotion/react@11.11.4`
- `@emotion/styled@11.11.5`
- `@mui/material@5.16.0`
- `@mui/icons-material@5.16.0`

These globals currently introduce the vulnerable transitive packages. Examples:

- `eslint` brings in `ajv`, `cross-spawn`, `js-yaml`, `minimatch`, and `flatted`
- `tailwindcss` brings in `glob`, `micromatch`, `picomatch`, `yaml`, `cross-spawn`, and `minimatch`
- `postcss` brings in `nanoid`
- Emotion and MUI packages currently pin `@babel/runtime`

## Bounded Next Action

To actually remediate the findings, do one of these outside the repo:

1. Update the global npm toolchain and rerun `npm audit`.
2. Replace the global tooling dependency with a repo-local, versioned `package.json` and `package-lock.json` if this project truly needs Node-based dev tooling.

If staying with global tooling, the practical remediation path is:

```bash
npm update -g eslint eslint-plugin-import eslint-plugin-jsx-a11y eslint-plugin-react @typescript-eslint/parser postcss tailwindcss @emotion/react @emotion/styled @mui/material @mui/icons-material
npm audit
```

If reproducibility matters, prefer moving any required Node tooling into a repo-local manifest instead of relying on global installs.

## Repository Decision

For sprint v1, this repository records the audit status and keeps the scope bounded:

- no repo-local Node manifest is introduced solely to silence a global audit
- no global dependency mutation is committed from this repo
- future work may add a repo-local Node tooling manifest if the project adopts stable Node-based workflows
