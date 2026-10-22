# Stage 9 — Optimize the existing Swift WebKit helper first

The paired macOS GitHub Actions benchmark in experimental Rust PR #11 found
**compiled Swift faster than the current Rust adapter** on the same fixture:
snapshot median about 15–20 ms (Swift) versus 76–78 ms (Rust).
Those are helper-level timings, **not** terminal FPS or all-process CPU.

More importantly, the **hybrid viewer currently starts `swift -swift-version 5
experiments/webkit_viewport.swift` when no helper override is set**. macOS
can therefore show a long-lived `swift-frontend` interpreter process. The
comparison above used `swiftc -swift-version 5 -O` and executed a native
binary instead.

Before changing any raster caches or scroll scheduling, isolate that variable.

## Run in Ghostty on macOS

From the checkout of `experiment/swift-compiled-helper`:

```bash
# Baseline (unmodified interpreter helper)
cargo run --release --example webkit_viewport_poc -- --hybrid-image README.md

# Compile optimized Swift once and launch with the existing helper override.
# Subsequent runs reuse the cached binary until the Swift source or toolchain changes.
bash experiments/run_compiled_swift_hybrid.sh --hybrid-image README.md

# For lower speculative WebKit activity, test the existing eco mode separately.
bash experiments/run_compiled_swift_hybrid.sh --hybrid-image-eco README.md
```

The script only sets `DOCTTY_WEBKIT_HELPER_BIN` for the launched experiment.
It does not modify system configuration, the Swift source, cache thresholds,
the Kitty image upload path, or the project's default preview behavior.

To inspect the cached binary without opening the terminal viewer:

```bash
bash experiments/run_compiled_swift_hybrid.sh --build-only
```

## Measure apples-to-apples

Compare the interpreter baseline and compiled Swift **in separate Ghostty
sessions**, using identical terminal dimensions, the same Markdown file and
the same interaction sequence. Activity Monitor > View > All Processes,
or the macOS `ps` utility, should help separate the processes.

| Step | Action | Record |
|---|---|---|
| 1 | Open README.md, idle for 60 seconds | Parent doctty CPU/RSS, Swift helper CPU/RSS, WebKit Web Content/GPU CPU/RSS |
| 2 | Hold j/k for ~20 seconds | Peak and sustained CPU, visible stutters |
| 3 | Scroll rapidly with the wheel, TOP→BOTTOM and back | Peak CPU, image lag, text fallback |
| 4 | Resize the terminal a few times | CPU spikes, image alignment |
| 5 | Leave it open for 10+ minutes | Memory growth and idle CPU |

Check `swift-frontend` separately:
```bash
pgrep -fl 'swift-frontend|doctty-webkit-swift'
```

Expected: the compiled-mode helper process uses a name like
`doctty-webkit-swift-arm64`; it does **not** run the Swift interpreter for
the active WebKit helper. `swift-frontend` may still run briefly while
`swiftc` compiles an updated helper; that compile-time cost is separate from
steady-state scrolling.

A meaningful win is **lower sustained total CPU across doctty + the helper +
WebKit child processes**, with unchanged visual quality and no increase in
scroll hitches or memory. A lower helper CPU alone is insufficient.

## CI scope and rollback

The new PR includes a macOS build and smoke test for a precompiled Swift helper.
This confirms JSON readiness, outline, nonzero-scroll snapshot and PNG
dimensions; it does not simulate Ghostty's graphical terminal protocol or
establish a CPU improvement.

Rollback: simply run the original `cargo run` baseline again, or unset
`DOCTTY_WEBKIT_HELPER_BIN` in your shell. PR #10, PR #11 and `main` are
unchanged by this isolated experiment.

Next, if the compiled version helps but CPU is still high, instrument PNG
`captureMs`, `encodeMs`, tile-cache hit rate and bytes transferred per
scroll. Optimize expensive operations only after identifying the dominant
cost; prioritize reducing WebKit snapshots and Kitty uploads without damaging
the existing hybrid preview.
