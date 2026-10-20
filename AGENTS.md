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
