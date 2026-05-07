# Agent In Rust

`agent-in-rust` is a small Rust starter for harness-first agent systems. The runtime stays intentionally compact, but the repo is structured around production-grade agent concerns: context engineering, explicit tool and skill contracts, retries, stop conditions, evals, observability, and legible traces.

## Current Status

- Runtime and architecture guidance live in `AGENTS.md` plus `docs/`.
- Sprint v5 is defining the packaging and install contract so the CLI can move from source-first usage to GitHub Releases with one-line installers.
- The public release binary name is `agent-in-rust`.

## Install

The install contract for v5 is documented in [docs/install.md](/Users/davidbong/Documents/ModernSoftwareDeveloperProject/Agent_In_Rust/docs/install.md).

Current supported paths:

- GitHub Releases will be the primary distribution surface for prebuilt binaries and installer scripts.
- One-line installer entrypoints are part of the v5 release contract for macOS/Linux and Windows.
- A manual download path from GitHub Releases will remain available for users who do not want installer scripts.
- Contributors can continue to build from source with Cargo during the packaging rollout.

Installer command shapes once releases are published:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/<OWNER>/<REPO>/releases/latest/download/agent-in-rust-installer.sh | sh
```

```powershell
irm https://github.com/<OWNER>/<REPO>/releases/latest/download/agent-in-rust-installer.ps1 | iex
```

These installer paths are designed to install the packaged binary without requiring a Rust toolchain.

Packaged verification uses a stable packaged verification string:

```bash
agent-in-rust --version
```

Expected output shape:

- local/source build: `agent-in-rust <version>`
- tagged release build: `agent-in-rust <version> (release tag v<version>)`

## Supported Targets

Initial release matrix: this is a limited v5 support contract. This sprint only promises prebuilt release artifacts for:

- macOS Intel (`x86_64-apple-darwin`)
- macOS Apple Silicon (`aarch64-apple-darwin`)
- Linux x86_64 GNU (`x86_64-unknown-linux-gnu`)
- Linux ARM64 GNU (`aarch64-unknown-linux-gnu`)
- Windows x86_64 MSVC (`x86_64-pc-windows-msvc`)

If your environment falls outside that matrix, the expected fallback is manual download when available or a local source build.

## Release Artifacts

Each GitHub Release is expected to include a bounded set of artifact names and formats:

- shell installer: `agent-in-rust-installer.sh`
- PowerShell installer: `agent-in-rust-installer.ps1`
- Unix archives: `.tar.gz`
- Windows archives: `.zip`
- checksum files: `.sha256` using `sha256`

Those naming assumptions are part of the release contract and are covered by repo regression tests.

## Manual Download

If you do not want to run installer scripts, the manual download path is:

1. open GitHub Releases
2. download the archive for their target
3. unpack it locally
4. place the binary on `PATH`
5. run `agent-in-rust --version`

This is the primary non-installer fallback for packaged releases.

## Build From Source

This section is the contributor fallback when packaged installers are not the right fit:

```bash
cargo install --locked --path .
agent-in-rust --version
```

Or build the binary directly:

```bash
cargo build
cargo run
```

For a release-style local build:

```bash
cargo build --release
./target/release/agent-in-rust --version
```

## Docs

- `docs/architecture.md` - runtime loop, planner flow, memory surfaces, and extension seams
- `docs/cli.md` - top-level commands, slash commands, and operator behavior
- `docs/install.md` - packaging, release, install, verification, and fallback contract
- `sprints/v5/PRD.md` - sprint contract for packaging and one-line install

## Future Extensions

These items are intentionally deferred and are not part of the v5 baseline:

- Homebrew
- Scoop
- crates.io publishing
- self-update

The current release contract is intentionally narrower: GitHub Releases, `cargo-dist`, one-line installers, manual download, and contributor/source fallback.
