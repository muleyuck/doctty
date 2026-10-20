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

As in [jqc](https://github.com/muleyuck/jqc), this repository uses [prek](https://github.com/j178/prek) rather than Lefthook. Rust and prek versions are pinned in `mise.toml`.

```bash
mise install
make hooks-install
```

For existing checkouts that already installed Lefthook, first run `lefthook uninstall` while the Lefthook CLI is available. This removes its old pre-commit and pre-push Git hooks. Then run the commands above to install prek. `make hooks-install` replaces an existing pre-commit shim.

The `.pre-commit-config.yml` checks YAML, TOML and merge conflict markers, then runs Rust formatting, `cargo check`, Clippy and tests **on each commit** (matching jqc's pre-commit policy). No pre-push hook is installed by prek.

To run the hooks manually against tracked files:

```bash
prek run --all-files
```

| Command | Purpose |
|---------|---------|
| `make fmt` / `make fmt-check` | `rustfmt` |
| `make lint` | `clippy` with warnings denied |
| `make test` | All tests |
| `make test-unit` / `test-integration` / `test-e2e` | Split suites |
| `make build` | Build `target/release/doctty` and refresh the local `bin/doctty` copy so `./bin/doctty` runs the latest local build |
| `make hooks-install` | Install the prek pre-commit hook (overwriting the previous pre-commit shim) |

## Contributing

Issues and pull requests are welcome. Please run `make ci` before opening a PR so formatting, Clippy, and tests match what GitHub Actions enforces.

## Distribution

- `make build` compiles and signs the release binary, copying it to Git-ignored `bin/doctty`.
- `cargo install --path . --locked` installs from a source checkout.
- CI runs formatting, Clippy and tests with read-only GitHub permissions; it does not commit binaries.
- Versioned GitHub Releases and Homebrew installation are planned. The legacy install script and `doctty update` are removed.

### Contributors

Everyone who lands a change shows up automatically on the [GitHub contributors graph](https://github.com/muleyuck/doctty/graphs/contributors).
