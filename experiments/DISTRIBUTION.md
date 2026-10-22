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
  doctty-hybrid                       # launch wrapper; default Eco mode
  webkit_viewport_poc                 # compiled Rust experiment
  doctty-webkit-swift                 # compiled, optimized native Swift helper
  SHA256SUMS                          # checksums of packaged files
  README.txt
```

Once unpacked, the test invocation in Ghostty or Kitty is:

```sh
./doctty-hybrid /absolute/path/to/README.md
```

The launcher locates the helper next to itself using
`DOCTTY_WEBKIT_HELPER_BIN`; it never runs `swift`, `swiftc`, or Cargo.
The opt-in previous prefetch mode is still available via
`./doctty-hybrid --hybrid-image-prefetch /absolute/path/to/README.md`.

## Important distinction: production doctty still differs

The stable `doctty` CLI currently renders with the separate one-shot
snapshot helper embedded in `src/io/webkit_snapshot/script.rs`. It runs
that Swift script through `swift` at runtime. The persistent hybrid/Eco
renderer lives in `examples/webkit_viewport_poc.rs`, **not** in the main CLI.

Thus the preview archive is only a distribution proof-of-concept. Shipping
the general-purpose `doctty` command without requiring a local compiler
will also require integrating the chosen rendering architecture into
the main application and building/bundling its helper on CI.

## Before an official macOS release

- Integrate the accepted Swift/Eco rendering mode into the production CLI,
  preserving existing flags, headless behavior, and navigation contracts.
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
