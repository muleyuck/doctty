# CI-built Swift helper distribution — experimental preview

The intended public release model is: **CI compiles Rust and Swift; the end user
downloads an already-built macOS bundle.** No Swift compiler, Rust compiler or
Cargo installation should be required on the user's computer.

## Current preview artifact (NOT a production release)

GitHub Actions workflow `.github/workflows/macos-hybrid-bundle.yml` builds
`experiments/webkit_viewport.swift` with optimized `swiftc -O` and
both `doctty` and `examples/webkit_viewport_poc.rs` with
`cargo build --release --locked --features bundled-eco`.
The `bundled-eco` feature is intentionally **off** for `cargo install`,
`make build`, and normal source-development builds.
The initial target is Apple Silicon (`macos-15` / `arm64`) only.
Intel support is intentionally deferred. Each archive is checksum-verified,
checks all three executables' native arm64 architecture using `lipo`,
exercises the compiled Swift protocol, and runs the packaged CLI via
a pseudo-terminal with stub sidecars. CI also verifies the ordinary
no-feature Cargo default on macOS. The arm64 tar archive is stored as
a downloadable Actions artifact for 14 days. **It does not publish a GitHub Release or change the main branch.**

The archive has this structure:

```text
doctty-hybrid-macos-arm64/             # Apple Silicon CI runner
  doctty                              # normal CLI; compiled Eco is macOS default
  doctty-hybrid                       # launch wrapper; default Eco mode
  webkit_viewport_poc                 # compiled Rust experiment
  doctty-webkit-swift                 # compiled, optimized native Swift helper
  SHA256SUMS                          # checksums of packaged files
  README.txt
```

Once unpacked, the integration test invocation in Ghostty or Kitty is:

```sh
./doctty /absolute/path/to/README.md
```

The normal interactive `./doctty /absolute/path/to/README.md` command
selects the compiled Eco viewer when the `bundled-eco` Cargo feature is
enabled (as in this CI-built macOS archive). Source-only Cargo installs,
which lack companions and do not enable that feature, keep the old
interactive viewer by default.
It uses the neighboring Rust companion and native Swift helper without
invoking `swift`, `swiftc` or Cargo. The hidden `--experimental-eco` flag
remains a compatibility alias. The hidden `--legacy-renderer` developer
override selects the old one-shot snapshot code, which still invokes
the Swift interpreter; this is NOT part of the no-compiler release path.
If the two companions are missing from a CI-bundled build, interactive
macOS fails with a clear packaging error instead of silently falling back
to interpreted Swift. Source-only installs invoke the old renderer by
design, not as a runtime fallback from Eco.
It supports `--theme light`, `--theme dark`, and `--theme system`
using resolved GitHub CSS colors. The normal CLI passes Mermaid mode
through to the Eco HTML renderer: Mermaid is enabled by default
(using optional `mmdc` / `npx` if available), and `--no-mermaid` explicitly
disables it. The standalone PoC retains light theme and disabled Mermaid
defaults unless variables are passed by the packaged CLI.
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
Press `r` to manually reload the Markdown without needing `--watch`.
This uses the same worker teardown, HTML regeneration, bounded cache reset,
and relative scroll progress preservation as the `--watch` reload path.
If `--watch` is active, a successful manual reload acknowledges the
document revision to avoid immediately reloading it again. Missing or
transiently unreadable sources preserve the old rendered state.
Other viewer behavior may still differ. Existing headless/plain-text
fallback is unchanged. This is a transitional bridge, not feature parity.

The standalone `doctty-hybrid` launcher also locates the helper next to
itself using
`DOCTTY_WEBKIT_HELPER_BIN`; it never runs `swift`, `swiftc`, or Cargo.
The opt-in previous prefetch mode is still available via
`./doctty-hybrid --hybrid-image-prefetch /absolute/path/to/README.md`.

## Default routing and remaining release limitations

The **CI-built macOS interactive** `doctty` command defaults to
precompiled Eco because the package script builds `doctty` with
`--features bundled-eco`, while stdin/piped/headless behavior is
unchanged. **Source-only Cargo installs** omit `bundled-eco` and continue
using the historical WebKit snapshot path. That path invokes `swift`
at runtime and remains a source-developer workflow, not an end-user
binary distribution requirement. The old one-shot helper embedded in
`src/io/webkit_snapshot/script.rs` can also be explicitly selected with
`--legacy-renderer` for development. An incomplete CI-built Eco archive
must fail rather than silently launching the old Swift interpreter.

**This is still a draft CI preview archive, not an installable signed
production release.** The Eco rendering loop currently lives in a
separately compiled Rust example executable; a future refactor may fold
that code into the CLI itself. This does not prevent the fully prebuilt
archive from launching `doctty README.md` directly, but production
signing, version matching, clean-machine testing, and eventual
distribution still need to be completed.

## turnray-style distribution (no Apple paid account)

The proposed `.github/workflows/release-please.yml` mirrors turnray:
after successful `CI` on a trusted `main` push, release-please opens a
version bump PR or creates a release tag. The release job builds the
same arm64 Rust and optimized Swift bundle, signs all three executables
**ad-hoc** (not with Developer ID), generates
`doctty-<version>-macos-arm64.zip` plus its checksum, and attaches both
to GitHub Releases. A Homebrew Cask template in `packaging/doctty.rb.in`
then allows `brew install --cask muleyuck/tap/doctty` when the separate
tap GitHub App credentials are configured. No Apple Developer Program
subscription is required.

A `doctty-cli` shell launcher bundled inside the ZIP resolves symlinks
created by Homebrew and starts the actual Rust executable beside both
required companion binaries. The normal `doctty` command still runs Eco
without the user installing Rust, Swift, or a compiler.

**Limitations:** An ad-hoc signature does not establish publisher identity
or satisfy Apple's notarization checks. macOS Gatekeeper can block a
quarantined copy, including Homebrew-installed files. Users should verify
the official GitHub Release and published SHA256 before intentionally
allowing the app, and must not disable Gatekeeper globally. This is a
tradeoff inherited from turnray's current unsigned/notarization-free
distribution and should be revisited if public adoption grows.

The release workflow cannot publish while this branch remains a draft PR;
no GitHub Release or Homebrew tap changes are made by PR CI. The tap
publication step is skipped if `TAP_APP_CLIENT_ID` or
`TAP_APP_PRIVATE_KEY` are absent from doctty's Actions secrets.

## Before an official macOS release

- Refactor Eco's separate Rust companion into the main CLI when practical,
  while preserving its bounded caches and low-CPU settling scheduler.
- Delete or isolate the developer-only one-shot Swift interpreter path
  before formal release; no compiler or interpreter fallback is allowed
  in distributed builds.
- Establish minimum supported macOS version and perform clean-machine testing
  on Apple Silicon (CI runs on macOS 15; older systems have not been validated).
  Intel is outside the initial release scope and can be reconsidered if demand emerges.
- Verify the versioned, ad-hoc-signed ZIP and Homebrew Cask on an
  actual Apple Silicon Mac, including Gatekeeper and symlinked CLI behavior.
- Run the release-please workflow after integration approval to publish
  GitHub Release assets. Configure Homebrew tap GitHub App secrets before
  publishing the Cask. Developer ID and Apple notarization are deferred
  rather than required for the initial release.
- Test clean-machine startup, image/keyboard behavior, upgrades, and paths
  containing spaces, without Xcode/Swift or Cargo installed.

The preview CI smoke validates the packed helper's JSON and PNG protocol,
not Ghostty rendering quality or signed application installability.
See `experiments/SWIFT_PERFORMANCE.md` for the Eco performance findings.
