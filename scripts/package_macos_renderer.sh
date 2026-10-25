#!/usr/bin/env bash
# Assemble the precompiled macOS renderer bundle used by CI and release packaging.
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
NAME="doctty-macos-$ARCH"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
STAGING="$(mktemp -d "$OUT_DIR/.doctty-staging.XXXXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
BUNDLE="$STAGING/$NAME"
mkdir -p "$BUNDLE"

# Reuse the existing optimized helper build and source/toolchain cache.
SWIFT_BIN="$(bash "$ROOT/scripts/build_macos_webkit_helper.sh" --build-only)"
# Distinguish the complete CI bundle from a source-only Cargo install.
# The feature is enabled only when both native companions are shipped.
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" --bin doctty --bin doctty-renderer
install -m 0755 "$ROOT/target/release/doctty" "$BUNDLE/doctty"
install -m 0755 "$ROOT/target/release/doctty-renderer" "$BUNDLE/doctty-renderer"
install -m 0755 "$SWIFT_BIN" "$BUNDLE/doctty-webkit-swift"

cat > "$BUNDLE/README.txt" <<'DOC'
doctty — precompiled Apple Silicon renderer bundle
============================================

This archive contains:
  doctty               CLI; standard renderer is the macOS interactive default
  doctty-renderer           compiled Rust viewport companion
  doctty-webkit-swift  compiled, optimized native Swift/WebKit helper
  SHA256SUMS           checksums of the included files

Keep all executables in the same directory. Open Markdown in Ghostty or Kitty:
  ./doctty /absolute/path/to/README.md

Supported CLI options include --theme light/dark/system, --watch,
--no-mermaid, and the normal keyboard navigation (j/k, g/G,
PageUp/PageDown, r, o, q). Standard piped output remains unchanged.

Neither a Rust toolchain nor the Swift compiler is needed to run this
prebuilt bundle. macOS supplies WebKit at runtime.

This CI preview has NOT been notarized by Apple; do not disable
Gatekeeper or blindly bypass quarantine for unverified downloads.
DOC

(
  cd "$BUNDLE"
  shasum -a 256 doctty doctty-renderer doctty-webkit-swift README.txt > SHA256SUMS
)
ARCHIVE="$OUT_DIR/$NAME.tar.gz"
tar -C "$STAGING" -czf "$ARCHIVE" "$NAME"
(
  cd "$OUT_DIR"
  shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
)
printf '%s\n' "$ARCHIVE"
