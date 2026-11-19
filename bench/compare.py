"""Compare two benchmark result directories (stdlib only)."""

import argparse
import json
import sys
import zlib
from pathlib import Path

MISMATCH_KEYS = (
    "geometry",
    "fixtures",
    "host_model",
    "macos",
    "ghostty",
    "theme",
    "font_size",
    "mermaid",
)


def meta_mismatches(a: dict, b: dict) -> list[str]:
    return [key for key in MISMATCH_KEYS if a.get(key) != b.get(key)]


def _delta_pct(before, after) -> float | None:
    if not before or after is None:
        return None
    return (after - before) / before * 100


def _overlap(before: dict, after: dict) -> bool:
    bounds = (before.get("min"), before.get("max"), after.get("min"), after.get("max"))
    if any(value is None for value in bounds):
        return False
    return before["min"] <= after["max"] and after["min"] <= before["max"]


def compare_summaries(a: dict, b: dict) -> list[dict]:
    rows = []
    for key in sorted(a.keys() | b.keys()):
        scenario, _, fixture = key.partition("/")
        before_metrics = a.get(key, {}).get("metrics", {})
        after_metrics = b.get(key, {}).get("metrics", {})
        for name in sorted(before_metrics.keys() | after_metrics.keys()):
            before = before_metrics.get(name, {})
            after = after_metrics.get(name, {})
            rows.append(
                {
                    "scenario": scenario,
                    "fixture": fixture,
                    "metric": name,
                    "before": before.get("median"),
                    "after": after.get("median"),
                    "delta_pct": _delta_pct(before.get("median"), after.get("median")),
                    "within_spread": _overlap(before, after),
                }
            )
    return rows


def _load_pixels(directory: Path, name: str) -> tuple[int, int, bytes] | None:
    rgbz = directory / "pixels" / f"{name}.rgbz"
    meta = directory / "pixels" / f"{name}.json"
    if not rgbz.is_file() or not meta.is_file():
        return None
    size = json.loads(meta.read_text())
    return size["width"], size["height"], zlib.decompress(rgbz.read_bytes())


def _names(directory: Path) -> set[str]:
    return {path.stem for path in (directory / "pixels").glob("*.rgbz")}


def compare_pixels(a_dir: Path, b_dir: Path) -> list[dict]:
    rows = []
    for name in sorted(_names(a_dir) | _names(b_dir)):
        before = _load_pixels(a_dir, name)
        after = _load_pixels(b_dir, name)
        if before is None or after is None:
            rows.append({"name": name, "status": "missing", "differing_pixels": None})
        elif before[:2] != after[:2]:
            rows.append({"name": name, "status": "different", "differing_pixels": None})
        else:
            first, second = before[2], after[2]
            differing = sum(
                first[i : i + 3] != second[i : i + 3] for i in range(0, max(len(first), len(second)), 3)
            )
            status = "different" if differing else "identical"
            rows.append({"name": name, "status": status, "differing_pixels": differing})
    return rows


def _cell(value) -> str:
    return "n/a" if value is None else str(value)


def render_markdown(rows: list[dict], pixel_rows: list[dict]) -> str:
    lines = [
        "| scenario | fixture | metric | before (median) | after (median) | delta | within spread |",
        "|---|---|---|---|---|---|---|",
    ]
    for row in rows:
        delta = "n/a" if row["delta_pct"] is None else f"{row['delta_pct']:+.1f}%"
        spread = "yes" if row["within_spread"] else "no"
        lines.append(
            f"| {row['scenario']} | {row['fixture']} | {row['metric']} | "
            f"{_cell(row['before'])} | {_cell(row['after'])} | {delta} | {spread} |"
        )
    if pixel_rows:
        lines += ["", "| pixels | status | differing pixels |", "|---|---|---|"]
        for row in pixel_rows:
            lines.append(f"| {row['name']} | {row['status']} | {_cell(row['differing_pixels'])} |")
    return "\n".join(lines) + "\n"


def _read(directory: Path, name: str) -> dict:
    return json.loads((directory / name).read_text())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Compare two benchmark result directories.")
    parser.add_argument("before_dir", type=Path)
    parser.add_argument("after_dir", type=Path)
    parser.add_argument("--allow-mismatch", action="store_true")
    args = parser.parse_args(argv)

    mismatches = meta_mismatches(_read(args.before_dir, "meta.json"), _read(args.after_dir, "meta.json"))
    if mismatches:
        keys = ", ".join(mismatches)
        if not args.allow_mismatch:
            print(f"error: results are not comparable; differing meta keys: {keys}", file=sys.stderr)
            return 1
        print(f"warning: differing meta keys: {keys}", file=sys.stderr)

    rows = compare_summaries(_read(args.before_dir, "summary.json"), _read(args.after_dir, "summary.json"))
    sys.stdout.write(render_markdown(rows, compare_pixels(args.before_dir, args.after_dir)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
