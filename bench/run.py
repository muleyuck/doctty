"""Run the doctty benchmark scenarios in Ghostty and record results (stdlib only)."""

import argparse
import datetime
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import fixtures as fixtures_mod  # noqa: E402
import stats  # noqa: E402
from ghostty import AppleScriptError, AutomationDenied, Driver, DriverTimeout, GhosttyDriver  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
BENCH = Path(__file__).resolve().parent
DEFAULT_BINARY = REPO / "bin" / "doctty"
FIXTURE_NAMES = ("small", "large", "raw-html", "mermaid-images")
SCENARIOS = ("startup-cold", "startup-warm", "reload-key", "reload-watch", "scroll")
STARTUP_TIMEOUT = 30.0
STEP_TIMEOUT = 10.0
POLL = 0.02
SCROLL_INTERVAL = 0.05
RELOADS = 10
FAILED_RUNS_LIMIT = 3
WEBKIT_EXIT_GRACE = 5.0
MIN_GHOSTTY = (1, 4)


class RunError(Exception):
    def __init__(self, status: str, detail: str = ""):
        super().__init__(detail or status)
        self.status = status


class UnsafeCachePath(RuntimeError):
    pass


def native_cache_dir(environ) -> Path:
    home = environ.get("HOME", "")
    if not home or not os.path.isabs(home) or len(Path(home).parts) < 2:
        raise UnsafeCachePath(f"refusing to derive a cache path from HOME={home!r}")
    return Path(home) / "Library" / "Caches" / "doctty" / "native"


def clear_native_cache(environ=os.environ) -> None:
    target = native_cache_dir(environ)
    home = Path(environ["HOME"])
    # A symlink anywhere below HOME could redirect rmtree outside it.
    below_home = target.relative_to(home)
    for component in (*reversed(below_home.parents[:-1]), below_home):
        link = home / component
        if link.is_symlink():
            raise UnsafeCachePath(f"refusing to delete through symlink {link}")
    if target.is_dir():
        shutil.rmtree(target)


def touch_identical(path: Path) -> None:
    path.write_bytes(path.read_bytes())


# Ghostty sometimes opens a window small and resizes it a moment later. Wait until `stty size`
# repeats `reads` times in a row (or `limit` reads pass) before exec, so the launch sees the
# final size. Startup is timed from doctty's own cli_start, so the wait is not measured.
_SETTLE = (
    'last=; same=0; n=0; while :; do size=$(stty size 2>/dev/null); '
    'if [ -n "$size" ] && [ "$size" = "$last" ]; then same=$((same + 1)); else same=1; last=$size; fi; '
    'n=$((n + 1)); if [ -n "$size" ] && [ "$same" -ge "$1" ] || [ "$n" -ge "$2" ]; then break; fi; '
    'sleep "$3"; done; shift 3; exec "$@"'
)
SETTLE_READS = 4  # 300 ms of an unchanged size at SETTLE_POLL
SETTLE_LIMIT = 50
SETTLE_POLL = 0.1


def settled_argv(
    argv: list[str], *, reads: int = SETTLE_READS, limit: int = SETTLE_LIMIT, poll: float = SETTLE_POLL
) -> list[str]:
    return ["/bin/sh", "-c", _SETTLE, "sh", str(reads), str(limit), str(poll), *argv]


def launch_argv(binary: Path, fixture: Path, theme: str, watch: bool, no_mermaid: bool) -> list[str]:
    argv = [str(binary), "--theme", theme]
    if watch:
        argv.append("--watch")
    if no_mermaid:
        argv.append("--no-mermaid")
    return settled_argv(argv + [str(fixture)])


def read_trace(path: Path) -> list[dict]:
    events = []
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return events
    for line in text.splitlines():
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue  # the last line may be mid-write
    return events


# ---- ps sampling -------------------------------------------------------------------


def _cpu_seconds(text: str) -> float:
    total = 0.0
    for part in text.split(":"):
        total = total * 60 + float(part)
    return total


def parse_ps(output: str) -> list[dict]:
    rows = []
    for line in output.splitlines():
        parts = line.split(None, 3)
        if len(parts) < 4:
            continue
        try:
            rows.append(
                {
                    "pid": int(parts[0]),
                    "comm": parts[3],
                    "cpu_s": _cpu_seconds(parts[1]),
                    "rss_kb": int(parts[2]),
                }
            )
        except ValueError:
            continue
    return rows


def _ps() -> list[dict]:
    # macOS ps cuts comm to 16 characters unless it is the last column.
    out = subprocess.run(["ps", "-axo", "pid=,cputime=,rss=,comm="], capture_output=True, text=True)
    return parse_ps(out.stdout)


def _is_webkit(row: dict) -> bool:
    return "WebKit" in row["comm"]


def _is_interesting(row: dict) -> bool:
    return _is_webkit(row) or "doctty" in row["comm"]


class PsSampler:
    def __init__(self, path: Path, interval: float = 0.1):
        self.path = path
        self.interval = interval
        self.webkit_before = {row["pid"] for row in _ps() if _is_webkit(row)}
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._loop, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def _loop(self) -> None:
        with open(self.path, "w", encoding="utf-8") as handle:
            while not self._stop.is_set():
                tick = time.time()  # one shared `t` per tick: stats.process_usage sums RSS per `t`
                for row in _ps():
                    if _is_interesting(row):
                        handle.write(json.dumps({"t": tick, **row}) + "\n")
                handle.flush()
                self._stop.wait(self.interval)

    def stop(self) -> None:
        self._stop.set()
        if self._thread.is_alive():
            self._thread.join()

    def new_webkit_alive(self) -> set[int]:
        return {row["pid"] for row in _ps() if _is_webkit(row)} - self.webkit_before


# ---- one run ------------------------------------------------------------------------


class _Run:
    def __init__(self, driver, terminal, trace, clock, sleep):
        self.driver, self.terminal, self.trace = driver, terminal, trace
        self.clock, self.sleep = clock, sleep

    def events(self) -> list[dict]:
        return read_trace(self.trace)

    def wait(self, pick, timeout: float):
        """Poll until pick(events) returns a non-None value."""
        deadline = self.clock() + timeout
        while True:
            events = self.events()
            found = pick(events)
            if found is not None:
                return found
            exited = any(e["ev"] == "cli_exit" for e in events)
            if exited or self.driver.terminal_pid(self.terminal) is None:
                # Re-read: the awaited event may have landed between the read and the pid check.
                found = pick(self.events())
                if found is not None:
                    return found
                raise RunError("crash", "process ended before the expected event")
            if self.clock() >= deadline:
                raise RunError("timeout")
            self.sleep(POLL)

    def wait_event(self, start: int, ev: str, timeout: float, **match):
        def pick(events):
            for e in events[start:]:
                if e["ev"] == ev and all(e.get(k) == v for k, v in match.items()):
                    return e
            return None

        return self.wait(pick, timeout)


PACED_KEYS = ["j"] * 60 + ["ctrl-d"] * 20


def key_gap_ms(events: list[dict], start: int, count: int) -> float | None:
    """Median gap between the first `count` `j` / `ctrl-d` key presses after events[start].

    None when fewer than `count` presses are in the trace. Other keys (PageDown, g) cannot
    stand in for a missing paced key.
    """
    times = [
        e["t_us"]
        for e in events[start:]
        if e["ev"] == "key" and e["kind"] == "press" and e["key"] in ("j", "ctrl-d")
    ][:count]
    if len(times) < count:
        return None
    return statistics.median((later - earlier) / 1000 for earlier, later in zip(times, times[1:]))


def _scroll(run: _Run, start: int) -> float | None:
    """Returns the median gap (ms) between the paced scroll keys, None if their presses are missing."""
    run.driver.send_keys(run.terminal, PACED_KEYS, interval=SCROLL_INTERVAL)
    run.driver.send_keys(run.terminal, ["PageDown"])
    run.driver.send_keys(run.terminal, ["g"])
    run.driver.send_keys(run.terminal, ["g"])

    def final_g(events):
        presses = [
            i
            for i in range(start, len(events))
            if events[i]["ev"] == "key" and events[i]["key"] == "g" and events[i]["kind"] == "press"
        ]
        return (presses[1], events[presses[1]]) if len(presses) >= 2 else None

    index, key = run.wait(final_g, STEP_TIMEOUT)
    # The renderer traces keys in order, so by the final `g` every paced press is in the trace.
    gap = key_gap_ms(run.events(), start, len(PACED_KEYS))
    if not key["moved"]:
        return gap
    target = key["target_y"]

    def settled(events):
        for e in events[index + 1 :]:
            if e["ev"] == "frame_unchanged" and abs(e["y"] - target) < stats.Y_TOLERANCE:
                return e
            if e["ev"] == "frame_hq" and e.get("reason") == "scroll" and abs(e["y"] - target) < stats.Y_TOLERANCE:
                return e
        return None

    run.wait(settled, STEP_TIMEOUT)
    return gap


def _reloads(run: _Run, trigger) -> None:
    for _ in range(RELOADS):
        start = len(run.events())
        trigger()
        run.wait_event(start, "input_ready", STEP_TIMEOUT, after="reload")


def _first(events, ev):
    return next((e for e in events if e["ev"] == ev), None)


def collect_metrics(
    scenario: str, events: list[dict], ps_rows: list[dict] | None, webkit_new: set[int]
) -> tuple[dict, dict[str, list[int]]]:
    """Returns (metrics, unsampled pids per process group)."""
    metrics: dict = {}
    if scenario.startswith("startup"):
        for stage, value in stats.startup_metrics(events).items():
            metrics[f"startup_{stage}_ms"] = value
    if scenario.startswith("reload"):
        latencies = stats.reload_latencies(events)
        metrics["reload_ms"] = statistics.median(latencies) if latencies else None
        metrics["reloads_measured"] = len(latencies)
    if scenario == "scroll":
        scroll = stats.scroll_latencies(events)
        metrics["scroll_p50_ms"] = stats.percentile(scroll["latencies_ms"], 50)
        metrics["scroll_p95_ms"] = stats.percentile(scroll["latencies_ms"], 95)
        for name in ("nonmoving", "already_displayed", "coalesced", "missing"):
            metrics[f"scroll_{name}"] = scroll[name]
        metrics["scroll_measured"] = len(scroll["latencies_ms"])
    metrics.update(stats.counters(events))
    unsampled: dict[str, list[int]] = {}
    if ps_rows is not None:  # None: no sampler ran; [] : it ran but caught nothing
        pids = {
            group: {e["pid"] for e in events if e["ev"] == ev}
            for group, ev in (("doctty", "cli_start"), ("renderer", "renderer_start"), ("helper", "helper_spawned"))
            if any(e["ev"] == ev for e in events)
        }
        for group, usage in stats.process_usage(ps_rows, pids, webkit_new).items():
            if usage["unsampled"]:  # a process that ended between samples has no usable CPU/RSS
                unsampled[group] = usage["unsampled"]
                metrics[f"cpu_s_{group}"] = None
                metrics[f"peak_rss_kb_{group}"] = None
            else:
                metrics[f"cpu_s_{group}"] = usage["cpu_s"]
                metrics[f"peak_rss_kb_{group}"] = usage["peak_rss_kb"]
    return metrics, unsampled


def _terminal_ids(driver: Driver) -> set[str] | None:
    try:
        return driver.terminal_ids()
    except (AppleScriptError, DriverTimeout):
        return None


def pid_tty(pid: int) -> str | None:
    try:
        out = subprocess.run(["ps", "-o", "tty=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    except OSError:
        return None
    if not out or out.startswith("?"):
        return None
    return out if out.startswith("/") else f"/dev/{out}"


def _close_owned_terminals(driver: Driver, before: set[str], trace: Path, tty_of_pid) -> str | None:
    """Close the terminals this launch created, found by the tty of its cli_start process.

    A window the user opened meanwhile is also new since `before`, so only the tty tells them apart.
    Returns a note when no window could be closed.
    """
    cli = _first(read_trace(trace), "cli_start")
    owner_tty = tty_of_pid(cli["pid"]) if cli is not None else None
    after = _terminal_ids(driver)
    if owner_tty is None or after is None:
        return "could not identify the launched window; a window may have been left open"
    owned = []
    for candidate in sorted(after - before):
        try:
            tty = driver.terminal_tty(candidate)
        except DriverTimeout:
            tty = None
        if tty == owner_tty:
            owned.append(candidate)
    for terminal in owned:
        driver.close(terminal)
    return None if owned else "no new window matched the launched process; a window may have been left open"


def run_scenario(
    driver: Driver,
    scenario: str,
    fixture: Path,
    run_dir: Path,
    clock=time.monotonic,
    sleep=time.sleep,
    *,
    index: int = 1,
    theme: str = "light",
    font_size: float = 13.0,
    no_mermaid: bool = False,
    binary: Path = DEFAULT_BINARY,
    clear_cache=None,
    sampler_factory=None,
    extra_env: dict[str, str] | None = None,
    touch=touch_identical,
    tty_of_pid=pid_tty,
) -> dict:
    """One launch. scenario is a SCENARIOS name, "warmup" or "pixels" (startup only)."""
    stem = f"{scenario}-{fixture.stem}-{index}"
    trace = run_dir / f"trace-{stem}.jsonl"
    trace.unlink(missing_ok=True)
    env = {**(extra_env or {}), "DOCTTY_BENCH_TRACE_FILE": str(trace)}
    argv = launch_argv(binary, fixture, theme, scenario == "reload-watch", no_mermaid)
    if scenario == "startup-cold" and clear_cache is not None:
        clear_cache()
    ps_path = run_dir / f"ps-{stem}.jsonl"
    sampler = sampler_factory(ps_path) if sampler_factory else None
    result: dict = {"status": "ok", "metrics": {}}
    webkit_new: set[int] = set()
    terminal = None
    scroll_gap = None
    leak_note = None
    try:
        if sampler:
            sampler.start()
        try:
            before = _terminal_ids(driver)
            if before is None:
                raise RunError("timeout", "could not read the terminal list; not opening a window that could leak")
            try:
                terminal = driver.open(argv, env, fixture.parent, font_size)
            except BaseException as error:  # includes Ctrl-C: the window may exist before open() returns
                note = _close_owned_terminals(driver, before, trace, tty_of_pid)
                if isinstance(error, DriverTimeout):
                    leak_note = note
                raise
            run = _Run(driver, terminal, trace, clock, sleep)
            run.wait_event(0, "input_ready", STARTUP_TIMEOUT, after="startup")
            events = run.events()
            source = (_first(events, "helper_resolved") or {}).get("source")
            expected = {"startup-cold": "extracted", "startup-warm": "cache_hit"}.get(scenario)
            if expected and source != expected:
                result["status"] = "invalid"
                result["detail"] = f"helper_resolved.source={source!r}, expected {expected!r}"
            ready = len(events)
            if scenario == "reload-key":
                _reloads(run, lambda: driver.send_keys(terminal, ["r"]))
            elif scenario == "reload-watch":
                _reloads(run, lambda: touch(fixture))
            elif scenario == "scroll":
                scroll_gap = _scroll(run, ready)
                if scroll_gap is None:
                    result["status"] = "invalid"
                    result["detail"] = f"fewer than {len(PACED_KEYS)} paced key presses in the trace"
                elif scroll_gap > 2 * SCROLL_INTERVAL * 1000:
                    result["status"] = "invalid"
                    result["detail"] = (
                        f"median scroll key gap {scroll_gap:.1f} ms exceeds {2 * SCROLL_INTERVAL * 1000:.0f} ms"
                    )
            driver.send_keys(terminal, ["q"])
            run.wait_event(0, "cli_exit", STEP_TIMEOUT)
        except (RunError, DriverTimeout, AppleScriptError) as error:
            # AppleScriptError: the target terminal is gone, so the process died.
            result["status"] = (
                error.status if isinstance(error, RunError) else "timeout" if isinstance(error, DriverTimeout) else "crash"
            )
            result["detail"] = str(error)
            if leak_note:
                result["detail"] += f"; {leak_note}"
            if result["status"] == "crash":
                lines = trace.read_text(encoding="utf-8").splitlines()[-20:] if trace.exists() else []
                tail = run_dir / f"crash-{stem}.txt"
                tail.write_text("\n".join(lines) + "\n", encoding="utf-8")
                result["crash_tail"] = str(tail)
                result["renderer_exit"] = _first(read_trace(trace), "renderer_exit")
        if sampler and result["status"] != "crash":
            sleep(WEBKIT_EXIT_GRACE)
            webkit_new = sampler.new_webkit_alive()
    finally:
        if terminal is not None:
            driver.close(terminal)
        if sampler:
            sampler.stop()
    events = read_trace(trace)
    # Ghostty sometimes opens a window small and resizes it during startup, and the renderer
    # keeps the first size until the next reload; such a run measured a different viewport.
    sizes = list(dict.fromkeys((e["cols"], e["rows"]) for e in events if e["ev"] == "geometry"))
    if result["status"] == "ok" and len(sizes) > 1:
        result["status"] = "invalid"
        result["detail"] = "terminal size changed during the run: " + " -> ".join(f"{c}x{r}" for c, r in sizes)
    if result["status"] == "ok":
        rows = read_trace(ps_path) if sampler else None
        seen_webkit = {row["pid"] for row in rows or [] if "WebKit" in row["comm"]} - (
            sampler.webkit_before if sampler else set()
        )
        result["metrics"], unsampled = collect_metrics(scenario, events, rows, seen_webkit)
        if unsampled:
            result["unsampled"] = unsampled
        if sampler:
            result["metrics"]["webkit_contaminated"] = int(bool(webkit_new))
            if webkit_new:  # the spec excludes contaminated runs from WebKit CPU/RSS
                result["metrics"]["cpu_s_webkit"] = None
                result["metrics"]["peak_rss_kb_webkit"] = None
    if scroll_gap is not None:
        result["metrics"]["scroll_key_gap_ms"] = scroll_gap
    return result


def run_scenario_set(
    driver: Driver,
    scenario: str,
    fixture: Path,
    run_dir: Path,
    runs: int,
    *,
    mermaid: bool,
    clock=time.monotonic,
    sleep=time.sleep,
    entry: dict | None = None,
    **kwargs,
) -> dict:
    """entry, when given, is updated after every run so an interrupt keeps finished runs."""
    if fixture.stem == "mermaid-images" and not mermaid:
        return {"status": "skipped", "run_statuses": [], "runs": [], "metrics": {}}
    common = dict(clock=clock, sleep=sleep, no_mermaid=not mermaid, **kwargs)
    if scenario == "startup-warm":
        run_scenario(driver, "warmup", fixture, run_dir, index=0, **common)
    statuses, metrics, failed_runs, unsampled_runs = [], [], [], []

    def assemble(status: str) -> dict:
        return {
            "status": status,
            "run_statuses": list(statuses),
            "runs": list(metrics),
            "failed_runs": list(failed_runs),
            "unsampled": list(unsampled_runs),  # per ok run, aligned with "runs"
            "metrics": stats.summarize(metrics),
        }

    for n in range(1, runs + 1):
        result = run_scenario(driver, scenario, fixture, run_dir, index=n, **common)
        statuses.append(result["status"])
        if result["status"] == "ok":
            metrics.append(result["metrics"])
            unsampled_runs.append(result.get("unsampled", {}))
        else:  # diagnostics only: aggregation uses the ok runs
            failed_runs.append({k: result[k] for k in ("status", "detail", "metrics", "unsampled") if k in result})
        if entry is not None:
            entry.clear()
            entry.update(assemble("partial"))
    failed = sum(1 for s in statuses if s != "ok")
    return assemble("failed" if failed >= FAILED_RUNS_LIMIT or not metrics else "ok")


# ---- orchestration ------------------------------------------------------------------


def _out(*cmd: str) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, cwd=REPO).stdout.strip()


def ghostty_version() -> str | None:
    for exe in ("ghostty", "/Applications/Ghostty.app/Contents/MacOS/ghostty"):
        try:
            out = subprocess.run([exe, "+version"], capture_output=True, text=True, timeout=10)
        except (FileNotFoundError, subprocess.TimeoutExpired):
            continue
        if out.returncode == 0 and out.stdout.strip():
            return out.stdout.splitlines()[0].strip()
    return None


def version_at_least(text: str, minimum: tuple[int, int]) -> bool:
    match = re.search(r"(\d+)\.(\d+)", text)
    return bool(match) and (int(match.group(1)), int(match.group(2))) >= minimum


PERMISSION_HELP = """\
AppleScript access to Ghostty was denied (error -1743).
Allow it in System Settings > Privacy & Security > Automation: enable "Ghostty" under the
terminal app you launched `make bench` from, then run it again. See bench/README.md."""


def summary_markdown(summary: dict) -> str:
    lines = ["| scenario/fixture | status | metric | median | min | max | spread |", "|---|---|---|---|---|---|---|"]
    for key, entry in summary.items():
        if not entry["metrics"]:
            lines.append(f"| {key} | {entry['status']} | | | | | |")
        for name, m in entry["metrics"].items():
            flag = "high_spread" if m["high_spread"] else ""
            lines.append(f"| {key} | {entry['status']} | {name} | {_fmt(m['median'])} | {_fmt(m['min'])} | {_fmt(m['max'])} | {flag} |")
    return "\n".join(lines) + "\n"


def _fmt(value) -> str:
    return "" if value is None else f"{value:.1f}" if isinstance(value, float) else str(value)


def _write_results(out: Path, meta: dict, summary: dict) -> None:
    meta["geometry"] = find_geometry(out) or find_geometry(out, pixels=True) or meta.get("geometry")
    (out / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (out / "summary.md").write_text(summary_markdown(summary))


def find_geometry(out: Path, *, pixels: bool = False) -> dict | None:
    """The geometry most scenario launches ran at (pixel launches only when pixels=True);
    launches whose size changed are ignored."""
    counts: dict[tuple, int] = {}
    for trace in sorted(out.glob("trace-*.jsonl")):
        if trace.name.startswith("trace-pixels-") != pixels:
            continue
        events = [e for e in read_trace(trace) if e["ev"] == "geometry"]
        if not events or len({(e["cols"], e["rows"]) for e in events}) > 1:
            continue
        key = tuple(events[0][k] for k in ("cols", "rows", "px_w", "px_h"))
        counts[key] = counts.get(key, 0) + 1
    if not counts:
        return None
    best = max(counts, key=counts.get)  # ties: the first trace in name order
    return dict(zip(("cols", "rows", "px_w", "px_h"), best))


def mmdc_version(command: str, run=subprocess.run) -> str | None:
    # doctty runs DOCTTY_MERMAID_CMD as one executable path, so it is one argv element here too.
    try:
        out = run([command, "--version"], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return out.stdout.strip() or None


def preflight(system: str, get_ghostty, build, driver, stderr=sys.stderr) -> int | None:
    """Exit code when the run must not start, else None."""
    if system != "Darwin":
        print("error: the benchmark harness needs macOS and Ghostty", file=stderr)
        return 2
    ghostty = get_ghostty()
    if ghostty is None or not version_at_least(ghostty, MIN_GHOSTTY):
        print(f"error: Ghostty >= 1.4 required (`ghostty +version` gave {ghostty!r})", file=stderr)
        return 2
    if not build():
        print("error: make build failed", file=stderr)
        return 1
    try:
        driver.check_access()
    except AutomationDenied:
        print(PERMISSION_HELP, file=stderr)
        return 1
    except DriverTimeout:
        print("error: Ghostty did not answer the automation probe in time", file=stderr)
        return 1
    return None


def _cells(geometry: dict | None) -> tuple[int, int] | None:
    return geometry and (geometry["cols"], geometry["rows"])


def _pixels(driver, fixture: Path, theme: str, out: Path, expected: dict | None, **launch) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        extra_env = {**launch.pop("extra_env"), "DOCTTY_BENCH_PIXEL_DIR": tmp}
        result = run_scenario(driver, "pixels", fixture, out, index=1, theme=theme, extra_env=extra_env, **launch)
        first, size = Path(tmp) / "first.rgbz", Path(tmp) / "first.json"
        if result["status"] != "ok" or not first.is_file() or not size.is_file():
            print(f"error: pixel capture {fixture.stem}-{theme}: {result['status']}", file=sys.stderr)
            return False
        # The renderer may cap a tile's width (image limits), so compare cells, not the tile.
        geometry = _first(read_trace(out / f"trace-pixels-{fixture.stem}-1.jsonl"), "geometry")
        if expected and _cells(geometry) != _cells(expected):
            print(
                f"error: pixel capture {fixture.stem}-{theme}: terminal was {_cells(geometry)}, "
                f"other launches ran at {_cells(expected)}",
                file=sys.stderr,
            )
            return False
        (out / "pixels").mkdir(exist_ok=True)
        shutil.copy(first, out / "pixels" / f"{fixture.stem}-{theme}.rgbz")
        shutil.copy(size, out / "pixels" / f"{fixture.stem}-{theme}.json")
    return True


def run_all(
    driver,
    out: Path,
    fixtures_dir: Path,
    meta: dict,
    *,
    scenarios: list[str],
    fixture_names: list[str],
    runs: int,
    mermaid_cmd: str | None,
    font_size: float,
    theme: str,
    pixels: bool,
    clock=time.monotonic,
    sleep=time.sleep,
    sampler_factory=PsSampler,
    clear_cache=clear_native_cache,
) -> int:
    extra_env = {"DOCTTY_MERMAID_CMD": mermaid_cmd} if mermaid_cmd else {}
    summary: dict = {}
    failed = False
    pixel_failures = []
    try:
        for scenario in scenarios:
            for name in fixture_names:
                key = f"{scenario}/{name}"
                print(f"running {key}", flush=True)
                entry = summary[key] = {"status": "partial", "run_statuses": [], "runs": [], "metrics": {}}
                result = run_scenario_set(
                    driver, scenario, fixtures_dir / f"{name}.md", out, runs,
                    mermaid=bool(mermaid_cmd), clock=clock, sleep=sleep, entry=entry,
                    clear_cache=clear_cache, sampler_factory=sampler_factory,
                    font_size=font_size, theme=theme, extra_env=extra_env,
                )
                summary[key] = result
                failed |= result["status"] == "failed"
                _write_results(out, meta, summary)
        if pixels:
            expected = find_geometry(out)  # fixed before any capture, from scenario launches only
            for name in fixture_names:
                if name == "mermaid-images" and not mermaid_cmd:
                    continue
                for pixel_theme in ("light", "dark"):
                    ok = _pixels(
                        driver, fixtures_dir / f"{name}.md", pixel_theme, out, expected,
                        clock=clock, sleep=sleep, font_size=font_size,
                        no_mermaid=not mermaid_cmd, extra_env=extra_env,
                    )
                    if not ok:
                        pixel_failures.append(f"{name}-{pixel_theme}")
    except KeyboardInterrupt:
        _write_results(out, meta, summary)
        print(f"interrupted; partial results in {out}", file=sys.stderr)
        return 130
    _write_results(out, meta, summary)
    if pixel_failures:
        print(f"error: pixel capture failed for {', '.join(pixel_failures)}", file=sys.stderr)
    return 1 if failed or pixel_failures else 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--scenarios", default=",".join(SCENARIOS))
    parser.add_argument("--fixtures", default=",".join(FIXTURE_NAMES))
    parser.add_argument("--font-size", type=float, default=13)
    parser.add_argument("--theme", choices=("light", "dark"), default="light")
    parser.add_argument("--no-pixels", action="store_true")
    args = parser.parse_args(argv)
    scenarios = args.scenarios.split(",")
    fixture_names = args.fixtures.split(",")
    for name in scenarios:
        if name not in SCENARIOS:
            parser.error(f"unknown scenario {name!r}")
    for name in fixture_names:
        if name not in FIXTURE_NAMES:
            parser.error(f"unknown fixture {name!r}")

    driver = GhosttyDriver()
    code = preflight(
        platform.system(),
        ghostty_version,
        lambda: subprocess.run(["make", "build"], cwd=REPO).returncode == 0,
        driver,
    )
    if code is not None:
        return code

    fixtures_dir = BENCH / "fixtures"
    fixtures_mod.generate_large(fixtures_dir)
    mermaid_cmd = os.environ.get("DOCTTY_MERMAID_CMD")
    commit = _out("git", "rev-parse", "HEAD")
    out = BENCH / "results" / f"{datetime.datetime.now():%Y%m%d-%H%M%S}-{commit[:7]}"
    out.mkdir(parents=True)
    meta = {
        "commit": commit,
        "dirty": bool(_out("git", "status", "--porcelain")),
        "host_model": _out("sysctl", "-n", "hw.model"),
        "macos": _out("sw_vers", "-productVersion"),
        "ghostty": ghostty_version(),
        "geometry": None,
        "fixtures": fixtures_mod.fixture_hashes(fixtures_dir),
        "mmdc": mmdc_version(mermaid_cmd) if mermaid_cmd else None,
        "mermaid": "enabled" if mermaid_cmd else "disabled",
        "font_size": args.font_size,
        "theme": args.theme,
    }
    code = run_all(
        driver, out, fixtures_dir, meta,
        scenarios=scenarios, fixture_names=fixture_names, runs=args.runs,
        mermaid_cmd=mermaid_cmd, font_size=args.font_size, theme=args.theme,
        pixels=not args.no_pixels,
    )
    if code != 130:
        print(f"results: {out}")
    return code


if __name__ == "__main__":
    sys.exit(main())
