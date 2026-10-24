#!/usr/bin/env python3
"""macOS CI smoke for the cached, precompiled Swift hybrid-image helper.

The Swift helper accepts commands asynchronously and does not process them
before its navigation delegate has emitted 'ready'. Unlike the Rust adapter,
it does not queue input until WebKit has finished loading.
"""
import json
import os
import select
import struct
import subprocess
import sys
import tempfile
import time
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
        stderr_path = root / "helper-stderr.log"
        with stderr_path.open("wb") as stderr:
            process = subprocess.Popen(
                [str(binary.resolve()), str(html), str(root), str(png), "640", "400"],
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
                    raise AssertionError(
                        f"expected {expected}, got {response}"
                    )
                return response

            try:
                ready = receive(timeout=25.0)
                if ready.get("kind") != "ready":
                    raise AssertionError(f"expected ready, got {ready}")
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

                # quit intentionally does not emit a response.
                assert process.stdin is not None
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


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: smoke_compiled_swift_webkit.py COMPILED_SWIFT_BINARY")
    test(Path(sys.argv[1]))
