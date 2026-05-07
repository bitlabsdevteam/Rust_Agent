# Install

## Purpose

This document defines the v5 install and release contract before the packaging pipeline is implemented. It is the operator-facing companion to `sprints/v5/PRD.md`: the PRD defines the sprint scope, while this file defines how users are expected to install, verify, and troubleshoot packaged releases.

## Distribution Contract

- GitHub Releases is the primary distribution host for v5.
- `cargo-dist` is the authoritative packaging layer for archives, checksums, and generated installers.
- One-line install is the product goal for macOS/Linux and Windows.
- Manual download remains a first-class fallback for users who do not want to run installer scripts.
- Build From Source remains documented for contributors and environments outside the supported packaged flow.

## Supported Targets

Initial release matrix: this is a limited v5 support contract. The first packaged release explicitly supports:

- macOS Intel (`x86_64-apple-darwin`)
- macOS Apple Silicon (`aarch64-apple-darwin`)
- Linux x86_64 GNU (`x86_64-unknown-linux-gnu`)
- Linux ARM64 GNU (`aarch64-unknown-linux-gnu`)
- Windows x86_64 MSVC (`x86_64-pc-windows-msvc`)

This matrix is intentionally small. It covers the common desktop and server targets the current CLI can support cleanly without widening the packaging surface during v5.

If a target is not in the initial release matrix, the fallback path is manual download when available or local source build.

## GitHub Releases

Each tagged release is expected to publish:

- platform archives
- checksums
- shell installer output
- PowerShell installer output
- release notes with the verification command

GitHub Releases is the source of truth for the first public install experience. Package managers such as Homebrew and Scoop are explicitly deferred follow-on work.

## Release Artifacts

The v5 artifact naming contract is intentionally narrow:

- shell installer: `agent-in-rust-installer.sh`
- PowerShell installer: `agent-in-rust-installer.ps1`
- Unix archives: `.tar.gz`
- Windows archives: `.zip`
- checksum files: `.sha256` using `sha256`

If any of those names or formats change, the packaging regression tests should fail until the docs and release config are updated together.

## One-Line Install

The release pipeline publishes installer scripts alongside every GitHub Release. The stable docs-facing install path uses the GitHub `latest/download` endpoints:

macOS/Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/<OWNER>/<REPO>/releases/latest/download/agent-in-rust-installer.sh | sh
```

Windows PowerShell:

```powershell
irm https://github.com/<OWNER>/<REPO>/releases/latest/download/agent-in-rust-installer.ps1 | iex
```

These paths install the packaged binary without requiring a Rust toolchain. The tag-triggered release workflow also prints tag-specific installer commands into the GitHub Actions run summary for the exact release that was just published.

## Verify The Install

Packaged installs must expose a stable verification command:

```bash
agent-in-rust --version
```

Sprint Task 8 will tighten version reporting and align the packaged output with release tags/build metadata.

## Manual Download

Users who prefer not to run installer scripts should be able to:

1. open GitHub Releases
2. download the archive for their target
3. unpack it locally
4. place the binary on `PATH`
5. run `agent-in-rust --version`

Manual download is part of the baseline contract, not a fallback afterthought.

## Build From Source

This section is the contributor fallback and unsupported-platform fallback:

```bash
cargo install --locked --path .
agent-in-rust --version
```

If you want a local packaged-style binary without installing globally:

```bash
cargo build --release
./target/release/agent-in-rust --version
```

If the repository later adopts `cargo install --locked` as a documented contributor path, it should remain secondary to prebuilt release artifacts for end users.

## Troubleshooting Expectations

- If the packaged binary does not exist for a target, the docs should direct the user to manual download or source build immediately.
- If installer scripts fail, the docs should send the user to the matching GitHub Releases page and checksum/artifact list.
- If `--version` fails after install, that is treated as a release regression.

## Deferred For Later Sprints

Future Extensions that are intentionally deferred and not part of the v5 baseline:

- Homebrew
- Scoop
- crates.io publishing
- self-update
- service/daemon installation flows
