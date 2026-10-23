#!/usr/bin/env bash
# Sign the three prebuilt doctty executables with Developer ID, submit a DMG
# to Apple notary service and staple/validate its ticket. CI credentials only.
# This script NEVER generates or downloads certificates, and never submits
# unsigned binaries or a DMG without a release identity.
set -euo pipefail

usage() {
  cat <<'HELP'
Usage: bash experiments/sign_notarize_macos.sh PREBUILT_TAR_GZ OUT_DIR

Requires a prebuilt CI-produced doctty-hybrid-macos-arm64.tar.gz and:
  APPLE_DEVELOPER_IDENTITY    Developer ID Application certificate name
  APPLE_NOTARY_API_KEY_FILE   path to temporary App Store Connect .p8 key
  APPLE_NOTARY_KEY_ID        App Store Connect API key ID
  APPLE_NOTARY_ISSUER_ID     App Store Connect API issuer ID

The workflow imports the Developer ID certificate into an ephemeral
keychain before invoking this script. Output: stapled, notarized DMG
and its SHA256 checksum. Never commit certificates or keys.
HELP
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi
if [[ "$#" != 2 ]]; then
  usage >&2
  exit 2
fi

: "${APPLE_DEVELOPER_IDENTITY:?Developer ID Application identity is required}"
: "${APPLE_NOTARY_API_KEY_FILE:?Notary API key file is required}"
: "${APPLE_NOTARY_KEY_ID:?Notary API key ID is required}"
: "${APPLE_NOTARY_ISSUER_ID:?Notary API issuer ID is required}"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "Notarization preview requires an Apple Silicon macOS CI runner" >&2
  exit 2
fi
for cmd in codesign lipo tar shasum hdiutil xcrun python3 mktemp; do
  command -v "$cmd" >/dev/null || { echo "Missing tool: $cmd" >&2; exit 2; }
done
if [[ ! -r "$APPLE_NOTARY_API_KEY_FILE" ]]; then
  echo "Notary API key file is not readable" >&2
  exit 2
fi

ARCHIVE="$1"
OUT_DIR="$2"
if [[ ! -f "$ARCHIVE" ]]; then
  echo "Prebuilt archive not found: $ARCHIVE" >&2
  exit 2
fi
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/doctty-notary.XXXXXXXX")"
trap 'rm -rf "$STAGING"' EXIT

# The tar archive must be from the trusted CI packaging job.
tar -xzf "$ARCHIVE" -C "$STAGING"
BUNDLE="$STAGING/doctty-hybrid-macos-arm64"
if [[ ! -d "$BUNDLE" ]]; then
  echo "Unexpected archive layout: missing doctty-hybrid-macos-arm64" >&2
  exit 2
fi

for executable in doctty webkit_viewport_poc doctty-webkit-swift; do
  path="$BUNDLE/$executable"
  if [[ ! -x "$path" ]]; then
    echo "Missing executable in bundle: $executable" >&2
    exit 2
  fi
  lipo "$path" -verify_arch arm64 >&2
  codesign --force --sign "$APPLE_DEVELOPER_IDENTITY" --options runtime --timestamp "$path" >&2
  codesign --verify --verbose=2 --strict "$path" >&2
done

# The unsigned preview archive's README explicitly describes the old
# unnotarized tarball. Replace it for this review-only signed DMG to avoid
# shipping conflicting security instructions.
cat > "$BUNDLE/README.txt" <<'SIGNED_README'
doctty — Developer ID-signed Apple Silicon notarization preview
===============================================================

This disk image contains three hardened-runtime Developer ID-signed arm64
executables: doctty, webkit_viewport_poc and doctty-webkit-swift.
This DMG is distributed by the CI workflow only after Apple's notary service
returns Accepted and the notarization ticket is stapled to the DMG.

In Ghostty or Kitty, copy the entire set of files into the same local
directory before running ./doctty /absolute/path/to/README.md.
The three executables must remain together. A Swift/Rust compiler is
not required to use the compiled Eco renderer.

This is a testing preview, not a signed public GitHub Release.
Do not circumvent system Gatekeeper warnings. Report launch failures
before publishing a stable release.
SIGNED_README

# Signing changes the executables: replace pre-signing checksums.
(
  cd "$BUNDLE"
  shasum -a 256 doctty doctty-hybrid webkit_viewport_poc doctty-webkit-swift README.txt > SHA256SUMS
  shasum -a 256 -c SHA256SUMS >&2
)

# DMG supports notarization ticket stapling. Tarballs/standalone binaries
# cannot be stapled, so do not publish a re-tarred un-stapled payload.
DMG="$OUT_DIR/doctty-macos-arm64-notarized.dmg"
hdiutil create -ov -format UDZO -volname "doctty arm64" -srcfolder "$BUNDLE" "$DMG" >&2
REPORT="$STAGING/notary-response.json"
xcrun notarytool submit "$DMG" \
  --key "$APPLE_NOTARY_API_KEY_FILE" \
  --key-id "$APPLE_NOTARY_KEY_ID" \
  --issuer "$APPLE_NOTARY_ISSUER_ID" \
  --wait --output-format json > "$REPORT"

# Submission returning without an error is not sufficient: insist on
# Apple's explicit Accepted verdict before claiming a notarized release.
python3 - "$REPORT" <<'PY'
import json
import sys
from pathlib import Path

result = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
status = result.get("status")
print(f"Apple notarization status: {status}", file=sys.stderr)
if status != "Accepted":
    print("Apple did not accept the submission; do not distribute the DMG.", file=sys.stderr)
    sys.exit(1)
PY

xcrun stapler staple "$DMG" >&2
xcrun stapler validate "$DMG" >&2
(
  cd "$OUT_DIR"
  shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256"
  shasum -a 256 -c "$(basename "$DMG").sha256" >&2
)
printf "%s\n" "$DMG"
