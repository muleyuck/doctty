# CI-built Swift helper distribution — experimental preview

The intended public release model is: **CI compiles Rust and Swift; the end user
downloads an already-built macOS bundle.** No Swift compiler, Rust compiler or
Cargo installation should be required on the user's computer.

## Current preview artifact (NOT a production release)

GitHub Actions workflow `.github/workflows/macos-hybrid-bundle.yml` builds
`experiments/webkit_viewport.swift` with optimized `swiftc -O` and
`examples/webkit_viewport_poc.rs` with `cargo build --release --locked`.
The workflow assembles and smoke-tests an architecture-labeled tar archive,
verifies file hashes, and stores it as a downloadable Actions artifact for
14 days. **It does not publish a GitHub Release or change the main branch.**

The archive has this structure:

```text
doctty-hybrid-macos-arm64/             # name matches CI runner architecture
  doctty                              # normal CLI + opt-in Eco bridge
  doctty-hybrid                       # launch wrapper; default Eco mode
  webkit_viewport_poc                 # compiled Rust experiment
  doctty-webkit-swift                 # compiled, optimized native Swift helper
  SHA256SUMS                          # checksums of packaged files
  README.txt
```

Once unpacked, the integration test invocation in Ghostty or Kitty is:

```sh
./doctty --experimental-eco /absolute/path/to/README.md
```

The normal `./doctty /absolute/path/to/README.md` command still runs the
existing production viewer, which has NOT yet switched to Eco. The
`--experimental-eco` mode uses the bundled native Swift helper through
the neighboring Rust viewer, without invoking local Swift or Cargo.
It supports `--theme light`, `--theme dark`, and `--theme system` using
resolved GitHub CSS colors in the Eco preview. The standalone PoC stays
light by default unless `DOCTTY_WEBKIT_THEME=dark` is explicitly set.
The Eco bridge supports `--watch`, polling the source Markdown's modification
time/size at most once every 150 ms and waiting at least 250 ms for edits to
settle. Reload stops the old WebKit and raster workers, regenerates HTML,
and rebuilds the bounded image caches while preserving relative scroll
progress. It keeps the last valid HTML render if rebuilding temporarily fails.
Pressing `o` in the Eco viewer opens the first Markdown link using
the normal viewer's system-browser command and displays a short status
message. This link is re-parsed on `--watch` reload, not while scrolling.
The `g` / `G` keys jump to document top / last scrollable viewport,
matching the normal viewer, and reuse the Eco settle-and-capture schedule.
Other viewer behavior may still differ. Existing headless/plain-text
fallback is unchanged. This is a transitional bridge, not feature parity.

The standalone `doctty-hybrid` launcher also locates the helper next to
itself using
`DOCTTY_WEBKIT_HELPER_BIN`; it never runs `swift`, `swiftc`, or Cargo.
The opt-in previous prefetch mode is still available via
`./doctty-hybrid --hybrid-image-prefetch /absolute/path/to/README.md`.

## Important distinction: production doctty still differs

The default `doctty` viewer still renders with the separate one-shot
snapshot helper embedded in `src/io/webkit_snapshot/script.rs`. That
legacy mode invokes the Swift interpreter at runtime. Only the
`--experimental-eco` branch delegates to the bundled native Swift helper
through `examples/webkit_viewport_poc.rs`.

This is a reversible first CLI-integration step, but **not** the release-ready
default. We must finish feature parity and replace the legacy runtime script
before claiming the general-purpose `doctty` command has no Swift toolchain
dependency on the end-user computer.

## Before an official macOS release

- Complete the Eco integration into the production CLI, preserving the
  remaining navigation and interactive contracts, then remove the temporary
  `--experimental-eco` switch.
- Choose a predictable location for version-matched compiled Swift helpers;
  remove runtime Swift compilation and interpreter fallback from release builds.
- Build/test native Apple Silicon and Intel artifacts (or a tested universal
  combination), with clear macOS version support.
- Sign and notarize the distributed executables appropriately; do not ask
  users to disable Gatekeeper.
- Generate versioned SHA256 archives from a tagged release workflow and
  publish GitHub Release assets, then implement a Homebrew formula.
- Test clean-machine startup, image/keyboard behavior, upgrades, and paths
  containing spaces, without Xcode/Swift or Cargo installed.

The preview CI smoke validates the packed helper's JSON and PNG protocol,
not Ghostty rendering quality or signed application installability.
See `experiments/SWIFT_PERFORMANCE.md` for the Eco performance findings.
