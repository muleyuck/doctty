<div align="center">

# doctty

**Browser-quality Markdown, right in your terminal.**

<img src="docs/demo.gif" width="700" alt="doctty rendering markdown in Ghostty" />

[![CI](https://github.com/muleyuck/doctty/actions/workflows/ci.yml/badge.svg)](https://github.com/muleyuck/doctty/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.92%2B-orange.svg)](https://www.rust-lang.org/)

</div>

`doctty` is a blazing-fast Markdown viewer built in Rust. With the [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/), it renders rich, beautiful Markdown natively in your terminal, including typography, images, diagrams, syntax highlighting, and GFM features like callouts.

If you live in [Ghostty](https://ghostty.org/) or [Kitty](https://sw.kovidgoyal.net/kitty/), you no longer need to leave the terminal to read a README.

The [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) is also implemented by terminals including Ghostty, Konsole, st (with a patch), Warp, wayst, WezTerm, iTerm2, and xterm.js. `doctty` currently enables interactive mode in Ghostty and Kitty.

## Features

|                              |                                                                                                |
| ---------------------------- | ---------------------------------------------------------------------------------------------- |
| **GitHub CSS rendering**     | WebKit snapshots with GitHub's actual stylesheets — what you see on github.com is what you get |
| **Inline images**            | PNG, JPEG, GIF, WebP rendered directly in the terminal                                         |
| **Mermaid diagrams**         | Flowcharts, sequence diagrams, and more — rendered inline                                      |
| **Syntax highlighting**      | 100+ languages via [syntect](https://github.com/trishume/syntect)                              |
| **System/light/dark themes** | `system` by default, with explicit `light` / `dark` overrides                                  |
| **Watch mode**               | `--watch` auto-reloads on file change                                                          |
| **Pipe-friendly**            | Plain-text fallback when stdout is not a TTY — works in CI and scripts                         |
| **Vim navigation**           | `j`/`k`/`g`/`G`/PageUp/PageDown                                                                |

## Quick start

**macOS prebuilt Eco preview (draft, not an official release)**: the
`macOS Eco bundle validation` GitHub Actions workflow produces
an Apple Silicon (arm64) artifact named
`doctty-eco-prebuilt-macOS-arm64`, containing precompiled
`doctty`, its Rust viewport companion and the optimized Swift/WebKit
helper. Extract the inner `.tar.gz` and run in Ghostty or Kitty:

```bash
./doctty-macos-arm64/doctty README.md
```

The initial preview/release target is **Apple Silicon only**. Intel Mac
support is deferred until there is clear demand. CI verifies the native
arm64 architecture of all three executables. These are macOS 15 CI
builds; support on earlier macOS versions has not been established.

No Swift compiler, Rust toolchain or repository checkout is needed for this
prebuilt Eco preview.

**Planned turnray-style distribution (after release review):** CI will also
produce a versioned Apple Silicon ZIP containing ad-hoc-signed native binaries,
upload it with a SHA256 checksum to GitHub Releases, and optionally update the
`muleyuck/tap` Homebrew Cask. The intended commands will be:

```bash
brew install --cask muleyuck/tap/doctty
doctty README.md
```

The ZIP is **not Apple-notarized**. macOS Gatekeeper may block it when
quarantined; do not disable Gatekeeper globally or override warnings for
downloads you have not verified. Confirm the official Release URL and
SHA256 before manually allowing a trusted build. Until the integration PR
is merged and an actual release is published, use only the CI preview
artifact described above. Apple Developer Program enrollment is deferred.

**Building from source (developer workflow)** requires Rust 1.92+:

```bash
cargo install --path . --locked
```

Source-only installs do **not** include the compiled Swift and Rust Eco
companions. They preserve the historical interactive WebKit renderer by
default: `doctty README.md` still works with the Swift tooling normally
required for a source build. The hidden `--experimental-eco` flag remains solely as a compatibility alias
for developers who already installed both matching companions.

CI-built macOS bundles enable the Cargo `bundled-eco` feature and include
the two companions, so `doctty README.md` uses Eco automatically without
running Swift or Cargo on the end-user machine. Bundles with missing helpers
fail explicitly instead of falling back to an interpreter. Headless/piped
text output remains unchanged.

## Usage

```bash
doctty README.md                           # compiled Eco on macOS (prebuilt archive)
doctty --theme dark notes.md               # dark theme
doctty --watch examples/rich_markdown.md    # auto-reload on save
doctty --no-mermaid spec.md                # skip Mermaid rendering
doctty --version                           # print the current version
doctty ./README.md | head -n 50            # plain-text output (pipe/CI)
```

### Keyboard shortcuts

| Key                   | Action               |
| --------------------- | -------------------- |
| `j` / `Down`          | Scroll down          |
| `k` / `Up`            | Scroll up            |
| `g`                   | Jump to top          |
| `G`                   | Jump to bottom       |
| `PageUp` / `PageDown` | Page scroll          |
| `r`                   | Reload file          |
| `o`                   | Open link in browser |
| `q`                   | Quit                 |

## Requirements

- **Terminal**: [Ghostty](https://ghostty.org/) or [Kitty](https://sw.kovidgoyal.net/kitty/) ([Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) required)
- **Protocol ecosystem**: The [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) is also implemented by Ghostty, Konsole, st (with a patch), Warp, wayst, WezTerm, iTerm2, and xterm.js
- **Rich rendering**: macOS with the bundled precompiled Swift/WebKit helper (in the CI-built Eco archive). Linux runs in headless/plain-text mode.
- **Mermaid** (optional): `mmdc` or `npx @mermaid-js/mermaid-cli`, or set `DOCTTY_MERMAID_CMD`

## Installation

From a clone, build with Cargo for development:

```bash
cargo install --path . --locked
```

A source-only install uses the historical interactive renderer without any
extra flags and may require a working Swift toolchain. **Do not** enable
`--features bundled-eco` in a source-only Cargo installation: the feature
is reserved for archives that ship both compiled companion executables.
For compiler-free end-user installation, use the CI-built archive instead.

For development, `make build` creates `./bin/doctty`. The generated `bin/`
directory is not committed.

## Troubleshooting

| Symptom                            | What to try                                                                                                          |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `doctty: command not found`           | Ensure Cargo's bin directory (usually `$HOME/.cargo/bin`) is on `PATH`.                                                   |
| Plain text instead of rich viewer  | Requires a TTY in Ghostty or Kitty. Pipes and redirects trigger headless mode by design.                             |
| Mermaid diagrams show placeholders | Install `mmdc` or use `npx @mermaid-js/mermaid-cli`. Use `--no-mermaid` to skip entirely.                            |
| Graphic / snapshot issues          | Rich rendering uses WebKit on macOS. Check terminal output for diagnostic details.                                   |

## Markdown support

doctty uses [Comrak](https://github.com/kivikakk/comrak) for Markdown parsing and supports common [GitHub Flavored Markdown](https://github.github.com/gfm/) features such as tables, task lists, footnotes and callouts, as well as math and optional Mermaid diagrams. Rendering can differ from GitHub in edge cases.

## Contributing

Development requirements and verification commands are in [AGENTS.md](AGENTS.md). Run `mise install` to install the pinned tools, `make hooks-install` to enable prek hooks, and `make ci` before opening a PR.

## License

Based on [posaune0423/mdv](https://github.com/posaune0423/mdv), maintained here as [muleyuck/doctty](https://github.com/muleyuck/doctty). The original copyright notice and license are preserved unchanged.

Released under the [MIT License](LICENSE).
