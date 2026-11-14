"""Drive the real binary on a raw PTY at zero and tiny terminal sizes (#424).

Usage: python3 -B zero_size_pty.py BINARY

tmux cannot give a window zero rows or columns, so this helper opens the PTY
itself. Each run uses a disposable HOME and COVEN_HOME and no credentials.
Sizes are applied with TIOCSWINSZ and an explicit SIGWINCH: the child is a
session leader without a controlling terminal, so the kernel does not send
SIGWINCH by itself.

Prints one line per check, `ok <message>` or `FAIL <message> :: <evidence>`,
for cases/23_classic_zero_size.sh to report. Exits 0 when every check passed.
"""
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

BOOT = b"Coven v"
# Terminal queries the engine sends at startup, with the replies a terminal
# would give.
QUERIES = {
    b"\x1b[c": b"\x1b[?62;22c",
    b"\x1b[6n": b"\x1b[1;1R",
}
ESCAPES = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
RESTORED = {
    "alternate screen left": b"\x1b[?1049l",
    "cursor shown": b"\x1b[?25h",
}


def set_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class Engine:
    def __init__(self, binary, root, size):
        home = os.path.join(root, "home")
        project = os.path.join(root, "project")
        os.makedirs(os.path.join(home, ".coven-code"))
        os.makedirs(project)
        with open(os.path.join(home, ".coven-code", "settings.json"), "w", encoding="utf-8") as f:
            json.dump({"hasCompletedOnboarding": True}, f)
        self.master, slave = pty.openpty()
        if size is not None:
            set_size(slave, *size)
        env = {
            "PATH": os.defpath,
            "HOME": home,
            "COVEN_HOME": os.path.join(root, "coven"),
            "USER": "zero-size",
            "LOGNAME": "zero-size",
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
        }
        self.proc = subprocess.Popen(
            [binary], stdin=slave, stdout=slave, stderr=slave, cwd=project, env=env,
            start_new_session=True, close_fds=True,
        )
        os.close(slave)
        self.out = b""

    def pump(self, seconds, until=None, since=0):
        """Read output for up to `seconds`; stop early once `until` is visible after `since`."""
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
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
            for query, reply in QUERIES.items():
                if query in data:
                    os.write(self.master, reply * data.count(query))
        return until is not None and until in self.visible(since)

    def visible(self, since=0):
        return ESCAPES.sub(b"", self.out[since:])

    def alive(self):
        return self.proc.poll() is None

    def healthy(self):
        # A panic prints before the process finishes exiting.
        return self.alive() and b"panicked at" not in self.out

    def resize(self, cols, rows):
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

    def recover(self):
        """Back to 120x40; True when a full frame with the banner is drawn."""
        mark = len(self.out)
        set_size(self.master, 120, 40)
        if self.alive():
            os.kill(self.proc.pid, signal.SIGWINCH)
        return self.pump(8, until=BOOT, since=mark)

    def quit(self):
        # Esc first so no overlay holds Ctrl+C; the second Ctrl+C confirms.
        for key in (b"\x1b", b"\x03", b"\x03"):
            self.send(key)
        try:
            self.proc.wait(8)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.pump(0.3)
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


def run(binary, report, label, start, steps):
    with tempfile.TemporaryDirectory(prefix="coven-zero-size-") as root:
        engine = Engine(binary, root, start)
        if start is not None and start[0] > 0 and start[1] > 0:
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
            elif kind == "ctrl-c":
                engine.send(b"\x03", settle=1.0)
                report.check(engine.healthy(), f"{label}: survives the first Ctrl+C", tail(engine))
            elif kind == "type":
                engine.send(step[1].encode())
                report.check(engine.healthy(), f"{label}: survives typing", tail(engine))
            elif kind == "recover":
                report.check(engine.recover(), f"{label}: draws a full frame again at 120x40", tail(engine))
        code = engine.quit()
        text = engine.out.decode("utf-8", "replace")
        report.check(code == 0 and "panicked at" not in text,
                     f"{label}: Ctrl+C twice exits 0 without a panic", f"exit {code}: {tail(engine)}")
        missing = [name for name, seq in RESTORED.items() if seq not in engine.out]
        report.check(not missing, f"{label}: the terminal is restored", f"missing: {', '.join(missing)}")


def main():
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    binary = sys.argv[1]
    report = Report()
    for label, start in (("start 0x0", None), ("start 0x40", (0, 40)), ("start 120x0", (120, 0))):
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
    return 1 if report.failed else 0


if __name__ == "__main__":
    sys.exit(main())
