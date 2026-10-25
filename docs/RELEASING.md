# macOS first release runbook (Apple Silicon)

**Status:** release candidate validated; **no public doctty release has been published** as of 2026-10-25. Follow this runbook from `main` only after reviewing every gate below. Publishing is intentionally **manual-only**. No Apple Developer ID or notarization is available.

## 1. Decide whether the release is ready

- [ ] Review the desired **first version** and release notes. `Cargo.toml` and the Release Please manifest currently start at `0.1.1`, but this repository has no prior `doctty` tag or published release. **Do not assume the first generated release will be v0.1.1.** Release Please calculates the next version from conventional commits and its manifest; inspect its generated release PR before merging. See [Release Please bootstrapping](https://github.com/googleapis/release-please/blob/main/docs/manifest-releaser.md#bootstrapping).
- [ ] Confirm intended macOS support: **Apple Silicon / macOS 15+**. Intel and earlier macOS are not currently validated.
- [x] GitHub Actions macOS arm64 CI generated an ad-hoc-signed ZIP, checked Mach-O architectures and deployment minimum, validated code signatures and SHA256, and exercised compiled Swift/WebKit protocol plus a simulated Homebrew Cask symlink.
- [x] The user downloaded the CI ZIP on a Mac; after **two separate Gatekeeper approvals**, the compiled Eco viewer ran normally. The intermediate `WebKit helper closed its output` error was consistent with an independently blocked native helper; the specific binary was not proven.
- [ ] Verify the **downloaded artifact SHA256 on the receiving Mac** and repeat an independent clean-machine check for macOS 15, Ghostty/Kitty, relative images, diagrams, themes, `--watch`, spaces in paths, and memory/CPU.
- [ ] Verify a real Homebrew Cask install/uninstall and, on a subsequent version, upgrade. Simulated symlink tests **do not** prove this.
- [ ] Review repository Actions permissions (Release Please requires write access for contents and PRs) and **GitHub App secret names** `TAP_APP_CLIENT_ID` / `TAP_APP_PRIVATE_KEY`. The GitHub App must have Contents: write on `muleyuck/homebrew-tap`. Do not expose private keys in tickets, logs, or commits. If secrets are absent the current workflow uploads a release ZIP but skips Homebrew publishing.
- [ ] Agree to the user-visible trust tradeoff: ad-hoc-signed, **not Apple-notarized**. Gatekeeper approvals are expected on normal internet downloads. Do not represent it as a seamless, verified Apple-signed application.

## 2. Read-only local / CI preflight

From the repository root:

```bash
git switch main
git pull --ff-only
python3 scripts/check_release_readiness.py
bash -n scripts/package_macos_eco.sh
bash -n scripts/package_macos_release.sh
git fetch origin --tags
git tag --list 'v*'
gh release list --repo muleyuck/doctty
```

The Python preflight checks version/configuration alignment, manual-only release triggers, bundle/ZIP/Cask paths, and explicit Gatekeeper disclosure. It **never** publishes anything and **cannot** inspect secret values or replace a clean-device test.

For a fresh CI-built release candidate without a tag, run the separate **macOS Eco bundle validation** workflow on `main` with `workflow_dispatch`. Download the resulting `doctty-ad-hoc-signed-arm64-review` artifact and verify the inner ZIP against its matching `.zip.sha256` file before testing.

For a local build on macOS arm64, `bash scripts/package_macos_eco.sh` creates `target/dist/doctty-macos-arm64.tar.gz`; this **is not** a Gatekeeper download test. The local signing script can build a release-format ZIP from that archive, but no GitHub Release is necessary for the preflight.

## 3. Prepare the release PR — still no public publication

The repository's `.github/workflows/release-please.yml` runs **only** on an explicitly requested `workflow_dispatch` on the `main` branch. No merge, push or successful CI job automatically triggers it.

1. In GitHub **Actions → Release Please → Run workflow**, choose `main`.
2. **Important:** only perform this while there is no merged, unprocessed Release Please release PR; if such a PR has already been merged, the workflow may **create a tag and public GitHub Release**, not merely prepare a PR.
3. Review the generated release PR: proposed version, `Cargo.toml`, `Cargo.lock`, manifest, changelog and release notes. Reject unexpected version changes, credentials, or files.
4. Treat **merging the generated release PR** as the explicit approval to make a release on the *next manual workflow run*. Check the current release workflow before proceeding.

Do **not** start this step casually: a later manual run can publish the GitHub Release.

## 4. Publish manually — only after explicit authorization

Only once every gate is checked and the Release Please release PR is reviewed and merged:

1. Explicitly run **Actions → Release Please → Run workflow** on `main` again.
2. Verify the new `v<version>` tag and GitHub Release correspond to the reviewed Cargo package version.
3. Watch the macOS release packaging job until the `doctty-<version>-macos-arm64.zip` and `.zip.sha256` assets are attached. **Release Please may make the GitHub Release visible before asset upload completes**. Do not announce or link the release until the job finishes successfully.
4. Confirm SHA256 and downloaded-archive contents, three native executables (`doctty`, `doctty-eco`, `doctty-webkit-swift`), their signatures and correct launch path. Never move a helper out of the extracted bundle.
5. Confirm `muleyuck/homebrew-tap/Casks/doctty.rb` was updated with the actual version/SHA256; if it was skipped because secrets were not configured, **do not advertise Homebrew as available**.
6. Validate the public Homebrew path on a test Mac: `brew install --cask muleyuck/tap/doctty`; then `doctty /absolute/path/to/README.md`; after testing, `brew uninstall --cask muleyuck/tap/doctty`. Test `brew upgrade --cask` only when a later release exists.
7. Confirm the download URL, instructions, version and compatibility before announcing the release.

## 5. Gatekeeper and support notes

On a checksum-verified download from the official GitHub Release, macOS can block `doctty` **and separately** block `doctty-eco` or `doctty-webkit-swift`.

If this happens, open **System Settings → Privacy & Security**, review the blocked executable and use **Open Anyway** only after deciding to trust the verified build; retry `doctty`. More than one per-app approval may be required. A Swift helper EOF error does not by itself prove Gatekeeper is the cause. **Never** disable Gatekeeper globally or blindly remove quarantine.

Apple notarization and Intel distribution can be reassessed later. Their absence must remain explicit in README, Cask caveats and GitHub Release notes.

## 6. Recovery checklist

If the release-preparation workflow fails, do not create a tag manually as a workaround. Investigate the workflow error, correct the release PR or packaging on a normal review branch, and rerun CI. If a release is partially published without assets, halt promotion and announcement, inspect the Release/tag state and release job logs, and make an explicit repair or rollback decision before updating the Homebrew tap.
