import json
import tempfile
import unittest
from pathlib import Path

import stats


def ev(name, t_ms, **fields):
    """One trace event at `t_ms` milliseconds (stored as t_us like the real trace)."""
    return {"t_us": int(t_ms * 1000), "proc": "renderer", "ev": name, **fields}


def key(t_ms, target_y, moved=True, kind="press", from_y=0.0):
    return ev("key", t_ms, key="j", kind=kind, seq=1, from_y=from_y, target_y=target_y, moved=moved)


def frame(t_ms, y, reason="scroll"):
    return ev("frame_hq", t_ms, reason=reason, seq=1, y=y)


def ps(t, pid, cpu_s, rss_kb, comm="x"):
    return {"t": t, "pid": pid, "comm": comm, "cpu_s": cpu_s, "rss_kb": rss_kb}


class ScrollLatencies(unittest.TestCase):
    def test_each_moving_key_followed_by_its_frame_gets_an_exact_latency(self):
        events = [
            key(100, 50.0), frame(112, 50.0),
            key(200, 100.0), frame(231, 100.0),
            key(300, 150.0), frame(307, 150.0),
        ]
        result = stats.scroll_latencies(events)
        self.assertEqual(result["latencies_ms"], [12.0, 31.0, 7.0])
        self.assertEqual(
            (result["nonmoving"], result["already_displayed"], result["coalesced"], result["missing"]),
            (0, 0, 0, 0),
        )

    def test_clamped_key_is_counted_nonmoving_and_not_measured(self):
        result = stats.scroll_latencies([key(100, 0.0, moved=False)])
        self.assertEqual(result["nonmoving"], 1)
        self.assertEqual(result["latencies_ms"], [])
        self.assertEqual(result["missing"], 0)

    def test_earlier_key_is_coalesced_when_only_the_later_keys_frame_appears(self):
        events = [key(100, 50.0), key(110, 100.0), frame(140, 100.0)]
        result = stats.scroll_latencies(events)
        self.assertEqual(result["coalesced"], 1)
        self.assertEqual(result["latencies_ms"], [30.0])
        self.assertEqual(result["missing"], 0)

    def test_key_without_any_frame_before_the_end_is_missing(self):
        result = stats.scroll_latencies([key(100, 50.0), frame(120, 50.0), key(200, 90.0)])
        self.assertEqual(result["missing"], 1)
        self.assertEqual(result["latencies_ms"], [20.0])

    def test_reload_frame_at_the_target_is_not_a_scroll_frame(self):
        result = stats.scroll_latencies([key(100, 50.0), frame(120, 50.0, reason="reload")])
        self.assertEqual(result["latencies_ms"], [])
        self.assertEqual(result["missing"], 1)

    def test_release_key_is_not_counted_in_any_class(self):
        result = stats.scroll_latencies([key(100, 50.0, kind="release"), key(110, 0.0, moved=False, kind="release")])
        self.assertEqual(
            result,
            {"latencies_ms": [], "nonmoving": 0, "already_displayed": 0, "coalesced": 0, "missing": 0},
        )

    def test_key_resolved_by_frame_unchanged_is_already_displayed(self):
        events = [key(100, 50.0), ev("frame_unchanged", 101, y=50.0)]
        result = stats.scroll_latencies(events)
        self.assertEqual(result["already_displayed"], 1)
        self.assertEqual(result["latencies_ms"], [])
        self.assertEqual(result["missing"], 0)

    def test_frame_within_half_a_pixel_matches_and_half_a_pixel_does_not(self):
        hit = stats.scroll_latencies([key(100, 50.0), frame(110, 50.49)])
        self.assertEqual(hit["latencies_ms"], [10.0])
        miss = stats.scroll_latencies([key(100, 50.0), frame(110, 50.5)])
        self.assertEqual(miss["missing"], 1)

    def test_repeat_key_is_measured_like_a_press(self):
        result = stats.scroll_latencies([key(100, 50.0, kind="repeat"), frame(125, 50.0)])
        self.assertEqual(result["latencies_ms"], [25.0])

    def test_no_events_gives_zero_counts(self):
        self.assertEqual(
            stats.scroll_latencies([]),
            {"latencies_ms": [], "nonmoving": 0, "already_displayed": 0, "coalesced": 0, "missing": 0},
        )


class Percentile(unittest.TestCase):
    def test_empty_is_none(self):
        self.assertIsNone(stats.percentile([], 50))

    def test_single_value_is_every_percentile(self):
        self.assertEqual(stats.percentile([7.0], 50), 7.0)
        self.assertEqual(stats.percentile([7.0], 95), 7.0)

    def test_nearest_rank_on_twenty_unsorted_values(self):
        values = [float(v) for v in range(20, 0, -1)]
        self.assertEqual(stats.percentile(values, 50), 10.0)  # rank ceil(0.5*20) = 10
        self.assertEqual(stats.percentile(values, 95), 19.0)  # rank ceil(0.95*20) = 19
        self.assertEqual(stats.percentile(values, 100), 20.0)


class ReloadLatencies(unittest.TestCase):
    def test_two_reloads_give_two_latencies_and_startup_frame_is_ignored(self):
        events = [
            frame(50, 0.0, reason="startup"),
            ev("reload_start", 100, trigger="key"), frame(160, 0.0, reason="reload"),
            ev("reload_start", 200, trigger="watch"), frame(230, 0.0, reason="startup"),
            frame(275, 0.0, reason="reload"),
        ]
        self.assertEqual(stats.reload_latencies(events), [60.0, 75.0])

    def test_reload_without_a_frame_yields_nothing(self):
        self.assertEqual(stats.reload_latencies([ev("reload_start", 100, trigger="key")]), [])


class StartupMetrics(unittest.TestCase):
    def test_stage_deltas_are_measured_from_cli_start_using_first_occurrence(self):
        events = [
            ev("cli_start", 1000, pid=1),
            ev("cli_args_parsed", 1005),
            ev("renderer_start", 1040),
            ev("html_built", 1070),
            ev("helper_ready", 1300),
            ev("outline_ready", 1310),
            ev("snapshot", 1350),
            ev("kitty_upload", 1360),
            ev("kitty_upload", 1900),
            frame(1400, 0.0, reason="startup"),
        ]
        self.assertEqual(
            stats.startup_metrics(events),
            {
                "cli_args_parsed": 5.0, "renderer_start": 40.0, "html_built": 70.0,
                "helper_ready": 300.0, "outline_ready": 310.0, "snapshot": 350.0,
                "kitty_upload": 360.0, "frame_hq": 400.0,
            },
        )

    def test_missing_stage_is_none_and_missing_cli_start_does_not_crash(self):
        events = [ev("cli_start", 1000, pid=1), ev("renderer_start", 1040)]
        result = stats.startup_metrics(events)
        self.assertEqual(result["renderer_start"], 40.0)
        self.assertIsNone(result["frame_hq"])
        self.assertIsNone(result["outline_ready"])
        self.assertTrue(all(v is None for v in stats.startup_metrics([]).values()))


class Counters(unittest.TestCase):
    def test_hits_misses_skips_and_bytes_are_summed(self):
        events = [
            ev("cache", 1, hit=True), ev("cache", 2, hit=True), ev("cache", 3, hit=False),
            ev("prefetch_stale_skip", 4), ev("prefetch_stale_skip", 5),
            ev("upload_skipped", 6),
            ev("kitty_upload", 7, bytes=1000, ms=1.0, kind="tile"),
            ev("kitty_upload", 8, bytes=234, ms=1.0, kind="preview"),
        ]
        self.assertEqual(
            stats.counters(events),
            {"cache_hit": 2, "cache_miss": 1, "prefetch_stale_skip": 2, "upload_skipped": 1, "upload_bytes": 1234},
        )


class ProcessUsage(unittest.TestCase):
    PIDS = {"cli": {10}, "renderer": {20}, "helper": {30, 31, 32}}

    def test_cpu_is_the_last_cumulative_sample_with_zero_baseline(self):
        rows = [ps(1, 20, 4.0, 100), ps(2, 20, 5.5, 100), ps(1, 10, 0.25, 10)]
        usage = stats.process_usage(rows, self.PIDS, set())
        self.assertEqual(usage["renderer"]["cpu_s"], 5.5)
        self.assertEqual(usage["cli"]["cpu_s"], 0.25)

    def test_helper_pids_across_reloads_are_summed(self):
        rows = [ps(1, 30, 1.0, 50), ps(2, 31, 2.0, 60), ps(3, 32, 3.0, 70)]
        usage = stats.process_usage(rows, self.PIDS, set())
        self.assertEqual(usage["helper"]["cpu_s"], 6.0)

    def test_recorded_pid_never_sampled_is_unsampled_and_adds_nothing(self):
        rows = [ps(1, 30, 1.0, 50)]
        usage = stats.process_usage(rows, self.PIDS, set())
        self.assertEqual(usage["helper"]["unsampled"], [31, 32])
        self.assertEqual(usage["helper"]["cpu_s"], 1.0)
        self.assertEqual(usage["cli"]["unsampled"], [10])
        self.assertEqual(usage["cli"]["cpu_s"], 0.0)
        self.assertEqual(usage["cli"]["peak_rss_kb"], 0)

    def test_peak_rss_is_the_largest_per_time_sum_of_the_group(self):
        rows = [
            ps(1, 30, 0.1, 100), ps(1, 31, 0.1, 200),
            ps(2, 30, 0.2, 400), ps(2, 31, 0.2, 50),
            ps(3, 30, 0.3, 10),
        ]
        usage = stats.process_usage(rows, self.PIDS, set())
        self.assertEqual(usage["helper"]["peak_rss_kb"], 450)

    def test_webkit_group_uses_only_the_given_new_pids(self):
        rows = [ps(1, 900, 2.0, 300), ps(1, 901, 9.0, 999)]
        usage = stats.process_usage(rows, self.PIDS, {900})
        self.assertEqual(usage["webkit"]["cpu_s"], 2.0)
        self.assertEqual(usage["webkit"]["peak_rss_kb"], 300)
        self.assertEqual(usage["webkit"]["unsampled"], [])


class Summarize(unittest.TestCase):
    def runs(self, low, high):
        return [{"m": v} for v in (low, 95.0, 100.0, 105.0, high)]

    def test_spread_of_exactly_point_two_is_not_high(self):
        result = stats.summarize(self.runs(90.0, 110.0))["m"]
        self.assertEqual((result["median"], result["min"], result["max"]), (100.0, 90.0, 110.0))
        self.assertFalse(result["high_spread"])

    def test_spread_just_above_point_two_is_high(self):
        self.assertTrue(stats.summarize(self.runs(90.0, 111.0))["m"]["high_spread"])

    def test_zero_median_is_not_high_spread(self):
        runs = [{"m": v} for v in (0.0, 0.0, 0.0, 1.0, 2.0)]
        self.assertFalse(stats.summarize(runs)["m"]["high_spread"])

    def test_none_values_are_skipped_and_all_none_gives_none_stats(self):
        result = stats.summarize([{"a": 10.0, "b": None}, {"a": None, "b": None}, {"a": 30.0, "b": None}])
        self.assertEqual(result["a"]["median"], 20.0)
        self.assertEqual(result["b"], {"median": None, "min": None, "max": None, "high_spread": False})


class LoadJsonl(unittest.TestCase):
    def test_reads_one_object_per_line_and_skips_blank_lines(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "t.jsonl"
            path.write_text(json.dumps({"a": 1}) + "\n\n" + json.dumps({"a": 2}) + "\n")
            self.assertEqual(stats.load_jsonl(path), [{"a": 1}, {"a": 2}])


if __name__ == "__main__":
    unittest.main()
