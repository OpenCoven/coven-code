"""Offline regression coverage for release-note insertion."""
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    "append_patch_note", Path(__file__).with_name("append-patch-note.py")
)
patch_note = importlib.util.module_from_spec(spec)
spec.loader.exec_module(patch_note)


class PatchNoteTests(unittest.TestCase):
    def test_retry_preserves_existing_release_body(self):
        bullet = "- fix(tui): paste handling (`abc1234`)"
        body = "## Changes\n\nExisting notes\n\n**Full changelog:** example\n"
        once = patch_note.splice(body, bullet)
        self.assertEqual(patch_note.splice(once, bullet), once)
        self.assertTrue(once.endswith(body))

    def test_retry_of_earlier_bullet_preserves_chronological_order(self):
        body = f"{patch_note.HEADING}\n\n- first\n- second\n\n## Changes\n"
        self.assertEqual(patch_note.splice(body, "- first"), body)
        self.assertEqual(
            patch_note.splice(body, "- third"),
            body.replace("- second\n", "- second\n- third\n"),
        )

    def test_same_text_outside_patch_list_does_not_suppress_insertion(self):
        body = f"{patch_note.HEADING}\n\n- first\n\n## Changes\n- second\n"
        self.assertEqual(
            patch_note.splice(body, "- second"),
            body.replace("- first\n", "- first\n- second\n"),
        )

    def test_retry_handles_existing_line_ending_normalization(self):
        body = f"\r\n{patch_note.HEADING}\r\n\r\n- first\r\n"
        self.assertEqual(
            patch_note.splice(body, "- first\n"), body.replace("\r\n", "\n")
        )

    def test_empty_release_and_distinct_bullet_are_preserved(self):
        once = patch_note.splice("", "- fix (`abc1234`)")
        self.assertEqual(once, f"{patch_note.HEADING}\n\n- fix (`abc1234`)\n")
        self.assertEqual(
            patch_note.splice(once, "- fix (`def5678`)"),
            once + "- fix (`def5678`)\n",
        )


if __name__ == "__main__":
    unittest.main()
