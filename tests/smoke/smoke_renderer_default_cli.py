#!/usr/bin/env python3
"""Verify macOS packaged doctty chooses native without executing Swift.

The compiled CLI runs inside a pseudo-terminal with a renderer stub.
This checks argument forwarding, missing-renderer failure, and the compiled
Swift helper extraction used for source installs, without opening WebKit.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import pty
import shutil
import subprocess
import sys
import tempfile
import threading


KITTY_SUPPORT_REPLY = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c"


def answer_kitty_query(master: int) -> None:
    """Answer the terminal graphics capability query in a pseudo-terminal."""
    received = b""
    answered = False
    while True:
        try:
            chunk = os.read(master, 4096)
        except OSError:
            return
        if not chunk:
            return
        received += chunk
        if not answered and b"a=q" in received and b"\x1b[c" in received:
            os.write(master, KITTY_SUPPORT_REPLY)
            answered = True


def run_interactive(
    executable: Path,
    markdown: Path,
    flags: list[str],
    record: Path,
    trace: Path | None = None,
) -> int:
    master, slave = pty.openpty()
    responder = threading.Thread(target=answer_kitty_query, args=(master,), daemon=True)
    responder.start()
    try:
        env = os.environ.copy()
        env.update(
            {
                "TERM_PROGRAM": "ghostty",
                "TERM": "xterm-256color",
                "DOCTTY_RENDERER_SMOKE_RECORD": str(record),
            }
        )
        if trace is not None:
            env["DOCTTY_BENCH_TRACE_FILE"] = str(trace)
        result = subprocess.run(
            [str(executable), *flags, str(markdown)],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            cwd=markdown.parent,
            env=env,
            check=False,
            timeout=20,
        )
        return result.returncode
    finally:
        os.close(slave)
        os.close(master)


def read_trace(trace: Path) -> list[dict]:
    return [json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()]


CLI_EVENT_ORDER = [
    "cli_start",
    "cli_args_parsed",
    "helper_resolved",
    "renderer_spawn",
    "renderer_exit",
    "cli_exit",
]


def check(bundle: Path, source_install: bool = False) -> None:
    with tempfile.TemporaryDirectory(prefix="doctty compiled renderer smoke ") as root:
        test_dir = Path(root) / "installed bundle with spaces"
        test_dir.mkdir()
        doctty = test_dir / "doctty"
        shutil.copy2(bundle / "doctty", doctty)

        markdown = test_dir / "sample document.md"
        markdown.write_text("# Default test\n\nHello.\n", encoding="utf-8")
        record = test_dir / "invoked.json"

        # Missing helpers must fail, not implicitly run the old Swift script.
        assert run_interactive(doctty, markdown, [], record) != 0
        assert not record.exists()

        viewer = test_dir / "doctty-renderer"
        viewer.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "data = {\n"
            "    'args': sys.argv[1:],\n"
            "    'swift': os.environ.get('DOCTTY_WEBKIT_HELPER_BIN'),\n"
            "    'theme': os.environ.get('DOCTTY_WEBKIT_THEME'),\n"
            "    'mermaid': os.environ.get('DOCTTY_WEBKIT_MERMAID'),\n"
            "}\n"
            "Path(os.environ['DOCTTY_RENDERER_SMOKE_RECORD']).write_text(json.dumps(data))\n",
            encoding="utf-8",
        )
        viewer.chmod(0o755)
        helper = test_dir / "doctty-webkit-swift"
        if not source_install:
            helper.write_bytes(b"CI-only native helper placeholder\n")
            helper.chmod(0o755)

        cases = [
            ([], ["--hybrid-image", str(markdown)], "enabled", None),
            (
                ["--watch", "--theme", "dark", "--no-mermaid"],
                ["--hybrid-image", str(markdown), "--watch"],
                "disabled",
                "dark",
            ),
        ]
        for flags, expected_args, expected_mermaid, expected_theme in cases:
            if record.exists():
                record.unlink()
            exit_code = run_interactive(doctty, markdown, flags, record)
            if exit_code != 0:
                raise AssertionError(f"doctty failed to launch the standard renderer: {exit_code}")
            data = json.loads(record.read_text(encoding="utf-8"))
            assert data["args"] == expected_args, data
            if source_install:
                extracted = Path(data["swift"])
                assert extracted.is_file() and os.access(extracted, os.X_OK), data
                assert "Library/Caches/doctty/native" in str(extracted), data
                assert extracted != helper, data
            else:
                assert data["swift"] == str(helper), data
            assert data["mermaid"] == expected_mermaid, data
            if expected_theme is not None:
                assert data["theme"] == expected_theme, data

        # Bench tracing: the success path reports every CLI event in order.
        trace = test_dir / "trace.jsonl"
        assert run_interactive(doctty, markdown, [], record, trace) == 0
        events = read_trace(trace)
        assert [e["ev"] for e in events] == CLI_EVENT_ORDER, events
        assert all(e["proc"] == "cli" for e in events), events
        by_name = {e["ev"]: e for e in events}
        assert by_name["renderer_exit"]["code"] == 0, events
        assert by_name["renderer_exit"]["signal"] is None, events
        assert by_name["cli_exit"]["ok"] is True, events
        assert by_name["cli_args_parsed"]["mermaid"] == "enabled", events
        if source_install:
            assert by_name["helper_resolved"]["source"] in ("extracted", "cache_hit"), events
        else:
            assert by_name["helper_resolved"]["source"] == "adjacent", events

        # A failing renderer makes the CLI fail and still closes the trace.
        viewer.write_text("#!/usr/bin/env python3\nimport sys\nsys.exit(3)\n", encoding="utf-8")
        trace.unlink()
        assert run_interactive(doctty, markdown, [], record, trace) != 0
        events = read_trace(trace)
        by_name = {e["ev"]: e for e in events}
        assert by_name["renderer_exit"]["code"] == 3, events
        assert events[-1]["ev"] == "cli_exit" and events[-1]["ok"] is False, events

    print("OK: interactive doctty uses the compiled renderer and Swift helper")


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "--source-install"):
        raise SystemExit("usage: smoke_renderer_default_cli.py /path/to/bundle [--source-install]")
    check(Path(sys.argv[1]), source_install=len(sys.argv) == 3)
