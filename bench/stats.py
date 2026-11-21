"""Aggregate benchmark trace and ps samples into metrics (stdlib only)."""

import json
import math
import statistics
from pathlib import Path

STARTUP_STAGES = (
    "cli_args_parsed",
    "renderer_start",
    "html_built",
    "helper_ready",
    "outline_ready",
    "snapshot",
    "kitty_upload",
    "frame_hq",
)
Y_TOLERANCE = 0.5
HIGH_SPREAD = 0.2


def load_jsonl(path: Path) -> list[dict]:
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def _ordered(events: list[dict]) -> list[dict]:
    # The cli and renderer processes append to one file, so file order is not time order.
    return sorted(events, key=lambda e: e["t_us"])


def _ms(later: dict, earlier: dict) -> float:
    return (later["t_us"] - earlier["t_us"]) / 1000.0


def startup_metrics(events: list[dict]) -> dict:
    events = _ordered(events)
    start = next((e for e in events if e["ev"] == "cli_start"), None)
    result = {}
    for stage in STARTUP_STAGES:
        first = next((e for e in events if e["ev"] == stage), None)
        result[stage] = None if start is None or first is None else _ms(first, start)
    return result


def reload_latencies(events: list[dict]) -> list[float]:
    events = _ordered(events)
    latencies = []
    for index, event in enumerate(events):
        if event["ev"] != "reload_start":
            continue
        for later in events[index + 1 :]:
            if later["ev"] == "reload_start":
                break
            if later["ev"] == "frame_hq" and later.get("reason") == "reload":
                latencies.append(_ms(later, event))
                break
    return latencies


def _at(y: float, target: float) -> bool:
    return abs(y - target) < Y_TOLERANCE


def scroll_latencies(events: list[dict]) -> dict:
    events = _ordered(events)
    result = {"latencies_ms": [], "nonmoving": 0, "already_displayed": 0, "coalesced": 0, "missing": 0}
    for index, event in enumerate(events):
        if event["ev"] != "key" or event["kind"] not in ("press", "repeat"):
            continue
        if not event["moved"]:
            result["nonmoving"] += 1
            continue
        target = event["target_y"]
        later_targets = []
        outcome = "missing"
        for later in events[index + 1 :]:
            if later["ev"] == "key" and later["kind"] in ("press", "repeat") and later["moved"]:
                later_targets.append(later["target_y"])
            elif later["ev"] == "frame_unchanged" and _at(later["y"], target):
                outcome = "already_displayed"
                break
            elif later["ev"] == "frame_hq" and later.get("reason") == "scroll":
                if _at(later["y"], target):
                    outcome = "measured"
                    result["latencies_ms"].append(_ms(later, event))
                    break
                if any(_at(later["y"], t) for t in later_targets):
                    outcome = "coalesced"
                    break
        if outcome in ("already_displayed", "coalesced", "missing"):
            result[outcome] += 1
    return result


def counters(events: list[dict]) -> dict:
    result = {"cache_hit": 0, "cache_miss": 0, "prefetch_stale_skip": 0, "upload_skipped": 0, "upload_bytes": 0}
    for event in events:
        name = event["ev"]
        if name == "cache":
            result["cache_hit" if event["hit"] else "cache_miss"] += 1
        elif name in ("prefetch_stale_skip", "upload_skipped"):
            result[name] += 1
        elif name == "kitty_upload":
            result["upload_bytes"] += event["bytes"]
    return result


def process_usage(ps_rows: list[dict], pids: dict[str, set[int]], webkit_new: set[int]) -> dict:
    groups = {**pids, "webkit": webkit_new}
    # ps lists an exited process (a zombie) with its comm in parentheses and cputime
    # and rss 0; such a row is not a measurement. cputime only grows, so take the max.
    max_cpu: dict[int, float] = {}
    rss_by_time: dict[int, dict[float, int]] = {}
    for row in ps_rows:
        if row["comm"].startswith("("):
            continue
        pid = row["pid"]
        max_cpu[pid] = max(max_cpu.get(pid, 0.0), row["cpu_s"])
        rss_by_time.setdefault(pid, {})[row["t"]] = row["rss_kb"]
    usage = {}
    for group, members in groups.items():
        sampled = [pid for pid in members if pid in max_cpu]
        totals: dict[float, int] = {}
        for pid in sampled:
            for t, rss in rss_by_time[pid].items():
                totals[t] = totals.get(t, 0) + rss
        usage[group] = {
            "cpu_s": sum(max_cpu[pid] for pid in sampled),
            "peak_rss_kb": max(totals.values(), default=0),
            "unsampled": sorted(pid for pid in members if pid not in max_cpu),
        }
    return usage


def percentile(values: list[float], p: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil(p / 100 * len(ordered)))
    return ordered[rank - 1]


def summarize(runs: list[dict]) -> dict:
    names = {name for run in runs for name in run}
    summary = {}
    for name in sorted(names):
        values = [run[name] for run in runs if run.get(name) is not None]
        if not values:
            summary[name] = {"median": None, "min": None, "max": None, "high_spread": False}
            continue
        median = statistics.median(values)
        low, high = min(values), max(values)
        summary[name] = {
            "median": median,
            "min": low,
            "max": high,
            "high_spread": median != 0 and (high - low) / median > HIGH_SPREAD,
        }
    return summary
