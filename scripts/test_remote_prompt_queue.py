"""Exercise literal remote FIFO and cancellation with an isolated CLI and local HTTP.

Usage: python3 scripts/test_remote_prompt_queue.py /path/to/coven-code
Requires Unix and tmux. No external provider is contacted.
"""

import argparse
import http.server
import json
import os
import select
import socket
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import threading
import time


def exercise(binary, cancel_first, expect_startup_cancellation=False):
    requests = []
    lock = threading.Lock()
    release = threading.Event()
    delivered = 0
    first_connection = None
    cancelled_before_headers = False
    failures = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def handle_request(self):
            nonlocal delivered, first_connection, cancelled_before_headers
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            body = b"[]"
            content_type = "application/json"
            if self.command == "POST" and self.path == "/v1/messages":
                with lock:
                    index = len(requests)
                    if index == 0:
                        first_connection = self.connection
                    elif not release.is_set():
                        # A queued turn may start before headers only when the
                        # cancelled request has actually relinquished its socket.
                        readable, _, _ = select.select([first_connection], [], [], 0)
                        try:
                            closed = bool(readable) and not first_connection.recv(
                                1, socket.MSG_PEEK
                            )
                        except ConnectionResetError:
                            closed = True
                        if closed:
                            cancelled_before_headers = True
                        else:
                            failures.append("Second request started while first was live")
                    requests.append(json.loads(raw))
                if index == 0 and not release.wait(15):
                    with lock:
                        failures.append("First response gate timed out")
                    return
                events = [
                    (
                        "message_start",
                        {
                            "message": {
                                "id": f"fixture-{index}",
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
                            "delta": {
                                "type": "text_delta",
                                "text": f"fixture reply {index}",
                            },
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
                    f"event: {name}\ndata: {json.dumps(data)}\n\n"
                    for name, data in events
                ).encode()
                content_type = "text/event-stream"
            elif self.path.endswith("/poll"):
                with lock:
                    if delivered == 0:
                        texts = (
                            ["fixture first"]
                            if cancel_first
                            else ["fixture first", "/exit"]
                        )
                        payload = [
                            {
                                "type": "user_message",
                                "content": text,
                                "session_id": "fixture",
                                "message_id": f"first-{i}",
                            }
                            for i, text in enumerate(texts)
                        ]
                        delivered = 1
                    elif cancel_first and delivered == 1 and requests:
                        payload = [
                            {
                                "type": "user_message",
                                "content": "/exit",
                                "session_id": "fixture",
                                "message_id": "second",
                            },
                            {
                                "type": "cancel",
                                "session_id": "fixture",
                                "reason": "fixture",
                            },
                        ]
                        delivered = 2
                    else:
                        payload = []
                body = json.dumps(payload).encode()
            try:
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        do_POST = do_GET = do_DELETE = handle_request

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    tmux = [shutil.which("tmux"), "-L", f"coven-prompt-queue-{os.getpid()}"]
    try:
        with tempfile.TemporaryDirectory(prefix="coven-remote-queue-") as temp:
            home = Path(temp)
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
            }
            command = shlex.join(
                [
                    "/usr/bin/env",
                    "-i",
                    *[f"{k}={v}" for k, v in env.items()],
                    binary,
                    "--api-base",
                    f"http://127.0.0.1:{server.server_port}",
                ]
            )
            command += " 2>" + shlex.quote(str(home / "stderr"))
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
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                with lock:
                    count = len(requests)
                if count:
                    break
                time.sleep(0.05)
            assert count == 1, ("Initial overlapping requests", count)
            if cancel_first:
                # Observe the control event. The socket check above distinguishes
                # prompt cancellation from an overlapping query launch.
                deadline = time.monotonic() + 6
                while time.monotonic() < deadline:
                    pane = subprocess.run(
                        tmux + ["capture-pane", "-t", "probe", "-p", "-S", "-100"],
                        capture_output=True,
                        text=True,
                        check=True,
                        timeout=5,
                    ).stdout
                    with lock:
                        cancelled = cancelled_before_headers
                    if "Cancelled by remote control." in pane or cancelled:
                        break
                    time.sleep(0.05)
                assert "Cancelled by remote control." in pane or cancelled, pane
            if cancel_first and expect_startup_cancellation:
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline:
                    with lock:
                        count = len(requests)
                    if count == 2:
                        break
                    time.sleep(0.02)
                assert count == 2, "Cancellation still waits for response headers"
                with lock:
                    assert cancelled_before_headers, "Cancelled request did not close before next turn"
            else:
                deadline = time.monotonic() + 0.5
                while time.monotonic() < deadline:
                    with lock:
                        assert not failures, failures
                        if not cancel_first:
                            assert len(requests) == 1, "Second query started before first response"
                    time.sleep(0.02)
            with lock:
                assert not failures, failures
            release.set()
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                with lock:
                    count = len(requests)
                if count >= 2:
                    break
                time.sleep(0.05)
            assert count == 2, "Queued prompt never became a second turn"
            with lock:
                captured = list(requests)
            last = captured[1]["messages"][-1]
            assert last["role"] == "user", last
            content = last["content"]
            text = (
                content
                if isinstance(content, str)
                else "".join(block.get("text", "") for block in content)
            )
            assert text == "/exit", last
            if not cancel_first:
                assert "fixture reply 0" in json.dumps(captured[1]["messages"]), (
                    "Second turn lost first response"
                )
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                pane = subprocess.run(
                    tmux + ["capture-pane", "-t", "probe", "-p", "-S", "-100"],
                    capture_output=True,
                    text=True,
                    check=True,
                    timeout=5,
                ).stdout
                if "fixture reply 1" in pane:
                    break
                time.sleep(0.05)
            assert "fixture reply 1" in pane, pane
            with lock:
                assert len(requests) == 2, "Unexpected additional query"
                assert not failures, failures
            print(
                json.dumps(
                    {
                        "cancel_first": cancel_first,
                        "provider_turns": count,
                        "literal_slash_prompt": True,
                        "second_reply_visible": True,
                        "startup_cancellation_required": cancel_first and expect_startup_cancellation,
                        "cancelled_before_headers": cancelled_before_headers,
                    }
                ),
                flush=True,
            )
    finally:
        release.set()
        subprocess.run(tmux + ["kill-server"], capture_output=True, timeout=10)
        server.shutdown()
        server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument(
        "--expect-startup-cancellation",
        action="store_true",
        help="require cancelled HTTP request to close and next turn to start before headers",
    )
    args = parser.parse_args()
    if not shutil.which("tmux"):
        parser.error("tmux is required")
    binary = str(args.binary.resolve(strict=True))
    exercise(binary, False)
    exercise(binary, True, args.expect_startup_cancellation)


if __name__ == "__main__":
    main()
