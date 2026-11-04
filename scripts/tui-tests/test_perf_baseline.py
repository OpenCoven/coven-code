"""Performance harness profile regressions; spawns no engine."""
import json
import os
import subprocess
import tempfile
import unittest

import perf_baseline as perf


class ProfileTests(unittest.TestCase):
    def test_classic_profile_sets_no_selector(self):
        # Classic baselines stay comparable with the recorded #425 numbers.
        env = perf.session_env("/h", "classic")
        self.assertNotIn("COVEN_CODE_UI", env)
        self.assertEqual(env["HOME"], "/h")

    def test_next_profile_selects_next(self):
        self.assertEqual(perf.session_env("/h", "next")["COVEN_CODE_UI"], "next")

    def test_environment_carries_no_inherited_credentials(self):
        os.environ["ANTHROPIC_API_KEY"] = "must-not-reach-the-engine"
        self.addCleanup(os.environ.pop, "ANTHROPIC_API_KEY")
        self.assertNotIn("ANTHROPIC_API_KEY", perf.session_env("/h", "next"))

    def test_each_renderer_has_its_own_usable_frame(self):
        self.assertEqual(perf.ready_needles("classic", False), [perf.BOOT, perf.PROMPT])
        self.assertEqual(perf.ready_needles("next", False), [b"preview", b"ready"])

    def test_a_resume_that_failed_does_not_count_as_a_usable_frame(self):
        # The engine falls back to an empty session when a resume fails, which
        # would otherwise measure as a very fast long conversation.
        self.assertIn(perf.SEEDED_MARK, perf.ready_needles("next", True))
        self.assertIn(perf.RESUMED_MARK, perf.ready_needles("classic", True))
        for ui in ("classic", "next"):
            self.assertFalse({perf.SEEDED_MARK, perf.RESUMED_MARK} & set(perf.ready_needles(ui, False)))


class SeededHistoryTests(unittest.TestCase):
    def seed(self, lines):
        root = tempfile.TemporaryDirectory()
        self.addCleanup(root.cleanup)
        home = os.path.join(root.name, "home")
        project = os.path.join(root.name, "project")
        os.makedirs(project)
        session_id = perf.seed_session(home, project, lines)
        path = os.path.join(home, ".coven-code", "sessions", f"{session_id}.json")
        with open(path, encoding="utf-8") as f:
            return json.load(f), project

    def test_session_holds_the_requested_lines(self):
        session, project = self.seed(2_500)
        answers = [m for m in session["messages"] if m["role"] == "assistant"]
        self.assertEqual(sum(m["content"].count("\n") for m in answers), 2_500)
        self.assertEqual(session["working_dir"], project)

    def test_seeded_text_cannot_satisfy_a_keystroke_probe(self):
        # A repaint of seeded text must not look like the key being painted.
        session, _ = self.seed(250)
        text = "".join(m["content"] for m in session["messages"]).encode()
        self.assertFalse(set(text) & set(perf.KEYS))


class ReadDeadlineTests(unittest.TestCase):
    def test_a_deadline_that_just_passed_is_not_an_error(self):
        read_end, write_end = os.pipe()
        self.addCleanup(os.close, read_end)
        self.addCleanup(os.close, write_end)
        session = perf.Session.__new__(perf.Session)
        session.master = read_end
        session.answered = {}
        self.assertEqual(session.read(-0.001), (b"", None))


class StreamingProfileTests(unittest.TestCase):
    def fake(self, chunks):
        root = tempfile.TemporaryDirectory()
        self.addCleanup(root.cleanup)
        path = perf.write_fake_claude(root.name, chunks=chunks, delay="0")
        out = subprocess.run([path], input=b"", capture_output=True, timeout=10, check=True)
        return [json.loads(line) for line in out.stdout.splitlines()]

    def test_fake_transport_streams_each_chunk_then_finishes(self):
        events = self.fake(3)
        self.assertEqual([e["type"] for e in events],
                         ["system", "assistant", "assistant", "assistant", "result"])
        texts = [e["message"]["content"][0]["text"] for e in events[1:-1]]
        self.assertEqual(len(set(texts)), 3)
        self.assertTrue(all(perf.STREAM_MARK.decode() in t for t in texts))

    def test_streamed_text_cannot_satisfy_a_keystroke_probe(self):
        text = "".join(e["message"]["content"][0]["text"] for e in self.fake(3)[1:-1])
        self.assertFalse(set(text.encode()) & set(perf.STREAM_KEYS))

    def test_stream_probes_avoid_letters_in_the_working_status(self):
        # The status row repaints during a turn; a probe key must not match it.
        self.assertFalse(set(b"working") & set(perf.STREAM_KEYS))

    def test_stream_environment_reaches_only_the_fake(self):
        env = perf.session_env("/h", "next", claude_bin="/tmp/fake")
        self.assertEqual(env["COVEN_CODE_CLAUDE_BIN"], "/tmp/fake")
        self.assertNotIn("COVEN_CODE_CLAUDE_BIN", perf.session_env("/h", "next"))

    def test_paint_gaps_merge_chunks_of_one_frame(self):
        # A frame split across PTY reads is one paint, not several tiny gaps.
        self.assertEqual(perf.paint_gaps_ms([0.0, 0.0002, 0.020, 0.0403]), [20.0, 20.3])
        self.assertEqual(perf.paint_gaps_ms([1.0]), [])


if __name__ == "__main__":
    unittest.main()
