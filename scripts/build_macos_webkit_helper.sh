#!/usr/bin/env bash
# Build the optimized Swift/WebKit helper used by the precompiled renderer bundle.
# Optional arguments still allow developers to inspect the viewport.
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This WebKit helper requires macOS." >&2
  exit 2
fi
command -v swiftc >/dev/null || { echo "swiftc not found" >&2; exit 2; }
command -v cargo >/dev/null || { echo "cargo not found" >&2; exit 2; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/native/macos/webkit_viewport.swift"
CACHE_DIR="${DOCTTY_SWIFT_HELPER_CACHE_DIR:-$ROOT/target/swift-webkit}"
mkdir -p "$CACHE_DIR"
BINARY="$CACHE_DIR/doctty-webkit-swift-$(uname -m)"
VERSION_FILE="$BINARY.swiftc-version"
SWIFTC_VERSION="$(swiftc --version)"
CACHED_VERSION=""
if [[ -f "$VERSION_FILE" ]]; then
  CACHED_VERSION="$(cat "$VERSION_FILE")"
fi

if [[ ! -x "$BINARY" || "$SOURCE" -nt "$BINARY" || "$SWIFTC_VERSION" != "$CACHED_VERSION" ]]; then
  echo "Building optimized Swift WebKit helper (one-time, cached)..." >&2
  TEMP_BINARY="$(mktemp "$CACHE_DIR/.doctty-webkit-swift.XXXXXXXX")"
  trap 'rm -f "$TEMP_BINARY"' EXIT
  swiftc -swift-version 5 -O "$SOURCE" -o "$TEMP_BINARY"
  chmod +x "$TEMP_BINARY"
  mv -f "$TEMP_BINARY" "$BINARY"
  printf '%s' "$SWIFTC_VERSION" > "$VERSION_FILE"
  trap - EXIT
fi

# Useful for CI smoke testing or manually inspecting the executable.
if [[ "${1:-}" == "--build-only" && "$#" -eq 1 ]]; then
  printf '%s\n' "$BINARY"
  exit 0
fi

# Preserve all existing terminal preview and scroll logic; only substitute
# the prebuilt Swift helper instead of running Swift in interpreter mode.
if [[ "$#" -eq 0 ]]; then
  set -- --hybrid-image "$ROOT/README.md"
fi
export DOCTTY_WEBKIT_HELPER_BIN="$BINARY"
exec cargo run --release --manifest-path "$ROOT/Cargo.toml" --bin doctty-renderer -- "$@"
