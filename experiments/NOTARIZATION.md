# Apple Silicon Developer ID signing and notarization (preparation)

**Status: prepared, not yet notarized.** This remains a draft integration
workflow. It does not publish a GitHub Release or run on pull requests.

Apple notarization works for signed CLI executables distributed outside the
Mac App Store. The initial supported architecture is **macOS arm64** only.

## Prerequisites (account owner action)

1. Join the [Apple Developer Program](https://developer.apple.com/programs/)
   (generally USD $99/year, locally priced where available). Enrolling and
   paying are user/account-owner actions. Confirm the team has **Developer ID**
   certificate access.
2. Obtain a **Developer ID Application** signing certificate and its matching
   private key from the Apple Developer team, then export a password-protected
   `.p12`. It must not be an Apple Development or Mac App Distribution
   certificate. Keep the original key securely backed up.
3. In App Store Connect, obtain an API key authorized for notarization, its
   key ID and issuer ID. Download and securely retain its `.p8` file.
   Whether the account can create/access the appropriate key depends on
   Apple team roles.
4. Store the six values below in **GitHub repository Actions secrets**.
   Never commit key files or paste credentials into issues, PRs or chats:

   | GitHub Actions secret | Required value |
   | --- | --- |
   | `MACOS_CERTIFICATE_P12_BASE64` | Base64-encoded Developer ID Application `.p12` |
   | `MACOS_CERTIFICATE_PASSWORD` | Password for that `.p12` |
   | `MACOS_DEVELOPER_IDENTITY` | Full signing identity, e.g. `Developer ID Application: Publisher (TEAMID)` |
   | `MACOS_NOTARY_API_KEY_BASE64` | Base64-encoded Notary/App Store Connect `.p8` API key |
   | `MACOS_NOTARY_KEY_ID` | API key ID |
   | `MACOS_NOTARY_ISSUER_ID` | API issuer UUID |

   On macOS, encode files for secrets using
   `base64 < certificate.p12 | tr -d '\n'` and
   `base64 < AuthKey_XXXX.p8 | tr -d '\n'`.
   Store the passwords and API keys only in GitHub secrets or a suitable
   secret manager. If any secret has ever been exposed, revoke and replace it.

## Workflow

Once the reviewed workflow exists on the default `main` branch and the
account owner has configured secrets, open GitHub **Actions → macOS notarized
Apple Silicon preview (manual) → Run workflow**. Select `main` and enter
`NOTARIZE` as confirmation.

The workflow performs the following steps, with no publishing permissions:

- Build optimized Rust `doctty`, the Rust Eco viewport companion, and
  optimized Swift/WebKit helper in the usual CI environment.
- Only **after the build**, temporarily import Developer ID credentials into
  an ephemeral macOS keychain, with the API key in a temporary permission-600
  file. Certificates and keys are not included in Actions artifacts.
- Sign all **three** Mach-O executables with hardened runtime and secure
  timestamp, verify code signatures, and replace the bundle's checksums.
- Create an arm64 `.dmg`, submit it to Apple's service using
  `xcrun notarytool submit --wait`, and require an explicit `Accepted` result.
- Staple and validate the notarization ticket on the DMG, then generate
  `.sha256` and upload both files as a **14-day review-only Actions artifact**.
  An invalid or rejected submission fails and uploads nothing.

The supported output is a notarized DMG, not a raw tarball. ZIP archives and
individual command-line executables cannot directly carry stapled tickets
in the same way as a DMG. Do not repackage signed executables into an
unsigned/unstapled tarball and label it as notarized.

## Release gates remaining

- Confirm accepted notarization using **the actual Apple Developer team**;
  without the certificate/API credentials no Apple submission has occurred.
- Test the downloaded stapled DMG on a clean Apple Silicon Mac using Gatekeeper
  in default mode, including paths with spaces, Ghostty/Kitty graphics, links,
  Mermaid, themes and watch/reload.
- Decide minimum supported macOS version (CI currently uses macOS 15).
- Publish a versioned tagged release only after QA. The separate future
  GitHub Releases/Homebrew automation should consume **accepted, stapled** assets.
- Keep Intel Mac support deferred unless user demand justifies it.

Apple documentation: [Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution),
[Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow),
and [Developer ID](https://developer.apple.com/developer-id/).
