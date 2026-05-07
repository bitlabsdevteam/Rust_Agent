- [x] Task 1: Create sprint v5 packaging docs and install contract (P0)
  - Acceptance: `sprints/v5/PRD.md` and `sprints/v5/TASKS.md` define the release, packaging, and install direction clearly enough to implement without reopening the design.
  - Files: `sprints/v5/PRD.md`, `sprints/v5/TASKS.md`, `README.md`, `docs/install.md`
  - Completed: 2026-05-07 — Added the repo-level install contract, operator-facing install doc, and explicit release-contract decisions for the remaining v5 packaging tasks.

- [x] Task 2: Decide and normalize the public binary name (P0)
  - Acceptance: The repository has one stable user-facing binary name for releases, install docs, and verification commands, and the naming choice is reflected consistently in package metadata and docs.
  - Files: `Cargo.toml`, `src/main.rs`, `docs/cli.md`, `README.md`
  - Completed: 2026-05-07 — Chose `agent-in-rust` as the public release binary and pinned that name across Cargo metadata, help text, CLI docs, and install/verification examples.

- [x] Task 3: Add `cargo-dist` metadata for release artifacts (P0)
  - Acceptance: The project is configured to build the intended release targets, archives, checksums, and installer outputs through `cargo-dist`.
  - Files: `Cargo.toml`, optional dist config files, release docs as needed
  - Completed: 2026-05-07 — Added `cargo-dist` workspace metadata for GitHub CI, shell/PowerShell installers, archive/checksum formats, and the initial macOS/Linux/Windows release targets.

- [x] Task 4: Add tagged GitHub Actions release automation (P0)
  - Acceptance: A tag-triggered workflow plans and builds release artifacts, publishes them to GitHub Releases, and exposes installer script URLs tied to the release output.
  - Files: `.github/workflows/release.yml`, repo settings/docs as needed
  - Completed: 2026-05-07 — Added a tag-driven GitHub Actions release workflow that runs `cargo-dist` plan/build/host phases, uploads build artifacts between jobs, publishes to GitHub Releases, and writes stable installer script URLs into the workflow summary.

- [x] Task 5: Define the supported target matrix for v5 (P0)
  - Acceptance: The initial release matrix is explicit, limited, and documented, including at least common macOS, Linux, and Windows targets appropriate for this CLI.
  - Files: `Cargo.toml`, `README.md`, `docs/install.md`
  - Completed: 2026-05-07 — Added an explicit v5 support matrix comment to `Cargo.toml` and documented the exact macOS, Linux, and Windows target triples in the README and install guide, backed by a regression test that enforces the limited public matrix contract.

- [x] Task 6: Add shell and PowerShell installer support through the release pipeline (P1)
  - Acceptance: The release output includes a one-line install path for macOS/Linux and Windows that installs the packaged binary without requiring a Rust toolchain.
  - Files: `Cargo.toml`, generated installer config, `README.md`, `docs/install.md`
  - Completed: 2026-05-07 — Added concrete shell and PowerShell installer commands to the README and install guide, expanded the release workflow summary to print tag-specific one-line install commands, and added a regression test that enforces concrete installer paths across workflow and docs.

- [x] Task 7: Add manual download and contributor fallback install docs (P1)
  - Acceptance: The repo documents how to install from release archives manually and how to build/install from source for contributors.
  - Files: `README.md`, `docs/install.md`, `docs/cli.md`
  - Completed: 2026-05-07 — Added explicit manual-download steps, contributor fallback install commands, and packaged verification guidance to the README, install guide, and CLI guide, backed by a regression test that enforces those fallback surfaces.

- [x] Task 8: Ensure version reporting and packaged verification are stable (P1)
  - Acceptance: Packaged installs have a clear verification command such as `--version`, and the reported version aligns with the release tag/build metadata.
  - Files: `src/main.rs`, `Cargo.toml`, tests as needed, `README.md`
  - Completed: 2026-05-07 — Added a first-class `--version`/`version` CLI command with stable output, release-tag-aware version metadata for packaged builds via the release workflow, and regression coverage that ties the CLI output to package metadata and the tagged-release build path.

- [x] Task 9: Add packaging and release regression coverage (P1)
  - Acceptance: The repo has bounded checks for release configuration validity, installer/documentation drift, and artifact naming assumptions so packaging changes fail loudly.
  - Files: tests, CI config, `Cargo.toml`, `README.md`, `docs/install.md`
  - Completed: 2026-05-07 — Added a dedicated artifact-naming regression that locks archive/checksum formats and installer basenames against the docs and release workflow, and documented the bounded v5 artifact naming contract in the README and install guide.

- [x] Task 10: Document future extensions without implementing them (P2)
  - Acceptance: The sprint closes with explicit notes on deferred follow-ons such as Homebrew, Scoop, crates.io, and self-update so future work can build from the v5 baseline cleanly.
  - Files: `sprints/v5/PRD.md`, `README.md`, `docs/install.md`
  - Completed: 2026-05-07 — Added explicit future-extension notes to the v5 PRD, README, and install guide covering Homebrew, Scoop, crates.io, and self-update as deferred follow-ons outside the v5 baseline, backed by a regression test that keeps those scope boundaries visible.
