#!/usr/bin/env python3
"""Synthetic Classic performance baseline over a raw pseudo-terminal.

Measures, per terminal size and run, against a disposable home with no
credentials and no model call:

- cold usable frame: spawn -> both the boot string and composer prompt drawn
- input-to-paint: one keystroke written -> first output whose visible text
  contains it
- resize-to-paint: TIOCSWINSZ + SIGWINCH -> first output with visible text
- idle RSS: resident set size after the first frame settles
- idle writes: PTY writes per second with no input or visible change

The harness answers the terminal queries a real emulator would (primary
device attributes, cursor position, keyboard-enhancement probe) so startup
does not include a query timeout. It records every query it answered.

This is a baseline instrument, not #417's final instrumentation: it observes
bytes at the PTY boundary, so it includes kernel/PTY scheduling and cannot
attribute time to individual render phases.

Usage:
  python3 scripts/tui-tests/perf_baseline.py --bin PATH [--runs N]
      [--keys N] [--size 80x24 --size 120x40] [--json OUT]
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

BOOT = b"Coven v"  # matched after escape sequences are removed
PROMPT = "❯".encode()  # composer prompt glyph
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


class Session:
    def __init__(self, binary, cols, rows, home, project):
        self.master, slave = pty.openpty()
        set_size(slave, cols, rows)
        env = {
            "PATH": os.defpath,
            "HOME": home,
            "USER": "baseline",
            "LOGNAME": "baseline",
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
        }
        self.started = time.perf_counter()
        self.proc = subprocess.Popen(
            [binary], stdin=slave, stdout=slave, stderr=slave, cwd=project,
            env=env, start_new_session=True, close_fds=True,
        )
        os.close(slave)
        self.answered = {}

    def read(self, timeout):
        """Return (bytes, timestamp) for the next chunk, or (b'', None)."""
        ready, _, _ = select.select([self.master], [], [], timeout)
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


def run_size(binary, cols, rows, runs, keys):
    cold, inputs, resizes, rss, idle, failures, answered = [], [], [], [], [], [], {}
    for i in range(runs):
        with tempfile.TemporaryDirectory(prefix="coven-perf-") as root:
            home = os.path.join(root, "home")
            project = os.path.join(root, "project")
            os.makedirs(os.path.join(home, ".coven-code"))
            os.makedirs(project)
            with open(os.path.join(home, ".coven-code", "settings.json"), "w",
                      encoding="utf-8") as f:
                f.write('{"hasCompletedOnboarding": true}')
            s = Session(binary, cols, rows, home, project)
            try:
                at = s.wait_for([BOOT, PROMPT], 20)
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
                for q, n in s.answered.items():
                    answered[q] = answered.get(q, 0) + n
            finally:
                s.close()
    rss = [r for r in rss if r is not None]
    return {
        "cells": f"{cols}x{rows}",
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
    args = ap.parse_args()
    sizes = args.size or ["80x24", "120x40"]
    binary = os.path.abspath(args.bin)
    results = []
    for size in sizes:
        cols, rows = (int(v) for v in size.split("x"))
        results.append(run_size(binary, cols, rows, args.runs, args.keys))
    report = {"binary": binary, "results": results}
    text = json.dumps(report, indent=2)
    print(text)
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    return 1 if any(r["failures"] for r in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
