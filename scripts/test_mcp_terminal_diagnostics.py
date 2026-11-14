"""Exact-binary Classic/Next PTY regression for #443. Uses only local fixtures.

Usage: python3 scripts/test_mcp_terminal_diagnostics.py BINARY OUTPUT_DIRECTORY
Keeps raw PTY output, panes, logs, process receipts and a JSON result manifest.
"""
import argparse
import http.server
import json
import os
import pathlib
import shlex
import shutil
import subprocess
import sys
import threading
import time


def wait_for(predicate, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError("Timed out: " + description)


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


class Provider(http.server.BaseHTTPRequestHandler):
    active = threading.Event()
    def log_message(self, *_):
        pass

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        events = [
            ("message_start", {"message": {"id": "fixture", "model": "fixture", "usage": {"input_tokens": 1, "output_tokens": 0}}}),
            ("content_block_start", {"index": 0, "content_block": {"type": "text", "text": ""}}),
        ]
        events += [("content_block_delta", {"index": 0, "delta": {"type": "text_delta", "text": f"fixture-stream-{i} "}}) for i in range(60)]
        events += [
            ("content_block_stop", {"index": 0}),
            ("message_delta", {"delta": {"stop_reason": "end_turn"}, "usage": {"input_tokens": 1, "output_tokens": 60}}),
            ("message_stop", {}),
        ]
        self.active.set()
        try:
            for name, data in events:
                self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode())
                self.wfile.flush()
                time.sleep(0.05)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            self.active.clear()


def journey(binary, output, ui, port, persistent, width, height):
    profile = f"{ui}-{width}x{height}-{'file' if persistent else 'memory'}"
    root = output / profile
    root.mkdir()
    engine = root / "engine"
    engine.mkdir()
    fixture = root / "fixture"
    fixture.mkdir()
    script = pathlib.Path(__file__).parent / "fixtures/noisy_mcp.py"
    (engine / "settings.json").write_text(json.dumps({
        "hasCompletedOnboarding": True,
        "config": {"mcp_servers": [{"name": "noisy", "command": sys.executable, "args": [str(script.resolve()), str(fixture)], "env": {}, "server_type": "stdio"}]},
    }))
    env = {
        "HOME": str(root), "COVEN_HOME": str(root / "coven"),
        "COVEN_CODE_HOME": str(engine), "COVEN_CODE_UI": ui,
        "COVEN_CODE_DISABLE_MODELS_FETCH": "1", "ANTHROPIC_API_KEY": "local-fixture",
        "PATH": "/opt/homebrew/bin:/usr/bin:/bin", "SHELL": "/bin/bash", "TERM": "xterm-256color",
    }
    tmux = [shutil.which("tmux"), "-L", f"coven443-{os.getpid()}-{profile}"]
    def run(*args):
        return subprocess.run(tmux + list(args), check=True, capture_output=True, text=True, timeout=10).stdout
    checks = {}
    pid = None
    try:
        run("new-session", "-d", "-s", "probe", "-x", str(width), "-y", str(height), "-c", str(root), "/bin/bash --noprofile --norc")
        run("set-option", "-t", "probe", "remain-on-exit", "on")
        run("set-option", "-g", "set-clipboard", "on")
        raw = root / "terminal.raw"
        run("pipe-pane", "-t", "probe", "cat > " + shlex.quote(str(raw)))
        command = ["/usr/bin/env", "-i", *[f"{k}={v}" for k, v in env.items()], str(binary), "--api-base", f"http://127.0.0.1:{port}"]
        if persistent:
            command.append("--diagnostics")
        run("send-keys", "-t", "probe", "-l", "exec " + shlex.join(command))
        run("send-keys", "-t", "probe", "Enter")
        wait_for(lambda: (fixture / "connected").exists(), "MCP handshake")
        pid = int((fixture / "pid").read_text())
        def pane():
            return run("capture-pane", "-p", "-t", "probe")
        wait_for(lambda: "Coven" in pane(), "renderer startup")
        checks["diagnostic_path_visible"] = "Diagnostics:" in pane()
        for phase in ["idle", "active"]:
            if phase == "active":
                run("send-keys", "-t", "probe", "-l", "local streaming fixture")
                run("send-keys", "-t", "probe", "Enter")
                wait_for(Provider.active.is_set, "actual provider streaming")
            raw_start = len(raw.read_bytes())
            (fixture / "trigger").write_text(phase)
            wait_for(lambda: (fixture / f"noise-{phase}-done").exists(), "drain oversized output")
            time.sleep(1.2)
            capture = pane()
            (root / f"{phase}.pane.txt").write_text(capture)
            private = "".join(p.read_text() for p in (engine / "logs").glob("session-*.log"))
            (root / f"{phase}.diagnostics.txt").write_text(private)
            marker = f"MCP-NOISE-{phase}"
            checks[f"{phase}_no_raw_noise"] = marker.encode() not in raw.read_bytes()[raw_start:]
            checks[f"{phase}_persistence_policy"] = (engine / "logs").exists() == persistent and bool(list((engine / "logs").glob("session-*.log"))) == persistent
            run("send-keys", "-t", "probe", "-l", "/status logs")
            run("send-keys", "-t", "probe", "Enter")
            wait_for(lambda: "Diagnostics · line" in pane(), "diagnostic reader")
            run("send-keys", "-t", "probe", "/")
            run("send-keys", "-t", "probe", "-l", marker)
            run("send-keys", "-t", "probe", "Enter")
            reader = wait_for(lambda: pane() if "Diagnostics · line" in pane() and marker in pane() else None, "reachable diagnostic marker")
            (root / f"{phase}.reader.txt").write_text(reader)
            checks[f"{phase}_reachable_diagnostics"] = marker in reader and (not persistent or marker in private)
            checks[f"{phase}_reader_controls_safe"] = all(control not in raw.read_bytes()[raw_start:] for control in [b"\x1b]52;c;TUNQLUNMSVBCT0FSRA==", b"\x1b]0;MCP-PWNED-TITLE", b"\xc2\x9b31m", b"\x00"])
            run("send-keys", "-t", "probe", "Escape")
            wait_for(lambda: "Diagnostics · line" not in pane(), "diagnostic reader closed")
            checks[f"{phase}_sanitized_diagnostics"] = all(c not in private for c in ["\x1b", "\x00", "\x07", "\x9b"])
            checks[f"{phase}_title_safe"] = "MCP-PWNED-TITLE" not in run("display-message", "-p", "-t", "probe", "#{pane_title}")
            checks[f"{phase}_clipboard_safe"] = "MCP-CLIPBOARD" not in run("list-buffers")
            checks[f"{phase}_log_bounded"] = all(p.stat().st_size <= 1024 * 1024 for p in (engine / "logs").glob("session-*.log"))
        wait_for(lambda: not Provider.active.is_set(), "provider turn completed")
        time.sleep(0.3)
        run("send-keys", "-t", "probe", "C-c")
        time.sleep(0.3)
        run("send-keys", "-t", "probe", "C-c")
        wait_for(lambda: run("display-message", "-p", "-t", "probe", "#{pane_dead}").strip() == "1", "engine exit")
        wait_for(lambda: not alive(pid), "MCP child retirement", timeout=8)
        checks["diagnostics_not_in_sessions"] = all("MCP-NOISE" not in p.read_text() for p in (engine / "sessions").glob("*.json"))
        checks["child_retired"] = True
        checks["clean_exit"] = run("display-message", "-p", "-t", "probe", "#{pane_dead_status}").strip() == "0"
        return checks
    except Exception as error:
        checks["error"] = str(error)
        try:
            (root / "failure.pane.txt").write_text(pane())
        except Exception:
            pass
        return checks
    finally:
        subprocess.run(tmux + ["kill-server"], capture_output=True, timeout=10)
        if pid and alive(pid):
            os.kill(pid, 15)


def headless(binary, output, port):
    root = output / "headless"
    root.mkdir()
    engine = root / "engine"
    engine.mkdir()
    fixture = root / "fixture"
    fixture.mkdir()
    (fixture / "trigger").write_text("headless")
    script = pathlib.Path(__file__).parent / "fixtures/noisy_mcp.py"
    (engine / "settings.json").write_text(json.dumps({
        "hasCompletedOnboarding": True,
        "config": {"mcp_servers": [{"name": "noisy", "command": sys.executable, "args": [str(script.resolve()), str(fixture)], "env": {}, "server_type": "stdio"}]},
    }))
    env = {
        "HOME": str(root), "COVEN_HOME": str(root / "coven"),
        "COVEN_CODE_HOME": str(engine), "COVEN_CODE_DISABLE_MODELS_FETCH": "1",
        "ANTHROPIC_API_KEY": "local-fixture", "PATH": "/opt/homebrew/bin:/usr/bin:/bin",
    }
    result = subprocess.run([str(binary), "--print", "fixture", "--output-format", "stream-json", "--max-turns", "1", "--api-base", f"http://127.0.0.1:{port}"],
                            env=env, cwd=root, capture_output=True, timeout=20)
    (root / "stdout.jsonl").write_bytes(result.stdout)
    (root / "stderr.txt").write_bytes(result.stderr)
    frames = [json.loads(line) for line in result.stdout.splitlines()]
    pid = int((fixture / "pid").read_text())
    try:
        wait_for(lambda: not alive(pid), "headless child retirement", timeout=8)
        return {
            "success": result.returncode == 0,
            "json_frames": bool(frames) and any(frame.get("type") == "result" for frame in frames),
            "no_stdout_noise": b"MCP-NOISE" not in result.stdout,
            "reachable_stderr": b"MCP-NOISE-headless" in result.stderr,
            # Preserve the CLI's existing ANSI log styling; reject child controls.
            "sanitized_stderr": all(control not in result.stderr for control in [b"\x1b]", b"\x1b[2J", b"\x1b[H", b"\x00", b"\x07", b"\xc2\x9b"]),
            "child_retired": True,
        }
    finally:
        if alive(pid):
            os.kill(pid, 15)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    results = {}
    try:
        for ui in ["classic", "next"]:
            for width, height in [(80, 24), (120, 40)]:
                for persistent in [False, True]:
                    profile = f"{ui}-{width}x{height}-{'file' if persistent else 'memory'}"
                    try:
                        results[profile] = journey(args.binary.resolve(strict=True), args.output.resolve(), ui, server.server_port, persistent, width, height)
                    except Exception as error:
                        results[profile] = {"error": str(error)}
                    print(profile, json.dumps(results[profile]), flush=True)
        try:
            results["headless"] = headless(args.binary.resolve(strict=True), args.output.resolve(), server.server_port)
        except Exception as error:
            results["headless"] = {"error": str(error)}
        print("headless", json.dumps(results["headless"]), flush=True)
    finally:
        server.shutdown()
        server.server_close()
        (args.output / "results.json").write_text(json.dumps(results, indent=2))
    return 0 if all(value is True for checks in results.values() for value in checks.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
