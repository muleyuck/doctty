# Stage 9 — Optimize the existing Swift WebKit helper first

The paired macOS GitHub Actions benchmark in experimental Rust PR #11 found
**compiled Swift faster than the current Rust adapter** on the same fixture:
snapshot median about 15–20 ms (Swift) versus 76–78 ms (Rust)
**before** the Rust adapter's callback-poll improvement. Subsequent Rust
timings approached roughly 22 ms median. These are helper-level timings,
**not** terminal FPS or all-process CPU; the Rust-vs-Swift choice should
not be decided from that earlier benchmark alone.

More importantly, the **hybrid viewer currently starts `swift -swift-version 5
experiments/webkit_viewport.swift` when no helper override is set**. macOS
can therefore show a long-lived `swift-frontend` interpreter process. The
comparison above used `swiftc -swift-version 5 -O` and executed a native
binary instead.

Before changing any raster caches or scroll scheduling, isolate that variable.

## Stage 10: locally observed Eco result and experimental default

In a local Ghostty test reported by the user on 2026-10-22,
the **compiled Swift normal hybrid image mode** reached about **60–90%**
CPU during sustained fast scrolling, whereas **compiled Swift Eco mode**
peaked around **3%** for the observed helper process. No additional
perceptible stutter was reported; temporary lower fidelity while moving is
expected. This is a strong signal to reduce speculative WebKit work, but
is **not** a controlled measurement of total process-tree CPU or a
guarantee that other documents and terminals behave similarly.

On the isolated `experiment/eco-default` branch, `--hybrid-image` now
uses the existing Eco snapshot scheduler (capture only after scrolling
settles and the viewport is missing). The explicit
`--hybrid-image-eco` form remains an alias. The earlier speculative
prefetch behavior remains available as `--hybrid-image-prefetch`
for comparisons and rollback. Other modes and the known-good branch are
unchanged. This does not change the compiled Swift helper or PNG quality
after scrolling settles.

## Run in Ghostty on macOS

From the checkout of `experiment/swift-compiled-helper`:

```bash
# New default: compiled Swift with low-CPU Eco scheduling.
bash experiments/run_compiled_swift_hybrid.sh --hybrid-image README.md

# Historical explicit Eco form (identical to the default).
bash experiments/run_compiled_swift_hybrid.sh --hybrid-image-eco README.md

# Control: compiled Swift with the previous speculative prefetch behavior.
bash experiments/run_compiled_swift_hybrid.sh --hybrid-image-prefetch README.md

# Optional interpreter comparison: defaults to Eco on this branch as well.
cargo run --release --example webkit_viewport_poc -- --hybrid-image README.md
```

The script only sets `DOCTTY_WEBKIT_HELPER_BIN` for the launched experiment.
It does not modify system configuration, the Swift source, cache thresholds,
the Kitty image upload path, or the project's default preview behavior.

To inspect the cached binary without opening the terminal viewer:

```bash
bash experiments/run_compiled_swift_hybrid.sh --build-only
```

## Measure apples-to-apples

Compare Eco and the previous speculative prefetch mode **in separate
Ghostty sessions** using the same compiled Swift helper, terminal dimensions,
Markdown file and input sequence. To isolate compilation/interpreter cost,
compare interpreter and compiled helpers with the **same** image mode. Activity Monitor > View > All Processes,
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

Rollback the Eco scheduling experiment using
`bash experiments/run_compiled_swift_hybrid.sh --hybrid-image-prefetch README.md`.
To return to Swift interpreter execution, run `cargo run --release --example
webkit_viewport_poc -- --hybrid-image-prefetch README.md` without
`DOCTTY_WEBKIT_HELPER_BIN`. No merge to `main` is implied by this change.

If high CPU returns or Eco causes missed content, instrument PNG
`captureMs`, `encodeMs`, tile-cache hit rate and bytes transferred per
scroll. Optimize expensive operations only after identifying the dominant
cost; prioritize reducing WebKit snapshots and Kitty uploads without damaging
the existing hybrid preview.
