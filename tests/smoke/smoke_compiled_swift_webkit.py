#!/usr/bin/env python3
"""macOS CI smoke for the cached, precompiled Swift hybrid-image helper.

The Swift helper accepts commands asynchronously and does not process them
before its navigation delegate has emitted 'ready'. Unlike the Rust adapter,
it does not queue input until WebKit has finished loading.
"""
import base64
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
def helper_session(
    binary: Path,
    html: str,
    width: int,
    height: int,
    pixel_width: int,
    files: dict[str, str] | None = None,
):
    """Start the helper on `html`; yield (ready, request, image_path, process, send, receive).

    `files` are written under the read root before launch. `send` writes one request
    without reading a reply; `receive(timeout)` reads one reply line.
    """
    with tempfile.TemporaryDirectory(prefix="doctty-swift-compiled-") as directory:
        root = Path(directory)
        for name, content in (files or {}).items():
            (root / name).write_text(content, encoding="utf-8")
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

            def send(command: dict) -> None:
                assert process.stdin is not None
                process.stdin.write((json.dumps(command) + "\n").encode("utf-8"))
                process.stdin.flush()

            def request(command: dict, expected: str) -> dict:
                send(command)
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
                yield ready, request, image, process, send, receive
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


RELOAD_SIZE = (640, 400, 960)
FONT = Path(__file__).parents[2] / "src/render/assets/fonts/MonaSansVF.woff2"


def reload_page(
    heading: str,
    image: str = "img.svg",
    lazy: bool = False,
    font: bool = True,
    spacer: int = 3000,
    head: str = "",
) -> str:
    """A page that uses a file in the read root and the bundled font the way production does."""
    face = ""
    if font:
        data = base64.b64encode(FONT.read_bytes()).decode("ascii")
        face = (
            "@font-face{font-family:'Mona Test';"
            f"src:url(data:font/woff2;base64,{data}) format('woff2')}}"
        )
    loading = " loading='lazy'" if lazy else ""
    tag = f"<img src='{image}'{loading} width='200' height='100'>"
    return (
        "<!doctype html><html><head><meta charset='utf-8'><style>"
        f"{face}"
        "html,body{margin:0;background:#fff}"
        "h1{font-family:'Mona Test',sans-serif;font-size:48px;padding:20px}"
        f"p.end{{margin-top:{spacer}px;font-family:'Mona Test',sans-serif;font-size:32px}}"
        f"</style>{head}</head><body><h1>{heading}</h1>"
        f"{'' if lazy else tag}"
        f"<p class='end'>end of {heading}</p>{tag if lazy else ''}</body></html>"
    )


def svg(color: str) -> str:
    return (
        "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='100'>"
        f"<rect width='200' height='100' fill='{color}'/></svg>"
    )


def capture_pair(request, image: Path, document_height: float) -> dict:
    """Snapshot at y=0 and at the scroll limit; return decompressed pixels and sizes."""
    limit = document_height - RELOAD_SIZE[1] * 2
    assert limit > 0, document_height
    captured = {}
    for name, y in (("top", 0), ("limit", limit)):
        snapshot = request({"type": "snapshot", "y": y}, "snapshot")
        captured[name] = (
            snapshot["imageWidth"],
            snapshot["imageHeight"],
            zlib.decompress(image.read_bytes()),
        )
    return captured


def fresh_capture_with_height(binary: Path, html: str, files: dict[str, str]) -> tuple:
    with helper_session(binary, html, *RELOAD_SIZE, files=files) as (
        ready,
        request,
        image,
        _process,
        _send,
        _receive,
    ):
        return ready["height"], capture_pair(request, image, ready["height"])


def fresh_capture(binary: Path, html: str, files: dict[str, str]) -> dict:
    with helper_session(binary, html, *RELOAD_SIZE, files=files) as (
        ready,
        request,
        image,
        _process,
        _send,
        _receive,
    ):
        return capture_pair(request, image, ready["height"])


def test_reload(binary: Path) -> None:
    files = {"img.svg": svg("#e11"), "lazy.svg": svg("#1a1")}
    page_a = reload_page("page A", image="missing-a.svg")
    page_b = reload_page("page B")
    page_c = reload_page("page C")

    def reload_to(request, html: str) -> dict:
        return request({"type": "reload", "html": html}, "reloaded")

    # 1, 2: a reloaded page is pixel-identical to a fresh launch, and no stray ready follows.
    with helper_session(binary, page_a, *RELOAD_SIZE, files=files) as (
        _ready,
        request,
        image,
        _process,
        send,
        receive,
    ):
        reloaded = reload_to(request, page_b)
        request({"type": "dimensions"}, "dimensions")
        assert capture_pair(request, image, reloaded["height"]) == fresh_capture(
            binary, page_b, files
        ), "reload A->B differs from a fresh B"

        # 3: two reloads in a row end on the second page.
        reload_to(request, page_b)
        reloaded = reload_to(request, page_c)
        assert capture_pair(request, image, reloaded["height"]) == fresh_capture(
            binary, page_c, files
        ), "reload B->C differs from a fresh C"

        # 4: requests that need a loaded page get an immediate error; the reload still replies.
        for command in (
            {"type": "snapshot", "y": 0},
            {"type": "resize", "width": 640, "height": 400, "pixelWidth": 960},
            {"type": "outline"},
        ):
            send({"type": "reload", "html": page_b})
            send(command)
            rejected = receive()
            assert rejected["kind"] == "error", (command, rejected)
            reply = receive()
            assert reply["kind"] == "reloaded", (command, reply)
        # 4: a reload while another is pending supersedes it.
        # Small pages: a 500 KB font would still be streaming into the pipe while the
        # first reload completes, and the second request would no longer be pending.
        small_b = reload_page("small B", font=False)
        small_c = reload_page("small C", font=False)
        send({"type": "reload", "html": small_b})
        send({"type": "reload", "html": small_c})
        superseded = receive()
        assert superseded["kind"] == "error", superseded
        reloaded = receive()
        assert reloaded["kind"] == "reloaded", reloaded
        assert capture_pair(request, image, reloaded["height"]) == fresh_capture(
            binary, small_c, files
        ), "superseded reload differs from a fresh C"

        # A page of a clearly different height: `reloaded` reports the new height.
        tall = reload_page("page T", spacer=7000)
        reloaded = reload_to(request, tall)
        tall_fresh = fresh_capture_with_height(binary, tall, files)
        assert reloaded["height"] == tall_fresh[0], (reloaded, tall_fresh[0])
        assert reloaded["height"] > 7000, reloaded
        assert capture_pair(request, image, reloaded["height"]) == tall_fresh[1], (
            "reload to a taller page differs from fresh"
        )

        # A readiness check that throws: one error, no later line, no stuck phase,
        # and the helper can still reload. (A navigation failure cannot be induced
        # without a test hook: the page itself is served from memory.)
        throwing = reload_page(
            "page X",
            head="<script>Object.defineProperty(document,'images',"
            "{get(){throw new Error('boom')}})</script>",
        )
        send({"type": "reload", "html": throwing})
        failure = receive()
        assert failure["kind"] == "error", failure
        try:
            extra = receive(timeout=3.0)
        except TimeoutError:
            extra = None
        assert extra is None, f"reply after the error: {extra}"
        phase = request({"type": "metrics"}, "metrics")["loadPhase"]
        assert phase not in ("navigating", "readiness"), phase
        request({"type": "snapshot", "y": 0}, "error")
        reloaded = reload_to(request, page_b)
        assert capture_pair(request, image, reloaded["height"]) == fresh_capture(
            binary, page_b, files
        ), "reload after a failed reload differs from fresh"

        # 6: a missing image does not stall or fail the reload.
        reloaded = reload_to(request, reload_page("page M", image="no-such.svg"))
        assert capture_pair(request, image, reloaded["height"]) == fresh_capture(
            binary, reload_page("page M", image="no-such.svg"), files
        ), "reload with a missing image differs from fresh"

    # 5: a changed image file is fetched again on reload.
    with helper_session(binary, page_b, *RELOAD_SIZE, files=files) as (
        ready,
        request,
        image,
        _process,
        _send,
        _receive,
    ):
        before = capture_pair(request, image, ready["height"])
        changed = {**files, "img.svg": svg("#11e")}
        (image.parent / "img.svg").write_text(changed["img.svg"], encoding="utf-8")
        reloaded = reload_to(request, page_b)
        after = capture_pair(request, image, reloaded["height"])
        assert after != before, "changed image file was served from cache"
        assert after == fresh_capture(binary, page_b, changed), "reload differs from fresh"

    # 7: a stale readiness check stays silent once a newer reload started.
    small_c = reload_page("small C", font=False)
    page_lazy = reload_page("page L", image="lazy.svg", lazy=True, font=False)
    with helper_session(binary, page_a, *RELOAD_SIZE, files=files) as (
        _ready,
        request,
        image,
        _process,
        send,
        receive,
    ):
        send({"type": "reload", "html": page_lazy})
        deadline = time.monotonic() + 2
        while True:
            phase = request({"type": "metrics"}, "metrics")["loadPhase"]
            if phase == "readiness":
                break
            assert time.monotonic() < deadline, f"never reached readiness, last {phase}"
            time.sleep(0.02)
        send({"type": "reload", "html": small_c})
        superseded = receive()
        assert superseded["kind"] == "error", superseded
        reloaded = receive()
        assert reloaded["kind"] == "reloaded", reloaded
        try:
            extra = receive(timeout=3.0)
        except TimeoutError:
            extra = None
        assert extra is None, f"stale callback replied: {extra}"
        assert capture_pair(request, image, reloaded["height"]) == fresh_capture(
            binary, small_c, files
        ), "reload after a superseded reload differs from fresh"


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
            _send,
            _receive,
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
    with helper_session(binary, html, 640, 400, 0) as (
        ready,
        request,
        image,
        _process,
        _send,
        _receive,
    ):
        outline = request({"type": "outline"}, "outline")
        snapshot = request({"type": "snapshot", "y": 128}, "snapshot")
        metrics = request({"type": "metrics"}, "metrics")

        assert ready["height"] > 3000, ready
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
    test_reload(binary)
    print("reload smoke passed")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: smoke_compiled_swift_webkit.py COMPILED_SWIFT_BINARY")
    test(Path(sys.argv[1]))
