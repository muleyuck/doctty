# doctty benchmark harness

Captures startup, reload and scroll baselines for the WebKit rich view (issue #94) by
driving real Ghostty windows, and compares two captures. macOS only. Python 3 standard
library only.

## One-time setup

1. **Ghostty >= 1.4** with AppleScript enabled. Fix the window size so every capture has the
   same geometry, in your Ghostty config:

   ```
   window-width = 120
   window-height = 40
   ```

   Restart Ghostty after changing it. `compare.py` refuses to compare captures whose
   geometry differs.
2. **AppleScript permission.** The first `make bench` asks macOS to let your terminal
   control Ghostty. Allow it in System Settings > Privacy & Security > Automation
   (enable "Ghostty" under the app you run `make bench` from). If you see error -1743,
   the permission is missing; fix it there and run again.
3. **Isolated session.** Quit Safari, Mail and every other app that uses WebKit, and keep
   the machine idle and on AC power during a run. The harness records WebKit processes it
   did not start, and flags a run `webkit_contaminated` when new WebKit processes stay alive
   more than 5 s after doctty exits.
4. **Mermaid (optional).** Without `DOCTTY_MERMAID_CMD`, every launch gets `--no-mermaid`
   and the `mermaid-images` fixture is `skipped` (`meta.json` records `"mermaid":
   "disabled"`). To include Mermaid, pin a Mermaid CLI version and point doctty at it, for
   example:

   ```
   npm install --prefix ~/.cache/doctty-bench-mmdc @mermaid-js/mermaid-cli@<version>
   export DOCTTY_MERMAID_CMD=~/.cache/doctty-bench-mmdc/node_modules/.bin/mmdc
   ```

   `mmdc --version` is recorded in `meta.json`. Captures with different Mermaid settings
   are not comparable.

## Capture

```
make bench
```

This runs `make build`, then 5 runs of each scenario on each fixture (`small`, `large`,
`raw-html`, `mermaid-images`) with `--theme light`, then one pixel capture per fixture and
theme. Options: `python3 bench/run.py --runs N --scenarios a,b --fixtures a,b
--font-size 13 --theme light|dark --no-pixels`.

Scenarios: `startup-cold` (native helper cache deleted first; must show
`helper_resolved.source == "extracted"`), `startup-warm` (one unmeasured warm-up launch;
must show `cache_hit`), `reload-key` (10 x `r`), `reload-watch` (10 identical-content
rewrites with `--watch`), `scroll` (`j` x60, `Ctrl-d` x20, `PageDown`, `gg`; the 80 paced
keys go in one osascript call with a 50 ms delay between keys, and the run is `invalid` if the
median gap between their traced `key` presses exceeds 100 ms). Every input
is sent only after the matching `input_ready` event. Timeouts: 30 s startup, 10 s per
reload or scroll wait. A scenario with 3 or more failed runs is `failed` and `run.py`
exits 1 at the end. Ctrl-C closes open windows, writes partial results and exits 130.

`startup-cold` deletes exactly `~/Library/Caches/doctty/native` (resolved from `$HOME`;
the run refuses to start if `HOME` is unset, empty, relative or `/`).

Results go to `bench/results/<YYYYmmdd-HHMMSS>-<short sha>/` (git-ignored): `meta.json`,
`summary.json`, `summary.md`, `trace-*.jsonl`, `ps-*.jsonl`, `pixels/`.

**First high-quality frame:** the first `frame_hq` event the renderer writes right after it
flushes a full-quality placement to stdout. Startup time is `cli_start` to that event, and
reload latency is `reload_start` to the next `frame_hq` with reason `reload`.

Scroll latency pairs each moving `key` event with the first `frame_hq` (reason `scroll`) at
its `target_y`; keys that did not move, were already displayed or were coalesced are
counted but excluded. See the design doc for the full rules.

## Compare two captures

```
python3 bench/compare.py bench/results/<before> bench/results/<after>
```

It refuses (exit 1) when geometry, fixture or asset hashes, host model, macOS, Ghostty,
theme, font size or Mermaid mode differ; `--allow-mismatch` turns that into a warning
(for example when comparing a light run with a `--theme dark` run).

## What to post on the issue or PR

- The `summary.md` of each capture.
- The `compare.py` output.
- The measured doctty commit for each capture (`meta.json` `commit`, `dirty`). A commit
  without the trace instrumentation cannot be measured.
- Anything flagged `high_spread`, `webkit_contaminated` or `missing`.

## Unverified assumption: WebKit helper processes exit with doctty

The contamination check assumes the WebKit processes started for a run exit within 5 s
after doctty exits. This has not been checked on a real run. To check it: run
`make bench`, then look in `ps-*.jsonl` and `summary.json` for `webkit_contaminated: 1`.
Or by hand: start `bin/doctty bench/fixtures/small.md`, press `q`, and run
`ps -axo pid,comm | grep WebKit` after 5 s; any process that was not there before the
launch means the assumption is false. Record the answer here once it is known.
