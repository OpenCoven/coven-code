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
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("replace_state", HERE / "replace-state.py")
replace_state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replace_state)

NEW = "## Current state: 2026-11-19\n\nNew facts.\n\n**Next action:** merge #1.\n"
OLD = "## Current state: 2026-11-18\n\nOld facts.\n- a bullet\n\n**Next action:** review #1.\n"


class ReplaceSectionTests(unittest.TestCase):
    def replaced(self, body, section=NEW, **kw):
        return replace_state.replace_section(body, section, **kw)

    def refused(self, body, section=NEW, pattern="", **kw):
        with self.assertRaisesRegex(replace_state.SectionError, pattern, msg=repr(body)):
            replace_state.replace_section(body, section, **kw)

    def test_section_ends_after_its_next_action_line_and_the_rest_is_kept(self):
        prefix = "## Review: 2026-11-06\n\nKeep.\n\n"
        tail = "Original report text.\n\n### Prompt\n\nKeep too.\n"
        self.assertEqual(self.replaced(prefix + OLD + "\n" + tail), prefix + NEW + "\n" + tail)

    def test_next_action_at_end_of_body(self):
        self.assertEqual(self.replaced("Intro.\n\n" + OLD.rstrip("\n")), "Intro.\n\n" + NEW)

    def test_heading_details_or_whitespace_right_after_the_blank_line_is_kept(self):
        for tail in ("## Outcome\n\nKeep.\n", "<details>\nlog\n</details>\n", "   \nKeep.\n", "- keep\n"):
            self.assertEqual(self.replaced(OLD + "\n" + tail), NEW + "\n" + tail, tail)

    def test_section_without_a_next_action_is_refused(self):
        for body in (
            "## Current state: 2026-11-18\n\nOld facts.\n",
            "## Current state: 2026-11-18\n\nOld.\n\nPlain original text.\n",
            "## Current state: 2026-11-18\n\nOld.\n\n## Outcome\n\nKeep.\n",
        ):
            self.refused(body, pattern="Next action")

    def test_structure_before_the_next_action_is_refused(self):
        # Each of these could be original text that GitHub renders as its own block.
        risky = (
            "## Original report", "### Done", "#### Notes", "Original\n===", "TITLE\n-----",
            "---", "- - -", "* * *", "```", "```x``` is the command", "~~~", "<h2>Original</h2>",
            "<DETAILS>", "<details open>", "<!-- note -->", "end -->", "<pre>",
        )
        for line in risky:
            body = f"## Current state: 2026-11-18\n\nOld.\n\n{line}\n\nKEEP\n\n**Next action:** x.\n\nKeep.\n"
            self.refused(body, pattern="edit it by hand")

    def test_next_action_paragraph_running_on_is_refused(self):
        for after in ("KEEP original text.", "Col | Col2", "### Heading", "- item"):
            body = f"## Current state: 2026-11-18\n\nOld.\n\n**Next action:** x\n{after}\n\nKeep.\n"
            self.refused(body, pattern="runs on")

    def test_heading_inside_a_comment_or_details_is_refused(self):
        for opener, closer in (("<!--", "-->"), ("<details>", "</details>")):
            body = f"Intro.\n{opener}\n\n{OLD}{closer}\n\nKEEP\n"
            self.refused(body, pattern="after HTML")

    def test_heading_inside_a_fence_or_html_block_is_refused(self):
        example = "## Current state: 2026-11-18\n\nOld.\n\n**Next action:** x.\n"
        for opener, closer in (("```", "```"), ("~~~", "~~~"), ("````md", "````"), ("<pre>", "</pre>"),
                               ("<textarea>", "</textarea>")):
            body = f"How to write one:\n\n{opener}\n\n{example}\n{closer}\n\nKeep.\n"
            self.refused(body, pattern="edit it by hand")
            self.refused(body, pattern="edit it by hand", insert=True)

    def test_an_arrow_does_not_close_a_later_comment(self):
        body = "```mermaid\nA --> B\n```\n\n<!--\n\n" + OLD + "\nHIDDEN\n-->\nKeep\n"
        self.refused(body, pattern="after a code fence")
        self.refused("A --> B\n\n<!--\n\n" + OLD + "\nHIDDEN\n-->\nKeep\n", pattern="after HTML")

    def test_fences_in_lists_or_html_before_the_heading_are_refused(self):
        example = "## Current state: 2026-01-01\n\nBlocked.\n\n**Next action:** do Y\n\nMORE\n```\n"
        for prefix in (
            "- ```sh\n  cargo test\n  ```\n\nTemplate:\n\n```\n\n",
            "<!--\n```\n-->\n\n```\n\n",
            "<div>\n```\n</div>\n\n```\n\n",
            "````\n```\n\n",
            "<?php\n\n",
            "<![CDATA[\n\n",
            "<!DOCTYPE x\n\n",
        ):
            self.refused(prefix + example, pattern="edit it by hand")

    def test_link_and_footnote_definitions_are_refused(self):
        for line in ("[docs]: https://other.example", "[^1]: a note", "  [1]: https://x.example"):
            with self.assertRaises(replace_state.SectionError, msg=line):
                replace_state.normalize_section(NEW.replace("New facts.", line))
            self.refused(OLD.replace("Old facts.", line))

    def test_lines_of_unusual_whitespace_are_refused(self):
        for space in ("\u00a0", "\x0b", "\u3000", " \u00a0 "):
            self.refused(
                "Intro\n\n" + OLD + space + "\nTail para.\n", pattern="unusual whitespace"
            )
            self.refused(OLD.replace("Old facts.\n", "- item\n" + space + "\n") + "\nlazy\n")
            with self.assertRaises(replace_state.SectionError):
                replace_state.normalize_section(NEW.replace("New facts.", "a\n" + space + "\nb"))
        self.assertEqual(self.replaced(OLD + " \t\nKeep.\n"), NEW + "\n \t\nKeep.\n")

    def test_near_miss_next_action_markers_are_refused(self):
        tail = "\nOriginal report: crash.\n\n**Next action:** (template) fill in.\n\nKeep.\n"
        for marker in (
            "**Next Action:** review",
            "**Next action**: review",
            "Next action: review",
            "- next-action: x",
            "**Next actions:** review.",
            "**Next action (Val):** review.",
            "Next steps. **Next action** is review.",
            "> Next Actions (owner Val): review",
            "Blocked on #12; next actions: review",
        ):
            body = OLD.replace("**Next action:** review #1.", marker) + tail
            self.assertNotEqual(body, OLD + tail)
            self.refused(body, pattern="looks like a Next action")
            with self.assertRaises(replace_state.SectionError):
                replace_state.normalize_section(NEW.replace("New facts.", marker))

    def test_next_action_in_prose_is_fine(self):
        body = OLD.replace("Old facts.", "The previous next action is done.") + "\nKeep.\n"
        self.assertEqual(self.replaced(body), NEW + "\nKeep.\n")

    def test_non_ascii_digits_in_the_heading_do_not_match(self):
        self.assertIsNone(replace_state.HEADING.fullmatch("## Current state: \u0662\u0660\u0662\u0666-11-19"))

    def test_nested_definitions_are_refused(self):
        for line in ("> [spec]: https://example.com/spec", "- [x]: url", "> [^1]: note", "text [a] : b", "Old[^n].", "$$"):
            with self.assertRaises(replace_state.SectionError, msg=line):
                replace_state.normalize_section(NEW.replace("New facts.", line))
            self.refused(OLD.replace("Old facts.", line) + "\nSee the [spec].\n")

    def test_next_action_must_follow_a_blank_line(self):
        joined = "## Current state: 2026-11-19\n\n1. a\n2. b\n**Next action:** y\n"
        with self.assertRaisesRegex(replace_state.SectionError, "blank line"):
            replace_state.normalize_section(joined)
        old = "## Current state: 2026-11-01\n\n- a\n**Next action:** x\n\n- tail item\n"
        self.refused(old, pattern="blank line")

    def test_both_comment_closers_are_html(self):
        for closer in ("a --> b", "a --!> b"):
            self.refused(OLD.replace("Old facts.", closer))
            self.refused(closer + "\n\n" + OLD, pattern="after HTML")

    def test_html_heading_inside_a_line_of_the_old_section_is_refused(self):
        self.refused(OLD.replace("Old facts.", "state <h2>Original report</h2>"))
        self.refused(OLD.replace("Old facts.", "a rule <hr> here"))

    def test_indented_text_after_the_section_is_refused(self):
        self.refused(OLD + "\n    indented code or list continuation\n", pattern="indented")
        self.assertEqual(self.replaced(OLD + "\n   \nKeep.\n"), NEW + "\n   \nKeep.\n")

    def test_heading_must_follow_a_blank_line(self):
        self.refused("Intro text\n" + OLD, pattern="blank line")

    def test_lone_carriage_return_is_refused(self):
        self.refused("## Current state: 2026-11-18\n\nOld.\r## Original\r\rTEXT\n\n**Next action:** x.\n")
        self.refused(OLD.replace("review #1.", "x.\rORIGINAL CONTINUATION"))
        with self.assertRaises(replace_state.SectionError):
            replace_state.normalize_section(NEW.replace("New facts.", "New.\rfacts."))

    def test_html_anywhere_in_a_line_is_refused(self):
        for line in ("New. <details><summary>s</summary>", "text <!-- hide", "a --> b"):
            section = f"## Current state: 2026-11-19\n\n{line}\n\n**Next action:** y.\n"
            with self.assertRaises(replace_state.SectionError, msg=line):
                replace_state.normalize_section(section)
            self.refused(OLD.replace("Old facts.", line))
        with self.assertRaises(replace_state.SectionError):
            replace_state.normalize_section(NEW.replace("merge #1.", "y <!--"))

    def test_second_rewrite_guard_refuses_a_result_that_moves(self):
        with mock.patch.object(replace_state, "splice", side_effect=["A\n", "B\n"]):
            self.refused(OLD, pattern="second rewrite")

    def test_bom_at_the_start_of_the_body_is_dropped(self):
        self.assertEqual(self.replaced("\ufeff" + OLD + "\nKeep.\n"), NEW + "\nKeep.\n")

    def test_unusual_line_separators_do_not_split_lines(self):
        prefix = "Intro\x0cpage\u2028line\x85end.\n\n"
        tail = "Old.\u2028```\x0c\n\n## Original\x1c\n\nKEEP\n"
        self.assertEqual(self.replaced(prefix + OLD + "\n" + tail), prefix + NEW + "\n" + tail)

    def test_absent_section_is_an_error_without_insert(self):
        body = "## What happens\n\nKeep.\n"
        self.refused(body, pattern="--insert")
        self.assertEqual(self.replaced(body, insert=True), NEW + "\n" + body)

    def test_insert_into_empty_body(self):
        self.assertEqual(self.replaced(None, insert=True), NEW)

    def test_crlf_body_is_normalized(self):
        body = (OLD + "\nKeep.\n").replace("\n", "\r\n")
        self.assertEqual(self.replaced(body), NEW + "\nKeep.\n")

    def test_two_sections_are_refused_even_if_one_is_fenced(self):
        self.refused(OLD + "\n" + OLD, pattern="2 Current state headings")
        self.refused(OLD + "\n```\n## Current state: 2026-01-01\n```\n", pattern="2 Current state headings")

    def test_loose_heading_is_not_duplicated_by_insert(self):
        for body in (
            "## Current state: 2026-11-18 (sweep v4)\n\nOld.\n",
            "### Current state\n\nOld.\n",
            "<h2>Current state</h2>\n\nOld.\n",
        ):
            self.refused(body, pattern="another form", insert=True)

    def test_section_file_rules(self):
        bad = (
            "New facts only.\n",
            "## Current state\n\nundated\n\n**Next action:** a.\n",
            "Intro\n" + NEW,
            "## Current state: 2026-11-19\n\nA.\n",
            "## Current state: 2026-11-19\n\n**Next action:** a.\n\nTrailing.\n",
            "## Current state: 2026-11-19\n\n**Next action:** a.\ncontinued.\n",
            "## Current state: 2026-11-19\n\n### Done\n\n**Next action:** a.\n",
            "## Current state: 2026-11-19\n\n---\n\n**Next action:** a.\n",
            "## Current state: 2026-11-19\n\n```\ncode\n```\n\n**Next action:** a.\n",
        )
        for section in bad:
            # normalize_section itself, not only the second-rewrite guard behind it.
            with self.assertRaises(replace_state.SectionError, msg=section):
                replace_state.normalize_section(section)
            with self.assertRaises(replace_state.SectionError, msg=section):
                replace_state.replace_section(OLD, section)
        ok = "﻿## Current state: 2026-11-19\n\n- done\n| a | b |\n\n**Next action:** a.\n"
        self.assertEqual(self.replaced(OLD), NEW)
        self.assertEqual(self.replaced(OLD, ok), ok.lstrip("﻿"))

    def test_every_result_survives_a_second_rewrite(self):
        bodies = (
            "Intro.\n\n" + OLD + "\nOriginal text.\n\n### Prompt\n",
            "## What happens\n\nNo headings below.\n",
            "",
        )
        other = NEW.replace("New facts.", "Newer facts.")
        for body in bodies:
            once = self.replaced(body, insert=True)
            self.assertEqual(self.replaced(once, other), once.replace("New facts.", "Newer facts."), body)

    def test_structure_inside_a_quote_or_list_is_refused(self):
        # A quoted or listed heading is still original text that a section
        # with no marker would otherwise run over.
        for line in (
            "> ## Original report", "- ## Original report", "1. ## Original report",
            "> > ### Done", "  * #### Notes", "2) ## Steps", "> ```", "- ~~~", "> ---",
        ):
            body = f"## Current state: 2026-11-18\n\nOld.\n\n{line}\n> crash\n\n**Next action:** x.\n\nKeep.\n"
            self.refused(body, pattern="edit it by hand")
        for line in ("> quoted text", "- item", "1. step", "- `code` in a list", "-1 is a number"):
            body = f"## Current state: 2026-11-18\n\nOld.\n\n{line}\n\n**Next action:** x.\n\nKeep.\n"
            self.assertEqual(self.replaced(body), NEW + "\nKeep.\n", line)

    def test_random_bodies_either_refuse_or_keep_prefix_and_tail(self):
        import random

        safe = ["Plain.", "- item", "| a | b |", "> quote", "   ", "\t"]
        blocks = safe + [
            "## Heading", "### Sub", "Title\n===", "Title\n---", "---", "* * *",
            "```\n## fenced\n```", "~~~\ncode\n~~~", "<details>\nx\n</details>", "<!-- c -->",
            "<h2>H</h2>", "**Next action:** stray.", "A --> B", "x <!-- y",
        ]
        openers = ["```", "~~~", "<!--", "<details>", "<pre>", "- ```sh", "<div>", "<?x", "    ```"]
        rng = random.Random(406)
        accepted = 0
        for _ in range(3000):
            prefix_blocks = rng.sample(blocks, rng.randint(0, 3))
            if rng.random() < 0.2:
                prefix_blocks.append(rng.choice(openers))
            prefix = "\n\n".join(prefix_blocks)
            middle_blocks = rng.sample(blocks, rng.randint(0, 3))
            middle = "\n\n".join(middle_blocks)
            tail = "\n\n".join(rng.sample(blocks, rng.randint(0, 4)))
            old = "## Current state: 2026-11-18\n\nOld.\n\n" + (middle + "\n\n" if middle else "")
            old += "**Next action:** old.\n" if rng.random() < 0.8 else ""
            body = (prefix + "\n\n" if prefix else "") + old + ("\n" + tail + "\n" if tail else "")
            try:
                out = self.replaced(body)
            except replace_state.SectionError:
                continue
            accepted += 1
            # Accepted only if nothing before the Next action line could be structure.
            self.assertNotIn(prefix_blocks[-1] if prefix_blocks else None, openers, body)
            before_action = []
            for block in middle_blocks:
                if block.startswith("**Next action:**"):
                    break
                before_action.append(block)
            self.assertTrue(set(before_action) <= set(safe), body)
            # Everything before the heading and after the first Next action
            # line it accepted must survive verbatim.
            start = body.index("## Current state:")
            action_end = body.index("\n", body.index("**Next action:**", start))
            kept = body[action_end + 1 :].lstrip("\n")
            self.assertEqual(out, body[:start] + NEW + ("\n" + kept if kept else ""), body)
        self.assertGreater(accepted, 300)

    def test_same_section_is_unchanged(self):
        body = NEW + "\nKeep.\n"
        self.assertEqual(self.replaced(body), body)


FAKE_GH = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json, os, sys
    root = os.environ["FAKE_GH_DIR"]
    args = sys.argv[1:]
    with open(os.path.join(root, "calls.log"), "a") as log:
        log.write(json.dumps(args) + "\\n")
    if "-X" in args:
        assert args[args.index("-X") + 1] == "PATCH", args
        field = args[args.index("-F") + 1]
        assert field.startswith("body=@"), field
        with open(field[len("body=@"):]) as src, open(os.path.join(root, "patched.md"), "w") as dst:
            dst.write(src.read())
        print("https://github.com/example/issues/1")
    else:
        counter = os.path.join(root, "get-count")
        n = int(open(counter).read()) if os.path.exists(counter) else 0
        open(counter, "w").write(str(n + 1))
        name = "issue-reread.json" if n and os.path.exists(os.path.join(root, "issue-reread.json")) else "issue.json"
        with open(os.path.join(root, name)) as f:
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
        (self.dir / "issue.json").write_text(
            json.dumps({"number": 5, "body": self.body, "updated_at": "2026-11-19T01:00:00Z"})
        )
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
        [before] = self.out.glob("5.*.before.md")
        [after] = self.out.glob("5.*.after.md")
        self.assertEqual(before.read_text(), self.body)
        self.assertEqual(after.read_text(), "Intro.\n\n" + NEW + "\nKeep.\n")
        self.assertFalse((self.dir / "patched.md").exists())
        self.assertEqual([c for c in self.calls() if "-X" in c], [])

    def test_apply_patches_the_after_body(self):
        result = self.script("--apply", "--repo", "Example/repo")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.dir / "patched.md").read_text(), "Intro.\n\n" + NEW + "\nKeep.\n")
        patch = [c for c in self.calls() if "-X" in c]
        self.assertEqual(len(patch), 1)
        self.assertEqual(patch[0][:4], ["api", "-X", "PATCH", "repos/Example/repo/issues/5"])

    def test_default_out_dir_inside_a_git_work_tree_is_refused(self):
        repo = self.dir / "repo"
        repo.mkdir()
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        env = dict(
            os.environ, PATH=f"{self.dir}{os.pathsep}{os.environ['PATH']}", FAKE_GH_DIR=str(self.dir)
        )
        result = subprocess.run(
            [sys.executable, str(HERE / "replace-state.py"), "5", str(self.section)],
            env=env,
            cwd=repo,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("--out-dir", result.stderr)
        self.assertEqual(list(repo.iterdir()), [repo / ".git"])

    def test_before_copy_is_the_exact_original(self):
        crlf = self.body.replace("\n", "\r\n")
        (self.dir / "issue.json").write_text(json.dumps({"number": 5, "body": crlf}))
        self.assertEqual(self.script().returncode, 0)
        [before] = self.out.glob("5.*.before.md")
        self.assertEqual(before.read_bytes(), crlf.encode())

    def test_only_a_byte_order_mark_to_drop_is_no_change(self):
        same = "\ufeffIntro.\n\n" + NEW + "\nKeep.\n"
        (self.dir / "issue.json").write_text(json.dumps({"number": 5, "body": same}))
        result = self.script("--apply")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no change", result.stdout)
        self.assertEqual([c for c in self.calls() if "-X" in c], [])

    def test_a_second_run_keeps_the_first_copies(self):
        self.assertEqual(self.script().returncode, 0)
        self.assertEqual(self.script().returncode, 0)
        self.assertEqual(len(list(self.out.glob("5.*.before.md"))), 2)

    def test_apply_refuses_when_the_issue_changed_after_it_was_read(self):
        (self.dir / "issue-reread.json").write_text(
            json.dumps({"number": 5, "body": "edited", "updated_at": "2026-11-19T02:00:00Z"})
        )
        result = self.script("--apply")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("changed", result.stderr)
        self.assertFalse((self.dir / "patched.md").exists())

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
