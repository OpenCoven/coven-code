#!/usr/bin/env python3
"""Exercise CLI bridge disconnect with localhost HTTP and tmux (about 3 minutes).

No provider turns are submitted. Pass a built debug binary and an output directory.
Requires tmux; intentionally separate from the fast Python/unit test suites.
"""

import argparse
import http.server
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    tmux_bin = shutil.which("tmux")
    if tmux_bin is None:
        parser.error("tmux is required")

    requests = []
    lock = threading.Lock()
    started = time.monotonic()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def handle_request(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            with lock:
                requests.append({
                    "at": time.monotonic() - started,
                    "method": self.command,
                    "path": self.path,
                })
            status = 401 if self.command == "GET" and self.path.endswith("/poll") else 200
            body = b"[]"
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        do_POST = do_GET = do_DELETE = handle_request

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    tmux = [tmux_bin, "-L", "coven-bridge-test-" + str(os.getpid())]
    try:
        with tempfile.TemporaryDirectory(prefix="coven-cli-bridge-test-") as temp:
            home = Path(temp)
            engine = home / ".coven-code"
            engine.mkdir()
            (engine / "settings.json").write_text(json.dumps({"hasCompletedOnboarding": True}))
            for cache in [home / "Library/Caches/coven-code", home / ".cache/coven-code"]:
                cache.mkdir(parents=True)
                (cache / "update_check.txt").write_text("")
            env = {
                "HOME": str(home),
                "PATH": "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
                "SHELL": "/bin/bash",
                "TERM": "xterm-256color",
                "COVEN_CODE_HOME": str(engine),
                "COVEN_HOME": str(home / ".coven"),
                "ANTHROPIC_API_KEY": "local-fixture-not-a-real-key",
                "COVEN_CODE_BRIDGE_TOKEN": "local-fixture-token",
                "COVEN_CODE_BRIDGE_URL": f"http://127.0.0.1:{server.server_port}",
            }
            command = shlex.join([
                "/usr/bin/env", "-i", *[f"{key}={value}" for key, value in env.items()], str(binary)
            ])
            subprocess.run(
                tmux + ["new-session", "-d", "-s", "audit", "-x", "120", "-y", "36",
                        "-c", temp, command],
                check=True, timeout=10,
            )
            deadline = time.monotonic() + 210
            disconnected_at = None
            while time.monotonic() < deadline:
                pane = subprocess.run(
                    tmux + ["capture-pane", "-t", "audit", "-p", "-S", "-100"],
                    capture_output=True, text=True, check=True, timeout=5,
                ).stdout
                if disconnected_at is None and (
                    "Bridge disconnected:" in pane or "Remote control connection lost." in pane
                ):
                    disconnected_at = time.monotonic() - started
                    (args.output_dir / "pane.txt").write_text(pane)
                if disconnected_at is not None and time.monotonic() - started - disconnected_at >= 5:
                    break
                with lock:
                    polled = any(item["path"].endswith("/poll") for item in requests)
                if not polled and time.monotonic() - started > 15:
                    (args.output_dir / "pane.txt").write_text(pane)
                    raise AssertionError("No primary poll observed within 15 seconds")
                time.sleep(0.2)

            with lock:
                captured = list(requests)
            secondary = [item for item in captured if "/api/bridge/" in item["path"]]
            after = [item for item in secondary
                     if disconnected_at is not None and item["at"] > disconnected_at]
            result = {
                "ui_disconnect_at": disconnected_at,
                "secondary_requests_after_ui_disconnect": len(after),
                "requests": captured,
            }
            (args.output_dir / "requests.json").write_text(json.dumps(result, indent=2))
            print(json.dumps({key: value for key, value in result.items() if key != "requests"}))
            assert disconnected_at is not None, "No UI disconnect within bounded observation"
            assert secondary, "Secondary polling never started; shutdown assertion would be vacuous"
            assert any(item["method"] == "DELETE" for item in captured), "Primary never deregistered"
            assert not after, "Secondary polling continued after displayed disconnection"
            print("PASS: actual CLI secondary polling stopped after displayed disconnection")
    finally:
        subprocess.run(tmux + ["kill-server"], capture_output=True, timeout=10)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
