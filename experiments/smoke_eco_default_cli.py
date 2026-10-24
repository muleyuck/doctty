#!/usr/bin/env python3
"""Verify macOS packaged doctty chooses native Eco without executing Swift.

The compiled CLI runs inside a pseudo-terminal, but the two companion files
are test stubs. This checks argument/environment forwarding and the
fail-closed missing-helper contract without running WebKit or a real browser.
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
                "DOCTTY_ECO_SMOKE_RECORD": str(record),
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


def check(bundle: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="doctty compiled eco smoke ") as root:
        test_dir = Path(root) / "installed bundle with spaces"
        test_dir.mkdir()
        doctty = test_dir / "doctty"
        shutil.copy2(bundle / "doctty", doctty)

        markdown = test_dir / "sample document.md"
        markdown.write_text("# Default Eco test\n\nHello.\n", encoding="utf-8")
        record = test_dir / "invoked.json"

        # Missing helpers must fail, not implicitly run the old Swift script.
        assert run_interactive(doctty, markdown, [], record) != 0
        assert not record.exists()

        viewer = test_dir / "webkit_viewport_poc"
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
            "Path(os.environ['DOCTTY_ECO_SMOKE_RECORD']).write_text(json.dumps(data))\n",
            encoding="utf-8",
        )
        viewer.chmod(0o755)
        helper = test_dir / "doctty-webkit-swift"
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
                raise AssertionError(f"doctty failed to launch default Eco: {exit_code}")
            data = json.loads(record.read_text(encoding="utf-8"))
            assert data["args"] == expected_args, data
            assert data["swift"] == str(helper), data
            assert data["mermaid"] == expected_mermaid, data
            if expected_theme is not None:
                assert data["theme"] == expected_theme, data

    print("OK: normal interactive doctty uses adjacent compiled Eco companions")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: smoke_eco_default_cli.py /path/to/unpacked/bundle")
    check(Path(sys.argv[1]))
