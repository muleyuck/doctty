# Rust WebKit adapter — isolated experiment

**Status:** Stage 3 basic JSON-command loop experiment, **not** a drop-in replacement for the Swift helper.

This package intentionally contains the Objective-C FFI boundary in its own Rust
crate. The root doctty crate prohibits `unsafe`; that remains unchanged.
The known-good `--hybrid-image` mode and its Swift implementation remain intact.

## Scope of first milestone

- Build a native macOS Rust executable with `objc2-web-kit`.
- Initialize `NSApplication` / `WKWebView` on the main thread.
- Load the same local HTML path with an explicit resource access root.
- Evaluate the Swift-compatible document readiness probe with an Objective-C completion block on the main thread.
- Return the actual document height once fonts/images have loaded (or after bounded retries).
- Exercise the shared JSON protocol with unit tests.
- Keep stdin reading on a background thread while AppKit remains on the main thread.
- After readiness, handle `dimensions`, `metrics`, and `quit` over newline-delimited JSON; reject unsupported commands explicitly.

**Not yet implemented:** DOM outline/selection, screenshots, PNG encoding,
incremental resize, snapshot metrics, full protocol integration.
The adapter must not be configured as `DOCTTY_WEBKIT_HELPER_BIN` yet.

## macOS build and smoke test

```bash
cargo test --manifest-path experiments/rust-webkit-adapter/Cargo.toml
cargo build --release --manifest-path experiments/rust-webkit-adapter/Cargo.toml
experiments/rust-webkit-adapter/target/release/doctty-rust-webkit-adapter \
  /absolute/path/to/page.html /absolute/path/to/resource-dir
```

## Safety of the DOM bridge

The asynchronous WKWebView callback returns a JavaScript-created JSON string. We only
reinterpret its Objective-C result as `NSString` for this known controlled script.
The callback uses an owned `RcBlock`, and the run loop stays on the main thread.
An explicit timeout prevents an indefinitely blocked response. `ready.height`
now comes from document layout rather than a placeholder, but real macOS
navigation validation is still needed.

## Gate before moving on

Do not replace the Swift backend until this native bootstrap and later
snapshot/outline protocol stages pass CI **and** actual Ghostty tests.
Swift/Rust CPU measurements require matching document and scrolling actions.
