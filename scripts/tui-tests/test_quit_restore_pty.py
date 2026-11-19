"""quit_restore_pty.py against fake engines on a real PTY; no engine, tmux
server or cargo."""
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent

# Takes the terminal the way setup_terminal does, draws the text the driver
# waits for, exits on the second Ctrl+C and restores as BEHAVIOUR says.
FAKE = r'''
import os, sys, termios, tty
BEHAVIOUR = {behaviour!r}
fd = 0
saved = termios.tcgetattr(fd)

def write(data):
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()

tty.setraw(fd)
write(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?2004h")
if BEHAVIOUR != "no-push":
    write(b"\x1b[>9u")
ready = b"preview" if os.environ.get("COVEN_CODE_UI") == "next" else b"Coven v0.0.0"
write(b"\x1b[?25l\x1b[1;1H" + ready)
interrupts = 0
while interrupts < 2:
    data = os.read(fd, 64)
    if not data:
        sys.exit(1)  # the driver went away; never spin on a closed PTY
    interrupts += data.count(b"\x03")
termios.tcsetattr(fd, termios.TCSANOW, saved)
pop = b"\x1b[<1u"
leave = b"\x1b[?1049l\x1b[?1006l\x1b[?1000l\x1b[?2004l"
if BEHAVIOUR == "pop-after-leave":
    write(leave + pop)
elif BEHAVIOUR == "pop-on-both-screens":
    write(pop + leave + pop)
elif BEHAVIOUR == "no-pop":
    write(leave)
else:
    write(pop + leave)
write(b"\x1b[?25h")
'''

PUSHED = "keyboard flags are pushed on the alternate screen"
NOTHING_LEFT = "quitting leaves no terminal mode or keyboard flags behind"
POP_FIRST = "the one keyboard pop is sent before leaving the alternate screen"
MAIN_SCREEN = "no keyboard pop is sent on the main screen"


class QuitRestoreDriverTests(unittest.TestCase):
    def drive(self, behaviour):
        with tempfile.TemporaryDirectory(prefix="coven-fake-engine-") as temp:
            engine = pathlib.Path(temp) / "coven-code"
            engine.write_text(f"#!{sys.executable}\n" + FAKE.format(behaviour=behaviour), encoding="utf-8")
            engine.chmod(0o700)
            result = subprocess.run(
                [sys.executable, "-B", str(HERE / "quit_restore_pty.py"), str(engine), "80", "24"],
                capture_output=True, text=True, encoding="utf-8", timeout=180,
            )
        failed = [line for line in result.stdout.splitlines() if line.startswith("FAIL ")]
        return result, failed

    def assert_only_fails(self, behaviour, *checks):
        """Both renderers fail exactly `checks`."""
        result, failed = self.drive(behaviour)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        expected = sorted(f"FAIL {ui}: {check}" for ui in ("classic", "next") for check in checks)
        self.assertEqual(sorted(line.split(" :: ")[0] for line in failed), expected, result.stdout)

    def test_pop_before_leaving_the_alternate_screen_passes(self):
        result, failed = self.drive("faithful")
        self.assertEqual(failed, [], result.stdout + result.stderr)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for ui in ("classic", "next"):
            for check in (PUSHED, NOTHING_LEFT, POP_FIRST, MAIN_SCREEN):
                self.assertIn(f"ok {ui}: {check}", result.stdout)

    def test_pop_after_leaving_the_alternate_screen_fails(self):
        # The order restore_terminal had before #410.
        self.assert_only_fails("pop-after-leave", NOTHING_LEFT, POP_FIRST, MAIN_SCREEN)

    def test_second_pop_on_the_main_screen_fails(self):
        self.assert_only_fails("pop-on-both-screens", POP_FIRST, MAIN_SCREEN)

    def test_missing_pop_fails(self):
        self.assert_only_fails("no-pop", NOTHING_LEFT, POP_FIRST)

    def test_engine_that_pushes_no_flags_fails(self):
        self.assert_only_fails("no-push", PUSHED)


if __name__ == "__main__":
    unittest.main()
