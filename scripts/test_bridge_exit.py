"""Verify CLI waits for bridge deregistration after restoring its terminal.

Usage: python3 scripts/test_bridge_exit.py /path/to/coven-code
Requires tmux and Unix. Runs local HTTP fixtures only; takes about 35 seconds.
"""

import argparse
import http.server
import threading
import tempfile
import pathlib
import subprocess
import shlex
import shutil
import os
import json
import time


def test_exit(binary, stalled):
    received = threading.Event()
    replied = threading.Event()
    polled = threading.Event()

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def request(self):
            self.rfile.read(int(self.headers.get("Content-Length", "0")))
            if self.command == "DELETE":
                received.set()
                time.sleep(35 if stalled else 0.75)
            if self.path.endswith("/poll"):
                polled.set()
            self.send_response(200)
            self.send_header("Content-Length", "2")
            self.end_headers()
            try:
                self.wfile.write(b"[]")
            except (BrokenPipeError, ConnectionResetError):
                pass
            if self.command == "DELETE":
                replied.set()

        do_GET = do_POST = do_DELETE = request

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    tmux = [shutil.which("tmux"), "-L", f"coven-primary-exit-{os.getpid()}"]
    try:
        with tempfile.TemporaryDirectory(prefix="coven-primary-exit-") as temp:
            home = pathlib.Path(temp)
            engine = home / "engine"
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
            env = {
                "HOME": temp,
                "PATH": "/opt/homebrew/bin:/usr/bin:/bin",
                "SHELL": "/bin/bash",
                "TERM": "xterm-256color",
                "COVEN_CODE_HOME": str(engine),
                "COVEN_HOME": str(home / "coven"),
                "ANTHROPIC_API_KEY": "local-fixture",
                "COVEN_CODE_BRIDGE_TOKEN": "local-fixture",
                "COVEN_CODE_BRIDGE_URL": f"http://127.0.0.1:{server.server_port}",
                "COVEN_CODE_DISABLE_MODELS_FETCH": "1",
                "RUST_LOG": "info",
            }
            marker = home / "exited"
            command = (
                shlex.join(
                    [
                        "/usr/bin/env",
                        "-i",
                        *[f"{k}={v}" for k, v in env.items()],
                        binary,
                    ]
                )
                + " 2>"
                + shlex.quote(str(home / "stderr"))
                + '; printf "%s" "$?" >'
                + shlex.quote(str(marker))
                + "; sleep 30"
            )
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
                    tmux + ["capture-pane", "-t", "probe", "-p"],
                    capture_output=True,
                    text=True,
                    check=True,
                    timeout=5,
                ).stdout
                if polled.is_set() and "Remote session active" in pane:
                    break
                time.sleep(0.05)
            assert polled.is_set() and "Remote session active" in pane, pane
            start = time.monotonic()
            subprocess.run(
                tmux + ["send-keys", "-t", "probe", "C-d"], check=True, timeout=5
            )
            time.sleep(0.15)
            subprocess.run(
                tmux + ["send-keys", "-t", "probe", "C-d"], check=True, timeout=5
            )
            deadline = time.monotonic() + 5
            while (
                not received.is_set()
                and (not marker.exists())
                and (time.monotonic() < deadline)
            ):
                time.sleep(0.02)
            assert received.is_set(), "No deregistration request before exit"
            alternate = subprocess.run(
                tmux + ["display-message", "-t", "probe", "-p", "#{alternate_on}"],
                capture_output=True,
                text=True,
                check=True,
                timeout=5,
            ).stdout.strip()
            assert alternate == "0", (
                "Terminal remains in alternate screen during cleanup"
            )
            assert not marker.exists(), "CLI exited before delayed cleanup"
            while time.monotonic() - start < 35 and (not marker.exists()):
                time.sleep(0.02)
            result = {
                "stalled": stalled,
                "terminal_restored_before_cleanup": alternate == "0",
                "exited": marker.exists(),
                "delete_received": received.is_set(),
                "delete_replied_before_exit": replied.is_set(),
                "exit_seconds": time.monotonic() - start,
            }
            print(json.dumps(result), flush=True)
            assert marker.exists() and marker.read_text() == "0", result
            if stalled:
                assert not replied.is_set() and 29 < result["exit_seconds"] < 35, result
                assert "Bridge deregister HTTP error" in (home / "stderr").read_text()
            else:
                assert replied.is_set(), "CLI exited before deregistration response"
    finally:
        subprocess.run(tmux + ["kill-server"], capture_output=True, timeout=10)
        server.shutdown()
        server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    args = parser.parse_args()
    if not shutil.which("tmux"):
        parser.error("tmux is required")
    binary = str(args.binary.resolve(strict=True))
    test_exit(binary, False)
    test_exit(binary, True)


if __name__ == "__main__":
    main()
