#!/usr/bin/env python3
"""macOS CI smoke for the cached, precompiled Swift hybrid-image helper.

The Swift helper accepts commands asynchronously and does not process them
before its navigation delegate has emitted 'ready'. Unlike the Rust adapter,
it does not queue input until WebKit has finished loading.
"""
import json
import os
import select
import subprocess
import sys
import tempfile
import time
import zlib
from contextlib import contextmanager
from pathlib import Path

STRIPE_PAGE_HEIGHT = 2000
STRIPE_PERIOD = 10


def stripe_html() -> str:
    """Alternating white/black 10 pt stripes: every boundary is a multiple of 10 pt."""
    stripes = "".join(
        f"<div style='position:absolute;left:0;right:0;top:{top}px;height:{STRIPE_PERIOD}px;"
        f"background:{'#000' if (top // STRIPE_PERIOD) % 2 else '#fff'}'></div>"
        for top in range(0, STRIPE_PAGE_HEIGHT, STRIPE_PERIOD)
    )
    return (
        "<!doctype html><html><head><meta charset='utf-8'><style>"
        "html,body{margin:0;padding:0}"
        f"body{{position:relative;height:{STRIPE_PAGE_HEIGHT}px;overflow:hidden}}"
        f"</style></head><body>{stripes}</body></html>"
    )


def fit_rect_rows(rgbz: bytes, snapshot: dict) -> tuple[float, float]:
    """Least-squares fit of image row against page offset; returns the fitted
    rows of the rect top and of the rect bottom (page offset captureHeight)."""
    width, height = snapshot["imageWidth"], snapshot["imageHeight"]
    data = zlib.decompress(rgbz)
    column = width // 2
    luminance = [data[(row * width + column) * 3] for row in range(height)]
    edges = [
        row
        for row in range(1, height)
        if (luminance[row - 1] >= 128) != (luminance[row] >= 128)
    ]
    first = (int(snapshot["actualY"]) // STRIPE_PERIOD + 1) * STRIPE_PERIOD
    offsets = [first + STRIPE_PERIOD * i - snapshot["actualY"] for i in range(len(edges))]
    mean_offset = sum(offsets) / len(edges)
    mean_edge = sum(edges) / len(edges)
    slope = sum((o - mean_offset) * (e - mean_edge) for o, e in zip(offsets, edges)) / sum(
        (o - mean_offset) ** 2 for o in offsets
    )
    intercept = mean_edge - slope * mean_offset
    return intercept, intercept + slope * snapshot["captureHeight"]


@contextmanager
def helper_session(binary: Path, html: str, width: int, height: int, pixel_width: int):
    """Start the helper on `html`; yield (ready, request, image_path, process)."""
    with tempfile.TemporaryDirectory(prefix="doctty-swift-compiled-") as directory:
        root = Path(directory)
        # The page is served from memory at this path; no file is written.
        page_path = root / "fixture.html"
        image = root / "viewport.rgbz"
        stderr_path = root / "helper-stderr.log"
        with stderr_path.open("wb") as stderr:
            process = subprocess.Popen(
                [
                    str(binary.resolve()),
                    "doctty-file",
                    str(page_path),
                    str(root),
                    str(image),
                    str(width),
                    str(height),
                    str(pixel_width),
                ],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=stderr,
                bufsize=0,
            )
            pending = bytearray()

            def receive(timeout: float = 20.0) -> dict:
                assert process.stdout is not None
                fd = process.stdout.fileno()
                deadline = time.monotonic() + timeout
                while True:
                    if b"\n" in pending:
                        line, _, tail = pending.partition(b"\n")
                        pending[:] = tail
                        return json.loads(line)
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError("Swift helper did not send a JSON reply")
                    ready, _, _ = select.select([fd], [], [], remaining)
                    if not ready:
                        raise TimeoutError("Swift helper JSON response timed out")
                    chunk = os.read(fd, 65536)
                    if not chunk:
                        raise RuntimeError(
                            f"Swift helper stdout closed (exit={process.poll()})"
                        )
                    pending.extend(chunk)

            def request(command: dict, expected: str) -> dict:
                assert process.stdin is not None
                process.stdin.write((json.dumps(command) + "\n").encode("utf-8"))
                process.stdin.flush()
                response = receive()
                if response.get("kind") != expected:
                    raise AssertionError(f"expected {expected}, got {response}")
                return response

            try:
                assert process.stdin is not None
                process.stdin.write((json.dumps({"html": html}) + "\n").encode("utf-8"))
                process.stdin.flush()
                ready = receive(timeout=25.0)
                if ready.get("kind") != "ready":
                    raise AssertionError(f"expected ready, got {ready}")
                yield ready, request, image, process
                # quit intentionally does not emit a response.
                process.stdin.write(b'{"type":"quit"}\n')
                process.stdin.flush()
                process.wait(timeout=10)
                assert process.returncode == 0, process.returncode
            except Exception:
                stderr.flush()
                print(
                    stderr_path.read_text(encoding="utf-8", errors="replace")[-2000:],
                    file=sys.stderr,
                )
                raise
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)
                if process.stdin:
                    process.stdin.close()
                if process.stdout:
                    process.stdout.close()


def test_stripe_geometry(binary: Path, backing: int) -> None:
    """Every tile holds the full two-viewport rect, down to its last pixel row."""
    # (width, height, pixelWidth). 959 has a fractional height, 656 is a size
    # WebKit returns correctly but mis-scaled, 960 an integer height; all three
    # take the exact path. width*backing takes WebKit's own 1:1 snapshot.
    # 800x300 resizes the CSS dimensions away from the launch size.
    cases = [
        (640, 400, 959),
        (640, 400, 656),
        (640, 400, 960),
        (640, 400, 640 * backing),
        (800, 300, 959),
    ]
    for width, height, pixel_width in cases:
        expected_height = round(height * 2 * pixel_width / width)
        with helper_session(binary, stripe_html(), 640, 400, pixel_width) as (
            _ready,
            request,
            image,
            _process,
        ):
            resized = request(
                {"type": "resize", "width": width, "height": height, "pixelWidth": pixel_width},
                "resized",
            )
            scroll_limit = resized["height"] - height * 2
            assert scroll_limit > 0, resized
            for y, expected_y in ((0, 0), (scroll_limit, scroll_limit)):
                snapshot = request({"type": "snapshot", "y": y}, "snapshot")
                label = f"{width}x{height} pixelWidth={pixel_width} y={y}: {snapshot}"
                assert abs(snapshot["actualY"] - expected_y) <= 0.5, label
                assert snapshot["imageWidth"] == pixel_width, label
                assert snapshot["imageHeight"] == expected_height, label
                assert snapshot["captureHeight"] == height * 2, label
                top, bottom = fit_rect_rows(image.read_bytes(), snapshot)
                print(
                    f"stripe fit {width}x{height} pixelWidth={pixel_width} y={y}: "
                    f"top={top:.3f} bottom={bottom:.3f} imageHeight={snapshot['imageHeight']} "
                    f"captureMs={snapshot['captureMs']:.1f} encodeMs={snapshot['encodeMs']:.1f}"
                )
                assert abs(top) <= 0.5, (top, label)
                assert abs(bottom - snapshot["imageHeight"]) <= 0.5, (bottom, label)


def test(binary: Path) -> None:
    html = (
        "<!doctype html><html><head><meta charset='utf-8'>"
        "<style>body{margin:0;background:white}main{height:3500px}"
        "h1{padding:30px}</style></head><body><main>"
        "<h1>Compiled Swift helper smoke test</h1></main></body></html>"
    )
    with helper_session(binary, html, 640, 400, 0) as (ready, request, image, _process):
        outline = request({"type": "outline"}, "outline")
        snapshot = request({"type": "snapshot", "y": 128}, "snapshot")
        metrics = request({"type": "metrics"}, "metrics")

        assert ready["height"] > 3000, ready
        for key in ("navMs", "assetsMs"):
            assert isinstance(ready[key], (int, float)), ready
            assert ready[key] >= 0, ready
        assert any(
            "Compiled Swift helper smoke test" in item.get("text", "")
            for item in outline["items"]
        ), outline
        assert abs(snapshot["actualY"] - 128) <= 2, snapshot
        assert abs(snapshot["requestedY"] - 128) <= 2, snapshot
        assert metrics["requests"] == metrics["completed"] == 1, metrics
        data = image.read_bytes()
        width, height = snapshot["imageWidth"], snapshot["imageHeight"]
        assert width >= 640 and height >= 800, (width, height)
        # Kitty f=24,o=z: zlib-compressed, tightly packed RGB.
        assert len(zlib.decompress(data)) == width * height * 3, snapshot
        assert snapshot["imageBytes"] == len(data), snapshot
        assert snapshot["captureHeight"] == 800, snapshot
        backing = width // 640
        print(
            json.dumps(
                {
                    "status": "passed",
                    "document_height": ready["height"],
                    "image_dimensions": [width, height],
                    "image_bytes": len(data),
                    "snapshot_elapsed_ms": snapshot["elapsedMs"],
                }
            )
        )

        # A terminal pixel width replaces the backing-scale width.
        request(
            {"type": "resize", "width": 640, "height": 400, "pixelWidth": 960},
            "resized",
        )
        scaled = request({"type": "snapshot", "y": 0}, "snapshot")
        scaled_width, scaled_height = scaled["imageWidth"], scaled["imageHeight"]
        assert len(zlib.decompress(image.read_bytes())) == scaled_width * scaled_height * 3
        assert scaled_width == 960, (scaled_width, scaled_height)
        assert scaled_height == 1200, (scaled_width, scaled_height)
        assert scaled["captureHeight"] == 800, scaled

    test_stripe_geometry(binary, backing)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: smoke_compiled_swift_webkit.py COMPILED_SWIFT_BINARY")
    test(Path(sys.argv[1]))
