# Development

## Local build

Build the release binary and run the local copy:

```bash
make build
./bin/doctty ./some.md
```

## Quality gates (same as CI)

```bash
make ci    # fmt-check, clippy -D warnings, full test suite
```

GitHub Actions uses the reusable [`muleyuck/github-actions` `ci-rust.yml`](https://github.com/muleyuck/github-actions/blob/main/.github/workflows/ci-rust.yml): Linux runs `make ci`, and macOS runs `cargo build --release --locked`. Both use Rust 1.99.0 with `rustfmt` and `clippy` configured by `mise.toml`, matching jqc's Rust setup. Run `mise install rust` locally to install the same toolchain.

## Git hooks

This repo uses [`lefthook`](https://github.com/evilmartians/lefthook) for local git hooks.

```bash
brew install lefthook
make hooks-install
```

Configured hooks:

- `pre-commit`: `cargo fmt --all -- --check` and `cargo check --workspace --all-targets --all-features`
- `pre-push`: `make ci`

| Command | Purpose |
|---------|---------|
| `make fmt` / `make fmt-check` | `rustfmt` |
| `make lint` | `clippy` with warnings denied |
| `make test` | All tests |
| `make test-unit` / `test-integration` / `test-e2e` | Split suites |
| `make build` | Build `target/release/doctty` and refresh the local `bin/doctty` copy so `./bin/doctty` runs the latest local build |
| `make hooks-install` | Install git hooks from `lefthook.yml` into `.git/hooks` |

## Contributing

Issues and pull requests are welcome. Please run `make ci` before opening a PR so formatting, Clippy, and tests match what GitHub Actions enforces.

## Distribution

- `make build` compiles and signs the release binary, copying it to Git-ignored `bin/doctty`.
- `cargo install --path . --locked` installs from a source checkout.
- CI runs formatting, Clippy and tests with read-only GitHub permissions; it does not commit binaries.
- Versioned GitHub Releases and Homebrew installation are planned. The legacy install script and `doctty update` are removed.

### Contributors

Everyone who lands a change shows up automatically on the [GitHub contributors graph](https://github.com/muleyuck/doctty/graphs/contributors).
