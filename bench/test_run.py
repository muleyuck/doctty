import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ghostty  # noqa: E402
import run  # noqa: E402


class FakeDriver:
    """Appends scripted trace events as keys arrive; time only moves through sleep().

    Key events reach the trace KEY_LAG seconds after the key was sent, like a renderer that
    handles input after send_keys has returned.
    """

    KEY_LAG = 0.2

    def __init__(self, behaviors=None, send_cost=0.0):
        self.now = 1000.0
        self.behaviors = behaviors or []
        self.send_cost = send_cost
        self.opens = []
        self.open_times = []
        self.keys = []
        self.send_calls = []
        self.ids_calls = 0
        self.scroll_y = 0.0
        self.fail_ids_calls = set()
        self.terminals = set()
        self.preexisting = set()
        self.key_times = []
        self.closed = []
        self.pending = []
        self.alive = {}
        self.behavior = {}
        self.g_presses = {}
        self.ready_count_at_r = []
        self.ready_count_at_rewrite = []
        self.events_at_q = []

    def clock(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds
        for item in [p for p in self.pending if p[0] <= self.now]:
            self.pending.remove(item)
            self._emit(item[1], **{"t_us": round(item[0] * 1e6), **item[2]})

    def _emit(self, ev, proc="renderer", **fields):
        line = {"t_us": round(self.now * 1e6), "proc": proc, "ev": ev, **fields}
        with open(self.trace_path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(line) + "\n")

    def _later(self, delay, ev, **fields):
        self.pending.append((self.now + delay, ev, fields))

    def open(self, argv, env, cwd, font_size):
        index = len(self.opens)
        self.opens.append({"argv": argv, "env": env, "cwd": cwd, "font_size": font_size})
        self.open_times.append(self.now)
        self.behavior = self.behaviors[index] if index < len(self.behaviors) else {}
        if self.behavior.get("raise_on_open"):
            raise ghostty.DriverTimeout("open timed out")
        self.trace_path = env["DOCTTY_BENCH_TRACE_FILE"]
        terminal = f"term-{index}"
        self.terminals.add(terminal)
        if self.behavior.get("also_create"):
            self.terminals.add(self.behavior["also_create"])
        self.alive[terminal] = True
        self.g_presses[terminal] = 0
        if not self.behavior.get("no_cli_start"):
            self._emit("cli_start", proc="cli", pid=1000 + index)
        if self.behavior.get("leak_on_open"):
            raise ghostty.DriverTimeout("open timed out after the window appeared")
        if self.behavior.get("interrupt_on_open"):
            raise KeyboardInterrupt
        if self.behavior.get("crash"):
            self.alive[terminal] = False
            for _ in range(self.behavior.get("extra_lines", 0)):
                self._emit("prefetch_stale_skip")
            self._emit("renderer_exit", proc="cli", code=None, signal=9)
            return terminal
        self._emit("helper_resolved", proc="cli", source=self.behavior.get("source", "extracted"), ms=1.0)
        self._emit("renderer_start", pid=2000 + index)
        for cols, rows, px_w in self.behavior.get("geometry", [(120, 40, 2)]):
            self._emit("geometry", cols=cols, rows=rows, px_w=px_w, px_h=1)
        if "pixel" in self.behavior:
            pixel_dir = Path(env["DOCTTY_BENCH_PIXEL_DIR"])
            (pixel_dir / "first.rgbz").write_bytes(self.behavior["pixel"])
            width = self.behavior.get("pixel_width", 2)
            (pixel_dir / "first.json").write_text(json.dumps({"width": width, "height": 1}))
        if self.behavior.get("no_ready"):
            return terminal
        delay = self.behavior.get("startup_delay", 0.0)
        if delay:
            self._later(delay, "frame_hq", reason="startup", seq=1, y=0.0)
            self._later(delay, "input_ready", after="startup")
        else:
            self._emit("frame_hq", reason="startup", seq=1, y=0.0)
            self._emit("input_ready", after="startup")
        return terminal

    def terminal_ids(self):
        self.ids_calls += 1
        if self.ids_calls in self.fail_ids_calls:
            raise ghostty.DriverTimeout("listing timed out")
        return self.preexisting | self.terminals

    def send_keys(self, terminal, keys, interval=0.0):
        self.send_calls.append((list(keys), interval))
        if self.behavior.get("raise_on_send"):
            raise ghostty.DriverTimeout("send timed out")
        if self.behavior.get("gone_on_send"):
            raise ghostty.AppleScriptError("terminal is gone")
        if self.behavior.get("interrupt_on_send"):
            raise KeyboardInterrupt
        key_gap = self.behavior.get("key_gap", interval)
        for i, key in enumerate(keys):
            self.key_times.append((key, self.now))
            self.keys.append(key)
            dropped = key != "PageDown" and i >= len(keys) - self.behavior.get("drop_keys", 0)
            if key in ("j", "ctrl-d", "PageDown") and not dropped:
                self._press_scroll_key(key, round((self.now + i * key_gap) * 1e6))
            if key == "q":
                self.events_at_q.append(self._events())
                self._emit("cli_exit", proc="cli", ok=True)
                self.alive[terminal] = False
            elif key == "r":
                self.ready_count_at_r.append(self._ready_reload_count())
                self._start_reload()
            elif key == "g":
                self._press_g(terminal)
        self.now += self.send_cost + max(len(keys) - 1, 0) * interval

    def on_rewrite(self):
        self.ready_count_at_rewrite.append(self._ready_reload_count())
        self._start_reload()

    def _press_scroll_key(self, key, t_us):
        """With scroll_frames, j / ctrl-d move the view and a scroll frame lands 100 ms later."""
        moves = self.behavior.get("scroll_frames") and key != "PageDown"
        from_y = self.scroll_y
        if moves:
            self.scroll_y += 40.0 if key == "ctrl-d" else 10.0
        self._later(
            self.KEY_LAG, "key", key=key, kind="press", seq=0, from_y=from_y, target_y=self.scroll_y,
            moved=bool(moves), t_us=t_us,
        )
        if moves:
            self._later(self.KEY_LAG, "frame_hq", reason="scroll", seq=0, y=self.scroll_y, t_us=t_us + 100_000)

    def _start_reload(self):
        self._emit("reload_start", trigger="key")
        delay = self.behavior.get("reload_delay", 0.3)
        self._later(delay, "frame_hq", reason="reload", seq=0, y=0.0)
        self._later(delay, "input_ready", after="reload")

    def _press_g(self, terminal):
        self.g_presses[terminal] += 1
        seq = self.g_presses[terminal]
        final = seq == 2
        moved = bool(self.behavior.get("final_g_moved")) if final else False
        self._later(self.KEY_LAG, "key", key="g", kind="press", seq=seq, from_y=5.0, target_y=0.0, moved=moved)
        self._later(self.KEY_LAG, "key", key="g", kind="release", seq=seq, from_y=0.0, target_y=0.0, moved=False)
        if final and moved:
            if self.behavior.get("frame_at_other_y_first"):
                self._later(0.5, "frame_hq", reason="scroll", seq=2, y=300.0)
                self._later(1.5, "frame_hq", reason="scroll", seq=3, y=0.0)
            else:
                self._later(0.5, "frame_unchanged", y=0.0)

    def _events(self):
        return [json.loads(line) for line in Path(self.trace_path).read_text().splitlines()]

    def _ready_reload_count(self):
        return sum(1 for e in self._events() if e["ev"] == "input_ready" and e.get("after") == "reload")

    def terminal_tty(self, terminal):
        if terminal in self.terminals or terminal in self.preexisting:
            return f"/dev/tty-{terminal}"
        return None

    def terminal_pid(self, terminal):
        return 1000 if self.alive.get(terminal) else None

    def close(self, terminal):
        self.closed.append(terminal)
        self.terminals.discard(terminal)


def fake_tty_of_pid(pid):
    """The fake's cli_start pid is 1000 + open index; its terminal is term-<index>."""
    return f"/dev/tty-term-{pid - 1000}"


class FakeSampler:
    """Stands in for PsSampler: writes canned ps rows and reports scripted new WebKit pids."""

    def __init__(self, path, still_alive):
        self.webkit_before = set()
        self._still_alive = still_alive
        rows = [
            {"t": 1.0, "pid": 1000, "comm": "doctty", "cpu_s": 0.5, "rss_kb": 100},
            {"t": 2.0, "pid": 1000, "comm": "doctty", "cpu_s": 1.5, "rss_kb": 120},
            {"t": 2.0, "pid": 99, "comm": "com.apple.WebKit.WebContent", "cpu_s": 12.0, "rss_kb": 3456},
        ]
        Path(path).write_text("".join(json.dumps(r) + "\n" for r in rows))

    def start(self):
        pass

    def stop(self):
        pass

    def new_webkit_alive(self):
        return set(self._still_alive)


class RunScenarioTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)
        self.fixture = self.dir / "small.md"
        self.fixture.write_text("# hi\n")

    def go(self, driver, scenario, **kwargs):
        kwargs.setdefault("tty_of_pid", fake_tty_of_pid)
        return run.run_scenario(
            driver, scenario, self.fixture, self.dir, clock=driver.clock, sleep=driver.sleep, **kwargs
        )

    def test_cold_startup_requires_extracted_helper(self):
        ok = self.go(FakeDriver([{"source": "extracted"}]), "startup-cold")
        self.assertEqual(ok["status"], "ok")
        self.assertIsNotNone(ok["metrics"]["startup_frame_hq_ms"])

        hit = self.go(FakeDriver([{"source": "cache_hit"}]), "startup-cold")
        self.assertEqual(hit["status"], "invalid")

    def test_terminal_resized_during_the_run_is_invalid(self):
        # Real run: Ghostty opened the window at 56x20, then the first reload saw 204x59.
        driver = FakeDriver([{"source": "extracted", "geometry": [(56, 20, 2856), (204, 59, 2856)]}])
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "invalid")
        self.assertIn("56x20 -> 204x59", result["detail"])

    def test_pixel_width_change_at_the_same_cell_size_stays_ok(self):
        driver = FakeDriver([{"source": "extracted", "geometry": [(204, 59, 2800), (204, 59, 2856)]}])
        self.assertEqual(self.go(driver, "startup-cold")["status"], "ok")

    def test_warm_startup_requires_cache_hit(self):
        self.assertEqual(self.go(FakeDriver([{"source": "cache_hit"}]), "startup-warm")["status"], "ok")
        self.assertEqual(self.go(FakeDriver([{"source": "extracted"}]), "startup-warm")["status"], "invalid")

    def test_cold_startup_clears_cache_before_launch(self):
        driver = FakeDriver()
        calls = []
        self.go(driver, "startup-cold", clear_cache=lambda: calls.append(len(driver.opens)))
        self.assertEqual(calls, [0])

    def test_no_key_is_sent_before_delayed_startup_input_ready(self):
        driver = FakeDriver([{"startup_delay": 2.0}])
        result = self.go(driver, "reload-key")
        self.assertEqual(result["status"], "ok")
        first_key_time = driver.key_times[0][1]
        self.assertGreaterEqual(first_key_time - driver.open_times[0], 2.0)

    def test_reload_key_waits_for_input_ready_before_next_r(self):
        driver = FakeDriver()
        result = self.go(driver, "reload-key")
        self.assertEqual(result["status"], "ok")
        self.assertEqual(driver.keys, ["r"] * 10 + ["q"])
        self.assertEqual(driver.ready_count_at_r, list(range(10)))

    def test_reload_watch_rewrites_identical_bytes_ten_times_each_after_input_ready(self):
        driver = FakeDriver()
        seen = []

        def touch(path):
            before = path.read_bytes()
            run.touch_identical(path)
            seen.append(path.read_bytes() == before)
            driver.on_rewrite()

        result = self.go(driver, "reload-watch", touch=touch)
        self.assertEqual(result["status"], "ok")
        self.assertEqual(seen, [True] * 10)
        self.assertEqual(driver.ready_count_at_rewrite, list(range(10)))
        self.assertEqual(driver.keys, ["q"])
        self.assertEqual(self.fixture.read_text(), "# hi\n")

    def test_scroll_final_g_not_moved_completes_without_a_frame(self):
        driver = FakeDriver([{"final_g_moved": False}])
        result = self.go(driver, "scroll")
        self.assertEqual(result["status"], "ok")
        self.assertEqual(driver.keys[-3:], ["g", "g", "q"])
        self.assertFalse(any(e["ev"] == "frame_unchanged" for e in driver.events_at_q[0]))

    def test_scroll_final_g_moved_waits_for_frame_unchanged(self):
        driver = FakeDriver([{"final_g_moved": True}])
        result = self.go(driver, "scroll")
        self.assertEqual(result["status"], "ok")
        # q is sent only after frame_unchanged was written; a release `g` is not the final key.
        self.assertTrue(any(e["ev"] == "frame_unchanged" for e in driver.events_at_q[0]))

    def test_scroll_final_g_ignores_scroll_frame_at_a_different_y(self):
        driver = FakeDriver([{"final_g_moved": True, "frame_at_other_y_first": True}])
        result = self.go(driver, "scroll")
        self.assertEqual(result["status"], "ok")
        scroll_ys = [e["y"] for e in driver.events_at_q[0] if e["ev"] == "frame_hq" and e["reason"] == "scroll"]
        self.assertEqual(scroll_ys, [300.0, 0.0])

    def test_scroll_key_sequence(self):
        driver = FakeDriver()
        self.go(driver, "scroll")
        self.assertEqual(driver.keys, ["j"] * 60 + ["ctrl-d"] * 20 + ["PageDown", "g", "g", "q"])

    def test_scroll_sends_its_80_paced_keys_in_one_call(self):
        driver = FakeDriver()
        self.go(driver, "scroll")
        self.assertEqual(driver.send_calls[0], (["j"] * 60 + ["ctrl-d"] * 20, run.SCROLL_INTERVAL))

    def test_scroll_records_the_measured_key_gap(self):
        result = self.go(FakeDriver([{"key_gap": 0.05}]), "scroll")
        self.assertEqual(result["status"], "ok")
        self.assertAlmostEqual(result["metrics"]["scroll_key_gap_ms"], 50.0, delta=0.01)

    def test_scroll_with_keys_arriving_too_slowly_is_invalid(self):
        result = self.go(FakeDriver([{"key_gap": 0.15}]), "scroll")
        self.assertEqual(result["status"], "invalid")
        self.assertIn("scroll key gap", result["detail"])
        self.assertAlmostEqual(result["metrics"]["scroll_key_gap_ms"], 150.0, delta=0.01)

    def test_scroll_with_one_paced_press_missing_is_invalid_even_though_pagedown_is_traced(self):
        result = self.go(FakeDriver([{"drop_keys": 1}]), "scroll")
        self.assertEqual(result["status"], "invalid")
        self.assertIn("paced key presses", result["detail"])
        self.assertNotIn("scroll_key_gap_ms", result["metrics"])

    def test_missing_input_ready_times_out_within_limit(self):
        driver = FakeDriver([{"no_ready": True}])
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "timeout")
        self.assertLessEqual(driver.now - 1000.0, run.STARTUP_TIMEOUT + 1)
        self.assertEqual(driver.closed, ["term-0"])

    def test_osascript_timeout_while_sending_is_recorded_as_timeout(self):
        driver = FakeDriver([{"raise_on_send": True}])
        result = self.go(driver, "reload-key")
        self.assertEqual(result["status"], "timeout")
        self.assertEqual(driver.closed, ["term-0"])

    def test_open_timeout_closes_only_the_window_this_launch_created(self):
        driver = FakeDriver([{"leak_on_open": True, "also_create": "user-opened-meanwhile"}])
        driver.preexisting = {"user-window"}
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "timeout")
        self.assertEqual(driver.closed, ["term-0"])
        self.assertNotIn("left open", result["detail"])

    def test_open_timeout_without_cli_start_closes_nothing_and_says_so(self):
        driver = FakeDriver([{"leak_on_open": True, "no_cli_start": True}])
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "timeout")
        self.assertEqual(driver.closed, [])
        self.assertIn("may have been left open", result["detail"])

    def test_ctrl_c_during_open_closes_the_window_and_still_interrupts(self):
        driver = FakeDriver([{"interrupt_on_open": True}])
        with self.assertRaises(KeyboardInterrupt):
            self.go(driver, "startup-cold")
        self.assertEqual(driver.closed, ["term-0"])

    def test_open_timeout_is_still_reported_when_listing_terminals_after_open_fails(self):
        driver = FakeDriver([{"leak_on_open": True}])
        driver.fail_ids_calls = {2}
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "timeout")
        self.assertEqual(driver.closed, [])

    def test_failed_terminal_listing_before_open_means_no_window_is_opened(self):
        driver = FakeDriver()
        driver.fail_ids_calls = {1}
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "timeout")
        self.assertIn("terminal list", result["detail"])
        self.assertEqual(driver.opens, [])
        self.assertEqual(driver.terminals, set())

    def test_terminal_gone_while_sending_is_a_crash_with_tail_saved(self):
        driver = FakeDriver([{"gone_on_send": True}])
        result = self.go(driver, "reload-key")
        self.assertEqual(result["status"], "crash")
        self.assertIn("terminal is gone", result["detail"])
        self.assertIn("cli_start", Path(result["crash_tail"]).read_text())

    def test_process_gone_without_cli_exit_is_crash_with_tail_saved(self):
        driver = FakeDriver([{"crash": True}])
        result = self.go(driver, "startup-cold")
        self.assertEqual(result["status"], "crash")
        self.assertEqual(result["renderer_exit"]["signal"], 9)
        tail = Path(result["crash_tail"]).read_text().splitlines()
        self.assertIn("renderer_exit", tail[-1])

    def test_crash_tail_keeps_only_the_last_20_trace_lines(self):
        driver = FakeDriver([{"crash": True, "extra_lines": 30}])
        result = self.go(driver, "startup-cold")
        tail = Path(result["crash_tail"]).read_text().splitlines()
        self.assertEqual(len(tail), 20)
        self.assertIn("renderer_exit", tail[-1])

    def test_contaminated_run_is_flagged_and_excluded_from_webkit_usage(self):
        result = self.go(
            FakeDriver(), "startup-cold", sampler_factory=lambda p: FakeSampler(p, still_alive={99})
        )
        metrics = result["metrics"]
        self.assertEqual(metrics["webkit_contaminated"], 1)
        self.assertIsNone(metrics["cpu_s_webkit"])
        self.assertIsNone(metrics["peak_rss_kb_webkit"])
        self.assertEqual(metrics["cpu_s_doctty"], 1.5)

    def test_unsampled_process_is_reported_not_counted_as_zero(self):
        result = self.go(
            FakeDriver(), "startup-cold", sampler_factory=lambda p: FakeSampler(p, still_alive=set())
        )
        self.assertEqual(result["unsampled"], {"renderer": [2000]})
        self.assertIsNone(result["metrics"]["cpu_s_renderer"])
        self.assertIsNone(result["metrics"]["peak_rss_kb_renderer"])

    def test_clean_run_reports_webkit_usage(self):
        result = self.go(
            FakeDriver(), "startup-cold", sampler_factory=lambda p: FakeSampler(p, still_alive=set())
        )
        metrics = result["metrics"]
        self.assertEqual(metrics["webkit_contaminated"], 0)
        self.assertEqual(metrics["cpu_s_webkit"], 12.0)
        self.assertEqual(metrics["peak_rss_kb_webkit"], 3456)


class CollectMetricsTest(unittest.TestCase):
    def events(self, helper_pids):
        return [
            {"t_us": 0, "ev": "cli_start", "pid": 1},
            {"t_us": 1, "ev": "renderer_start", "pid": 2},
            *({"t_us": 2, "ev": "helper_spawned", "pid": pid} for pid in helper_pids),
        ]

    def row(self, t, pid, cpu_s, rss_kb):
        return {"t": t, "pid": pid, "comm": "x", "cpu_s": cpu_s, "rss_kb": rss_kb}

    def test_every_spawned_helper_counts_toward_cpu_and_peak_rss(self):
        rows = [
            self.row(1.0, 10, 1.0, 100),
            self.row(2.0, 10, 1.0, 100),
            self.row(2.0, 11, 2.0, 200),
            self.row(3.0, 12, 3.0, 400),
        ]
        metrics, unsampled = run.collect_metrics("startup-cold", self.events([10, 11, 12]), rows, set())
        self.assertEqual(metrics["cpu_s_helper"], 6.0)
        self.assertEqual(metrics["peak_rss_kb_helper"], 400)
        self.assertEqual(unsampled, {"doctty": [1], "renderer": [2]})

    def test_zero_samples_make_every_recorded_pid_unsampled(self):
        metrics, unsampled = run.collect_metrics("startup-cold", self.events([10]), [], set())
        self.assertIsNone(metrics["cpu_s_helper"])
        self.assertIsNone(metrics["peak_rss_kb_doctty"])
        self.assertEqual(unsampled, {"doctty": [1], "renderer": [2], "helper": [10]})

    def test_a_helper_that_was_never_sampled_makes_its_usage_unknown(self):
        rows = [self.row(1.0, 10, 1.0, 100)]
        metrics, unsampled = run.collect_metrics("startup-cold", self.events([10, 99]), rows, set())
        self.assertIsNone(metrics["cpu_s_helper"])
        self.assertIsNone(metrics["peak_rss_kb_helper"])
        self.assertEqual(unsampled["helper"], [99])


class PidTtyTest(unittest.TestCase):
    def tty(self, stdout):
        completed = subprocess.CompletedProcess([], 0, stdout=stdout)
        with mock.patch("run.subprocess.run", return_value=completed):
            return run.pid_tty(1)

    def test_ps_tty_name_becomes_a_device_path(self):
        self.assertEqual(self.tty("ttys016\n"), "/dev/ttys016")

    def test_no_controlling_tty_gives_none(self):
        self.assertIsNone(self.tty("??\n"))


class SettledArgvTest(unittest.TestCase):
    """Runs the real wrapper under /bin/sh with a fake stty that replays a size sequence."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.reads = self.dir / "reads"
        stty = self.dir / "stty"
        # Prints line N of sizes on the Nth call, repeating the last line after that.
        stty.write_text(
            '#!/bin/sh\necho x >> "$READS"\nn=$(wc -l < "$READS")\n'
            'line=$(sed -n "${n}p" "$SIZES")\n[ -n "$line" ] || line=$(tail -n 1 "$SIZES")\necho "$line"\n'
        )
        stty.chmod(0o755)
        self.target = self.dir / "target"
        self.target.write_text('#!/bin/sh\nfor a in "$@"; do echo "$a"; done > "$ARGS_OUT"\n')
        self.target.chmod(0o755)

    def launch(self, sizes, reads=4, limit=10):
        (self.dir / "sizes").write_text("\n".join(sizes) + "\n")
        argv = run.settled_argv([str(self.target), "a b", "--theme"], reads=reads, limit=limit, poll=0)
        env = {
            **os.environ,
            "PATH": f"{self.dir}:{os.environ['PATH']}",
            "READS": str(self.reads),
            "SIZES": str(self.dir / "sizes"),
            "ARGS_OUT": str(self.dir / "args"),
        }
        subprocess.run(argv, env=env, check=True, timeout=10)
        return len(self.reads.read_text().splitlines()), (self.dir / "args").read_text().splitlines()

    def test_starts_once_the_size_repeats_after_a_resize(self):
        count, args = self.launch(["20 56"] * 3 + ["59 204"] * 5)
        self.assertEqual(count, 7)  # 3 reads of the old size do not count toward the 4 of the new one
        self.assertEqual(args, ["a b", "--theme"])

    def test_starts_anyway_after_the_limit_when_the_size_keeps_changing(self):
        count, args = self.launch(["20 56", "59 204"] * 10, limit=7)
        self.assertEqual(count, 7)
        self.assertEqual(args, ["a b", "--theme"])

    def test_an_unreadable_size_never_counts_as_settled(self):
        count, _ = self.launch([""], limit=6)
        self.assertEqual(count, 6)

    def test_launch_argv_wraps_the_doctty_command(self):
        argv = run.launch_argv(Path("/b/doctty"), Path("/f/small.md"), "dark", True, False)
        self.assertEqual(argv[0], "/bin/sh")
        self.assertEqual(argv[-5:], ["/b/doctty", "--theme", "dark", "--watch", "/f/small.md"])


class RunScenarioSetTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)
        self.fixtures = self.dir / "fixtures"
        self.fixtures.mkdir()
        for name in ("small", "mermaid-images"):
            (self.fixtures / f"{name}.md").write_text("# x\n")

    def go(self, driver, scenario, fixture, runs=5, mermaid=False):
        return run.run_scenario_set(
            driver, scenario, self.fixtures / f"{fixture}.md", self.dir, runs,
            mermaid=mermaid, clock=driver.clock, sleep=driver.sleep,
        )

    def test_three_failed_runs_fail_the_scenario(self):
        crash = {"crash": True}
        driver = FakeDriver([crash, crash, crash])
        result = self.go(driver, "startup-cold", "small")
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["run_statuses"], ["crash"] * 3 + ["ok"] * 2)

    def test_a_run_whose_terminal_vanished_is_a_crash_and_the_next_run_still_executes(self):
        result = self.go(FakeDriver([{"gone_on_send": True}]), "reload-key", "small", runs=2)
        self.assertEqual(result["run_statuses"], ["crash", "ok"])

    def test_two_failed_runs_do_not_fail_the_scenario(self):
        crash = {"crash": True}
        result = self.go(FakeDriver([crash, crash]), "startup-cold", "small")
        self.assertEqual(result["status"], "ok")
        self.assertEqual(len(result["runs"]), 3)

    def test_a_timed_out_launch_is_recorded_and_the_next_run_still_starts(self):
        result = self.go(FakeDriver([{"raise_on_open": True}]), "startup-cold", "small", runs=2)
        self.assertEqual(result["run_statuses"], ["timeout", "ok"])

    def test_without_mermaid_cmd_every_launch_disables_mermaid_and_fixture_is_skipped(self):
        driver = FakeDriver()
        self.assertEqual(self.go(driver, "startup-cold", "mermaid-images")["status"], "skipped")
        self.assertEqual(driver.opens, [])
        self.go(driver, "reload-key", "small", runs=2)
        self.assertEqual(len(driver.opens), 2)
        for opened in driver.opens:
            self.assertIn("--no-mermaid", opened["argv"])

    def test_with_mermaid_cmd_no_launch_disables_mermaid(self):
        driver = FakeDriver()
        result = self.go(driver, "startup-cold", "mermaid-images", runs=2, mermaid=True)
        self.assertEqual(result["status"], "ok")
        for opened in driver.opens:
            self.assertNotIn("--no-mermaid", opened["argv"])

    def test_warm_scenario_launches_one_unmeasured_warm_up_first(self):
        driver = FakeDriver([{"source": "extracted"}] + [{"source": "cache_hit"}] * 2)
        result = self.go(driver, "startup-warm", "small", runs=2)
        self.assertEqual(len(driver.opens), 3)
        self.assertEqual(result["status"], "ok")
        self.assertEqual(len(result["runs"]), 2)

    def test_invalid_run_keeps_its_diagnostics_but_stays_out_of_the_aggregate(self):
        driver = FakeDriver([{"key_gap": 0.15}, {}])
        result = self.go(driver, "scroll", "small", runs=2)
        self.assertEqual(result["run_statuses"], ["invalid", "ok"])
        self.assertEqual(len(result["runs"]), 1)
        self.assertAlmostEqual(result["metrics"]["scroll_key_gap_ms"]["median"], 50.0, delta=0.01)
        self.assertEqual(result["unsampled"], [{}])
        (invalid,) = result["failed_runs"]
        self.assertEqual(invalid["status"], "invalid")
        self.assertIn("scroll key gap", invalid["detail"])
        self.assertAlmostEqual(invalid["metrics"]["scroll_key_gap_ms"], 150.0, delta=0.01)

    def test_watch_scenario_launches_with_watch_flag(self):
        driver = FakeDriver()
        self.go(driver, "reload-watch", "small", runs=1)
        self.assertIn("--watch", driver.opens[0]["argv"])


class RunAllTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.out = Path(self._tmp.name) / "out"
        self.out.mkdir()
        self.fixtures = Path(self._tmp.name) / "fixtures"
        self.fixtures.mkdir()
        (self.fixtures / "small.md").write_text("# x\n")

    def go(self, driver, *, scenarios=(), runs=3, pixels=False):
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
            code = run.run_all(
                driver, self.out, self.fixtures, {"theme": "light"},
                scenarios=list(scenarios), fixture_names=["small"], runs=runs,
                mermaid_cmd=None, font_size=13.0, theme="light", pixels=pixels,
                clock=driver.clock, sleep=driver.sleep, sampler_factory=None, clear_cache=None,
            )
        return code, stderr.getvalue()

    def test_ctrl_c_keeps_finished_runs_closes_the_window_and_exits_130(self):
        driver = FakeDriver([{}, {"interrupt_on_send": True}])
        code, _ = self.go(driver, scenarios=["startup-cold"])
        self.assertEqual(code, 130)
        self.assertEqual(driver.closed, ["term-0", "term-1"])
        entry = json.loads((self.out / "summary.json").read_text())["startup-cold/small"]
        self.assertEqual(entry["status"], "partial")
        self.assertEqual(entry["run_statuses"], ["ok"])
        self.assertEqual(len(entry["runs"]), 1)
        self.assertTrue((self.out / "meta.json").is_file())

    def summary_metrics(self, scenario):
        driver = FakeDriver([{"scroll_frames": True}] * 2)
        code, _ = self.go(driver, scenarios=[scenario], runs=2)
        self.assertEqual(code, 0)
        entry = json.loads((self.out / "summary.json").read_text())[f"{scenario}/small"]
        return {name: m["median"] for name, m in entry["metrics"].items()}

    def test_reload_latency_reaches_summary_json(self):
        metrics = self.summary_metrics("reload-key")
        self.assertAlmostEqual(metrics["reload_ms"], 300.0, delta=0.01)
        self.assertEqual(metrics["reloads_measured"], 10)

    def test_scroll_latency_reaches_summary_json(self):
        metrics = self.summary_metrics("scroll")
        self.assertAlmostEqual(metrics["scroll_p50_ms"], 100.0, delta=0.01)
        self.assertAlmostEqual(metrics["scroll_p95_ms"], 100.0, delta=0.01)
        self.assertEqual(metrics["scroll_measured"], 80)
        self.assertEqual(metrics["scroll_missing"], 0)

    def test_unsampled_pids_reach_summary_json(self):
        driver = FakeDriver()
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
            run.run_all(
                driver, self.out, self.fixtures, {"theme": "light"},
                scenarios=["startup-cold"], fixture_names=["small"], runs=1,
                mermaid_cmd=None, font_size=13.0, theme="light", pixels=False,
                clock=driver.clock, sleep=driver.sleep,
                sampler_factory=lambda p: FakeSampler(p, still_alive=set()), clear_cache=None,
            )
        entry = json.loads((self.out / "summary.json").read_text())["startup-cold/small"]
        self.assertEqual(entry["unsampled"], [{"renderer": [2000]}])

    def test_failed_scenario_exits_1(self):
        crash = {"crash": True}
        code, _ = self.go(FakeDriver([crash, crash, crash]), scenarios=["startup-cold"])
        self.assertEqual(code, 1)

    def test_pixel_files_are_copied_per_fixture_and_theme(self):
        driver = FakeDriver([{"pixel": b"LIGHT"}, {"pixel": b"DARK"}])
        code, _ = self.go(driver, pixels=True)
        self.assertEqual(code, 0)
        pixels = self.out / "pixels"
        self.assertEqual((pixels / "small-light.rgbz").read_bytes(), b"LIGHT")
        self.assertEqual((pixels / "small-dark.rgbz").read_bytes(), b"DARK")
        self.assertEqual(json.loads((pixels / "small-dark.json").read_text())["width"], 2)
        self.assertTrue(driver.opens[0]["env"]["DOCTTY_BENCH_PIXEL_DIR"])
        self.assertIn("light", driver.opens[0]["argv"])
        self.assertIn("dark", driver.opens[1]["argv"])

    def test_pixel_capture_failure_is_reported_and_exits_nonzero(self):
        driver = FakeDriver([{"pixel": b"LIGHT"}, {}])
        code, stderr = self.go(driver, pixels=True)
        self.assertEqual(code, 1)
        self.assertIn("small-dark", stderr)
        self.assertTrue((self.out / "pixels" / "small-light.rgbz").is_file())
        self.assertFalse((self.out / "pixels" / "small-dark.rgbz").exists())


    def test_rejected_pixel_capture_does_not_shift_the_reference_size(self):
        # One scenario launch, then a stale light capture: a tie must not make 56x20 the reference.
        stale = {"pixel": b"LIGHT", "geometry": [(56, 20, 2)]}
        driver = FakeDriver([{}, stale, {"pixel": b"DARK"}])
        code, stderr = self.go(driver, scenarios=["startup-cold"], runs=1, pixels=True)
        self.assertEqual(code, 1)
        self.assertIn("small-light", stderr)
        self.assertNotIn("small-dark", stderr)
        self.assertTrue((self.out / "pixels" / "small-dark.rgbz").is_file())
        self.assertEqual(json.loads((self.out / "meta.json").read_text())["geometry"]["cols"], 120)

    def test_pixel_capture_at_a_different_terminal_size_fails(self):
        # Real run: one pixel launch stayed at 56x20 while every other launch ran at 120x40.
        stale = {"pixel": b"LIGHT", "geometry": [(56, 20, 2)]}
        driver = FakeDriver([{}, {}, stale, {"pixel": b"DARK"}])
        code, stderr = self.go(driver, scenarios=["startup-cold"], runs=2, pixels=True)
        self.assertEqual(code, 1)
        self.assertIn("small-light", stderr)
        self.assertIn("(56, 20)", stderr)
        self.assertFalse((self.out / "pixels" / "small-light.rgbz").exists())
        self.assertTrue((self.out / "pixels" / "small-dark.rgbz").is_file())

    def test_capped_tile_width_at_the_usual_terminal_size_is_kept(self):
        # The renderer caps tile width under image-size limits; that is not a size change.
        driver = FakeDriver([{}, {}, {"pixel": b"LIGHT", "pixel_width": 1}, {"pixel": b"DARK"}])
        code, _ = self.go(driver, scenarios=["startup-cold"], runs=2, pixels=True)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads((self.out / "pixels" / "small-light.json").read_text())["width"], 1)


class PreflightTest(unittest.TestCase):
    class Driver:
        def __init__(self, denied=False, hangs=False):
            self.denied = denied
            self.hangs = hangs

        def check_access(self):
            if self.denied:
                raise ghostty.AutomationDenied("-1743")
            if self.hangs:
                raise ghostty.DriverTimeout("no answer")

    def check(self, system="Darwin", version="Ghostty 1.4.0", build_ok=True, denied=False, hangs=False):
        built = []
        stderr = io.StringIO()
        code = run.preflight(
            system, lambda: version, lambda: built.append(1) or build_ok, self.Driver(denied, hangs), stderr
        )
        return code, built, stderr.getvalue()

    def test_exit_codes(self):
        cases = [
            ({"system": "Linux"}, 2),
            ({"version": None}, 2),
            ({"version": "Ghostty 1.3.9"}, 2),
            ({"version": "Ghostty 1.4.0"}, None),
            ({"version": "Ghostty 2.0.1"}, None),
            ({"build_ok": False}, 1),
            ({"denied": True}, 1),
            ({"hangs": True}, 1),
        ]
        for kwargs, expected in cases:
            with self.subTest(kwargs=kwargs):
                self.assertEqual(self.check(**kwargs)[0], expected)

    def test_build_is_not_started_when_the_environment_is_unsupported(self):
        for kwargs in ({"system": "Linux"}, {"version": "Ghostty 1.3.9"}, {"version": None}):
            with self.subTest(kwargs=kwargs):
                self.assertEqual(self.check(**kwargs)[1], [])

    def test_permission_denial_prints_the_steps(self):
        _, _, stderr = self.check(denied=True)
        self.assertIn("-1743", stderr)
        self.assertIn("Automation", stderr)


class MmdcVersionTest(unittest.TestCase):
    def test_command_path_with_spaces_is_one_executable(self):
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "Mermaid CLI" / "mmdc"
            script.parent.mkdir()
            script.write_text('#!/bin/sh\necho "mmdc 9.8.7 $1"\n')
            script.chmod(0o755)
            self.assertEqual(run.mmdc_version(str(script)), "mmdc 9.8.7 --version")

    def test_missing_executable_gives_none(self):
        self.assertIsNone(run.mmdc_version("/nonexistent/mmdc"))


class PsTest(unittest.TestCase):
    def test_comm_is_the_last_column_so_ps_does_not_truncate_it(self):
        line = " 4242   0:01.50  20480 /Users/a b/Library/Caches/doctty/native/doctty-webkit-helper\n"
        completed = subprocess.CompletedProcess([], 0, stdout=line)
        with mock.patch("run.subprocess.run", return_value=completed) as ps:
            rows = run._ps()
        self.assertEqual(ps.call_args.args[0], ["ps", "-axo", "pid=,cputime=,rss=,comm="])
        self.assertEqual(
            rows,
            [
                {
                    "pid": 4242,
                    "comm": "/Users/a b/Library/Caches/doctty/native/doctty-webkit-helper",
                    "cpu_s": 1.5,
                    "rss_kb": 20480,
                }
            ],
        )
        self.assertTrue(run._is_interesting(rows[0]))


class OsascriptTest(unittest.TestCase):
    def test_page_down_is_sent_with_ghosttys_key_name(self):
        with mock.patch("ghostty._osascript", return_value="") as osascript:
            ghostty.GhosttyDriver().send_keys("term-1", ["PageDown", "ctrl-d"])
        self.assertEqual(osascript.call_args.args[1:], ("term-1", "0.0", "pageDown", "", "d", "control"))

    def test_subprocess_timeout_becomes_driver_timeout(self):
        expired = subprocess.TimeoutExpired(cmd="osascript", timeout=15)
        with mock.patch("ghostty.subprocess.run", side_effect=expired):
            with self.assertRaises(ghostty.DriverTimeout):
                ghostty.GhosttyDriver().check_access()


class FindGeometryTest(unittest.TestCase):
    @staticmethod
    def write(out, name, *sizes):
        lines = [
            json.dumps({"t_us": i, "ev": "geometry", "cols": c, "rows": r, "px_w": w, "px_h": h})
            for i, (c, r, w, h) in enumerate(sizes)
        ]
        (out / f"trace-{name}.jsonl").write_text("\n".join(lines) + "\n")

    def test_no_geometry_gives_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(run.find_geometry(Path(tmp)))

    def test_pixel_launches_count_only_when_asked(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            self.write(out, "pixels-small-1", (56, 20, 2856, 1712))
            self.assertIsNone(run.find_geometry(out))
            self.assertEqual(run.find_geometry(out, pixels=True)["cols"], 56)

    def test_most_launches_win_and_resized_launches_are_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            # Name order puts the resized and the odd launch first.
            self.write(out, "a-resized-1", (56, 20, 2856, 1712), (204, 59, 2856, 1712))
            self.write(out, "b-odd-1", (56, 20, 2856, 1712))
            self.write(out, "c-small-1", (204, 59, 2856, 1712))
            self.write(out, "d-small-2", (204, 59, 2856, 1712))
            self.assertEqual(run.find_geometry(out), {"cols": 204, "rows": 59, "px_w": 2856, "px_h": 1712})

class TouchIdenticalTest(unittest.TestCase):
    def test_rewrites_same_bytes_and_bumps_mtime(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "a.md"
            path.write_bytes(b"same\n")
            os.utime(path, (1_000_000, 1_000_000))
            run.touch_identical(path)
            self.assertEqual(path.read_bytes(), b"same\n")
            self.assertGreater(path.stat().st_mtime, 1_000_000)


class NativeCacheDirTest(unittest.TestCase):
    def test_resolves_exact_directory_under_home(self):
        self.assertEqual(
            run.native_cache_dir({"HOME": "/Users/someone"}),
            Path("/Users/someone/Library/Caches/doctty/native"),
        )

    def test_refuses_unsafe_home(self):
        for env in ({}, {"HOME": ""}, {"HOME": "relative/path"}, {"HOME": "/"}):
            with self.subTest(env=env), self.assertRaises(run.UnsafeCachePath):
                run.native_cache_dir(env)

    def test_clear_removes_only_the_native_directory(self):
        with tempfile.TemporaryDirectory() as home:
            native = Path(home) / "Library/Caches/doctty/native"
            sibling = Path(home) / "Library/Caches/doctty/other"
            native.joinpath("deep").mkdir(parents=True)
            native.joinpath("deep/f").write_text("x")
            sibling.mkdir(parents=True)
            run.clear_native_cache({"HOME": home})
            self.assertFalse(native.exists())
            self.assertTrue(sibling.is_dir())

    def test_clear_without_native_directory_leaves_everything_else(self):
        with tempfile.TemporaryDirectory() as home:
            sibling = Path(home) / "Library/Caches/doctty/other"
            sibling.mkdir(parents=True)
            run.clear_native_cache({"HOME": home})
            self.assertFalse((Path(home) / "Library/Caches/doctty/native").exists())
            self.assertTrue(sibling.is_dir())

    def test_clear_refuses_a_symlink_at_any_level_below_home(self):
        for link_rel in ("Library", "Library/Caches", "Library/Caches/doctty", "Library/Caches/doctty/native"):
            with self.subTest(link=link_rel), tempfile.TemporaryDirectory() as home, tempfile.TemporaryDirectory() as outside:
                # `outside` holds the real tree; the symlink at link_rel points at the matching part of it.
                real = Path(outside)
                (real / "Library/Caches/doctty/native").mkdir(parents=True)
                (real / "Library/Caches/doctty/native/f").write_text("keep")
                link = Path(home) / link_rel
                link.parent.mkdir(parents=True, exist_ok=True)
                link.symlink_to(real / link_rel)
                with self.assertRaises(run.UnsafeCachePath):
                    run.clear_native_cache({"HOME": home})
                self.assertEqual((real / "Library/Caches/doctty/native/f").read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
