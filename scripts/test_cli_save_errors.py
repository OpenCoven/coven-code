#!/usr/bin/env python3
"""Owned, offline tmux regression for explicit rename persistence failures.

No model request is needed. An owned loopback HTTP/proxy trap records unexpected
requests; startup model refresh is disabled and update caches are seeded fresh.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time


def run_case(binary, blocked, private_diagnostics):
    requests = []

    class Trap(http.server.BaseHTTPRequestHandler):
        def reject(self):
            requests.append((self.command, self.path))
            self.send_error(503)
        do_GET = reject
        do_POST = reject
        do_CONNECT = reject
        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Trap)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    with tempfile.TemporaryDirectory(prefix="coven-save-ui-") as temp:
        root = Path(temp)
        home, project = root / "home", root / "project"
        engine = home / "engine"
        sessions = engine / "sessions"
        sessions.mkdir(parents=True)
        project.mkdir()
        empty_path = root / "bin"
        empty_path.mkdir()
        (engine / "settings.json").write_text(json.dumps({"hasCompletedOnboarding": True}))
        for cache in [home / "Library/Caches", home / ".cache"]:
            directory = cache / "coven-code"
            directory.mkdir(parents=True)
            (directory / "update_check.txt").write_text("")
        session_id = "11111111-1111-4111-8111-111111111111"
        path = sessions / (session_id + ".json")
        original = json.dumps({
            "id": session_id, "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z", "model": "claude-sonnet-4-6",
            "title": "original-title", "working_dir": str(project),
            "messages": [{"role": "user", "content": "history-fixture-loaded"}],
        }).encode()
        path.write_bytes(original)
        proxy = f"http://127.0.0.1:{server.server_port}"
        env = {
            "HOME": str(home), "USERPROFILE": str(home),
            "COVEN_CODE_HOME": str(engine), "COVEN_HOME": str(home / "coven"),
            "COVEN_CAVE_HOME": str(home / "cave"),
            "XDG_CACHE_HOME": str(home / ".cache"), "PATH": str(empty_path),
            "SHELL": "/bin/sh", "TERM": "xterm-256color",
            "SAVE_STDERR": str(root / "stderr"),
            "ANTHROPIC_API_KEY": "owned-fixture-no-credential",
            "COVEN_CODE_DISABLE_MODELS_FETCH": "1",
            "HTTP_PROXY": proxy, "HTTPS_PROXY": proxy, "ALL_PROXY": proxy,
            "http_proxy": proxy, "https_proxy": proxy, "all_proxy": proxy,
            "NO_PROXY": "127.0.0.1", "no_proxy": "127.0.0.1",
        }
        tmux = [shutil.which("tmux"), "-f", "/dev/null", "-L", f"save-{os.getpid()}-{int(blocked)}"]
        def call(*args):
            return subprocess.run(tmux + list(args), check=True, capture_output=True, text=True, timeout=10).stdout
        def capture():
            return call("capture-pane", "-t", "fixture", "-p")
        def wait_for(needle):
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                pane = capture()
                if needle in pane:
                    return pane
                time.sleep(0.1)
            raise AssertionError(f"missing {needle!r}:\n{pane}")
        try:
            launch = [str(binary), "--resume", session_id, "--api-base", proxy]
            if not private_diagnostics:
                launch = ["/bin/sh", "-c", 'exec "$@" 2>"$SAVE_STDERR"', "fixture", *launch]
            call("new-session", "-d", "-s", "fixture", "-x", "140", "-y", "45",
                 "-c", str(project), "/usr/bin/env", "-i",
                 *[f"{k}={v}" for k, v in env.items()], *launch)
            wait_for("history-fixture-loaded")
            if blocked:
                path.rename(root / "original.json")
                path.mkdir()
                (path / "sentinel").write_bytes(b"untouched")
            call("send-keys", "-t", "fixture", "-l", "/session rename fixture-title")
            call("send-keys", "-t", "fixture", "Enter")
            pane = wait_for("Session not saved; retry before exit." if blocked else 'Session renamed to "fixture-title".')
            if blocked:
                assert 'Session renamed to "fixture-title".' not in pane, pane
                assert "Session renamed in memory; not saved." in pane, pane
                assert (root / "original.json").read_bytes() == original
                assert (path / "sentinel").read_bytes() == b"untouched"
                assert sorted(p.name for p in path.iterdir()) == ["sentinel"]
                if private_diagnostics:
                    logs = "".join(p.read_text() for p in (engine / "logs").glob("session-*.log"))
                else:
                    logs = (root / "stderr").read_text()
                assert "Failed to persist session" in logs and session_id in logs and "rename" in logs, logs
                assert "Is a directory" in logs, logs
                assert "fixture-title" not in logs and "history-fixture-loaded" not in logs, logs
                (path / "sentinel").unlink()
                path.rmdir()
                call("send-keys", "-t", "fixture", "-l", "/session rename retry-title")
                call("send-keys", "-t", "fixture", "Enter")
                pane = wait_for('Session renamed to "retry-title".')
                assert "Session not saved" not in pane, pane
                saved = json.loads(path.read_text())
                assert saved["title"] == "retry-title"
                assert saved["messages"][0]["content"] == "history-fixture-loaded"
            else:
                saved = json.loads(path.read_text())
                assert saved["title"] == "fixture-title"
                assert saved["messages"][0]["content"] == "history-fixture-loaded"
                assert "Session not saved" not in pane
            assert requests == [], requests
            print(f"PASS {'blocked' if blocked else 'writable'} rename; no HTTP requests")
        finally:
            subprocess.run(tmux + ["kill-server"], capture_output=True, timeout=10)
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            assert requests == [], requests


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--private-diagnostics", action="store_true")
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert binary.is_file() and shutil.which("tmux")
    print(f"Binary SHA256 {hashlib.sha256(binary.read_bytes()).hexdigest()}")
    failures = []
    for blocked in [False, True]:
        try:
            run_case(binary, blocked, args.private_diagnostics)
        except Exception as error:
            failures.append(f"blocked={blocked}: {error}")
    assert not failures, "\n".join(failures)


if __name__ == "__main__":
    main()
