"""panic_restore_pty.py and case 30 against fake probes on a real PTY; no
engine, tmux server or cargo."""
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent

# Writes what setup_terminal and the panic hook write, in the same order
# (title cleared, keyboard pop if flags were pushed, then leave the alternate
# screen, once per panic), then misbehaves as BEHAVIOUR says.
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
    if BEHAVIOUR == "keeps-opost":
        attrs = termios.tcgetattr(fd)
        attrs[1] |= termios.OPOST
        termios.tcsetattr(fd, termios.TCSANOW, attrs)
    write(sys.stdout, b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h\x1b[?2004h")
    if KEYBOARD:
        write(sys.stdout, b"\x1b[>9u")
    write(sys.stdout, b"\x1b]0;Coven\x07\x1b[?25l")

def restore():
    termios.tcsetattr(fd, termios.TCSANOW, saved)
    pop = b"\x1b[<1u" if KEYBOARD or BEHAVIOUR == "unpushed-pop" else b""
    title = b"" if BEHAVIOUR == "title-kept" else b"\x1b]0;\x07"
    if BEHAVIOUR == "title-set-again":
        title += b"\x1b]0;Coven\x07"
    modes = b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2004l"
    if BEHAVIOUR == "pop-after-leave":
        write(sys.stdout, title + b"\x1b[?1049l" + modes + pop + b"\x1b[?25h")
    elif BEHAVIOUR == "pop-on-both-screens":
        # The second pop is the stray one, sent whether or not flags were pushed.
        write(sys.stdout, title + pop + b"\x1b[?1049l" + modes + b"\x1b[<1u\x1b[?25h")
    else:
        write(sys.stdout, title + pop + b"\x1b[?1049l" + modes + b"\x1b[?25h")

def message(thread, text):
    write(sys.stderr, b"\nthread '" + thread + b"' (7) panicked at fake.rs:1:1:\n" + text + b"\n")

setup()
write(sys.stdout, b"\x1b[1;1Hpanic-restore: ready")
os.read(fd, 64)  # one byte in raw mode, a whole line in canonical mode
setups = 1
if sys.argv[2] == "worker":
    if BEHAVIOUR == "worker-restores":
        restore()
    if BEHAVIOUR != "worker-silent":
        message(b"<unnamed>", b"panic-restore: deliberate worker-thread panic")
    if BEHAVIOUR == "worker-dies":
        sys.exit(101)
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
    for _ in range(setups if BEHAVIOUR == "restore-per-setup" else 1):
        restore()
if BEHAVIOUR != "message-first":
    message(b"main", b"panic-restore: deliberate main-thread panic")
sys.exit(101)
'''


def write_fake_probe(path, behaviour="faithful"):
    path.write_text(f"#!{sys.executable}\n" + FAKE.format(behaviour=behaviour), encoding="utf-8")
    path.chmod(0o700)


class PanicRestoreDriverTests(unittest.TestCase):
    def drive(self, behaviour):
        with tempfile.TemporaryDirectory(prefix="coven-fake-probe-") as temp:
            probe = pathlib.Path(temp) / "panic_restore"
            write_fake_probe(probe, behaviour)
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
        self.assertIn("ok next resume: the hook restores once (2 setup_terminal calls)", result.stdout)
        self.assertIn("ok classic main without keyboard flags: no keyboard pop is sent when none was pushed",
                      result.stdout)

    def test_missing_restore_fails(self):
        self.assert_fails("no-restore", "every termios field is restored",
                          "output processing is restored before the panic message", "leaves the alternate screen",
                          "turns mouse capture off", "turns bracketed paste off", "shows the cursor",
                          "pops the keyboard flags it pushed", "the keyboard pop is sent before leaving",
                          "no keyboard pop is sent on the main screen")

    def test_restore_on_a_worker_panic_fails(self):
        self.assert_fails("worker-restores", "a worker-thread panic does not restore the terminal")

    def test_worker_panic_that_ends_the_probe_fails(self):
        check = "the probe keeps running and drawing after a worker-thread panic"
        _, failed = self.assert_fails("worker-dies", check)
        self.assertEqual([line for line in failed if check not in line], [])

    def test_restore_after_the_message_fails(self):
        result, _ = self.assert_fails("message-first", "output processing is restored before the panic message",
                                      "leaves the alternate screen before the panic message")
        self.assertIn("ok classic main: every termios field is restored", result.stdout)

    def test_probe_that_never_takes_the_terminal_fails(self):
        self.assert_fails("no-setup", "the probe puts the PTY in raw mode", "the probe enters TUI mode",
                          "keyboard flags are pushed")

    def test_raw_mode_must_turn_output_processing_off(self):
        # Otherwise the CRLF check could not tell when termios came back.
        check = "the probe puts the PTY in raw mode"
        _, failed = self.assert_fails("keeps-opost", check)
        self.assertEqual([line for line in failed if check not in line], [])

    def test_pop_after_leaving_the_alternate_screen_fails(self):
        # The hook's order before #410: the pop lands on the main screen's
        # stack and the flags stay pushed on the alternate one.
        # A resume restores and sets up again, so there the alternate
        # screen's stack is already two deep before the panic.
        checks = ("pops the keyboard flags it pushed before the panic message",
                  "the keyboard pop is sent before leaving the alternate screen",
                  "no keyboard pop is sent on the main screen",
                  "resume after resuming: keyboard flags are pushed, one level deep")
        result, failed = self.assert_fails("pop-after-leave", *checks)
        self.assertEqual([line for line in failed if not any(check in line for check in checks)], [])
        # Six runs push flags (two lines each), two of them resume (one more
        # each), and all six pop on the main screen (one more each). The
        # opted-out run sends no pop.
        self.assertEqual(len(failed), 20, result.stdout)
        self.assertIn("ok classic main without keyboard flags: no keyboard pop is sent when none was pushed",
                      result.stdout)

    def test_second_pop_on_the_main_screen_fails(self):
        # What suspend_terminal wrote before #410: its own pop, then the
        # restore's.
        checks = ("no keyboard pop is sent on the main screen",
                  "no keyboard pop is sent when none was pushed")
        result, failed = self.assert_fails("pop-on-both-screens", *checks)
        self.assertEqual([line for line in failed if not any(check in line for check in checks)], [])
        # All seven runs, and the opted-out one sent a pop at all.
        self.assertEqual(len(failed), 8, result.stdout)
        self.assertIn("FAIL classic main without keyboard flags: no keyboard pop is sent on the main screen",
                      result.stdout)

    def test_restore_once_per_setup_fails(self):
        # main before #417: suspend set the terminal up again and chained a
        # second hook, so a panic after a resume restored twice and the second
        # pop was sent on the main screen.
        checks = ("the hook restores once (2 setup_terminal calls)", "no keyboard pop is sent on the main screen")
        result, failed = self.assert_fails("restore-per-setup", *checks)
        self.assertEqual([line for line in failed if not any(check in line for check in checks)], [])
        # The two runs that resume, each failing both.
        self.assertEqual(len(failed), 4, result.stdout)

    def test_each_restore_detail_fails_on_its_own(self):
        for behaviour, check in (
            ("unpushed-pop", "no keyboard pop is sent when none was pushed"),
            ("title-kept", "clears the title set at startup before the panic message"),
            ("title-set-again", "clears the title set at startup before the panic message"),
            ("worker-silent", "current behaviour: a worker-thread panic prints its message onto the live screen"),
        ):
            with self.subTest(behaviour):
                _, failed = self.assert_fails(behaviour, check)
                self.assertEqual([line for line in failed if check not in line], [])


class PanicRestoreCaseTests(unittest.TestCase):
    """Case 30's probe lookup, through the actual runner."""

    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="coven panic case ")
        self.addCleanup(temp.cleanup)
        self.root = pathlib.Path(temp.name)
        suite = self.root / "scripts" / "tui-tests"
        (suite / "cases").mkdir(parents=True)
        for name in ("run.sh", "lib.sh", "panic_restore_pty.py", "zero_size_pty.py"):
            shutil.copy2(HERE / name, suite / name)
        shutil.copy2(HERE / "cases" / "30_panic_restore.sh", suite / "cases" / "30_panic_restore.sh")
        self.run_sh = suite / "run.sh"
        self.tools = self.root / "tools"
        self.tools.mkdir()
        (self.tools / "tmux").write_text(
            '#!/bin/sh\nif [ "$1" = "-V" ]; then printf "tmux fixture\\n"; fi\nexit 0\n', encoding="utf-8")
        (self.tools / "tmux").chmod(0o700)
        (self.tools / "python3").symlink_to(sys.executable)
        # A cargo profile directory in the layout without deps/.
        self.target = self.root / "target dir" / "debug"
        (self.target / "examples").mkdir(parents=True)
        self.bin = self.target / "coven-code"
        self.bin.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        self.bin.chmod(0o700)
        self.probe = self.target / "examples" / "panic_restore"
        self.sources = self.root / "src dir"
        self.sources.mkdir()

    def dep_info(self, path, *deps):
        """Cargo's form: "target: dep dep ...", spaces in paths escaped."""
        target, *escaped = (str(item).replace(" ", "\\ ") for item in (path, *deps))
        path.with_name(path.name + ".d").write_text(f"{target}: {' '.join(escaped)}\n", encoding="utf-8")

    def source(self, name, age):
        path = self.sources / name
        path.write_text("", encoding="utf-8")
        stamp = time.time() - age
        os.utime(path, (stamp, stamp))
        return path

    def run_case(self, **env):
        bash = shutil.which("bash")
        if bash is None:
            self.fail("These harness regressions require bash")
        result = subprocess.run(
            [bash, str(self.run_sh)], cwd=self.root,
            env={"PATH": str(self.tools) + os.pathsep + os.defpath, "HOME": str(self.root),
                 "COVEN_BIN": str(self.bin), "REQUIRE_TMUX": "1", **env},
            capture_output=True, text=True, encoding="utf-8", timeout=180,
        )
        return result, re.sub(r"\x1b\[[0-9;]*m", "", result.stdout)

    def test_missing_probe_fails_for_a_cargo_build(self):
        self.dep_info(self.bin, self.source("main.rs", 1000))
        result, out = self.run_case()
        self.assertEqual(result.returncode, 1, out + result.stderr)
        self.assertIn("FAIL panic probe is built", out)
        self.assertIn("--example panic_restore", out)

    def test_probe_that_is_not_executable_fails(self):
        self.probe.write_text("", encoding="utf-8")
        result, out = self.run_case(COVEN_PANIC_PROBE=str(self.probe))
        self.assertEqual(result.returncode, 1, out + result.stderr)
        self.assertIn("FAIL panic probe is built", out)
        self.assertIn(f"{self.probe} is not executable", out)

    def test_missing_probe_is_skipped_for_an_installed_binary(self):
        # The runner fails a run in which nothing passed, so add a passing case.
        (self.run_sh.parent / "cases" / "01_ok.sh").write_text(
            'register_case tc_ok\ntc_ok() { assert_eq yes yes "checked"; }\n', encoding="utf-8")
        result, out = self.run_case()
        self.assertEqual(result.returncode, 0, out + result.stderr)
        self.assertIn("skip COVEN_BIN is not a cargo build", out)

    def test_missing_explicit_probe_fails(self):
        result, out = self.run_case(COVEN_PANIC_PROBE=str(self.root / "missing"))
        self.assertEqual(result.returncode, 1, out + result.stderr)
        self.assertIn("FAIL panic probe is built", out)

    def test_probe_older_than_a_file_it_was_built_from_fails(self):
        write_fake_probe(self.probe)
        os.utime(self.probe, (time.time() - 500, time.time() - 500))
        edited = self.source("edited lib.rs", 100)
        self.dep_info(self.probe, self.source("old.rs", 1000), edited)
        result, out = self.run_case()
        self.assertEqual(result.returncode, 1, out + result.stderr)
        self.assertIn("FAIL panic probe is current", out)
        self.assertIn(f"{edited} is newer than", out)

    def current_probe(self, behaviour):
        write_fake_probe(self.probe, behaviour)
        os.utime(self.probe, (time.time() - 500, time.time() - 500))
        self.dep_info(self.probe, self.source("old.rs", 1000))

    def test_current_probe_runs_the_driver(self):
        self.current_probe("faithful")
        # A newer file the probe was not built from, such as a test-only
        # source, does not make it stale.
        self.source("tests_only.rs", 100)
        result, out = self.run_case()
        self.assertEqual(result.returncode, 0, out + result.stderr)
        self.assertIn(f"probe: {self.probe}", out)
        # Every check the driver reports is relayed.
        driver = subprocess.run(
            [sys.executable, "-B", str(HERE / "panic_restore_pty.py"), str(self.probe)],
            capture_output=True, text=True, encoding="utf-8", timeout=180,
        )
        checks = sum(line.startswith("ok ") for line in driver.stdout.splitlines())
        self.assertGreater(checks, 0, driver.stdout + driver.stderr)
        self.assertRegex(out, rf"Summary +{checks} passed +0 failed +0 skipped")

    def test_failing_driver_run_fails_the_case(self):
        self.current_probe("no-restore")
        result, out = self.run_case()
        self.assertEqual(result.returncode, 1, out + result.stderr)
        self.assertIn("FAIL classic main: every termios field is restored", out)
        self.assertIn("│ ", out)  # the evidence is passed on
        self.assertNotIn("panic PTY driver exited", out)


if __name__ == "__main__":
    unittest.main()
