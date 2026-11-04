#!/usr/bin/env python3
"""Synthetic Classic/Next performance baseline over a raw pseudo-terminal.

Measures, per terminal size and run, against a disposable home with no
credentials and no model call:

- cold usable frame: spawn -> both the boot string and composer prompt drawn
- input-to-paint: one keystroke written -> first output whose visible text
  contains it
- resize-to-paint: TIOCSWINSZ + SIGWINCH -> first output with visible text
- idle RSS: resident set size after the first frame settles
- idle writes: PTY writes per second with no input or visible change
- with --stream, during a running turn: input-to-paint, and the gap between
  paints as a boundary proxy for active render cost

The harness answers the terminal queries a real emulator would (primary
device attributes, cursor position, keyboard-enhancement probe) so startup
does not include a query timeout. It records every query it answered.

This is a baseline instrument, not #417's final instrumentation: it observes
bytes at the PTY boundary, so it includes kernel/PTY scheduling and cannot
attribute time to individual render phases.

Usage:
  python3 scripts/tui-tests/perf_baseline.py --bin PATH [--runs N]
      [--keys N] [--size 80x24 --size 120x40] [--json OUT]
      [--ui classic|next] [--history-lines N] [--stream]

--ui next selects the Next renderer through COVEN_CODE_UI. --history-lines N
resumes a seeded session holding N transcript lines, so input and resize are
measured with a long conversation on screen. The seed is synthetic and lives
in the disposable home. --stream runs a turn against a fake `claude` written
there, with no network or model.
"""
import argparse
import fcntl
import json
import os
import pty
import re
import select
import signal
import statistics
import struct
import subprocess
import tempfile
import termios
import time
import uuid

BOOT = b"Coven v"  # matched after escape sequences are removed
PROMPT = "❯".encode()  # composer prompt glyph
NEXT_READY = [b"preview", b"ready"]  # Next header and idle status
SEEDED_MARK = b"seeded"  # one word: a diffed frame may skip the blank after it
RESUMED_MARK = b"Resumed session"  # the CLI prints this only after a good load
STREAM_MARK = b"streamed"
STREAM_KEYS = b"qz"  # letters the working status and spinner rows do not use
FRAME_MERGE_MS = 1.0  # PTY reads closer than this belong to one paint
LINES_PER_ANSWER = 100
KEYS = b"qzxjvkwy"
QUERIES = {
    b"\x1b[c": b"\x1b[?62;22c",   # DA1
    b"\x1b[6n": b"\x1b[1;1R",     # cursor position report
    b"\x1b[?u": b"",              # keyboard enhancement probe: unsupported
}
# CSI and OSC sequences: styling splits visible text across escapes.
ESCAPES = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


def set_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def session_env(home, ui, claude_bin=None):
    """Environment for one engine: built from nothing, so no credential leaks in."""
    env = {
        "PATH": os.defpath,
        "HOME": home,
        "USER": "baseline",
        "LOGNAME": "baseline",
        "TERM": "xterm-256color",
        "LANG": "en_US.UTF-8",
    }
    if ui == "next":
        env["COVEN_CODE_UI"] = "next"
    if claude_bin:
        env["COVEN_CODE_CLAUDE_BIN"] = claude_bin
    return env


def ready_needles(ui, resumed):
    """Text that proves a usable frame.

    A resume must prove it loaded: the engine falls back to an empty session
    when a resume fails, which would otherwise measure as a very fast long
    conversation. Next must show seeded text. Classic's welcome header can
    leave no transcript rows at 80x24 even after a good resume, so it relies
    on the CLI's load confirmation.
    """
    if ui == "next":
        return NEXT_READY + ([SEEDED_MARK] if resumed else [])
    return [PROMPT, RESUMED_MARK] if resumed else [BOOT, PROMPT]


def seed_session(home, project, lines):
    """Write a synthetic session of `lines` answer lines; return its id.

    The text avoids every probe key, so repainting it cannot be mistaken for
    a keystroke reaching the screen.
    """
    sessions = os.path.join(home, ".coven-code", "sessions")
    os.makedirs(sessions, exist_ok=True)
    messages = []
    for start in range(0, lines, LINES_PER_ANSWER):
        end = min(start + LINES_PER_ANSWER, lines)
        messages.append({"role": "user", "content": "more"})
        messages.append({
            "role": "assistant",
            "content": "".join(f"seeded line {n:06d}\n" for n in range(start, end)),
        })
    session_id = str(uuid.uuid4())
    now = "2026-01-01T00:00:00Z"
    with open(os.path.join(sessions, f"{session_id}.json"), "w", encoding="utf-8") as f:
        json.dump({
            "id": session_id, "created_at": now, "updated_at": now,
            "messages": messages, "model": "seed", "title": None,
            "working_dir": project,
        }, f)
    return session_id


def write_fake_claude(directory, chunks=3000, delay="0.01"):
    """Write a fake `claude` that streams one line per chunk; return its path.

    It stops by itself after `chunks`, or on SIGPIPE once the engine is gone.
    """
    path = os.path.join(directory, "fake-claude")
    with open(path, "w", encoding="utf-8") as f:
        f.write(f"""#!/bin/sh
cat >/dev/null &
printf '%s\\n' '{{"type":"system","subtype":"init","session_id":"fake","model":"fake-model"}}'
i=1
while [ "$i" -le {chunks} ]; do
  printf '%s\\n' "{{\\"type\\":\\"assistant\\",\\"message\\":{{\\"model\\":\\"fake-model\\",\\"content\\":[{{\\"type\\":\\"text\\",\\"text\\":\\"{STREAM_MARK.decode()} line $i\\\\n\\"}}]}}}}"
  sleep {delay}
  i=$((i + 1))
done
printf '%s\\n' '{{"type":"result","subtype":"success","is_error":false,"usage":{{"input_tokens":3,"output_tokens":1}}}}'
""")
    os.chmod(path, 0o755)
    return path


def paint_gaps_ms(times, merge_ms=FRAME_MERGE_MS):
    """Gaps between paint starts; reads closer than merge_ms are one paint."""
    starts = []
    last = None
    for at in times:
        if last is None or (at - last) * 1000 > merge_ms:
            starts.append(at)
        last = at
    return [round((b - a) * 1000, 1) for a, b in zip(starts, starts[1:])]


class Session:
    def __init__(self, binary, cols, rows, home, project, ui="classic", resume=None,
                 claude_bin=None):
        self.master, slave = pty.openpty()
        set_size(slave, cols, rows)
        argv = [binary] + (["--resume", resume] if resume else [])
        self.started = time.perf_counter()
        self.proc = subprocess.Popen(
            argv, stdin=slave, stdout=slave, stderr=slave, cwd=project,
            env=session_env(home, ui, claude_bin), start_new_session=True, close_fds=True,
        )
        os.close(slave)
        self.answered = {}

    def read(self, timeout):
        """Return (bytes, timestamp) for the next chunk, or (b'', None)."""
        # A caller's deadline can pass between its check and this call.
        ready, _, _ = select.select([self.master], [], [], max(0, timeout))
        if not ready:
            return b"", None
        try:
            data = os.read(self.master, 65536)
        except OSError:
            return b"", None
        now = time.perf_counter()
        for query, reply in QUERIES.items():
            n = data.count(query)
            if n:
                self.answered[query.decode("ascii", "replace")] = (
                    self.answered.get(query.decode("ascii", "replace"), 0) + n)
                if reply:
                    os.write(self.master, reply * n)
        return data, now

    def wait_for(self, needles, timeout):
        buf = b""
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            data, at = self.read(deadline - time.perf_counter())
            if at is None:
                continue
            buf += data
            text = ESCAPES.sub(b"", buf)
            if all(n in text for n in needles):
                return at
        return None

    def wait_visible(self, timeout):
        """Timestamp of the next chunk carrying visible text, ignoring idle frames."""
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            data, at = self.read(deadline - time.perf_counter())
            if at is not None and ESCAPES.sub(b"", data).strip():
                return at
        return None

    def drain(self, quiet, limit=5.0):
        """Consume output until no visible text arrives for `quiet` seconds.

        Classic writes an escape-only frame on every idle tick, so chunks
        without visible text do not reset the quiet window.
        """
        deadline = time.perf_counter() + limit
        last = time.perf_counter()
        while time.perf_counter() - last < quiet and time.perf_counter() < deadline:
            data, at = self.read(quiet)
            if at is not None and ESCAPES.sub(b"", data).strip():
                last = at

    def visible_times(self, seconds):
        """Timestamps of the chunks carrying visible text for `seconds`."""
        times, end = [], time.perf_counter() + seconds
        while time.perf_counter() < end:
            data, at = self.read(end - time.perf_counter())
            if at is not None and ESCAPES.sub(b"", data).strip():
                times.append(at)
        return times

    def idle_writes_per_second(self, seconds=2.0):
        writes, end = 0, time.perf_counter() + seconds
        while time.perf_counter() < end:
            if self.read(end - time.perf_counter())[1] is not None:
                writes += 1
        return writes / seconds

    def rss_kib(self):
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(self.proc.pid)],
                             capture_output=True, text=True, check=False)
        return int(out.stdout.strip()) if out.stdout.strip() else None

    def close(self):
        if self.proc.poll() is None:
            for _ in range(2):
                os.write(self.master, b"\x03")
                time.sleep(0.3)
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait()
        os.close(self.master)


def pct(values, p):
    if not values:
        return None
    ordered = sorted(values)
    k = max(0, min(len(ordered) - 1, round(p / 100 * (len(ordered) - 1))))
    return ordered[k]


def summarize(values):
    return {
        "n": len(values),
        "p50_ms": round(statistics.median(values), 2) if values else None,
        "p95_ms": round(pct(values, 95), 2) if values else None,
        "max_ms": round(max(values), 2) if values else None,
    }


def measure_stream(s, keys):
    """Start a turn; return (paint gaps, input-to-paint samples, failures)."""
    os.write(s.master, b"\x15")  # clear the keys typed by the idle probe
    s.drain(0.1)
    os.write(s.master, b"go")
    s.drain(0.1)
    os.write(s.master, b"\r")
    if s.wait_for([STREAM_MARK], 15) is None:
        return [], [], ["turn never streamed within 15s"]
    s.visible_times(0.3)
    gaps = paint_gaps_ms(s.visible_times(2.0))
    inputs, failures = [], []
    for k in range(keys):
        key = STREAM_KEYS[k % len(STREAM_KEYS):k % len(STREAM_KEYS) + 1]
        sent = time.perf_counter()
        os.write(s.master, key)
        at = s.wait_for([key], 2)
        if at is None:
            failures.append(f"key {key!r} not painted in 2s during a turn")
        else:
            inputs.append((at - sent) * 1000)
        s.visible_times(0.05)
    return gaps, inputs, failures


def run_size(binary, cols, rows, runs, keys, ui="classic", history_lines=0, stream=False):
    cold, inputs, resizes, rss, idle, failures, answered = [], [], [], [], [], [], {}
    stream_gaps, stream_inputs = [], []
    for i in range(runs):
        with tempfile.TemporaryDirectory(prefix="coven-perf-") as root:
            home = os.path.join(root, "home")
            project = os.path.join(root, "project")
            os.makedirs(os.path.join(home, ".coven-code"))
            os.makedirs(project)
            with open(os.path.join(home, ".coven-code", "settings.json"), "w",
                      encoding="utf-8") as f:
                f.write('{"hasCompletedOnboarding": true}')
            resume = seed_session(home, project, history_lines) if history_lines else None
            fake = write_fake_claude(root) if stream else None
            s = Session(binary, cols, rows, home, project, ui, resume, fake)
            try:
                at = s.wait_for(ready_needles(ui, bool(resume)), 20)
                if at is None:
                    failures.append(f"run {i}: no usable frame within 20s")
                    continue
                cold.append((at - s.started) * 1000)
                s.drain(0.5)
                rss.append(s.rss_kib())
                idle.append(s.idle_writes_per_second())
                for k in range(keys):
                    key = KEYS[k % len(KEYS):k % len(KEYS) + 1]
                    sent = time.perf_counter()
                    os.write(s.master, key)
                    at = s.wait_for([key], 2)
                    if at is None:
                        failures.append(f"run {i}: key {key!r} not painted in 2s")
                    else:
                        inputs.append((at - sent) * 1000)
                    s.drain(0.08)
                for dc, dr in ((cols + 10, rows + 4), (cols, rows)):
                    set_size(s.master, dc, dr)
                    sent = time.perf_counter()
                    os.killpg(s.proc.pid, signal.SIGWINCH)
                    at = s.wait_visible(2)
                    if at is None:
                        failures.append(f"run {i}: no repaint after resize")
                    else:
                        resizes.append((at - sent) * 1000)
                    s.drain(0.3)
                if stream:
                    gaps, keyed, problems = measure_stream(s, keys)
                    stream_gaps += gaps
                    stream_inputs += keyed
                    failures += [f"run {i}: {p}" for p in problems]
                for q, n in s.answered.items():
                    answered[q] = answered.get(q, 0) + n
            finally:
                s.close()
    rss = [r for r in rss if r is not None]
    during_turn = {
        "stream_input_to_paint": summarize(stream_inputs),
        "stream_paint_gap": summarize(stream_gaps),
    } if stream else {}
    return {
        "cells": f"{cols}x{rows}",
        "ui": ui,
        "history_lines": history_lines,
        "runs": runs,
        "cold_usable_frame": summarize(cold),
        "input_to_paint": summarize(inputs),
        "resize_to_paint": summarize(resizes),
        "idle_rss_kib": {
            "n": len(rss),
            "p50": int(statistics.median(rss)) if rss else None,
            "max": max(rss) if rss else None,
        },
        "idle_writes_per_second_p50": (
            round(statistics.median(idle), 1) if idle else None),
        **during_turn,
        "queries_answered": {repr(k): v for k, v in answered.items()},
        "failures": failures,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", required=True)
    ap.add_argument("--runs", type=int, default=20)
    ap.add_argument("--keys", type=int, default=20)
    ap.add_argument("--size", action="append", default=[])
    ap.add_argument("--json")
    ap.add_argument("--ui", choices=["classic", "next"], default="classic")
    ap.add_argument("--history-lines", type=int, default=0)
    ap.add_argument("--stream", action="store_true")
    args = ap.parse_args()
    sizes = args.size or ["80x24", "120x40"]
    binary = os.path.abspath(args.bin)
    results = []
    for size in sizes:
        cols, rows = (int(v) for v in size.split("x"))
        results.append(run_size(binary, cols, rows, args.runs, args.keys,
                                args.ui, args.history_lines, args.stream))
    report = {"binary": binary, "results": results}
    text = json.dumps(report, indent=2)
    print(text)
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    return 1 if any(r["failures"] for r in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
