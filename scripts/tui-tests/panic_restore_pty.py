"""Check the terminal is given back after a panic (#417).

Usage: python3 -B panic_restore_pty.py PROBE

PROBE is the tui crate's `panic_restore` example
(`cargo build -p claurst-tui --example panic_restore`). It takes the terminal
with the real `claurst_tui::setup_terminal`, the call the interactive binary
makes, and panics when this driver sends it a byte, so no deliberately broken
build of the product is needed. Each run gets its own raw PTY.

Checks, per run:
- Before the panic the probe is really in TUI mode: raw termios, alternate
  screen, mouse capture, bracketed paste, hidden cursor and, unless opted
  out, pushed keyboard flags. Without this the restore checks prove nothing.
- A panic on the main thread restores every termios field, then leaves the
  alternate screen, turns mouse capture and bracketed paste off, pops the
  keyboard flags and shows the cursor, all before the panic message. The
  message is printed and the process exits 101. The pop is sent after
  leaving the alternate screen, where the flags were pushed; that order is
  pinned as current behaviour so a change to it is deliberate.
- A panic on a spawned thread changes none of that (the hook only restores
  on the thread that set the terminal up). The probe keeps running, and its
  later main-thread panic still restores.

Prints one line per check, `ok <message>` or `FAIL <message> :: <evidence>`,
for cases/30_panic_restore.sh to report. Exits 0 when every check passed.
"""
import copy
import fcntl
import os
import pty
import re
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

from zero_size_pty import Report, TerminalObserver, termios_restored

SIZE = (80, 24)
READY = "panic-restore: ready"
WORKER_DONE = "panic-restore: worker panicked"
# Newer toolchains print the thread id: `thread 'main' (1234) panicked at`.
MAIN_PANIC = re.compile(rb"thread 'main'(?: \(\d+\))? panicked at")
MAIN_MESSAGE = b"panic-restore: deliberate main-thread panic"
WORKER_MESSAGE = b"panic-restore: deliberate worker-thread panic"
LEAVE_ALT = b"\x1b[?1049l"
KEYBOARD_PUSH = re.compile(rb"\x1b\[>\d*u")
KEYBOARD_POP = re.compile(rb"\x1b\[<\d*u")
# Any of these after a worker panic means the hook restored on the wrong thread.
RESTORE_BYTES = (LEAVE_ALT, b"\x1b[?1000l", b"\x1b[?2004l", b"\x1b[?25h", b"\x1b[<")
MOUSE_MODES = (1000, 1002, 1003, 1006, 1015)
COOKED = termios.ICANON | termios.ECHO | termios.ISIG


class Probe:
    def __init__(self, path, root, renderer, mode, keyboard):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", SIZE[1], SIZE[0], 0, 0))
        self.termios_before = termios.tcgetattr(self.slave)
        self.observer = TerminalObserver(*SIZE)
        self.out = b""
        env = {
            "PATH": os.defpath,
            "HOME": root,
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
        }
        if not keyboard:
            env["COVEN_CODE_KEYBOARD_ENHANCEMENT"] = "0"
        self.proc = subprocess.Popen(
            [path, renderer, mode], stdin=self.slave, stdout=self.slave, stderr=self.slave,
            cwd=root, env=env, start_new_session=True, close_fds=True,
        )

    def pump(self, seconds, condition):
        """Read output until `condition()` holds or `seconds` pass."""
        deadline = time.monotonic() + seconds
        while not condition() and time.monotonic() < deadline:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                self.out += data
                self.observer.feed(data)
        return condition()

    def drain(self):
        """Read until the process has exited and the PTY has been quiet briefly."""
        quiet_since = None
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    data = b""
                if data:
                    self.out += data
                    self.observer.feed(data)
                    quiet_since = None
                    continue
            if self.proc.poll() is None:
                continue
            quiet_since = quiet_since or time.monotonic()
            if time.monotonic() - quiet_since > 0.3:
                break

    def send(self):
        # One byte wakes the probe in raw mode. If the terminal was already
        # wrongly given back, canonical mode holds input until a newline.
        os.write(self.master, b"x" if self.raw() else b"x\n")

    def raw(self):
        return not termios.tcgetattr(self.slave)[3] & COOKED

    def close(self):
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.wait()
        os.close(self.slave)
        os.close(self.master)


def text(data):
    return " ".join(data.decode("utf-8", "replace").split())


def check_tui_mode(report, label, probe, keyboard):
    """The probe has really taken the terminal."""
    modes = probe.observer.modes
    report.check(probe.raw() and probe.termios_before[3] & COOKED == COOKED,
                 f"{label}: the probe puts the PTY in raw mode",
                 repr((probe.termios_before[3], termios.tcgetattr(probe.slave)[3])))
    missing = [name for name, on in (
        ("alternate screen", modes.get(1049, False)),
        ("mouse capture", all(modes.get(mode, False) for mode in MOUSE_MODES)),
        ("bracketed paste", modes.get(2004, False)),
        ("hidden cursor", not modes[25]),
    ) if not on]
    report.check(not missing, f"{label}: the probe enters TUI mode", "missing " + ", ".join(missing))
    pushed = KEYBOARD_PUSH.search(probe.out) is not None
    if keyboard:
        report.check(pushed and probe.observer.keyboard_depth == 1,
                     f"{label}: keyboard flags are pushed once", text(probe.out[-300:]))
    else:
        report.check(not pushed, f"{label}: keyboard flags are not pushed when opted out",
                     text(probe.out[-300:]))


def check_main_panic(report, label, probe, keyboard):
    """Trigger the main-thread panic and check what was written after it."""
    # The TUI state at the trigger. Replaying only what the hook wrote before
    # the message on top of it means each mode must be switched off by the
    # panic path itself.
    observer = copy.deepcopy(probe.observer)
    mark = len(probe.out)
    probe.send()
    probe.drain()
    after = probe.out[mark:]
    report.check(probe.proc.returncode == 101, f"{label}: exits with the panic status 101",
                 f"exit {probe.proc.returncode}: {text(after[-300:])}")
    match = MAIN_PANIC.search(after)
    panic_at = match.start() if match else -1
    report.check(match is not None and MAIN_MESSAGE in after[panic_at:],
                 f"{label}: the panic message is printed", text(after[-300:]))
    report.check(termios_restored(probe.termios_before, termios.tcgetattr(probe.slave)),
                 f"{label}: every termios field is restored",
                 repr((probe.termios_before, termios.tcgetattr(probe.slave))))
    observer.feed(after[:panic_at] if panic_at >= 0 else after)
    modes = observer.modes
    for ok, what in (
        (not modes.get(1049, False), "leaves the alternate screen"),
        (not any(modes.get(mode, False) for mode in MOUSE_MODES), "turns mouse capture off"),
        (not modes.get(2004, False), "turns bracketed paste off"),
        (modes[25], "shows the cursor"),
    ):
        report.check(ok, f"{label}: {what} before the panic message", text(after[:300]))
    if keyboard:
        report.check(observer.keyboard_depth == 0,
                     f"{label}: pops the keyboard flags before the panic message", text(after[:300]))
        # Current behaviour, pinned so a change is deliberate: the flags were
        # pushed on the alternate screen, and the pop is sent after leaving it.
        pop = KEYBOARD_POP.search(after)
        leave = after.find(LEAVE_ALT)
        report.check(pop is not None and 0 <= leave < pop.start(),
                     f"{label}: the keyboard pop follows leaving the alternate screen", text(after[:300]))


def run(path, report, renderer, mode, keyboard=True):
    label = f"{renderer} {mode}" + ("" if keyboard else " without keyboard flags")
    with tempfile.TemporaryDirectory(prefix="coven-panic-restore-") as root:
        probe = Probe(path, root, renderer, mode, keyboard)
        try:
            started = probe.pump(20, lambda: READY in probe.observer.text())
            report.check(started, f"{label}: the probe draws its first frame", text(probe.out[-300:]))
            if not started:
                return
            check_tui_mode(report, label, probe, keyboard)
            if mode == "worker":
                mark = len(probe.out)
                probe.send()
                survived = probe.pump(15, lambda: WORKER_DONE in probe.observer.text())
                after = probe.out[mark:]
                report.check(survived and WORKER_MESSAGE in after,
                             f"{label}: a worker-thread panic is reported and the probe keeps drawing",
                             text(after[-300:]))
                if not survived:
                    return
                restored = [repr(seq) for seq in RESTORE_BYTES if seq in after]
                report.check(not restored and probe.raw(),
                             f"{label}: a worker-thread panic leaves the terminal alone",
                             f"raw={probe.raw()} restore bytes {', '.join(restored)}")
                check_tui_mode(report, f"{label} after the worker panic", probe, keyboard)
            check_main_panic(report, label, probe, keyboard)
        finally:
            probe.close()


def main():
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    path = sys.argv[1]
    report = Report()
    for renderer in ("classic", "next"):
        run(path, report, renderer, "main")
        run(path, report, renderer, "worker")
    run(path, report, "classic", "main", keyboard=False)
    return 1 if report.failed else 0


if __name__ == "__main__":
    sys.exit(main())
