"""Check the terminal is given back after a panic (#417).

Usage: python3 -B panic_restore_pty.py PROBE

PROBE is the tui crate's `panic_restore` example (from `src-rust`:
`cargo build -p claurst -p claurst-tui --bins --example panic_restore`). It
takes the terminal with the real `claurst_tui::setup_terminal`, the call the
interactive binary makes, and panics when this driver sends it a byte, so no
deliberately broken build of the product is needed. Each run gets its own
raw PTY.

Checks, per run:
- Before the panic the probe is really in TUI mode: raw termios (no line
  editing, echo, signals or output processing), alternate screen, mouse
  capture, bracketed paste, hidden cursor and, unless opted out, pushed
  keyboard flags. Without this the restore checks prove nothing.
- A panic on the main thread restores every termios field. Before the
  panic message it turns output processing back on (the message's first
  line break arrives translated to CRLF, which raw mode does not do), leaves
  the alternate screen, turns mouse capture and bracketed paste off, sends a
  keyboard pop and shows the cursor. The message is printed and the process
  exits 101.
- A panic on a spawned thread does not restore anything (the hook only
  restores on the thread that set the terminal up). The probe keeps running,
  and its later main-thread panic still restores.
- After a suspend and resume through the real `suspend_terminal`, a
  main-thread panic still restores.

Some details are pinned as current behaviour, not endorsed, so a change to
them is deliberate; their check lines contain "current behaviour":
- the keyboard pop is sent after leaving the alternate screen the flags were
  pushed on (case 13 requires suspend to pop on the alternate screen);
- a keyboard pop is sent even when nothing was pushed;
- the title set at startup is not cleared;
- every `setup_terminal` call chains another hook, so after a resume the
  restore is written twice;
- a worker-thread panic prints its message onto the live screen.

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
RESUMED = "panic-restore: resumed"
# Newer toolchains print the thread id: `thread 'main' (1234) panicked at`.
MAIN_PANIC = re.compile(rb"thread 'main'(?: \(\d+\))? panicked at")
MAIN_MESSAGE = b"panic-restore: deliberate main-thread panic"
# Raw mode turns output processing off, so the line break before the message
# arrives as CRLF only if termios was restored before the message was written.
COOKED_MESSAGE = re.compile(MAIN_PANIC.pattern + rb"[^\r\n]*\r\n" + re.escape(MAIN_MESSAGE))
WORKER_MESSAGE = b"panic-restore: deliberate worker-thread panic"
LEAVE_ALT = b"\x1b[?1049l"
KEYBOARD_PUSH = re.compile(rb"\x1b\[>\d*u")
KEYBOARD_POP = re.compile(rb"\x1b\[<\d*u")
TITLE = re.compile(rb"\x1b\][02];")
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
        """Read output until `condition()` holds, `seconds` pass or the probe
        has exited and its output is read. Once it holds, keep reading until
        the PTY is briefly quiet, so a snapshot of the terminal state is not
        taken in the middle of one frame."""
        deadline = time.monotonic() + seconds
        quiet_at = None
        while time.monotonic() < deadline:
            if quiet_at is None and condition():
                quiet_at = time.monotonic() + 0.15
            if quiet_at is not None and time.monotonic() >= quiet_at:
                break
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                self.out += data
                self.observer.feed(data)
                if quiet_at is not None:
                    quiet_at = time.monotonic() + 0.15
            elif self.proc.poll() is not None:
                break
        return condition()

    def drain(self, until):
        """Read until the process has exited, `until` has arrived and the PTY
        has been quiet briefly, or 15 s pass."""
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
            if self.proc.poll() is None or until not in self.out:
                continue
            quiet_since = quiet_since or time.monotonic()
            if time.monotonic() - quiet_since > 0.3:
                break

    def send(self):
        # One byte wakes the probe in raw mode. If the terminal was already
        # wrongly given back, canonical mode holds input until a newline, so
        # send just that: one byte either way, none left over for a later read.
        os.write(self.master, b"x" if self.raw() else b"\n")

    def raw(self):
        return not termios.tcgetattr(self.slave)[3] & COOKED

    def close(self):
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.wait()
        os.close(self.slave)
        os.close(self.master)


def text(data):
    """Evidence with escape sequences shown, never sent to the terminal."""
    return ascii(" ".join(data.decode("utf-8", "replace").split()))


def check_tui_mode(report, label, probe, keyboard):
    """The probe has really taken the terminal."""
    modes = probe.observer.modes
    # The CRLF check after the panic relies on raw mode turning output
    # processing off, so require that here.
    now = termios.tcgetattr(probe.slave)
    before = probe.termios_before
    report.check(probe.raw() and not now[1] & termios.OPOST
                 and before[3] & COOKED == COOKED
                 and before[1] & (termios.OPOST | termios.ONLCR) == termios.OPOST | termios.ONLCR,
                 f"{label}: the probe puts the PTY in raw mode",
                 f"lflag {before[3]} -> {now[3]}, oflag {before[1]} -> {now[1]}")
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
                     f"{label}: keyboard flags are pushed, one level deep", text(probe.out[-300:]))
    else:
        report.check(not pushed, f"{label}: keyboard flags are not pushed when opted out",
                     text(probe.out[-300:]))


def check_main_panic(report, label, probe, keyboard, setups):
    """Trigger the main-thread panic and check what was written after it."""
    # The TUI state at the trigger. Replaying only what the hook wrote before
    # the message on top of it means each mode must be switched off by the
    # panic path itself.
    observer = copy.deepcopy(probe.observer)
    mark = len(probe.out)
    probe.send()
    probe.drain(MAIN_MESSAGE)
    after = probe.out[mark:]
    report.check(probe.proc.returncode == 101, f"{label}: exits with the panic status 101",
                 f"exit {probe.proc.returncode}: {text(after[-300:])}")
    match = MAIN_PANIC.search(after)
    # Without a message there is no "before the message"; those checks fail.
    hook = after[:match.start()] if match else b""
    report.check(match is not None and MAIN_MESSAGE in after[match.start():],
                 f"{label}: the panic message is printed", text(after[-300:]))
    report.check(termios_restored(probe.termios_before, termios.tcgetattr(probe.slave)),
                 f"{label}: every termios field is restored",
                 repr((probe.termios_before, termios.tcgetattr(probe.slave))))
    report.check(COOKED_MESSAGE.search(after) is not None,
                 f"{label}: output processing is restored before the panic message", repr(after[-300:]))
    observer.feed(hook)
    modes = observer.modes
    for ok, what in (
        (match and not modes.get(1049, False), "leaves the alternate screen"),
        (match and not any(modes.get(mode, False) for mode in MOUSE_MODES), "turns mouse capture off"),
        (match and not modes.get(2004, False), "turns bracketed paste off"),
        (match and modes[25], "shows the cursor"),
    ):
        report.check(bool(ok), f"{label}: {what} before the panic message", text(after[:300]))
    pop = KEYBOARD_POP.search(hook)
    if keyboard:
        # One stack is modelled; see the pinned order below for per-screen stacks.
        report.check(bool(match) and observer.keyboard_depth == 0,
                     f"{label}: sends a keyboard pop before the panic message", text(after[:300]))
        leave = hook.find(LEAVE_ALT)
        report.check(pop is not None and 0 <= leave < pop.start(),
                     f"{label}: current behaviour: the keyboard pop is sent after leaving the alternate screen",
                     text(after[:300]))
    else:
        report.check(pop is not None,
                     f"{label}: current behaviour: a keyboard pop is sent though none was pushed",
                     text(after[:300]))
    report.check(bool(match) and TITLE.search(probe.out[:mark]) is not None and TITLE.search(after) is None,
                 f"{label}: current behaviour: the hook leaves the title set at startup", text(after[:300]))
    if setups > 1:
        report.check(hook.count(LEAVE_ALT) == setups,
                     f"{label}: current behaviour: the hook restores once per setup_terminal call ({setups})",
                     f"{hook.count(LEAVE_ALT)} restores: {text(after[:300])}")


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
            setups = 1
            if mode == "worker":
                mark = len(probe.out)
                probe.send()
                survived = probe.pump(15, lambda: WORKER_DONE in probe.observer.text())
                after = probe.out[mark:]
                report.check(survived, f"{label}: the probe keeps running and drawing after a worker-thread panic",
                             text(after[-300:]))
                report.check(WORKER_MESSAGE in after,
                             f"{label}: current behaviour: a worker-thread panic prints its message "
                             "onto the live screen", text(after[-300:]))
                if not survived:
                    return
                restored = [repr(seq) for seq in RESTORE_BYTES if seq in after]
                report.check(not restored and probe.raw(),
                             f"{label}: a worker-thread panic does not restore the terminal",
                             f"raw={probe.raw()} restore bytes {', '.join(restored)}")
                check_tui_mode(report, f"{label} after the worker panic", probe, keyboard)
            elif mode == "resume":
                probe.send()
                resumed = probe.pump(15, lambda: RESUMED in probe.observer.text())
                report.check(resumed, f"{label}: the probe suspends, resumes and draws again",
                             text(probe.out[-300:]))
                if not resumed:
                    return
                check_tui_mode(report, f"{label} after resuming", probe, keyboard)
                setups = 2
            check_main_panic(report, label, probe, keyboard, setups)
        finally:
            probe.close()


def main():
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    # Each probe runs in its own temporary directory.
    path = os.path.abspath(sys.argv[1])
    report = Report()
    for renderer in ("classic", "next"):
        for mode in ("main", "worker", "resume"):
            run(path, report, renderer, mode)
    run(path, report, "classic", "main", keyboard=False)
    return 1 if report.failed else 0


if __name__ == "__main__":
    sys.exit(main())
