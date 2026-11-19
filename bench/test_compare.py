import contextlib
import io
import json
import tempfile
import unittest
import zlib
from pathlib import Path

import compare

META = {
    "commit": "abc",
    "dirty": False,
    "host_model": "Mac14,2",
    "macos": "15.1",
    "ghostty": "Ghostty 1.4.0",
    "geometry": {"cols": 100, "rows": 40, "px_w": 1000, "px_h": 800},
    "fixtures": {"small.md": "h1", "assets/a.png": "h2"},
    "mmdc": None,
    "mermaid": "disabled",
    "font_size": 13,
    "theme": "light",
}


def metric(median, low=None, high=None):
    low = median if low is None else low
    high = median if high is None else high
    return {"median": median, "min": low, "max": high, "high_spread": False}


def write_result(directory: Path, meta=None, summary=None, pixels=None):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "meta.json").write_text(json.dumps(META if meta is None else meta))
    summary = {} if summary is None else summary
    (directory / "summary.json").write_text(json.dumps(summary))
    for name, (width, height, rgb) in (pixels or {}).items():
        (directory / "pixels").mkdir(exist_ok=True)
        (directory / "pixels" / f"{name}.rgbz").write_bytes(zlib.compress(rgb))
        (directory / "pixels" / f"{name}.json").write_text(
            json.dumps({"width": width, "height": height, "format": "rgb24+zlib"})
        )


def run_main(before: Path, after: Path, *extra):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = compare.main([str(before), str(after), *extra])
    return code, out.getvalue(), err.getvalue()


class MetaMismatchTest(unittest.TestCase):
    def test_identical_meta_has_no_mismatch(self):
        self.assertEqual(compare.meta_mismatches(META, dict(META)), [])

    def test_each_comparable_key_is_reported(self):
        changes = {
            "geometry": {"cols": 120, "rows": 40, "px_w": 1000, "px_h": 800},
            "fixtures": {"small.md": "other", "assets/a.png": "h2"},
            "host_model": "Mac15,1",
            "macos": "15.2",
            "ghostty": "Ghostty 1.5.0",
            "theme": "dark",
            "font_size": 14,
            "mermaid": "enabled",
        }
        for key, value in changes.items():
            with self.subTest(key=key):
                self.assertEqual(compare.meta_mismatches(META, {**META, key: value}), [key])

    def test_commit_and_dirty_differences_are_not_mismatches(self):
        other = {**META, "commit": "def", "dirty": True, "mmdc": "11.0"}
        self.assertEqual(compare.meta_mismatches(META, other), [])


class CompareSummariesTest(unittest.TestCase):
    def rows(self, before, after):
        return compare.compare_summaries(
            {"startup-cold/small": {"metrics": {"frame_hq": before}}},
            {"startup-cold/small": {"metrics": {"frame_hq": after}}},
        )

    def test_row_carries_scenario_fixture_and_delta(self):
        (row,) = self.rows(metric(100.0), metric(110.0))
        self.assertEqual(row["scenario"], "startup-cold")
        self.assertEqual(row["fixture"], "small")
        self.assertEqual(row["metric"], "frame_hq")
        self.assertEqual((row["before"], row["after"]), (100.0, 110.0))
        self.assertAlmostEqual(row["delta_pct"], 10.0)

    def test_negative_delta(self):
        (row,) = self.rows(metric(200.0), metric(150.0))
        self.assertAlmostEqual(row["delta_pct"], -25.0)

    def test_delta_is_none_when_before_is_zero_or_missing(self):
        for before in (metric(0), metric(None)):
            with self.subTest(before=before["median"]):
                (row,) = self.rows(before, metric(5.0))
                self.assertIsNone(row["delta_pct"])

    def test_touching_ranges_overlap(self):
        (row,) = self.rows(metric(10, 8, 12), metric(14, 12, 16))
        self.assertTrue(row["within_spread"])

    def test_disjoint_ranges_do_not_overlap(self):
        (row,) = self.rows(metric(10, 8, 12), metric(14, 12.5, 16))
        self.assertFalse(row["within_spread"])

    def test_ranges_with_no_data_do_not_overlap(self):
        (row,) = self.rows(metric(None), metric(5.0))
        self.assertFalse(row["within_spread"])


class ComparePixelsTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.a = Path(self._tmp.name) / "a"
        self.b = Path(self._tmp.name) / "b"

    def by_name(self):
        return {row["name"]: row for row in compare.compare_pixels(self.a, self.b)}

    def test_identical_pixels(self):
        px = (2, 1, bytes([1, 2, 3, 4, 5, 6]))
        write_result(self.a, pixels={"small-light": px})
        write_result(self.b, pixels={"small-light": px})
        self.assertEqual(
            self.by_name()["small-light"],
            {"name": "small-light", "status": "identical", "differing_pixels": 0},
        )

    def test_one_differing_pixel_counts_once_even_if_several_channels_differ(self):
        write_result(self.a, pixels={"p": (2, 1, bytes([1, 2, 3, 4, 5, 6]))})
        write_result(self.b, pixels={"p": (2, 1, bytes([1, 2, 3, 9, 9, 9]))})
        row = self.by_name()["p"]
        self.assertEqual((row["status"], row["differing_pixels"]), ("different", 1))

    def test_size_mismatch_has_no_pixel_count(self):
        write_result(self.a, pixels={"p": (2, 1, bytes(6))})
        write_result(self.b, pixels={"p": (1, 2, bytes(6))})
        row = self.by_name()["p"]
        self.assertEqual((row["status"], row["differing_pixels"]), ("different", None))

    def test_file_missing_on_one_side(self):
        write_result(self.a, pixels={"only-a": (1, 1, bytes(3))})
        write_result(self.b, pixels={"only-b": (1, 1, bytes(3))})
        rows = self.by_name()
        self.assertEqual(rows["only-a"]["status"], "missing")
        self.assertEqual(rows["only-b"]["status"], "missing")
        self.assertIsNone(rows["only-a"]["differing_pixels"])

    def test_no_pixels_directory_yields_no_rows(self):
        write_result(self.a)
        write_result(self.b)
        self.assertEqual(compare.compare_pixels(self.a, self.b), [])


class RenderMarkdownTest(unittest.TestCase):
    def test_table_rows_and_na(self):
        rows = [
            {"scenario": "s", "fixture": "f", "metric": "m", "before": 0, "after": 5.0,
             "delta_pct": None, "within_spread": True},
            {"scenario": "s", "fixture": "f", "metric": "n", "before": 100.0, "after": 110.0,
             "delta_pct": 10.0, "within_spread": False},
        ]
        pixels = [{"name": "f-light", "status": "different", "differing_pixels": 3}]
        lines = compare.render_markdown(rows, pixels).splitlines()
        self.assertIn("| s | f | m | 0 | 5.0 | n/a | yes |", lines)
        self.assertIn("| s | f | n | 100.0 | 110.0 | +10.0% | no |", lines)
        self.assertIn("| f-light | different | 3 |", lines)


class MainTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.a = Path(self._tmp.name) / "a"
        self.b = Path(self._tmp.name) / "b"
        summary = {"startup-cold/small": {"metrics": {"frame_hq": metric(100.0)}}}
        write_result(self.a, summary=summary)

    def test_mismatch_exits_1_and_names_the_key(self):
        write_result(self.b, meta={**META, "macos": "15.2"})
        code, out, err = run_main(self.a, self.b)
        self.assertEqual(code, 1)
        self.assertIn("macos", err)
        self.assertEqual(out, "")

    def test_allow_mismatch_warns_and_continues(self):
        summary = {"startup-cold/small": {"metrics": {"frame_hq": metric(110.0)}}}
        write_result(self.b, meta={**META, "macos": "15.2"}, summary=summary)
        code, out, err = run_main(self.a, self.b, "--allow-mismatch")
        self.assertEqual(code, 0)
        self.assertIn("macos", err)
        self.assertIn("| startup-cold | small | frame_hq | 100.0 | 110.0 | +10.0% | no |", out)

    def test_matching_results_print_table_and_exit_0(self):
        summary = {"startup-cold/small": {"metrics": {"frame_hq": metric(100.0)}}}
        write_result(self.b, summary=summary)
        code, out, err = run_main(self.a, self.b)
        self.assertEqual((code, err), (0, ""))
        self.assertIn("| startup-cold | small | frame_hq |", out)


if __name__ == "__main__":
    unittest.main()
