#!/usr/bin/env python3
"""Run deterministic WebFetch source qualification; never equate it to release acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
# Explicit groups retain new tests in a module; zero/ignored matches fail closed.
SUITES = [
    ("authority", ["-p", "claurst-tools", "--test", "web_fetch_authority"]),
    ("backend", ["-p", "claurst-tools", "--lib", "web_fetch::"]),
    ("query-journal", ["-p", "claurst-query", "--lib", "retrieval_"]),
    ("query-dispatch", ["-p", "claurst-query", "--lib", "web_fetch_"]),
    ("research", ["-p", "claurst-tui", "--lib", "application::research::"]),
    ("research-render", ["-p", "claurst-tui", "--test", "next_research"]),
    ("typing-reopen", ["-p", "claurst", "--bin", "coven-code",
                       "tool_result_tests::webfetch_history_tests::webfetch_history_survives_typing_and_reopen"]),
]
SUMMARY = re.compile(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;")


def qualified(output, returncode):
    summaries = SUMMARY.findall(output)
    return (returncode == 0 and bool(summaries)
            and sum(int(s[1]) for s in summaries) > 0
            and all(s[0] == "ok" and int(s[2]) == 0 and int(s[3]) == 0 for s in summaries))


def run(command, cwd, env, log, timeout):
    started = time.monotonic()
    with log.open("wb") as stream:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                 stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            code = 124
    return {"command": command, "returncode": code, "seconds": round(time.monotonic()-started, 3),
            "log": log.name, "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest()}


def source_identity():
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT)
    files = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")
    digests = {}
    for raw in sorted(set(files)):
        if raw:
            name = os.fsdecode(raw)
            path = ROOT / name
            digests[name] = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else "missing"
    return {"head": git("rev-parse", "HEAD").decode().strip(),
            "status": git("status", "--porcelain=v1").decode(),
            "files": digests,
            "source_sha256": hashlib.sha256(json.dumps(digests, sort_keys=True).encode()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory outside checkout")
    parser.add_argument("--timeout", type=int, default=1800, help="maximum seconds per Cargo invocation")
    parser.add_argument("--no-default-features", action="store_true",
                        help="qualify without optional default features; recorded separately from default builds")
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("this runner requires POSIX process-group termination")
    output = args.output.resolve()
    if output == ROOT or ROOT in output.parents or args.timeout <= 0:
        parser.error("output must be outside checkout; timeout must be positive")
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"schema": 1, "scope": "source fixtures; not packaged/live-model acceptance",
               "platform": platform.platform(), "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "default_features": not args.no_default_features,
               "status": "blocked", "checks": [], "remaining": [
                   "Published binary WebFetch to Research acceptance",
                   "Fresh model extraction success/fallback with a controlled model fixture",
                   "Compressed response qualification if automatic decoding is enabled"]}
    try:
        receipt["source_before"] = source_identity()
        cargo, rustc = shutil.which("cargo"), shutil.which("rustc")
        if not cargo or not rustc:
            raise RuntimeError("cargo and rustc must be installed; no Rust checks ran")
        receipt["toolchain"] = {name: subprocess.check_output([path, "--version"], timeout=30).decode().strip()
                                for name, path in [("cargo", cargo), ("rustc", rustc)]}
        # Tests get no ambient provider credentials, proxy, endpoint, plugin or user settings.
        # Cargo is offline/locked. Prepare dependencies separately before this check.
        with tempfile.TemporaryDirectory(prefix="webfetch-security-") as home:
            env = {key: os.environ[key] for key in ["PATH", "SYSTEMROOT", "WINDIR", "TMPDIR", "TMP", "TEMP"] if key in os.environ}
            env.update({"HOME": home, "USERPROFILE": home, "COVEN_CODE_HOME": home + "/code",
                        "COVEN_HOME": home + "/coven", "COVEN_CAVE_HOME": home + "/cave",
                        "XDG_CONFIG_HOME": home + "/config", "XDG_DATA_HOME": home + "/data",
                        "CARGO_HOME": os.environ.get("CARGO_HOME", str(Path.home()/".cargo")),
                        "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", str(Path.home()/".rustup")),
                        "CARGO_TERM_COLOR": "never", "RUST_BACKTRACE": "0"})
            for label, selection in SUITES:
                features = ["--no-default-features"] if args.no_default_features else []
                command = [cargo, "test", "--locked", "--offline", *features, *selection, "--", "--nocapture"]
                print(f"Checking {label}", flush=True)
                result = run(command, ROOT/"src-rust", env, output/f"{label}.log", args.timeout)
                result["name"] = label
                result["status"] = "passed" if qualified((output/result["log"]).read_text(errors="replace"), result["returncode"]) else "failed"
                receipt["checks"].append(result)
                if result["status"] != "passed":
                    break
            receipt["source_after"] = source_identity()
            receipt["status"] = "passed" if (len(receipt["checks"]) == len(SUITES)
                and all(c["status"] == "passed" for c in receipt["checks"])
                and receipt["source_before"] == receipt["source_after"]) else "failed"
    except (OSError, subprocess.SubprocessError, RuntimeError) as error:
        receipt["error"] = str(error)
    finally:
        (output/"receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"{receipt['status']}: {output / 'receipt.json'}")
    return 0 if receipt["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
