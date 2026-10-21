#!/usr/bin/env python3
"""Paired native Swift/Rust WKWebView capture benchmark for an interactive Mac.

This measures the helper executable, not Ghostty's rendering or WebKit's
separate content/GPU processes. Both helpers receive identical JSON commands.
"""
import argparse
import json
import math
import os
import resource
import select
import statistics
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
FRACTIONS = [0.0, 0.12, 0.36, 0.73, 0.97, 0.43, 0.82, 0.2, 0.61, 0.05, 0.9, 0.5]


def fixture(root: Path) -> Path:
    page = root / "comparison.html"
    paragraphs = "\n".join(
        f"<section><h2>Heading {index:03}</h2>"
        f"<p>Repeatable Markdown-style paragraph {index:03}. "
        "The quick brown fox jumps over the lazy dog. "
        "漢字とかなのサンプルテキスト。</p>"
        f"<pre>fn example_{index:03}() {{ println!(\"hello\"); }}</pre></section>"
        for index in range(240)
    )
    page.write_text(
        "<!doctype html><html><head><meta charset='utf-8'>"
        "<style>body{margin:0;background:#fff;color:#24292f;"
        "font:16px/1.5 -apple-system,BlinkMacSystemFont,sans-serif}"
        "article{padding:24px;max-width:850px}section{margin:0 0 18px}"
        "h2{font-size:22px}pre{padding:14px;background:#f6f8fa;"
        "font:14px/1.4 monospace}</style></head><body>"
        f"<article class='markdown-body'><h1>Native helper comparison</h1>{paragraphs}"
        "</article></body></html>",
        encoding="utf-8",
    )
    return page


class Helper:
    def __init__(self, binary: Path, html: Path, root: Path, png: Path, width: int, height: int, log: Path):
        self.log_path = log
        self.stderr = log.open("wb")
        self.process = subprocess.Popen(
            [str(binary.resolve()), str(html.resolve()), str(root.resolve()),
             str(png), str(width), str(height)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            bufsize=0,
        )
        self.pending = bytearray()

    def receive(self, timeout: float = 20.0) -> dict:
        deadline = time.monotonic() + timeout
        assert self.process.stdout is not None
        fd = self.process.stdout.fileno()
        while True:
            if b"\n" in self.pending:
                line, _, tail = self.pending.partition(b"\n")
                self.pending = bytearray(tail)
                try:
                    value = json.loads(line)
                except ValueError as exc:
                    raise RuntimeError(f"invalid helper JSON: {line[:400]!r}") from exc
                if value.get("kind") == "error":
                    raise RuntimeError(f"helper protocol error: {value}")
                return value
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("timed out waiting for native WebKit reply")
            readable, _, _ = select.select([fd], [], [], remaining)
            if not readable:
                raise TimeoutError("timed out waiting for native WebKit reply")
            chunk = os.read(fd, 65536)
            if not chunk:
                raise RuntimeError(f"helper stdout closed (exit={self.process.poll()})")
            self.pending.extend(chunk)
            if len(self.pending) > 16_000_000:
                raise RuntimeError("helper JSON response exceeds 16 MB")

    def request(self, command: dict, expected: str, timeout: float = 20.0) -> dict:
        assert self.process.stdin is not None
        self.process.stdin.write((json.dumps(command) + "\n").encode("utf-8"))
        self.process.stdin.flush()
        reply = self.receive(timeout)
        if reply.get("kind") != expected:
            raise RuntimeError(f"expected {expected}, got {reply.get('kind')}")
        return reply

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=5)
        self.stderr.close()
        if self.process.stdout:
            self.process.stdout.close()
        if self.process.stdin:
            self.process.stdin.close()


def percentile(values: list[float], probability: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(probability * len(ordered)) - 1)]


def summarize(values: list[float]) -> dict:
    return {
        "median": round(statistics.median(values), 2),
        "p95": round(percentile(values, 0.95), 2),
        "max": round(max(values), 2),
    }


def run_one(
    label: str, binary: Path, html: Path, root: Path, temp: Path,
    width: int, height: int, samples: int, repeat: int,
) -> dict:
    png = temp / f"{label}-{repeat}.png"
    log = temp / f"{label}-{repeat}.stderr.log"
    cpu_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    helper = Helper(binary, html, root, png, width, height, log)
    try:
        ready = helper.receive(25)
        if ready.get("kind") != "ready":
            raise RuntimeError(f"expected ready, got {ready}")
        doc_height = float(ready["height"])
        if not math.isfinite(doc_height) or doc_height < height * 2:
            raise RuntimeError(f"document too short for paired captures: {doc_height}")
        helper.request({"type": "outline"}, "outline", timeout=20)
        max_y = max(0, doc_height - 2 * height)
        positions = [round(max_y * FRACTIONS[i % len(FRACTIONS)], 2)
                     for i in range(samples)]
        # Untimed warmup fills WebKit's image/layout caches consistently.
        for y in (0.0, max_y * 0.4):
            helper.request({"type": "snapshot", "y": y}, "snapshot", timeout=25)

        latencies, capture, encode, write, bytes_written = [], [], [], [], []
        capture_started = time.monotonic()
        for y in positions:
            reply = helper.request({"type": "snapshot", "y": y}, "snapshot", timeout=25)
            if abs(float(reply["requestedY"]) - y) > 2:
                raise RuntimeError(f"requested scroll mismatch: y={y}, reply={reply}")
            if abs(float(reply["actualY"]) - y) > 3:
                raise RuntimeError(f"actual scroll mismatch: y={y}, reply={reply}")
            data = png.read_bytes()
            if data[:8] != PNG_MAGIC or data[12:16] != b"IHDR":
                raise RuntimeError("invalid captured PNG")
            image_w, image_h = struct.unpack(">II", data[16:24])
            if image_w < width or image_h < 2 * height or len(data) != reply["pngBytes"]:
                raise RuntimeError(f"invalid snapshot dimensions or PNG byte count: {reply}")
            latencies.append(float(reply["elapsedMs"]))
            capture.append(float(reply["captureMs"]))
            encode.append(float(reply["encodeMs"]))
            write.append(float(reply["writeMs"]))
            bytes_written.append(len(data))
        snapshot_wall_ms = (time.monotonic() - capture_started) * 1000
        metrics = helper.request({"type": "metrics"}, "metrics")
        if metrics["completed"] != samples + 2 or metrics["requests"] != samples + 2:
            raise RuntimeError(f"unexpected capture metrics: {metrics}")
        assert helper.process.stdin is not None
        helper.process.stdin.write(b'{"type":"quit"}\n')
        helper.process.stdin.flush()
        helper.process.wait(timeout=10)
        if helper.process.returncode != 0:
            raise RuntimeError(f"helper exited with code {helper.process.returncode}")
        cpu_after = resource.getrusage(resource.RUSAGE_CHILDREN)
        return {
            "label": label,
            "repeat": repeat,
            "samples": samples,
            "document_height_px": round(doc_height, 1),
            "snapshot_elapsed_ms": summarize(latencies),
            "capture_ms": summarize(capture),
            "encode_ms": summarize(encode),
            "write_ms": summarize(write),
            "png_bytes_median": int(statistics.median(bytes_written)),
            "snapshots_wall_ms": round(snapshot_wall_ms, 2),
            "helper_cpu_seconds": round(
                (cpu_after.ru_utime + cpu_after.ru_stime)
                - (cpu_before.ru_utime + cpu_before.ru_stime), 3
            ),
            "session_wall_seconds": round(time.monotonic() - started, 3),
        }
    except Exception as exc:
        raise RuntimeError(f"{label} session {repeat}: {exc}; stderr saved to {log}") from exc
    finally:
        helper.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--swift-bin", type=Path, required=True,
                        help="compiled native Swift helper, not the swift interpreter")
    parser.add_argument("--rust-bin", type=Path, required=True,
                        help="release-built Rust adapter binary")
    parser.add_argument("--html", type=Path, help="optional identical rendered HTML for both helpers")
    parser.add_argument("--read-root", type=Path, help="required with --html")
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--height", type=int, default=600)
    parser.add_argument("--samples", type=int, default=12)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--output", type=Path, help="optional JSON report path")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("native comparison requires macOS")
    if (args.html is None) != (args.read_root is None):
        parser.error("--html and --read-root must be provided together")
    if args.width < 1 or args.height < 1 or args.samples < 4 or not 1 <= args.repeats <= 10:
        parser.error("positive viewport, at least four samples and 1–10 repeats required")
    for binary in (args.swift_bin, args.rust_bin):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            parser.error(f"missing executable: {binary}")
    with tempfile.TemporaryDirectory(prefix="doctty-webkit-compare-") as directory:
        temp = Path(directory)
        html = args.html.resolve() if args.html else fixture(temp)
        root = args.read_root.resolve() if args.read_root else temp
        if not html.is_file() or not html.is_relative_to(root):
            parser.error("HTML must be a file within --read-root")
        rows = []
        for repeat in range(args.repeats):
            order = [("swift", args.swift_bin), ("rust", args.rust_bin)]
            if repeat % 2:
                order.reverse()  # AB/BA counterbalances warmup and thermal drift.
            for label, binary in order:
                row = run_one(label, binary, html, root, temp, args.width,
                              args.height, args.samples, repeat)
                rows.append(row)
                print(f"{label} #{repeat + 1}: median snapshot "
                      f"{row['snapshot_elapsed_ms']['median']}ms, "
                      f"helper CPU {row['helper_cpu_seconds']}s", file=sys.stderr)
        heights = [row["document_height_px"] for row in rows]
        if max(heights) - min(heights) > max(5, max(heights) * .01):
            raise RuntimeError(f"Swift and Rust document heights differ: {heights}")
        report = {
            "caveat": ("CPU time covers the direct helper process only; separate "
                       "WebKit content/GPU processes and Ghostty CPU are not "
                       "included. This is not a scrolling FPS benchmark."),
            "fixture": str(html) if args.html else "deterministic embedded fixture",
            "viewport": [args.width, args.height],
            "runs": rows,
            "paired_summary": {
                label: {
                    "median_snapshot_ms": round(statistics.median(
                        r["snapshot_elapsed_ms"]["median"]
                        for r in rows if r["label"] == label), 2),
                    "median_helper_cpu_seconds": round(statistics.median(
                        r["helper_cpu_seconds"] for r in rows if r["label"] == label), 3),
                }
                for label in ("swift", "rust")
            },
        }
        serialized = json.dumps(report, ensure_ascii=False, indent=2)
        print(serialized)
        if args.output:
            args.output.write_text(serialized + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
