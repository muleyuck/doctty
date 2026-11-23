"""Benchmark fixtures: generate large.md, hash fixtures, lint image sources."""

import hashlib
import re
from pathlib import Path

LARGE_MIN_BYTES = 500_000
RICH_MARKDOWN = Path(__file__).resolve().parent.parent / "examples" / "rich_markdown.md"

_REMOTE_IMAGE = re.compile(
    r'!\[[^\]]*\]\(\s*<?https?://'
    r'|<img\b[^>]*\bsrc\s*=\s*["\']https?://'
    r'|<source\b[^>]*\bsrcset\s*=\s*["\']\s*https?://',
    re.IGNORECASE,
)
_GIF_SOURCE = re.compile(
    r'!\[[^\]]*\]\(\s*<?[^)\s>]*\.gif\b'
    r'|<img\b[^>]*\bsrc\s*=\s*["\'][^"\']*\.gif\b'
    r'|<source\b[^>]*\bsrcset\s*=\s*["\'][^"\']*\.gif\b',
    re.IGNORECASE,
)


def _repetition(rich: str, n: int) -> str:
    # Image paths in rich_markdown.md are relative to examples/; large.md lives
    # next to assets/. Footnote labels are suffixed so definitions stay unique.
    text = rich.replace("](pixel.png", "](assets/pixel.png")
    return re.sub(r"\[\^(\w+)\]", lambda m: f"[^{m.group(1)}-{n}]", text)


def generate_large(fixtures_dir: Path) -> Path:
    small = (fixtures_dir / "small.md").read_text()
    rich = RICH_MARKDOWN.read_text()
    parts = [small]
    size = len(small.encode())
    n = 0
    while size < LARGE_MIN_BYTES:
        n += 1
        chunk = "\n" + _repetition(rich, n)
        parts.append(chunk)
        size += len(chunk.encode())
    out = fixtures_dir / "large.md"
    out.write_bytes("".join(parts).encode())
    return out


def fixture_hashes(fixtures_dir: Path) -> dict[str, str]:
    files = [p for p in fixtures_dir.glob("*.md") if p.is_file()]
    files += [p for p in (fixtures_dir / "assets").rglob("*") if p.is_file()]
    return {
        p.relative_to(fixtures_dir).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(files)
    }


def lint_fixture(text: str) -> list[str]:
    problems = [f"remote image source: {m.group(0)}" for m in _REMOTE_IMAGE.finditer(text)]
    problems += [f"gif image source: {m.group(0)}" for m in _GIF_SOURCE.finditer(text)]
    return problems
