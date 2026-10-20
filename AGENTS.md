# doctty — Project guidance

doctty is a Rust 2024 Markdown viewer for interactive terminals and headless/piped output.
These are shared repository standards for human contributors and coding agents. Use personal
Claude/Codex settings for model selection, delegation, and optional automation.

## Preserve these contracts

- Preserve existing CLI flags, output modes, and terminal behavior unless explicitly changing them.
- When renaming a CLI surface or changing installation/update behavior, follow its references
  through Rust code, tests, scripts, documentation, and CI. Verify the relevant contracts.
- Treat release binaries as build artifacts; do not manually patch a compiled binary.
- Keep the MIT LICENSE intact and preserve attribution to the upstream `posaune0423/mdv` project.

## Read context only when relevant

- `docs/TECH.md` — implementation architecture or rendering changes.
- `docs/STRUCTURE.md` — unfamiliar module boundaries.
- `docs/DEVELOPMENT.md` — local commands, hooks, and distribution workflow.
- `.agents/memory/` contains historical work notes, **not** the active task ledger.
  Do not read it at session start or append to it for routine tasks; consult it only
  when historical context is specifically needed.

Do not preload all project docs or past task logs before a small edit.

## Change and verification policy

- Keep changes focused. For mechanical, behavior-preserving renames, search callers and
  references, edit the affected surfaces, then run relevant existing checks.
  Do not manufacture a failing test for a rename that does not change behavior.
- For new or changed behavior, add or adjust tests covering the contract and failure cases.
- Prefer targeted checks while developing; use broader checks for cross-cutting or risky changes.
- Formatting: `cargo fmt --all -- --check`
- Type checking: `cargo check --workspace --all-targets --all-features`
- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Tests: `cargo test --workspace --all-targets --all-features`
- Run `make ci` before PR handoff when the environment permits. For CLI/distribution changes,
  also check the affected integration/contract tests and installation paths.
- If a local command is blocked by a sandbox or filesystem restriction, diagnose it once,
  report the limitation, and use CI results when available. Do not claim a check passed
  when it did not run successfully.

## Review and collaboration

- Review for actionable correctness, compatibility, security, and test gaps.
- Prefer one whole-change review; when fixes are made, verify the prior findings against
  the new diff and required surrounding context instead of reviewing unchanged code again.
  Re-open the full review only if the fix changes the design or risk boundary substantially.
- Planning documents, subagents, cross-model reviews, mutation tests, and model effort
  are optional execution methods, not mandatory steps for every contributor or edit.
- Report what changed, what was verified, and any remaining limitations.
