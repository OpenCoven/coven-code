"""panic_restore_pty.py against fake probes on a real PTY; no engine or cargo."""
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent

# Writes what setup_terminal and the panic hook write, in the same order, then
# misbehaves as BEHAVIOUR says.
FAKE = r'''
import os, sys, termios, tty
BEHAVIOUR = {behaviour!r}
KEYBOARD = os.environ.get("COVEN_CODE_KEYBOARD_ENHANCEMENT") != "0"
fd = 0
saved = termios.tcgetattr(fd)

def write(stream, data):
    stream.buffer.write(data)
    stream.buffer.flush()

def setup():
    if BEHAVIOUR == "no-setup":
        return
    tty.setraw(fd)
    write(sys.stdout, b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h\x1b[?2004h")
    if KEYBOARD:
        write(sys.stdout, b"\x1b[>9u")
    write(sys.stdout, b"\x1b]0;Coven\x07\x1b[?25l")

def restore():
    termios.tcsetattr(fd, termios.TCSANOW, saved)
    pop = b"" if BEHAVIOUR == "no-unpushed-pop" and not KEYBOARD else b"\x1b[<1u"
    modes = b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2004l"
    if BEHAVIOUR == "pop-first":
        write(sys.stdout, pop + b"\x1b[?1049l" + modes + b"\x1b[?25h")
    else:
        write(sys.stdout, b"\x1b[?1049l" + modes + pop + b"\x1b[?25h")
    if BEHAVIOUR == "title-reset":
        write(sys.stdout, b"\x1b]0;\x07")

def message(thread, text):
    write(sys.stderr, b"\nthread '" + thread + b"' (7) panicked at fake.rs:1:1:\n" + text + b"\n")

setup()
write(sys.stdout, b"\x1b[1;1Hpanic-restore: ready")
os.read(fd, 64)  # one byte in raw mode, a whole line in canonical mode
setups = 1
if sys.argv[2] == "worker":
    if BEHAVIOUR == "worker-restores":
        restore()
    message(b"<unnamed>", b"panic-restore: deliberate worker-thread panic")
    write(sys.stdout, b"\x1b[1;1Hpanic-restore: worker panicked")
    os.read(fd, 64)
elif sys.argv[2] == "resume":
    restore()
    setup()
    setups = 2
    write(sys.stdout, b"\x1b[2J\x1b[1;1Hpanic-restore: resumed")
    os.read(fd, 64)
if BEHAVIOUR == "message-first":
    message(b"main", b"panic-restore: deliberate main-thread panic")
if BEHAVIOUR != "no-restore":
    for _ in range(1 if BEHAVIOUR == "single-restore" else setups):
        restore()
if BEHAVIOUR != "message-first":
    message(b"main", b"panic-restore: deliberate main-thread panic")
sys.exit(101)
'''


class PanicRestoreDriverTests(unittest.TestCase):
    def drive(self, behaviour):
        with tempfile.TemporaryDirectory(prefix="coven-fake-probe-") as temp:
            probe = pathlib.Path(temp) / "panic_restore"
            probe.write_text(f"#!{sys.executable}\n" + FAKE.format(behaviour=behaviour), encoding="utf-8")
            probe.chmod(0o700)
            result = subprocess.run(
                [sys.executable, "-B", str(HERE / "panic_restore_pty.py"), str(probe)],
                capture_output=True, text=True, encoding="utf-8", timeout=180,
            )
        failed = [line for line in result.stdout.splitlines() if line.startswith("FAIL ")]
        return result, failed

    def assert_fails(self, behaviour, *checks):
        result, failed = self.drive(behaviour)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        for check in checks:
            self.assertTrue(any(check in line for line in failed), check + "\n" + result.stdout)
        return result, failed

    def test_hook_order_passes(self):
        result, failed = self.drive("faithful")
        self.assertEqual(failed, [], result.stdout + result.stderr)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("ok classic worker: a worker-thread panic does not restore the terminal", result.stdout)
        self.assertIn("ok next resume: current behaviour: the hook restores once per setup_terminal call (2)",
                      result.stdout)

    def test_missing_restore_fails(self):
        self.assert_fails("no-restore", "every termios field is restored",
                          "termios is restored before the panic message", "leaves the alternate screen",
                          "turns mouse capture off", "turns bracketed paste off", "shows the cursor",
                          "sends a keyboard pop")

    def test_restore_on_a_worker_panic_fails(self):
        self.assert_fails("worker-restores", "a worker-thread panic does not restore the terminal")

    def test_restore_after_the_message_fails(self):
        result, _ = self.assert_fails("message-first", "termios is restored before the panic message",
                                      "leaves the alternate screen before the panic message")
        self.assertIn("ok classic main: every termios field is restored", result.stdout)

    def test_probe_that_never_takes_the_terminal_fails(self):
        self.assert_fails("no-setup", "the probe puts the PTY in raw mode", "the probe enters TUI mode",
                          "keyboard flags are pushed")

    def test_each_pinned_behaviour_fails_on_its_own(self):
        for behaviour, check in (
            ("pop-first", "current behaviour: the keyboard pop is sent after leaving the alternate screen"),
            ("no-unpushed-pop", "current behaviour: a keyboard pop is sent though none was pushed"),
            ("title-reset", "current behaviour: the hook leaves the title set at startup"),
            ("single-restore", "current behaviour: the hook restores once per setup_terminal call"),
        ):
            with self.subTest(behaviour):
                _, failed = self.assert_fails(behaviour, check)
                self.assertEqual([line for line in failed if check not in line], [])


if __name__ == "__main__":
    unittest.main()
