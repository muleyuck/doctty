# Rust WebKit adapter — isolated experiment

**Status:** Stage 6 native resize/reflow experiment and diagnostic PNG smoke, **not** a drop-in replacement for the Swift helper.

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
- After readiness, handle `dimensions`, `metrics`, `outline`, `resize`, and `quit` over newline-delimited JSON.
- Resize the existing WKWebView on the main thread, relayout it and update the document height without restarting WebKit; return Swift-compatible `resized` JSON.
- Guard against a WebKit startup race: `about:blank` can report a plausible height before the requested `file:` document finishes loading.
- An opt-in 5-argument invocation also accepts `snapshot` requests, scrolls the DOM, captures twice the viewport height, encodes a PNG, and returns Swift-compatible timings. CI compiles this code, but matching native screenshot contents and scroll behavior still requires runtime validation.

**Not yet implemented:** DOM text selection, full production integration.
The native reflow/resizing code is experimental and still requires macOS CI and visual comparison to Swift.
Snapshot and PNG code is experimental and must not be used until the macOS CI check and an interactive runtime smoke test pass.
The adapter must not be configured as `DOCTTY_WEBKIT_HELPER_BIN` yet.

## macOS build and smoke test

```bash
cargo test --manifest-path experiments/rust-webkit-adapter/Cargo.toml
cargo build --release --manifest-path experiments/rust-webkit-adapter/Cargo.toml
experiments/rust-webkit-adapter/target/release/doctty-rust-webkit-adapter \
  /absolute/path/to/page.html /absolute/path/to/resource-dir

# Experimental snapshot protocol (stdin JSON-lines):
printf '%s\n' '{"type":"snapshot","y":0}' '{"type":"metrics"}' '{"type":"quit"}' | \\
  experiments/rust-webkit-adapter/target/release/doctty-rust-webkit-adapter \\
  /absolute/path/to/page.html /absolute/path/to/resource-dir /tmp/doctty-rust.png 800 600
```

## Native snapshot smoke test

Run the same automated smoke test from an **interactive macOS desktop** after
building the release binary:

```sh
python3 experiments/rust-webkit-adapter/tests/smoke_snapshot.py \
  experiments/rust-webkit-adapter/target/release/doctty-rust-webkit-adapter
```

It loads an isolated responsive HTML fixture, requests the DOM outline,
resizes the **same** WKWebView to a narrower viewport, checks that the DOM
height reflows and that `dimensions` and outline stay consistent, then makes
a nonzero scroll capture and checks JSON, PNG header, pixel dimensions, byte
count and snapshot metrics. Readiness must
wait for the actual file URL, not the initial `about:blank` document. A successful compilation or unit test cannot prove that WebKit
completed a snapshot. GitHub-hosted macOS runners may lack an interactive display,
so their diagnostic smoke step is `continue-on-error`; its status must be
inspected separately. Failure there does **not** establish a renderer regression,
and a green workflow does **not** imply that the runtime smoke passed.

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
