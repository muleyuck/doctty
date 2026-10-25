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


def run_interactive(executable: Path, markdown: Path, flags: list[str], record: Path) -> int:
    master, slave = pty.openpty()
    try:
        env = os.environ.copy()
        env.update(
            {
                "TERM_PROGRAM": "ghostty",
                "TERM": "xterm-256color",
                "DOCTTY_RENDERER_SMOKE_RECORD": str(record),
            }
        )
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

    print("OK: interactive doctty uses the compiled renderer and Swift helper")


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "--source-install"):
        raise SystemExit("usage: smoke_renderer_default_cli.py /path/to/bundle [--source-install]")
    check(Path(sys.argv[1]), source_install=len(sys.argv) == 3)
