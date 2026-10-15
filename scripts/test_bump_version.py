"""Exercise version stamping only in disposable repository fixtures."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SURFACES = (
    "src-rust/Cargo.toml",
    "src-rust/Cargo.lock",
    "npm/package.json",
    "README.md",
    "docs/index.md",
    "docs/installation.md",
    "src-rust/crates/acp/registry-template/agent.json",
)


class VersionStampTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for relative in (*SURFACES, "scripts/bump-version.py"):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)

    def snapshot(self):
        return {relative: (self.root / relative).read_bytes() for relative in SURFACES}

    def stamp(self):
        return subprocess.run(
            [sys.executable, "-B", str(self.root / "scripts/bump-version.py"), "v9.8.7"],
            capture_output=True, text=True, check=False,
        )

    def test_success_updates_every_surface_and_is_repeatable(self):
        before = self.snapshot()
        result = self.stamp()
        self.assertEqual(result.returncode, 0, result.stderr)
        after = self.snapshot()
        for relative in SURFACES:
            self.assertNotEqual(before[relative], after[relative], relative)
            self.assertIn(b"9.8.7", after[relative], relative)
        self.assertEqual(json.loads(after["npm/package.json"])["version"], "9.8.7")
        self.assertIn(b"Beta (v9.8.7).", after["README.md"])
        self.assertIn(b"`coven-code` **v9.8.7**", after["README.md"])
        self.assertNotIn(b"Claurst v9.8.7", after["README.md"])
        agent = json.loads(after[SURFACES[-1]])
        self.assertEqual(agent["version"], "9.8.7")
        self.assertEqual(self.stamp().returncode, 0)
        self.assertEqual(self.snapshot(), after)

    def test_missing_late_pattern_leaves_all_surfaces_unchanged(self):
        path = self.root / "docs/installation.md"
        path.write_text("No version example is present.\n", encoding="utf-8")
        before = self.snapshot()
        result = self.stamp()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no matches", result.stderr)
        self.assertEqual(self.snapshot(), before)

    def test_missing_archive_urls_leaves_all_surfaces_unchanged(self):
        path = self.root / SURFACES[-1]
        path.write_text('{"version": "0.7.0"}\n', encoding="utf-8")
        before = self.snapshot()
        result = self.stamp()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no /releases/download", result.stderr)
        self.assertEqual(self.snapshot(), before)

    def test_invalid_package_json_leaves_all_surfaces_unchanged(self):
        (self.root / "npm/package.json").write_text("{invalid", encoding="utf-8")
        before = self.snapshot()
        self.assertNotEqual(self.stamp().returncode, 0)
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
