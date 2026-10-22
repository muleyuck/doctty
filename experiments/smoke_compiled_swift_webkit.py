#!/usr/bin/env python3
"""macOS CI smoke for the cached, precompiled Swift hybrid-image helper."""
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


def test(binary: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="doctty-swift-compiled-") as directory:
        root = Path(directory)
        html = root / "fixture.html"
        png = root / "viewport.png"
        html.write_text(
            "<!doctype html><html><head><meta charset='utf-8'>"
            "<style>body{margin:0;background:white}main{height:3500px}"
            "h1{padding:30px}</style></head><body><main>"
            "<h1>Compiled Swift helper smoke test</h1></main></body></html>",
            encoding="utf-8",
        )
        requests = [
            {"type": "outline"},
            {"type": "snapshot", "y": 128},
            {"type": "metrics"},
            {"type": "quit"},
        ]
        result = subprocess.run(
            [str(binary.resolve()), str(html), str(root), str(png), "640", "400"],
            input="".join(json.dumps(req) + "\n" for req in requests),
            text=True,
            capture_output=True,
            timeout=40,
            check=False,
        )
        if result.returncode:
            raise AssertionError(
                f"Swift helper failed ({result.returncode}): {result.stderr[-1000:]}"
            )
        output = [json.loads(line) for line in result.stdout.splitlines()]
        kinds = [line.get("kind") for line in output]
        if kinds != ["ready", "outline", "snapshot", "metrics"]:
            raise AssertionError(f"Unexpected helper responses: {kinds}")
        ready, outline, snapshot, metrics = output
        assert ready["height"] > 3000, ready
        assert any(
            "Compiled Swift helper smoke test" in item.get("text", "")
            for item in outline["items"]
        ), outline
        assert abs(snapshot["actualY"] - 128) <= 2, snapshot
        assert abs(snapshot["requestedY"] - 128) <= 2, snapshot
        assert metrics["requests"] == metrics["completed"] == 1, metrics
        data = png.read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n", "PNG magic missing"
        assert data[12:16] == b"IHDR", "PNG IHDR missing"
        width, height = struct.unpack(">II", data[16:24])
        assert width >= 640 and height >= 800, (width, height)
        assert snapshot["pngBytes"] == len(data), snapshot
        print(
            json.dumps(
                {
                    "status": "passed",
                    "document_height": ready["height"],
                    "png_dimensions": [width, height],
                    "png_bytes": len(data),
                    "snapshot_elapsed_ms": snapshot["elapsedMs"],
                }
            )
        )


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: smoke_compiled_swift_webkit.py COMPILED_SWIFT_BINARY")
    test(Path(sys.argv[1]))
