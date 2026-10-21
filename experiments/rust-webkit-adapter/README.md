# Rust WebKit adapter — isolated experiment

**Status:** Stage 8 benchmark harness, Stage 7 DOM-selection parity and required PNG smoke. **Not** a production drop-in.

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
- After readiness, handle `dimensions`, `metrics`, `outline`, `resize`, `select`, and `quit` over newline-delimited JSON.
- Resize the existing WKWebView on the main thread, relayout it and update the document height without restarting WebKit; return Swift-compatible `resized` JSON.
- Guard against a WebKit startup race: `about:blank` can report a plausible height before the requested `file:` document finishes loading.
- An opt-in 5-argument invocation also accepts `snapshot` requests, scrolls the DOM, captures twice the viewport height, encodes a PNG, and returns Swift-compatible timings. CI compiles this code, but matching native screenshot contents and scroll behavior still requires runtime validation.

**Not yet implemented:** Production integration and full Ghostty interaction/visual parity.
The `select` command implements Swift's intentional bounded DOM text-selection bridge, not pointer-driven terminal mouse selection.
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
count and snapshot metrics, plus bounded text selection. Readiness must
wait for the actual file URL, not the initial `about:blank` document.
A successful compilation or unit test alone cannot prove that WebKit completed
a snapshot. The smoke has passed on hosted `macos-15` and is now a **required**
CI check: green adapter CI means this fixture's load, outline, responsive
resize, selection, scroll, and PNG validations passed. It is **not** proof of
Ghostty visual parity, realistic scrolling performance, or CPU/memory usage.

## Stage 8: reproducible Swift/Rust helper comparison (macOS only)

Before changing the terminal viewer, compare the two **compiled native** WebKit
helpers on the same local HTML and the same scroll requests. Running the old
Swift helper through the `swift` interpreter would include compilation and
skew any `swift-frontend` CPU comparison; build it once first:

```sh
cargo build --release --manifest-path experiments/rust-webkit-adapter/Cargo.toml
SWIFT_HELPER="$(mktemp -d)/doctty-swift-webkit"
swiftc -swift-version 5 -O experiments/webkit_viewport.swift -o "$SWIFT_HELPER"

python3 experiments/rust-webkit-adapter/tests/compare_webkit_helpers.py \
  --swift-bin "$SWIFT_HELPER" \
  --rust-bin experiments/rust-webkit-adapter/target/release/doctty-rust-webkit-adapter \
  --samples 12 --repeats 2 --output /tmp/doctty-webkit-comparison.json
```

The default fixture is deterministic and stored in a temporary directory.
For a more representative rendered Markdown HTML, supply both
`--html /absolute/path/to/rendered.html` and
`--read-root /absolute/path/to/assets-root`. The HTML must be inside the
resource root, and **both** backends will use that exact file.

The harness alternates Swift/Rust, then Rust/Swift to reduce warmup/thermal
order bias, sends two warmup captures, and records median/p95/max snapshot,
capture, encode and write times, document heights, PNG dimensions and byte
counts. It reports per-helper direct child process CPU time and session wall
time as additional rough diagnostics. It is not run as a performance gate in
GitHub Actions: hosted runner load can vary.

**Limitations:** Helper CPU seconds exclude WebKit content/GPU processes and
Ghostty itself; protocol snapshot latency is not terminal scroll FPS or
end-to-end UI latency. Inspect Activity Monitor on the same Mac during
matched `README.md` scroll sequences (idle, repeated j/k, fast wheel,
full top-to-bottom, resize). Record the WebKit and Ghostty processes alongside
the helper, including `swift-frontend` only for interpreter-based Swift runs.
No Swift-to-Rust default switch should occur before a visually matched
Ghostty test and stable CPU/RSS readings.

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
