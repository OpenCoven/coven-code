"""Raw-PTY acceptance observers; no engine, provider, or external packages."""
import copy
import sys
import unittest

import zero_size_pty as tiny


class TerminalObserverTests(unittest.TestCase):
    def test_cleanup_before_reenable_does_not_prove_restoration(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed(b"\x1b[?1049l\x1b[?25h\x1b[?1000;1006;2004l\x1b[<u")
        observer.feed(b"\x1b[?1049h\x1b[?25l\x1b[?1000;1006;2004h\x1b[>1u")
        self.assertEqual(set(observer.restoration_failures()), {
            "alternate screen active", "cursor hidden", "mouse mode 1000 active",
            "mouse mode 1006 active", "bracketed paste active", "keyboard stack depth 1",
        })
        observer.feed(b"\x1b[?1049l\x1b[?25h\x1b[?1000;1006;2004l\x1b[<u")
        self.assertEqual(observer.restoration_failures(), [])

    def test_every_keyboard_push_requires_a_later_pop(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed(b"\x1b[>1u\x1b[>2u\x1b[<u")
        self.assertIn("keyboard stack depth 1", observer.restoration_failures())
        observer.feed(b"\x1b[<u")
        self.assertEqual(observer.restoration_failures(), [])

    def test_current_screen_rejects_canary_erased_by_a_later_frame(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed(("\x1b[7;1H" + "─" * 40 + "\x1b[8;1H❯ qzv424canary"
                       + "\x1b[9;1H" + "─" * 40).encode())
        self.assertTrue(observer.composer_contains("qzv424canary"))
        observer.feed(b"\x1b[2J\x1b[Hold banner")
        self.assertFalse(observer.composer_contains("qzv424canary"))

    def test_canary_in_a_transcript_row_is_not_a_composer(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed("❯ \x1b[5;1Htranscript qzv424canary".encode())
        self.assertFalse(observer.composer_contains("qzv424canary"))

    def test_same_row_transcript_pointer_cannot_satisfy_composer_probe(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed(b"\x1b[2;1Htranscript " + "❯ qzv424canary1".encode())
        self.assertFalse(observer.composer_contains("qzv424canary1"))

    def test_scrollback_erase_keeps_visible_frame(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed(b"\x1b[2;1Hvisible frame\x1b[3J")
        self.assertIn("visible frame", observer.text())

    def test_unbordered_bottom_row_cannot_satisfy_composer_probe(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed("\x1b[8;1H❯ qzv424canary1".encode())
        self.assertFalse(observer.composer_contains("qzv424canary1"))

    def test_bordered_transcript_above_input_cannot_satisfy_composer_probe(self):
        observer = tiny.TerminalObserver(40, 40)
        observer.feed(("\x1b[2;1H" + "─" * 40 + "\x1b[3;1H❯ qzv424canary1"
                       + "\x1b[4;1H" + "─" * 40).encode())
        self.assertFalse(observer.composer_contains("qzv424canary1"))

    def test_split_utf8_and_escape_sequences_update_the_same_frame(self):
        observer = tiny.TerminalObserver(40, 12)
        stream = ("\x1b[?1049h\x1b[7;1H" + "─" * 40 + "\x1b[8;1H❯ qzv424canary"
                  + "\x1b[9;1H" + "─" * 40 + "\x1b[?25h").encode()
        for byte in stream:
            observer.feed(bytes([byte]))
        self.assertTrue(observer.composer_contains("qzv424canary"))
        observer.resize(120, 40)
        self.assertFalse(observer.composer_contains("qzv424canary"))

    def test_line_erase_and_cursor_moves_remove_stale_text(self):
        observer = tiny.TerminalObserver(40, 12)
        observer.feed(("\x1b[7;1H" + "─" * 40 + "\x1b[8;1H❯ qzv424canary"
                       + "\x1b[9;1H" + "─" * 40 + "\x1b[8;1H\x1b[2K❯ new").encode())
        self.assertFalse(observer.composer_contains("qzv424canary"))
        self.assertTrue(observer.composer_contains("new"))


class TermiosTests(unittest.TestCase):
    def attributes(self):
        return [1, 2, 3, 4, 9600, 9600, [b"a", b"b", 1, 0]]

    def test_every_persistent_field_is_compared(self):
        before = self.attributes()
        for index in range(7):
            after = copy.deepcopy(before)
            if index == 6:
                after[index][0] = b"z"
            else:
                after[index] += 1
            self.assertFalse(tiny.termios_restored(before, after), index)

    @unittest.skipUnless(sys.platform == "darwin", "macOS PENDIN only")
    def test_only_macos_pendin_is_ignored_in_local_flags(self):
        before = self.attributes()
        after = copy.deepcopy(before)
        after[3] |= 0x20000000
        self.assertTrue(tiny.termios_restored(before, after))
        after[0] |= 0x20000000
        self.assertFalse(tiny.termios_restored(before, after))


if __name__ == "__main__":
    unittest.main()
