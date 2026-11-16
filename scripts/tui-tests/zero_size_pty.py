"""Drive the real binary on a raw PTY at zero and tiny terminal sizes (#424).

Usage: python3 -B zero_size_pty.py BINARY

tmux cannot give a window zero rows or columns, so this helper opens the PTY
itself. Each run uses disposable HOME/Coven homes and no credentials. Set
COVEN_TUI_PTY_RECEIPTS to retain raw output, termios, and recovery frames.
Sizes are applied with TIOCSWINSZ and an explicit SIGWINCH: the child is a
session leader without a controlling terminal, so the kernel does not send
SIGWINCH by itself.

Prints one line per check, `ok <message>` or `FAIL <message> :: <evidence>`,
for cases/23_classic_zero_size.sh to report. Exits 0 when every check passed.
"""
import codecs
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unicodedata

BOOT = b"Coven v"
CANARY = "qzv424canary"
ERROR_MARK = "local-tiny-modal-failure"
# Terminal queries the engine sends at startup, with the replies a terminal
# would give.
QUERIES = {
    b"\x1b[c": b"\x1b[?62;22c",
    b"\x1b[6n": b"\x1b[1;1R",
}
ESCAPES = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
CSI = re.compile(r"\x1b\[([0-?]*)([ -/]*)([@-~])")


def termios_restored(before, after):
    """Compare every field, ignoring only macOS's transient pending-input bit."""
    before, after = list(before), list(after)
    if sys.platform == "darwin":
        before[3] &= ~0x20000000  # PENDIN is set by tcsetattr on macOS.
        after[3] &= ~0x20000000
    return before == after


class TerminalObserver:
    """Small VT screen/mode observer for the engine's Crossterm output.

    It retains a current cell grid rather than searching historical printable
    bytes. Resizing invalidates the grid until the engine repaints it.
    """
    def __init__(self, cols, rows):
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.modes = {25: True}
        self.keyboard_depth = 0
        self.resize(cols, rows)

    def resize(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.x = self.y = 0
        self.saved_cursor = (0, 0)

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b["):
                match = CSI.match(self.pending)
                if match is None:
                    break
                self.pending = self.pending[match.end():]
                self.csi(*match.groups())
            elif self.pending.startswith("\x1b]"):
                end = re.search(r"\x07|\x1b\\", self.pending)
                if end is None:
                    break
                self.pending = self.pending[end.end():]
            elif self.pending[0] == "\x1b":
                if len(self.pending) < 2:
                    break
                self.pending = self.pending[2:]
            else:
                ch, self.pending = self.pending[0], self.pending[1:]
                if ch == "\r":
                    self.x = 0
                elif ch == "\n":
                    self.y = min(self.y + 1, max(0, self.rows - 1))
                elif ch == "\b":
                    self.x = max(0, self.x - 1)
                elif ch == "\t":
                    self.x = min((self.x // 8 + 1) * 8, max(0, self.cols - 1))
                elif not unicodedata.category(ch).startswith("C"):
                    width = 0 if unicodedata.combining(ch) else (2 if unicodedata.east_asian_width(ch) in "WF" else 1)
                    if width and 0 <= self.y < self.rows and 0 <= self.x < self.cols:
                        self.grid[self.y][self.x] = ch
                        if width == 2 and self.x + 1 < self.cols:
                            self.grid[self.y][self.x + 1] = ""
                    self.x += width

    def csi(self, params, intermediate, final):
        if params.startswith("?") and final in "hl":
            for value in params[1:].split(";"):
                if value.isdigit():
                    self.modes[int(value)] = final == "h"
                    if int(value) == 1049 and final == "h":
                        self.resize(self.cols, self.rows)
            return
        if final == "u" and params.startswith(">"):
            self.keyboard_depth += 1
            return
        if final == "u" and params.startswith("<"):
            self.keyboard_depth = max(0, self.keyboard_depth - int(params[1:] or "1"))
            return
        if intermediate or any(ch not in "0123456789;" for ch in params):
            return
        values = [int(value or "0") for value in params.split(";")]
        n = values[0] or 1
        if final in "Hf":
            self.y = max(0, values[0] - 1)
            self.x = max(0, (values[1] if len(values) > 1 else 1) - 1)
        elif final == "G":
            self.x = n - 1
        elif final == "d":
            self.y = n - 1
        elif final == "A":
            self.y = max(0, self.y - n)
        elif final == "B":
            self.y += n
        elif final == "C":
            self.x += n
        elif final == "D":
            self.x = max(0, self.x - n)
        elif final == "J":
            for y, row in enumerate(self.grid):
                for x in range(self.cols):
                    if values[0] == 2 or (values[0] == 0 and (y, x) >= (self.y, self.x)) or (values[0] == 1 and (y, x) <= (self.y, self.x)):
                        row[x] = " "
        elif final == "K" and 0 <= self.y < self.rows:
            for x in range(self.cols):
                if values[0] == 2 or (values[0] == 0 and x >= self.x) or (values[0] == 1 and x <= self.x):
                    self.grid[self.y][x] = " "
        elif final == "s":
            self.saved_cursor = (self.x, self.y)
        elif final == "u":
            self.x, self.y = self.saved_cursor

    def text(self):
        return "\n".join("".join(row) for row in self.grid)

    def composer_contains(self, canary):
        # This probe uses a one-line, non-command input at 120x40. Its input
        # pane is near the footer, starts at column zero, and has horizontal
        # rules immediately above/below. Transcript text is not input evidence.
        rows = ["".join(row) for row in self.grid]
        rule = "─" * self.cols
        return any(
            rows[y].startswith("❯ " + canary)
            and rows[y - 1] == rule and rows[y + 1] == rule
            for y in range(max(1, self.rows - 8), self.rows - 1)
        )

    def restoration_failures(self):
        failures = []
        for mode in (1000, 1002, 1003, 1005, 1006, 1015):
            if self.modes.get(mode, False):
                failures.append(f"mouse mode {mode} active")
        for mode, description in ((2004, "bracketed paste active"), (1049, "alternate screen active")):
            if self.modes.get(mode, False):
                failures.append(description)
        if not self.modes[25]:
            failures.append("cursor hidden")
        if self.keyboard_depth:
            failures.append(f"keyboard stack depth {self.keyboard_depth}")
        return failures


def set_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class Engine:
    def __init__(self, binary, root, size, fake_failure=False):
        home = os.path.join(root, "home")
        project = os.path.join(root, "project")
        code_home = os.path.join(root, "code")
        os.makedirs(home)
        os.makedirs(code_home)
        os.makedirs(project)
        with open(os.path.join(code_home, "settings.json"), "w", encoding="utf-8") as f:
            json.dump({"hasCompletedOnboarding": True}, f)
        self.master, self.slave = pty.openpty()
        slave = self.slave
        self.termios_before = termios.tcgetattr(slave)
        self.termios_after = None
        self.observer = TerminalObserver(*(size or (0, 0)))
        if size is not None:
            set_size(slave, *size)
        env = {
            "PATH": os.defpath,
            "HOME": home,
            "COVEN_HOME": os.path.join(root, "coven"),
            "COVEN_CODE_HOME": os.path.join(root, "code"),
            "COVEN_CAVE_HOME": os.path.join(root, "cave"),
            "COVEN_CODE_DISABLE_MODELS_FETCH": "1",
            "USER": "zero-size",
            "LOGNAME": "zero-size",
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
        }
        # A local executable produces a genuine provider error without auth,
        # credentials, or a network request. It reads the owned stdin to EOF.
        if fake_failure:
            fake = os.path.join(root, "fake-claude")
            with open(fake, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '" + ERROR_MARK + "' >&2\nexit 1\n")
            os.chmod(fake, 0o755)
            env["COVEN_CODE_CLAUDE_BIN"] = fake
        self.proc = subprocess.Popen(
            [binary], stdin=slave, stdout=slave, stderr=slave, cwd=project, env=env,
            start_new_session=True, close_fds=True,
        )
        self.out = b""
        self.recoveries = 0

    def pump(self, seconds, until=None, since=0, condition=None):
        """Read output for up to `seconds`; stop early once `until` is visible after `since`."""
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if condition is not None and condition():
                return True
            if until is not None and until in self.visible(since):
                return True
            ready, _, _ = select.select([self.master], [], [], min(0.1, max(0, deadline - time.monotonic())))
            if not ready:
                continue
            try:
                data = os.read(self.master, 65536)
            except OSError:
                break
            if not data:
                break
            self.out += data
            self.observer.feed(data)
            for query, reply in QUERIES.items():
                if query in data:
                    os.write(self.master, reply * data.count(query))
        return condition() if condition is not None else until is not None and until in self.visible(since)

    def visible(self, since=0):
        return ESCAPES.sub(b"", self.out[since:])

    def alive(self):
        return self.proc.poll() is None

    def healthy(self):
        # A panic prints before the process finishes exiting.
        return self.alive() and b"panicked at" not in self.out

    def resize(self, cols, rows):
        self.observer.resize(cols, rows)
        set_size(self.master, cols, rows)
        if self.alive():
            os.kill(self.proc.pid, signal.SIGWINCH)
        self.pump(0.6)

    def send(self, data, settle=0.6):
        if self.alive():
            try:
                os.write(self.master, data)
            except OSError:
                pass
        self.pump(settle)

    def recover(self, pending_error=False):
        """Require a newly repainted error/composer and typed canary at 120x40."""
        self.resize(120, 40)
        if pending_error:
            if not self.pump(8, condition=lambda: ERROR_MARK in self.observer.text() and "Error" in self.observer.text()):
                return False
            # Verify persistence before dismissing the error to type again.
            self.send(b"\x1b")
        self.send(b"\x15")  # clear any earlier synthetic input
        self.recoveries += 1
        canary = f"{CANARY}{self.recoveries}"
        self.send(canary.encode())
        recovered = self.pump(8, condition=lambda: self.observer.composer_contains(canary))
        self.recovery_frame = self.observer.text()
        return recovered

    def quit(self):
        # Esc first so no overlay holds Ctrl+C; the second Ctrl+C confirms.
        for key in (b"\x1b", b"\x15", b"\x03", b"\x03"):
            self.send(key)
        try:
            self.proc.wait(8)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.pump(0.3)
        # Keep the PTY slave open until after the clean exit so tcgetattr
        # captures the exact persistent terminal state before fd teardown.
        self.termios_after = termios.tcgetattr(self.slave)
        os.close(self.slave)
        os.close(self.master)
        return self.proc.returncode


class Report:
    def __init__(self):
        self.failed = 0

    def check(self, ok, message, evidence=""):
        if ok:
            print(f"ok {message}", flush=True)
        else:
            self.failed += 1
            detail = " ".join(evidence.split())[:300]
            print(f"FAIL {message} :: {detail}", flush=True)


def tail(engine):
    text = engine.out.decode("utf-8", "replace")
    m = re.search(r"panicked at [^\r\n]*[\r\n]+[^\r\n]*", text)
    if m:
        return m.group(0)
    return engine.visible().decode("utf-8", "replace")[-300:]


def run(binary, report, label, start, steps, fake_failure=False):
    with tempfile.TemporaryDirectory(prefix="coven-zero-size-") as root:
        engine = Engine(binary, root, start, fake_failure=fake_failure)
        if start is not None and start[0] >= 60 and start[1] >= 16:
            engine.pump(20, until=BOOT)
        else:
            engine.pump(2.5)
        report.check(engine.healthy(), f"{label}: the engine starts", tail(engine))
        for step in steps:
            if not engine.healthy():
                report.check(False, f"{label}: still running for the next step", tail(engine))
                break
            kind = step[0]
            if kind == "size":
                engine.resize(step[1], step[2])
                report.check(engine.healthy(), f"{label}: survives {step[1]}x{step[2]}", tail(engine))
                if fake_failure:
                    report.check("Error:" in engine.observer.text(),
                                 f"{label}: bounded error visible at {step[1]}x{step[2]}", tail(engine))
            elif kind == "ctrl-c":
                engine.send(b"\x03", settle=1.0)
                report.check(engine.healthy(), f"{label}: survives the first Ctrl+C", tail(engine))
            elif kind == "type":
                engine.send(step[1].encode())
                report.check(engine.healthy(), f"{label}: survives typing", tail(engine))
            elif kind in ("recover", "recover-error"):
                report.check(engine.recover(pending_error=kind == "recover-error"),
                             f"{label}: current 120x40 composer accepts typed canary", tail(engine))
            elif kind == "error":
                engine.send(b"exercise local failure\r")
                visible = engine.pump(15, condition=lambda: ERROR_MARK in engine.observer.text() and "Error" in engine.observer.text())
                report.check(visible, f"{label}: local Claude failure opens Error modal", tail(engine))
        code = engine.quit()
        text = engine.out.decode("utf-8", "replace")
        report.check(code == 0 and "panicked at" not in text,
                     f"{label}: Ctrl+C twice exits 0 without a panic", f"exit {code}: {tail(engine)}")
        failures = engine.observer.restoration_failures()
        report.check(not failures, f"{label}: final terminal modes are restored", ", ".join(failures))
        report.check(termios_restored(engine.termios_before, engine.termios_after),
                     f"{label}: all persistent termios fields are restored",
                     repr((engine.termios_before, engine.termios_after)))
        receipts = os.environ.get("COVEN_TUI_PTY_RECEIPTS")
        if receipts:
            os.makedirs(receipts, exist_ok=True)
            name = re.sub(r"[^a-zA-Z0-9_-]", "-", label)
            with open(os.path.join(receipts, name + ".bin"), "wb") as f:
                f.write(engine.out)
            with open(os.path.join(receipts, name + ".json"), "w", encoding="utf-8") as f:
                json.dump({"label": label, "exit": code, "termios_before": engine.termios_before,
                           "termios_after": engine.termios_after, "mode_failures": failures,
                           "final_modes": engine.observer.modes, "keyboard_depth": engine.observer.keyboard_depth},
                          f, default=lambda value: list(value), indent=2)
            with open(os.path.join(receipts, name + "-recovery.txt"), "w", encoding="utf-8") as f:
                f.write(getattr(engine, "recovery_frame", ""))


def main():
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    binary = sys.argv[1]
    report = Report()
    for label, start in (("start 0x0", None), ("start 0x40", (0, 40)), ("start 120x0", (120, 0)),
                          ("start 1x1", (1, 1)), ("start 20x6", (20, 6))):
        run(binary, report, label, start, [("ctrl-c",), ("recover",)])
    run(binary, report, "120x40 resized", (120, 40), [
        # #417's route: the first Ctrl+C at 120x0 shows the exit toast.
        ("size", 120, 0), ("ctrl-c",), ("recover",),
        ("size", 0, 40), ("type", "hi"),
        ("size", 0, 0),
        ("size", 1, 1),
        ("size", 20, 6),
        ("recover",),
    ])
    run(binary, report, "Error modal resized", (120, 40), [
        ("error",), ("size", 20, 1), ("size", 120, 1), ("recover-error",),
    ], fake_failure=True)
    return 1 if report.failed else 0


if __name__ == "__main__":
    sys.exit(main())
