#!/usr/bin/env python3
"""Exercise the isolated Rust WebKit helper on a local HTML/PNG round trip.

The test needs a working macOS GUI/WebKit session. An unattended GitHub-hosted
runner may not have one, so CI treats the result as diagnostic, not a release gate.
"""
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
VIEWPORT_WIDTH = 640
VIEWPORT_HEIGHT = 400
RESIZED_WIDTH = 480
RESIZED_HEIGHT = 320


def smoke(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="doctty-rust-snapshot-") as tmp:
        root = Path(tmp)
        page = root / "sample.html"
        png_path = root / "snapshot.png"
        page.write_text(
            "<!doctype html><html><head><meta charset='utf-8'>"
            "<style>body{margin:0;background:#fff} "
            "main{height:3500px;background:linear-gradient(#fef3c7,#1d4ed8)} "
            "@media(max-width:520px){main{height:4200px}} "
            "h1{padding:60px;color:#111}</style></head>"
            "<body><main><h1>Rust WebKit snapshot smoke</h1></main></body></html>",
            encoding="utf-8",
        )
        requests = [
            {"type": "outline"},
            {"type": "resize", "width": RESIZED_WIDTH, "height": RESIZED_HEIGHT},
            {"type": "dimensions"},
            {"type": "outline"},
            {"type": "select"},
            {"type": "snapshot", "y": 120},
            {"type": "metrics"},
            {"type": "quit"},
        ]
        payload = "".join(json.dumps(request) + "\n" for request in requests)
        try:
            result = subprocess.run(
                [
                    str(binary.resolve()),
                    str(page),
                    str(root),
                    str(png_path),
                    str(VIEWPORT_WIDTH),
                    str(VIEWPORT_HEIGHT),
                ],
                input=payload,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=45,
                check=False,
            )
        except subprocess.TimeoutExpired as error:
            raise AssertionError("Rust WebKit helper timed out") from error

        print("helper exit code:", result.returncode)
        if result.stderr:
            print("helper stderr:", result.stderr[-3000:])
        if result.returncode != 0:
            raise AssertionError("Rust WebKit helper exited unsuccessfully")

        records = [json.loads(line) for line in result.stdout.splitlines()]
        print("response kinds:", [record.get("kind") for record in records])
        errors = [record for record in records if record.get("kind") == "error"]
        if errors:
            raise AssertionError(f"WebKit helper errors: {errors}")
        if [item.get("kind") for item in records] != [
            "ready", "outline", "resized", "dimensions", "outline", "selection",
            "snapshot", "metrics"
        ]:
            raise AssertionError(f"unexpected response sequence: {records}")

        ready, outline, resized, dimensions, resized_outline, selection, snapshot, metrics = records
        if "Rust WebKit snapshot smoke" not in selection.get("text", ""):
            raise AssertionError(f"expected text missing from selection: {selection}")
        if len(selection["text"]) > 4096:
            raise AssertionError("DOM selection exceeded Swift's 4096-character bound")
        if outline.get("background") is None or not isinstance(outline.get("items"), list):
            raise AssertionError(f"invalid outline reply: {outline}")
        if not any("Rust WebKit snapshot smoke" in item.get("text", "")
                   for item in outline["items"]):
            raise AssertionError("expected heading missing from DOM outline")
        if ready["height"] < 3000 or ready["height"] > 4000:
            raise AssertionError(f"unexpected initial DOM height: {ready['height']}")
        if resized["height"] < 4000:
            raise AssertionError(f"DOM did not reflow at narrow width: {resized}")
        if abs(dimensions["height"] - resized["height"]) > 1:
            raise AssertionError("dimensions did not update after resize")
        if not any("Rust WebKit snapshot smoke" in item.get("text", "")
                   for item in resized_outline.get("items", [])):
            raise AssertionError("outline missing heading after reflow")
        if abs(snapshot["height"] - resized["height"]) > 1:
            raise AssertionError("snapshot still reports stale document height")
        if abs(snapshot["requestedY"] - 120) > 1:
            raise AssertionError(f"incorrect snapshot origin: {snapshot}")
        if abs(snapshot["actualY"] - 120) > 2:
            raise AssertionError(f"scroll did not reach requested origin: {snapshot}")

        data = png_path.read_bytes()
        if data[:8] != PNG_MAGIC or data[12:16] != b"IHDR":
            raise AssertionError("invalid PNG header")
        width, height = struct.unpack(">II", data[16:24])
        if width < RESIZED_WIDTH or height < RESIZED_HEIGHT * 2:
            raise AssertionError(f"unexpected PNG dimensions: {width}x{height}")
        if snapshot["pngBytes"] != len(data):
            raise AssertionError("reported PNG bytes differ from saved file")
        if metrics["requests"] != 1 or metrics["completed"] != 1:
            raise AssertionError(f"unexpected snapshot metrics: {metrics}")
        print(
            json.dumps(
                {
                    "status": "passed",
                    "png_width": width,
                    "png_height": height,
                    "png_bytes": len(data),
                    "document_height": ready["height"],
                    "resized_document_height": resized["height"],
                    "capture_ms": snapshot["captureMs"],
                    "encode_ms": snapshot["encodeMs"],
                    "write_ms": snapshot["writeMs"],
                },
                ensure_ascii=False,
            )
        )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: smoke_snapshot.py PATH_TO_RUST_ADAPTER")
    smoke(Path(sys.argv[1]))
