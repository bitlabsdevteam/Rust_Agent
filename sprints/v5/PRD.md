# Sprint v5 - Packaging And One-Line Install

## Sprint Overview

Sprint v5 turns the current Rust CLI from a source-first developer tool into an installable product with a simple operator experience. The goal is to follow the OpenClaw distribution pattern at the UX level, but use Rust-native release tooling for the implementation: prebuilt binaries, one-line shell and PowerShell installers, GitHub Releases, and a documented manual/source fallback.

The sprint goal is distribution, not runtime architecture. The current planner, tool, skill, subagent, memory, and observability behavior should remain intact while the repository gains a production-grade release pipeline and install surface.

## Goals

- Ship prebuilt binaries for the main supported desktop/server targets.
- Add one-command install flows for macOS/Linux and Windows.
- Use GitHub Releases as the first distribution host.
- Add release automation so tagged versions produce installable artifacts without manual packaging work.
- Document install, verification, manual download, and source-build paths clearly enough for a new user to get running without reading the codebase.

## User Stories

- As a new user, I want to install the CLI with a single command so I do not need a Rust toolchain just to try the project.
- As an operator, I want prebuilt release artifacts so installs are fast and predictable across common platforms.
- As a maintainer, I want tagged releases to produce checksums, archives, and installer scripts automatically so distribution is repeatable.
- As a contributor, I want the source-build path to remain documented so local development still works without release artifacts.
- As a future maintainer, I want the packaging and install contract written down so later work can add Homebrew, crates.io, or self-update without reopening the baseline design.

## Technical Architecture

### Stack

- Language: Rust
- Runtime surface: existing CLI binary
- Distribution host: GitHub Releases
- Release automation: GitHub Actions
- Packaging tool: `cargo-dist`
- Installer surfaces: shell installer for macOS/Linux, PowerShell installer for Windows
- Fallback install path: manual release download and `cargo install --locked` or local `cargo build` for contributors

### Target Architecture

```text
Git tag
  |
  v
GitHub Actions release workflow
  |
  v
cargo-dist plan/build/publish
  |
  +--> platform archives
  +--> checksums
  +--> shell installer
  +--> powershell installer
  |
  v
GitHub Release
  |
  +--> one-line install
  +--> manual download
  +--> source-build fallback
```

### Data Flow

1. A maintainer creates and pushes a version tag.
2. The release workflow runs `cargo-dist` planning and build steps for the configured targets.
3. Build jobs produce archives, checksums, and installer scripts.
4. The workflow publishes the artifacts to a GitHub Release.
5. The install docs point users to a one-line install path that resolves to the latest release artifacts.
6. Users verify the install with a stable CLI command such as `--version`.

### Architectural Principles For V5

- Prefer prebuilt binaries over source compilation for end-user installs.
- Keep packaging logic out of the runtime loop wherever possible.
- Make the install surface explicit and documented, not implicit in CI internals.
- Keep release behavior deterministic and tag-driven.
- Preserve contributor workflows while adding product-facing distribution.

### Planned Module And Repo Direction

- `Cargo.toml`: add distribution metadata and binary naming decisions.
- `.github/workflows/`: add release automation for tagged builds and artifact publishing.
- `README.md`: add install, verify, and source-build guidance.
- `docs/install.md`: add platform support, troubleshooting, and release/install expectations.
- CLI surface: ensure the binary name and `--version` output are stable and suitable for packaged installs.

## Scope Boundaries

### In Scope

- Release artifact generation
- One-line installers
- GitHub Release automation
- Checksums and basic artifact verification
- Install and troubleshooting docs
- Stable binary naming for installation and invocation

### Out of Scope

- Homebrew or Scoop publishing in the first pass
- crates.io publishing in the first pass
- Built-in self-update logic in the first pass
- Container deployment changes
- Runtime architecture changes to planning, routing, tools, skills, memory, or subagents
- Cross-platform daemon/service installation beyond normal CLI installation

## Acceptance Criteria

- A tagged release produces installable binaries for the configured targets.
- The release includes checksums and installer scripts.
- A macOS/Linux user can install the CLI with one shell command and run `--version`.
- A Windows user can install the CLI with one PowerShell command and run `--version`.
- The repo documents a source-build path for contributors and a manual-download path for users who do not want installer scripts.
- The packaging direction is documented well enough that later work can add package managers or self-update without redesigning the release baseline.

## Dependencies

- Existing Rust CLI entrypoint in `src/main.rs`
- Current repo documentation in `docs/`
- Existing Git repository and version-tag workflow
- GitHub Releases as the initial artifact host
- `cargo-dist` as the release/installer generation layer

## Release Contract

The v5 release contract is fixed before implementation so later tasks can execute without reopening design:

- GitHub Releases is the initial and only required distribution host for v5.
- `cargo-dist` owns release packaging, archive naming, checksums, and generated installer outputs.
- The release UX must include a shell installer for macOS/Linux and a PowerShell installer for Windows.
- Manual release download is a required fallback and must stay documented alongside installer-based flows.
- Source-build fallback remains documented for contributors and unsupported targets.
- Packaged installs must expose a stable verification command based on `--version`.
- The final public binary name and exact target matrix are separate explicit tasks in this sprint, not open design questions hidden inside packaging work.

## Future Extensions

These follow-ons are intentionally deferred and are not part of the v5 baseline:

- Homebrew publishing
- Scoop publishing
- crates.io publishing
- built-in self-update

The point of v5 is to leave a clean GitHub Releases plus `cargo-dist` baseline that future packaging work can extend without reopening the distribution contract.
