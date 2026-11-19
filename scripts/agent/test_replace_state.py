"""Offline tests for replace-state.py: section replacement, and the script against a fake gh."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("replace_state", HERE / "replace-state.py")
replace_state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replace_state)

NEW = "## Current state: 2026-11-19\n\nNew facts.\n\n**Next action:** merge #1.\n"
OLD = "## Current state: 2026-11-18\n\nOld facts.\n- a bullet\n\n**Next action:** review #1.\n"


class ReplaceSectionTests(unittest.TestCase):
    def test_section_ends_after_next_action_paragraph(self):
        body = "## Review: 2026-11-06\n\nKeep.\n\n" + OLD + "\nOriginal report text.\n\n### Prompt\n\nKeep too.\n"
        self.assertEqual(
            replace_state.replace_section(body, NEW),
            "## Review: 2026-11-06\n\nKeep.\n\n" + NEW + "\nOriginal report text.\n\n### Prompt\n\nKeep too.\n",
        )

    def test_section_followed_by_another_heading(self):
        body = "## Current state: 2026-11-18\n\nOld facts, no next action.\n\n## Outcome\n\nKeep.\n"
        self.assertEqual(replace_state.replace_section(body, NEW), NEW + "\n## Outcome\n\nKeep.\n")

    def test_next_action_after_the_next_heading_is_not_part_of_the_section(self):
        body = "## Current state: 2026-11-18\n\nOld.\n\n## Outcome\n\n**Next action:** not ours.\n"
        self.assertEqual(
            replace_state.replace_section(body, NEW),
            NEW + "\n## Outcome\n\n**Next action:** not ours.\n",
        )

    def test_details_block_ends_the_section(self):
        body = "## Current state: 2026-11-18\n\nOld.\n<details>\nlog\n</details>\n"
        self.assertEqual(replace_state.replace_section(body, NEW), NEW + "\n<details>\nlog\n</details>\n")

    def test_section_at_end_of_body(self):
        body = "## What happens\n\nKeep.\n\n## Current state: 2026-11-18\n\nOld facts.\n"
        self.assertEqual(replace_state.replace_section(body, NEW), "## What happens\n\nKeep.\n\n" + NEW)

    def test_next_action_paragraph_at_end_of_body(self):
        body = "Intro.\n\n" + OLD.rstrip("\n")
        self.assertEqual(replace_state.replace_section(body, NEW), "Intro.\n\n" + NEW)

    def test_absent_section_is_an_error_without_insert(self):
        body = "## What happens\n\nKeep.\n"
        with self.assertRaisesRegex(replace_state.SectionError, "--insert"):
            replace_state.replace_section(body, NEW)
        self.assertEqual(replace_state.replace_section(body, NEW, insert=True), NEW + "\n" + body)

    def test_insert_into_empty_body(self):
        self.assertEqual(replace_state.replace_section(None, NEW, insert=True), NEW)

    def test_crlf_body_is_normalized(self):
        body = (OLD + "\nKeep.\n").replace("\n", "\r\n")
        self.assertEqual(replace_state.replace_section(body, NEW), NEW + "\nKeep.\n")

    def test_two_sections_are_refused(self):
        with self.assertRaisesRegex(replace_state.SectionError, "2 Current state headings"):
            replace_state.replace_section(OLD + "\n" + OLD, NEW)

    def test_section_file_must_start_with_the_dated_heading(self):
        for bad in ("New facts only.\n", "## Current state\n\nundated\n", "Intro\n" + NEW):
            with self.assertRaises(replace_state.SectionError):
                replace_state.replace_section(OLD, bad)

    def test_same_section_is_unchanged(self):
        body = NEW + "\nKeep.\n"
        self.assertEqual(replace_state.replace_section(body, NEW), body)


FAKE_GH = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json, os, sys
    root = os.environ["FAKE_GH_DIR"]
    args = sys.argv[1:]
    with open(os.path.join(root, "calls.log"), "a") as log:
        log.write(json.dumps(args) + "\\n")
    if "-X" in args:
        field = args[args.index("-F") + 1]
        assert field.startswith("body=@"), field
        with open(field[len("body=@"):]) as src, open(os.path.join(root, "patched.md"), "w") as dst:
            dst.write(src.read())
        print("https://github.com/example/issues/1")
    else:
        with open(os.path.join(root, "issue.json")) as f:
            sys.stdout.write(f.read())
    """
)


class ScriptTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        gh = self.dir / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.body = "Intro.\n\n" + OLD + "\nKeep.\n"
        (self.dir / "issue.json").write_text(json.dumps({"number": 5, "body": self.body}))
        self.section = self.dir / "section.md"
        self.section.write_text(NEW)
        self.out = self.dir / "out"

    def tearDown(self):
        self.tmp.cleanup()

    def script(self, *args):
        env = dict(
            os.environ, PATH=f"{self.dir}{os.pathsep}{os.environ['PATH']}", FAKE_GH_DIR=str(self.dir)
        )
        return subprocess.run(
            [sys.executable, str(HERE / "replace-state.py"), "5", str(self.section), "--out-dir", str(self.out), *args],
            env=env,
            capture_output=True,
            text=True,
        )

    def calls(self):
        return [json.loads(line) for line in (self.dir / "calls.log").read_text().splitlines()]

    def test_preview_writes_files_and_never_patches(self):
        result = self.script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("-## Current state: 2026-11-18", result.stdout)
        self.assertIn("+## Current state: 2026-11-19", result.stdout)
        self.assertEqual((self.out / "5.before.md").read_text(), self.body)
        self.assertEqual((self.out / "5.after.md").read_text(), "Intro.\n\n" + NEW + "\nKeep.\n")
        self.assertFalse((self.dir / "patched.md").exists())
        self.assertEqual([c for c in self.calls() if "-X" in c], [])

    def test_apply_patches_the_after_body(self):
        result = self.script("--apply", "--repo", "Example/repo")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.dir / "patched.md").read_text(), "Intro.\n\n" + NEW + "\nKeep.\n")
        patch = [c for c in self.calls() if "-X" in c]
        self.assertEqual(len(patch), 1)
        self.assertEqual(patch[0][:4], ["api", "-X", "PATCH", "repos/Example/repo/issues/5"])

    def test_apply_without_a_change_does_not_patch(self):
        (self.dir / "issue.json").write_text(json.dumps({"body": NEW}))
        result = self.script("--apply")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no change", result.stdout)
        self.assertFalse((self.dir / "patched.md").exists())

    def test_missing_section_exits_1_and_writes_nothing(self):
        (self.dir / "issue.json").write_text(json.dumps({"body": "## What happens\n"}))
        result = self.script("--apply")
        self.assertEqual(result.returncode, 1)
        self.assertIn("--insert", result.stderr)
        self.assertFalse(self.out.exists())
        self.assertFalse((self.dir / "patched.md").exists())


if __name__ == "__main__":
    unittest.main()
