"""Exercise CLI diagnostic routing with local HTTP fixtures and isolated terminals.

Usage: python3 scripts/test_diagnostic_routing.py /path/to/debug/coven-code
Requires Unix PTYs and tmux. Uses dummy credentials, never an external provider.
"""

import argparse
import http.server
import json
import os
import pathlib
import pty
import shlex
import shutil
import subprocess
import tempfile
import threading
import time


def test_modes(binary):

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            events = [
                (
                    "message_start",
                    {
                        "message": {
                            "id": "fixture",
                            "model": "claude-sonnet-4-20250514",
                            "usage": {"input_tokens": 1, "output_tokens": 0},
                        }
                    },
                ),
                (
                    "content_block_start",
                    {"index": 0, "content_block": {"type": "text", "text": ""}},
                ),
                (
                    "content_block_delta",
                    {
                        "index": 0,
                        "delta": {"type": "text_delta", "text": "fixture response"},
                    },
                ),
                ("content_block_stop", {"index": 0}),
                (
                    "message_delta",
                    {
                        "delta": {"stop_reason": "end_turn"},
                        "usage": {"input_tokens": 1, "output_tokens": 1},
                    },
                ),
                ("message_stop", {}),
            ]
            body = "".join(
                (
                    f"event: {name}\ndata: {json.dumps(data)}\n\n"
                    for name, data in events
                )
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        for mode in [
            "dump",
            "print",
            "headless",
            "positional",
            "context",
            "open-failure",
        ]:
            with tempfile.TemporaryDirectory(prefix="coven-log-modes-") as tmp:
                home = pathlib.Path(tmp)
                engine = home / "engine"
                engine.mkdir()
                env = {
                    "HOME": tmp,
                    "PATH": "/usr/bin:/bin",
                    "TERM": "xterm-256color",
                    "COVEN_CODE_HOME": str(engine),
                    "COVEN_HOME": str(home / "coven"),
                    "ANTHROPIC_API_KEY": "local-fixture",
                    "COVEN_CODE_DISABLE_MODELS_FETCH": "1",
                    "RUST_LOG": "debug",
                }
                args = {
                    "dump": ["--dump-system-prompt"],
                    "print": ["--print", "fixture"],
                    "headless": ["--headless", "fixture"],
                    "positional": ["fixture"],
                    "context": ["--context", str(home / "brief.json")],
                    "open-failure": [],
                }[mode]
                brief = {
                    "contract_version": "3",
                    "trigger": "fixture",
                    "repo": {
                        "owner": "fixture",
                        "name": "fixture",
                        "clone_url": "https://example.invalid/fixture",
                        "default_branch": "main",
                    },
                    "task": {
                        "kind": "respond_to_mention",
                        "issue_number": 1,
                        "comment_body": "fixture",
                    },
                    "familiar": {"id": "fixture", "display_name": "Fixture"},
                    "workspace": {"root": tmp},
                }
                (home / "brief.json").write_text(json.dumps(brief))
                if mode == "open-failure":
                    (engine / "logs").write_text("preserved")
                master, slave = pty.openpty()
                chunks = []

                def read_stderr():
                    while True:
                        try:
                            chunk = os.read(master, 65536)
                        except OSError:
                            break
                        if not chunk:
                            break
                        chunks.append(chunk)

                reader = threading.Thread(target=read_stderr, daemon=True)
                reader.start()
                try:
                    result = subprocess.run(
                        [
                            binary,
                            "--verbose",
                            *args,
                            "--api-base",
                            f"http://127.0.0.1:{server.server_port}",
                            "--output-format",
                            "stream-json",
                            "--max-turns",
                            "1",
                        ],
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.PIPE,
                        stderr=slave,
                        env=env,
                        cwd=tmp,
                        timeout=20,
                    )
                finally:
                    os.close(slave)
                reader.join(timeout=5)
                assert not reader.is_alive(), "stderr reader did not finish"
                os.close(master)
                err = b"".join(chunks).decode()
                if mode == "open-failure":
                    assert (
                        result.returncode != 0
                        and "Could not open private diagnostics" in err
                    ), err
                    assert "\x1b[?1049h" not in err
                    assert (engine / "logs").read_text() == "preserved"
                else:
                    assert result.returncode == 0, (mode, err)
                    assert "Starting Coven Code" in err, (
                        mode,
                        result.returncode,
                        err,
                        result.stdout[:500],
                    )
                    assert not (engine / "logs").exists(), mode
                    if mode != "dump":
                        lines = result.stdout.decode().splitlines()
                        assert lines, (mode, err)
                        frames = [json.loads(line) for line in lines]
                        assert any(frame.get("type") == "result" for frame in frames)
                        assert "fixture response" in result.stdout.decode(), (mode, frames)
                print(mode, "PASS", result.returncode, flush=True)
    finally:
        server.shutdown()
        server.server_close()


def test_tui(binary, output):

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def request(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            body = b"[]"
            status = 401 if self.path.endswith("/poll") else 200
            self.send_response(status)
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(body)

        do_GET = do_POST = do_DELETE = request

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    results = []
    try:
        for redirected in [False, True]:
            socket = f"coven-logging-probe-{os.getpid()}-{int(redirected)}"
            tmux = [shutil.which("tmux"), "-L", socket]
            try:
                with tempfile.TemporaryDirectory(prefix="coven-logging-probe-") as temp:
                    home = pathlib.Path(temp)
                    engine = home / ".coven-code"
                    engine.mkdir()
                    (engine / "settings.json").write_text(
                        json.dumps({"hasCompletedOnboarding": True})
                    )
                    for cache in [
                        home / "Library/Caches/coven-code",
                        home / ".cache/coven-code",
                    ]:
                        cache.mkdir(parents=True)
                        (cache / "update_check.txt").write_text("")
                    log = output / f"stderr-{int(redirected)}.txt"
                    env = {
                        "HOME": temp,
                        "PATH": "/opt/homebrew/bin:/usr/bin:/bin",
                        "SHELL": "/bin/bash",
                        "TERM": "xterm-256color",
                        "COVEN_CODE_HOME": str(engine),
                        "COVEN_HOME": str(home / ".coven"),
                        "ANTHROPIC_API_KEY": "local-fixture-key",
                        "COVEN_CODE_BRIDGE_TOKEN": "local-fixture-token",
                        "COVEN_CODE_BRIDGE_URL": f"http://127.0.0.1:{server.server_port}",
                        "COVEN_CODE_DISABLE_MODELS_FETCH": "1",
                    }
                    command = shlex.join(
                        [
                            "/usr/bin/env",
                            "-i",
                            *[f"{k}={v}" for k, v in env.items()],
                            binary,
                        ]
                    )
                    if redirected:
                        command += " 2>" + shlex.quote(str(log))
                    subprocess.run(
                        tmux
                        + [
                            "new-session",
                            "-d",
                            "-s",
                            "probe",
                            "-x",
                            "120",
                            "-y",
                            "36",
                            "-c",
                            temp,
                            command,
                        ],
                        check=True,
                        timeout=10,
                    )
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline:
                        pane = subprocess.run(
                            tmux + ["capture-pane", "-t", "probe", "-p", "-S", "-100"],
                            capture_output=True,
                            text=True,
                            check=True,
                            timeout=5,
                        ).stdout
                        logged = log.read_text() if redirected and log.exists() else ""
                        files = list((engine / "logs").glob("session-*.log"))
                        private = files[0].read_text() if files else ""
                        observed = "Bridge poll error" in (
                            logged if redirected else private
                        )
                        if observed and "Remote session active" in pane:
                            break
                        time.sleep(0.1)
                    assert observed, "No actual bridge warning observed"
                    assert "Remote session active" in pane, (
                        "CLI did not render active bridge state"
                    )
                    (output / f"pane-{int(redirected)}.txt").write_text(pane)
                    result = {
                        "redirected": redirected,
                        "warning_in_pane": "Bridge poll error" in pane,
                        "warning_in_log": "Bridge poll error" in logged,
                        "warning_in_private_log": "Bridge poll error" in private,
                        "tui_active": True,
                    }
                    results.append(result)
                    if redirected:
                        assert (
                            not result["warning_in_pane"] and result["warning_in_log"]
                        )
                    else:
                        assert not result["warning_in_pane"]
                        assert "Diagnostics:" in pane
                        assert "\x1b" not in private
                        assert "Bridge poll error" in private
                        (output / "private.txt").write_text(private)
                    if redirected:
                        assert not files
            finally:
                subprocess.run(tmux + ["kill-server"], capture_output=True, timeout=10)
    finally:
        server.shutdown()
        server.server_close()
    (output / "result.json").write_text(json.dumps(results, indent=2))
    print(json.dumps(results, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    args = parser.parse_args()
    if not shutil.which("tmux"):
        parser.error("tmux is required")
    binary = str(args.binary.resolve(strict=True))
    test_modes(binary)
    with tempfile.TemporaryDirectory(prefix="coven-diagnostic-results-") as directory:
        test_tui(binary, pathlib.Path(directory))


if __name__ == "__main__":
    main()
