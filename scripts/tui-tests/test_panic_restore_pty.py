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
fd = 0
saved = termios.tcgetattr(fd)
if BEHAVIOUR != "no-setup":
    tty.setraw(fd)

def write(stream, data):
    stream.buffer.write(data)
    stream.buffer.flush()

def restore():
    termios.tcsetattr(fd, termios.TCSANOW, saved)
    write(sys.stdout, b"\x1b[?1049l\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
                      b"\x1b[?2004l\x1b[<1u\x1b[?25h")

def message(thread, text):
    write(sys.stderr, b"\nthread '" + thread + b"' (7) panicked at fake.rs:1:1:\n" + text + b"\n")

if BEHAVIOUR != "no-setup":
    write(sys.stdout, b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h\x1b[?2004h")
    if os.environ.get("COVEN_CODE_KEYBOARD_ENHANCEMENT") != "0":
        write(sys.stdout, b"\x1b[>9u")
    write(sys.stdout, b"\x1b[?25l")
write(sys.stdout, b"\x1b[1;1Hpanic-restore: ready")
os.read(fd, 64)  # one byte in raw mode, a whole line in canonical mode
if sys.argv[2] == "worker":
    if BEHAVIOUR == "worker-restores":
        restore()
    message(b"<unnamed>", b"panic-restore: deliberate worker-thread panic")
    write(sys.stdout, b"\x1b[1;1Hpanic-restore: worker panicked")
    os.read(fd, 64)
if BEHAVIOUR == "message-first":
    message(b"main", b"panic-restore: deliberate main-thread panic")
if BEHAVIOUR != "no-restore":
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
                capture_output=True, text=True, encoding="utf-8", timeout=120,
            )
        failed = [line for line in result.stdout.splitlines() if line.startswith("FAIL ")]
        return result, failed

    def test_hook_order_passes(self):
        result, failed = self.drive("faithful")
        self.assertEqual(failed, [], result.stdout + result.stderr)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("ok classic worker: a worker-thread panic leaves the terminal alone", result.stdout)

    def test_missing_restore_fails(self):
        result, failed = self.drive("no-restore")
        self.assertEqual(result.returncode, 1)
        for check in ("every termios field is restored", "termios is restored before the panic message",
                      "leaves the alternate screen",
                      "turns mouse capture off", "turns bracketed paste off", "shows the cursor",
                      "pops the keyboard flags"):
            self.assertTrue(any(check in line for line in failed), check + "\n" + result.stdout)

    def test_restore_on_a_worker_panic_fails(self):
        result, failed = self.drive("worker-restores")
        self.assertEqual(result.returncode, 1)
        self.assertTrue(any("worker-thread panic leaves the terminal alone" in line for line in failed),
                        result.stdout)

    def test_restore_after_the_message_fails(self):
        result, failed = self.drive("message-first")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ok classic main: every termios field is restored", result.stdout)
        for check in ("termios is restored before the panic message",
                      "leaves the alternate screen before the panic message"):
            self.assertTrue(any(check in line for line in failed), check + "\n" + result.stdout)

    def test_probe_that_never_takes_the_terminal_fails(self):
        result, failed = self.drive("no-setup")
        self.assertEqual(result.returncode, 1)
        for check in ("the probe puts the PTY in raw mode", "the probe enters TUI mode",
                      "keyboard flags are pushed once"):
            self.assertTrue(any(check in line for line in failed), check + "\n" + result.stdout)


if __name__ == "__main__":
    unittest.main()
