#!/usr/bin/env bash
# Build a self-contained experimental Ghostty/Kitty hybrid preview archive.
# Release signing/notarization and production integration are separate.
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS is required to build the Swift/WebKit bundle" >&2
  exit 2
fi
for tool in cargo swiftc install tar shasum mktemp; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 2; }
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${DOCTTY_HYBRID_DIST_DIR:-$ROOT/target/dist}"
ARCH="$(uname -m)"
NAME="doctty-hybrid-macos-$ARCH"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
STAGING="$(mktemp -d "$OUT_DIR/.doctty-staging.XXXXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
BUNDLE="$STAGING/$NAME"
mkdir -p "$BUNDLE"

# Reuse the existing optimized helper build and source/toolchain cache.
SWIFT_BIN="$(bash "$ROOT/experiments/run_compiled_swift_hybrid.sh" --build-only)"
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" --bin doctty --example webkit_viewport_poc
install -m 0755 "$ROOT/target/release/doctty" "$BUNDLE/doctty"
install -m 0755 "$ROOT/target/release/examples/webkit_viewport_poc" "$BUNDLE/webkit_viewport_poc"
install -m 0755 "$SWIFT_BIN" "$BUNDLE/doctty-webkit-swift"

# The packaged launcher always chooses the adjacent, precompiled Swift helper.
# No swift/swiftc/Cargo invocation happens on the user's Mac.
cat > "$BUNDLE/doctty-hybrid" <<'LAUNCHER'
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ ! -x "$ROOT/doctty-webkit-swift" || ! -x "$ROOT/webkit_viewport_poc" ]]; then
  echo "Incomplete doctty-hybrid bundle: expected helper and viewer alongside launcher" >&2
  exit 2
fi
if [[ $# -eq 0 ]]; then
  echo "usage: ./doctty-hybrid FILE.md" >&2
  echo "  optional: ./doctty-hybrid --hybrid-image-prefetch FILE.md" >&2
  exit 2
fi
if [[ "$1" != --* ]]; then
  set -- --hybrid-image "$@"
fi
export DOCTTY_WEBKIT_HELPER_BIN="$ROOT/doctty-webkit-swift"
exec "$ROOT/webkit_viewport_poc" "$@"
LAUNCHER
chmod 0755 "$BUNDLE/doctty-hybrid"

cat > "$BUNDLE/README.txt" <<'DOC'
Experimental doctty hybrid preview — macOS
============================================

This archive bundles:
  doctty                normal Rust CLI, plus opt-in --experimental-eco
  doctty-hybrid         direct preview launcher (default Eco)
  webkit_viewport_poc   compiled Rust preview executable
  doctty-webkit-swift   optimized, compiled Swift/WebKit helper
  SHA256SUMS            file integrity checks

No Swift compiler, Cargo, Rust toolchain, or repository checkout is needed
to run the bundled preview. macOS WebKit is supplied by the operating system.

From an unpacked directory in Ghostty or Kitty:
  ./doctty /absolute/path/to/README.md
      normal viewer, existing behavior
  ./doctty --experimental-eco /absolute/path/to/README.md
      opt-in Eco renderer via bundled native Swift, no compilation
  ./doctty --experimental-eco --theme dark /absolute/path/to/README.md
      opt-in Eco with dark GitHub theme (light and system also supported)
  ./doctty-hybrid /absolute/path/to/README.md
      direct standalone Eco experiment
  ./doctty-hybrid --hybrid-image-prefetch /absolute/path/to/README.md
      previous eager prefetch comparison

The Eco bridge does not yet support --watch or the normal viewer's full
link/navigation behavior. Explicit --theme light/dark/system works.
Existing non-Eco CLI and
piped/headless operation are unchanged.

Keep the bundled executables together. Do not rename the bundle contents.
This is a non-notarized PREVIEW archive and is NOT a signed or
notarized public release. Source-build fallback paths in the example are
not intended to be used from this archive. Do not bypass Gatekeeper
protections to run untrusted downloads.

A future release must remove the experimental Eco flag, reach feature
parity with the production CLI, codesign/notarize the macOS distribution,
and verify installation/upgrades.
DOC

(
  cd "$BUNDLE"
  shasum -a 256 doctty doctty-hybrid webkit_viewport_poc doctty-webkit-swift README.txt > SHA256SUMS
)
ARCHIVE="$OUT_DIR/$NAME.tar.gz"
tar -C "$STAGING" -czf "$ARCHIVE" "$NAME"
(
  cd "$OUT_DIR"
  shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
)
printf '%s\n' "$ARCHIVE"
