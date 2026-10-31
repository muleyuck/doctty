#!/usr/bin/env python3
"""Read-only release preflight for the unpublished Apple Silicon distribution.

Checks source/config contracts only; never creates tags, releases or tap commits.
The CI package job separately builds and verifies signed binaries and checksums.
"""
from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def read(relative: str) -> str:
    return (ROOT / relative).read_text(encoding="utf-8")


def verify(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"Release preflight FAILED: {message}")


def main() -> None:
    cargo = read("Cargo.toml")
    package = re.search(r"(?ms)^\[package\]\s*(.*?)(?=^\[|\Z)", cargo)
    verify(package is not None, "Cargo package metadata missing")
    version_match = re.search(r'(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$', package.group(1))
    verify(version_match is not None, "Cargo package version is not a stable SemVer triplet")
    version = version_match.group(1)

    manifest = json.loads(read(".github/.release-please-manifest.json"))
    config = json.loads(read(".github/release-please-config.json"))
    verify(manifest.get(".") == version, "Release Please manifest version differs from Cargo.toml")
    verify(config.get("release-type") == "rust", "Release Please must use the Rust releaser")
    verify(config.get("include-component-in-tag") is False, "Expected v<version> tags without component")
    verify(config.get("skip-github-release") is False, "Release Please config differs from the publish playbook")
    verify("." in config.get("packages", {}), "Release Please root package not configured")

    workflow = read(".github/workflows/release-please.yml")
    verify("scripts/package_macos_renderer.sh" in workflow,
           "Release workflow does not build the renderer archive")
    verify("scripts/package_macos_release.sh" in workflow,
           "Release workflow does not package the signed ZIP")
    verify("gh release upload" in workflow and "homebrew/publish@v1" in workflow,
           "Missing asset upload or Cask publication step")
    verify("Casks/doctty.rb" in workflow, "Homebrew Cask tap path mismatch")

    bundle_script = read("scripts/package_macos_renderer.sh")
    release_script = read("scripts/package_macos_release.sh")
    cask = read("packaging/doctty.rb.in")
    bins = ("doctty", "doctty-renderer", "doctty-webkit-swift")
    for name in bins:
        verify(f'"$BUNDLE/{name}"' in bundle_script,
               f"renderer archive is missing its {name} binary")
        verify(name in release_script, f"Release ZIP is missing {name}")
    verify('NAME="doctty-macos-$ARCH"' in bundle_script,
           "Unexpected renderer archive directory name")
    verify('doctty-$VERSION-macos-arm64.zip' in release_script,
           "Unexpected versioned release ZIP filename")
    verify('binary "doctty-macos-arm64/doctty-cli", target: "doctty"' in cask,
           "Homebrew entry point must preserve adjacent helper paths")
    verify("muleyuck/doctty/releases/download/v#{version}/doctty-#{version}-macos-arm64.zip" in cask,
           "Cask release URL disagrees with produced asset name")
    verify('version "@VERSION@"' in cask and 'sha256 "@SHA256@"' in cask,
           "Homebrew publish placeholders changed")
    verify("muleyuck/homebrew-tap" in read(".github/workflows/release-please.yml")
           or "Casks/doctty.rb" in workflow,
           "Homebrew publish configuration missing")

    for path in ("packaging/doctty.rb.in", "scripts/package_macos_release.sh", "README.md"):
        contents = read(path)
        verify("Gatekeeper" in contents, f"{path} does not explain Gatekeeper limitations")
    for path in ("packaging/doctty.rb.in", "scripts/package_macos_release.sh"):
        contents = read(path)
        verify("NOT Apple-notarized" in contents or "NOT notarized" in contents,
               f"{path} must state the lack of Apple notarization")
        verify("spctl --master-disable" not in contents,
               f"{path} must not disable Gatekeeper system-wide")
    # Like turnray, the cask clears quarantine on its own bundle only; a
    # manually downloaded ZIP is still left to Gatekeeper.
    verify('"com.apple.quarantine", "{{staged_path}}/doctty-macos-arm64"' in cask,
           "Cask must clear quarantine on its own staged bundle only")
    verify("xattr -dr" not in release_script,
           "Release ZIP instructions must not tell users to clear quarantine")

    print(f"Release preflight OK: Cargo + manifest v{version}")
    print("Artifact contracts: arm64 renderer bundle, versioned ZIP, sha256, Cask")
    print("This is a STATIC check only: no binaries, GitHub secrets, public")
    print("Release, real Homebrew install, or clean-Mac Gatekeeper were tested.")


if __name__ == "__main__":
    main()
