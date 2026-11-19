import re
import shutil
import tempfile
import unittest
from pathlib import Path

import fixtures

FIXTURES = Path(__file__).parent / "fixtures"
IMAGE_RE = re.compile(r'!\[[^\]]*\]\(([^)\s]+)|<img[^>]*\ssrc="([^"]+)"|srcset="([^"]+)"')


class GenerateLargeTest(unittest.TestCase):
    def test_two_runs_write_identical_bytes_of_at_least_500kb(self):
        with tempfile.TemporaryDirectory() as tmp:
            shutil.copy(FIXTURES / "small.md", tmp)
            first = fixtures.generate_large(Path(tmp)).read_bytes()
            second = fixtures.generate_large(Path(tmp)).read_bytes()
        self.assertEqual(first, second)
        self.assertGreaterEqual(len(first), 500_000)


class FixtureHashesTest(unittest.TestCase):
    def test_covers_markdown_and_assets_and_asset_change_touches_one_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "assets").mkdir()
            (root / "a.md").write_text("a")
            (root / "assets" / "x.png").write_bytes(b"\x00\x01")
            (root / "assets" / "y.png").write_bytes(b"\x02")
            before = fixtures.fixture_hashes(root)
            (root / "assets" / "x.png").write_bytes(b"\x00\x02")
            after = fixtures.fixture_hashes(root)
        self.assertEqual(set(before), {"a.md", "assets/x.png", "assets/y.png"})
        changed = {k for k in before if before[k] != after[k]}
        self.assertEqual(changed, {"assets/x.png"})

    def test_committed_fixtures_are_hashed(self):
        hashes = fixtures.fixture_hashes(FIXTURES)
        for name in ("small.md", "raw-html.md", "mermaid-images.md", "assets/pixel.png"):
            self.assertIn(name, hashes)


class LintFixtureTest(unittest.TestCase):
    def test_flags_each_remote_image_form(self):
        for text in (
            "![x](https://example.com/a.png)",
            '<img src="https://example.com/a.png" />',
            '<source srcset="http://example.com/a.png" />',
        ):
            with self.subTest(text=text):
                self.assertTrue(fixtures.lint_fixture(text))

    def test_flags_gif_sources(self):
        for text in ("![x](assets/a.gif)", '<img src="assets/a.GIF" />'):
            with self.subTest(text=text):
                self.assertTrue(fixtures.lint_fixture(text))

    def test_passes_remote_link_and_local_image(self):
        text = "[docs](https://example.com) ![x](assets/a.png)\n[![b](assets/b.svg)](https://example.com)"
        self.assertEqual(fixtures.lint_fixture(text), [])


class CommittedFixturesTest(unittest.TestCase):
    def check_dir(self, root):
        for md in sorted(root.glob("*.md")):
            text = md.read_text()
            self.assertEqual(fixtures.lint_fixture(text), [], md.name)
            for groups in IMAGE_RE.findall(text):
                for src in filter(None, groups):
                    self.assertTrue((root / src).is_file(), f"{md.name}: {src} missing")

    def test_committed_fixtures_lint_clean_with_existing_images(self):
        self.check_dir(FIXTURES)

    def test_generated_large_lints_clean_with_existing_images(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            shutil.copytree(FIXTURES / "assets", root / "assets")
            shutil.copy(FIXTURES / "small.md", root / "small.md")
            fixtures.generate_large(root)
            self.check_dir(root)

    def test_raw_html_is_about_20kb_and_mermaid_has_three_blocks(self):
        self.assertTrue(18_000 <= (FIXTURES / "raw-html.md").stat().st_size <= 22_000)
        self.assertEqual((FIXTURES / "mermaid-images.md").read_text().count("```mermaid"), 3)


if __name__ == "__main__":
    unittest.main()
