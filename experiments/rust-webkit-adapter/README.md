# Rust WebKit adapter — isolated experiment

**Status:** Stage 1 bootstrap, **not** a drop-in replacement for the Swift helper.

This package intentionally contains the Objective-C FFI boundary in its own Rust
crate. The root doctty crate prohibits `unsafe`; that remains unchanged.
The known-good `--hybrid-image` mode and its Swift implementation remain intact.

## Scope of first milestone

- Build a native macOS Rust executable with `objc2-web-kit`.
- Initialize `NSApplication` / `WKWebView` on the main thread.
- Load the same local HTML path with an explicit resource access root.
- Wait for WebKit's initial navigation and print the simple `ready` response.
- Exercise the shared JSON protocol with unit tests.

**Not yet implemented:** JavaScript/DOM outline, screenshots, PNG encoding,
incremental resize, resource lifecycle parity, full protocol integration.
The adapter must not be configured as `DOCTTY_WEBKIT_HELPER_BIN` yet.

## macOS build and smoke test

```bash
cargo test --manifest-path experiments/rust-webkit-adapter/Cargo.toml
cargo build --release --manifest-path experiments/rust-webkit-adapter/Cargo.toml
experiments/rust-webkit-adapter/target/release/doctty-rust-webkit-adapter \
  /absolute/path/to/page.html /absolute/path/to/resource-dir
```

## Gate before moving on

Do not replace the Swift backend until this native bootstrap and later
snapshot/outline protocol stages pass CI **and** actual Ghostty tests.
Swift/Rust CPU measurements require matching document and scrolling actions.
