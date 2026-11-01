# doctty — Project guidance

doctty is a Rust 2024 Markdown viewer for interactive terminals and headless/piped output.
These are shared repository standards for human contributors and coding agents. Use personal
Claude/Codex settings for model selection, delegation, and optional automation.

## Preserve these contracts

- Preserve existing CLI flags, output modes, and terminal behavior unless explicitly changing them.
- When changing CLI or distribution behavior, trace its consumers across code, tests, scripts,
  documentation, and CI (where applicable), then verify the affected contracts.
- Treat release binaries as build artifacts; do not manually patch a compiled binary.
- Keep the MIT LICENSE intact and preserve attribution to the upstream `posaune0423/mdv` project.

## Sources of truth

- `README.md` covers supported user-facing features, installation and usage.
- CLI definitions, Rust source and tests define current behavior; consult them when changing implementation details rather than relying on a separate architecture/requirements document.
- Keep implementation-specific explanations near the code. Record durable design decisions and alternatives in relevant Issues/PRs when necessary.

Do not preload unrelated project history for a small edit.

## Development environment

- Run `mise install` to install the pinned Rust toolchain and prek.
- Run `make hooks-install` to install prek's pre-commit hook; `prek run --all-files` checks all tracked files.
- `.pre-commit-config.yml` checks YAML/TOML, merge conflicts, Rust formatting, type checking, Clippy and tests.
- Run `make build` to compile the release binary and update the local, Git-ignored `bin/doctty`. On macOS, the build target also codesigns it.
- `cargo install --path . --locked` installs from a source checkout.
- GitHub Actions runs `make ci` on Linux; on macOS it runs `cargo build --release --locked` and
  the macOS-only `doctty-renderer` tests.
- Do not commit generated binaries or resurrect the removed install script and self-update command; versioned GitHub Releases/Homebrew distribution is a separate future task.

## Change and verification policy

- Keep changes focused. For mechanical, behavior-preserving renames, search callers and
  references, edit the affected surfaces, then run relevant existing checks.
  Do not manufacture a failing test for a rename that does not change behavior.
- For new or changed behavior, add or adjust tests covering the contract and failure cases.
- For rendering changes, check real TTY output, navigation, and startup behavior when available;
  a successful unit test or process launch alone does not prove visual correctness.
- Prefer targeted checks while developing; use broader checks for cross-cutting or risky changes.
- Formatting: `cargo fmt --all -- --check`
- Type checking: `cargo check --workspace --all-targets --all-features`
- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Tests: `cargo test --workspace --all-targets --all-features`
- GitHub Actions is the full-suite PR gate; run `make ci` locally for broad/high-risk changes
  when feasible. Existing Git hooks remain in effect. Check CLI/distribution contracts as applicable.
- If a local command is blocked by a sandbox or filesystem restriction, diagnose it once,
  report the limitation, and use CI results when available. Do not claim a check passed
  when it did not run successfully.

## Review and collaboration

- Review for actionable correctness, compatibility, security, and test gaps.
- Prefer one whole-change review. After fixes, review only the new diff, unresolved findings,
  and necessary surrounding context. Repeat a full review only for a material design/risk change.
  If blocking findings remain after two targeted fix/review attempts, report them and stop
  rather than starting an unbounded review loop.
- Planning documents, subagents, cross-model reviews, mutation tests, and model effort
  are optional execution methods, not mandatory steps for every contributor or edit.
- Report what changed, what was verified, and any remaining limitations.
