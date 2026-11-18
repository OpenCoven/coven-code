"""Check that quitting gives back the keyboard flags it pushed (#410).

Usage: python3 -B quit_restore_pty.py BINARY [COLS ROWS]

The kitty keyboard protocol keeps one flag stack per screen. The TUI pushes
its flags after entering the alternate screen, so the pop has to be sent
before leaving it. A pop sent afterwards lands on the main screen's stack,
which belongs to the shell: the alternate stack then keeps one entry per
run, and the next full-screen program that sets no flags itself (`less`)
gets every key as an escape code and cannot be driven from the keyboard.
tmux does not model per-screen keyboard stacks, so the order is read from
the bytes on a raw PTY.

Runs the real binary once per renderer (Classic and Next), quits with
Ctrl+C twice and checks:
- flags were pushed, on the alternate screen (without that the rest proves
  nothing);
- the exit status is 0;
- no terminal mode is left on and neither screen's keyboard stack keeps an
  entry (`TerminalObserver` keeps one stack per screen);
- exactly one keyboard pop is written, before leaving the alternate screen;
- no keyboard pop is written on the main screen.

Prints one line per check, `ok <message>` or `FAIL <message> :: <evidence>`,
for cases/06_quit.sh to report. Exits 0 when every check passed.
"""
import os
import re
import sys
import tempfile

from zero_size_pty import Engine, Report, tail

LEAVE_ALT = b"\x1b[?1049l"
KEYBOARD_POP = re.compile(rb"\x1b\[<\d*u")
# Text that proves each renderer has drawn.
RENDERERS = (("classic", b"Coven v"), ("next", b"preview"))


def run(binary, report, ui, ready, size):
    with tempfile.TemporaryDirectory(prefix="coven-quit-restore-") as root:
        engine = Engine(binary, root, size, ui=ui)
        started = engine.pump(30, until=ready) and engine.healthy()
        report.check(started, f"{ui}: the engine starts", tail(engine))
        depths = dict(engine.observer.keyboard_depths)
        report.check(depths == {"main": 0, "alternate": 1},
                     f"{ui}: keyboard flags are pushed on the alternate screen", repr(depths))
        code = engine.quit()
        report.check(code == 0, f"{ui}: Ctrl+C twice exits 0", f"exit {code}: {tail(engine)}")
        failures = engine.observer.restoration_failures()
        report.check(not failures, f"{ui}: quitting leaves no terminal mode or keyboard flags behind",
                     ", ".join(failures))
        # Escape sequences are spelled out, never sent to the terminal.
        ending = ascii(engine.out[-200:].decode("utf-8", "replace"))
        leave = engine.out.rfind(LEAVE_ALT)
        pops = [match.start() for match in KEYBOARD_POP.finditer(engine.out)]
        report.check(len(pops) == 1 and pops[0] < leave,
                     f"{ui}: the one keyboard pop is sent before leaving the alternate screen",
                     f"pops at {pops}, leave at {leave}: {ending}")
        report.check(leave >= 0 and KEYBOARD_POP.search(engine.out, leave) is None,
                     f"{ui}: no keyboard pop is sent on the main screen", ending)


def main():
    if len(sys.argv) not in (2, 4):
        print(__doc__, file=sys.stderr)
        return 2
    # Each engine runs in its own temporary directory.
    binary = os.path.abspath(sys.argv[1])
    size = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) == 4 else (120, 40)
    report = Report()
    for ui, ready in RENDERERS:
        run(binary, report, ui, ready, size)
    return 1 if report.failed else 0


if __name__ == "__main__":
    sys.exit(main())
