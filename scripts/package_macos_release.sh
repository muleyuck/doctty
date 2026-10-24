#!/usr/bin/env bash
# Create an ad-hoc-signed Apple Silicon ZIP for the turnray-style release path.
# The three native executables remain adjacent; the Homebrew entry point is
# a wrapper that execs the real Rust CLI from that same directory.
set -euo pipefail

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: bash scripts/package_macos_release.sh PREBUILT_TAR_GZ VERSION OUT_DIR"
  exit 0
fi
if [[ "$#" != 3 ]]; then
  echo "Usage: bash scripts/package_macos_release.sh PREBUILT_TAR_GZ VERSION OUT_DIR" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "The release ZIP must be built on an Apple Silicon macOS runner" >&2
  exit 2
fi
for tool in codesign ditto lipo shasum mktemp tar xcrun awk; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 2; }
done

ARCHIVE="$1"
VERSION="$2"
OUT_DIR="$3"
if [[ ! -f "$ARCHIVE" ]]; then
  echo "Missing prebuilt archive: $ARCHIVE" >&2
  exit 2
fi
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?$ ]]; then
  echo "Expected a version like 0.1.1, got: $VERSION" >&2
  exit 2
fi
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/doctty-release.XXXXXXXX")"
trap 'rm -rf "$STAGING"' EXIT

tar -xzf "$ARCHIVE" -C "$STAGING"
BUNDLE="$STAGING/doctty-macos-arm64"
if [[ ! -d "$BUNDLE" ]]; then
  echo "Invalid preview archive layout: missing doctty-macos-arm64" >&2
  exit 2
fi

for executable in doctty doctty-eco doctty-webkit-swift; do
  file="$BUNDLE/$executable"
  test -x "$file" || { echo "Missing executable: $executable" >&2; exit 2; }
  lipo "$file" -verify_arch arm64 >&2
  # The Cask declares Sequoia (15.0) as the conservative minimum.
  # Fail closed if any bundled binary requires a newer macOS release.
  minos="$(xcrun vtool -arch arm64 -show-build "$file" | awk '$1 == "minos" { print $2; exit }')"
  if [[ ! "$minos" =~ ^[0-9]+(\.[0-9]+){1,2}$ ]]; then
    echo "Cannot determine macOS deployment minimum for $executable" >&2
    exit 2
  fi
  major="${minos%%.*}"
  minor="${minos#*.}"
  minor="${minor%%.*}"
  if (( major > 15 || (major == 15 && minor > 0) )); then
    echo "$executable requires macOS $minos, newer than Cask minimum 15.0" >&2
    exit 2
  fi
  # Same non-notarized, ad-hoc signing model as turnray.
  codesign --force --sign - "$file" >&2
  codesign --verify --strict "$file" >&2
done

# Homebrew Cask symlinks this script, not the main doctty executable.
# Resolving BASH_SOURCE[0] preserves the real archive location when a
# symlink under /opt/homebrew/bin is used.
cat > "$BUNDLE/doctty-cli" <<'LAUNCHER'
#!/bin/bash
set -euo pipefail
SOURCE="${BASH_SOURCE[0]}"
while [[ -L "$SOURCE" ]]; do
  ROOT="$(cd -P "$(dirname "$SOURCE")" && pwd)"
  SOURCE="$(readlink "$SOURCE")"
  [[ "$SOURCE" = /* ]] || SOURCE="$ROOT/$SOURCE"
done
ROOT="$(cd -P "$(dirname "$SOURCE")" && pwd)"
exec "$ROOT/doctty" "$@"
LAUNCHER
chmod 0755 "$BUNDLE/doctty-cli"

cat > "$BUNDLE/README.txt" <<'DOC'
doctty — Apple Silicon release preview (unsigned by a Developer ID)
====================================================================

This ZIP mirrors turnray's no-paid-Apple-account distribution method:
three compiled arm64 executables signed ad-hoc, NOT notarized.

  doctty               Rust CLI; Eco mode is the default
  doctty-eco  Compiled Rust WebKit Eco companion
  doctty-webkit-swift  Compiled native Swift/WebKit helper
  doctty-cli           Homebrew launcher; resolves the complete bundle

Run ./doctty /absolute/path/to/README.md in Ghostty or Kitty.
Do not move the native executables out of this directory individually.
Neither Swift nor Rust needs to be installed by the user.

Gatekeeper might block this non-notarized ZIP. Do not disable Gatekeeper
globally or clear quarantine on unverified software. Only use the release
asset from the project's official GitHub Releases page after checking the
published SHA256 checksum. This is not an Apple-notarized release.
DOC

(
  cd "$BUNDLE"
  shasum -a 256 doctty doctty-cli doctty-eco doctty-webkit-swift README.txt > SHA256SUMS
  shasum -a 256 -c SHA256SUMS >&2
)

ZIP="$OUT_DIR/doctty-$VERSION-macos-arm64.zip"
ditto -c -k --keepParent "$BUNDLE" "$ZIP"
(
  cd "$OUT_DIR"
  shasum -a 256 "$(basename "$ZIP")" > "$(basename "$ZIP").sha256"
  shasum -a 256 -c "$(basename "$ZIP").sha256" >&2
)
printf '%s\n' "$ZIP"
