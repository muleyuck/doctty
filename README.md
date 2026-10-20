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

**Install from a local checkout** (requires Rust 1.92+):

```bash
cargo install --path . --locked
```

Versioned GitHub Releases and Homebrew distribution are planned. There is currently no installer script or in-app updater.

**Run** (in Ghostty or Kitty):

```bash
doctty README.md
```

## Usage

```bash
doctty README.md                           # system theme (default)
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
- **Rich rendering**: macOS (uses WebKit for HTML→PNG snapshots). Linux runs in headless/plain-text mode.
- **Mermaid** (optional): `mmdc` or `npx @mermaid-js/mermaid-cli`, or set `DOCTTY_MERMAID_CMD`

## Installation

From a clone, install with Cargo:

```bash
cargo install --path . --locked
```

For development, run `make build` and execute `./bin/doctty`. The `bin/` directory is generated locally and is not committed.

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
