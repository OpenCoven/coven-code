"""Offline tests for pr-ready.sh: the verdict, and the script against a fake gh."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("pr_ready", HERE / "pr_ready.py")
pr_ready = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pr_ready)

HEAD = "a" * 40


def pull(mergeable=True, merge_state="clean", **extra):
    data = {
        "number": 7,
        "title": "example",
        "state": "open",
        "draft": False,
        "merged": False,
        "head": {"sha": HEAD},
        "base": {"ref": "main"},
        "mergeable": mergeable,
        "mergeable_state": merge_state,
    }
    data.update(extra)
    return data


def run(id, name, conclusion="success", status="completed", suite=1, started="2026-11-19T01:00:00Z"):
    return {
        "id": id,
        "name": name,
        "status": status,
        "conclusion": conclusion if status == "completed" else None,
        "started_at": started,
        "check_suite": {"id": suite},
    }


AHEAD = {"status": "ahead", "ahead_by": 2, "behind_by": 0}
WORKFLOWS = [
    {"check_suite_id": 1, "name": "Rust CI"},
    {"check_suite_id": 2, "name": "Rust CI"},
    {"check_suite_id": 3, "name": "Release marker tests"},
]


def verdict(p=None, compare=AHEAD, checks=(), statuses=(), tries=1, min_checks=1):
    _, reasons = pr_ready.evaluate(
        p or pull(), compare, list(checks), list(statuses), WORKFLOWS, tries, min_checks
    )
    return pr_ready.verdict_line(reasons)


class VerdictTests(unittest.TestCase):
    def test_all_green_is_ready(self):
        self.assertEqual(verdict(checks=[run(1, "Format, lint, and test"), run(2, "CodeQL")]), "READY")

    def test_cancelled_duplicate_then_later_success_counts_as_success(self):
        checks = [
            run(10, "Format, lint, and test", "cancelled", suite=1, started="2026-11-19T01:00:00Z"),
            run(11, "Format, lint, and test", "success", suite=2, started="2026-11-19T01:05:00Z"),
        ]
        self.assertEqual(verdict(checks=checks), "READY")

    def test_latest_run_wins_even_when_it_failed(self):
        checks = [
            run(10, "Format, lint, and test", "success", suite=1, started="2026-11-19T01:00:00Z"),
            run(11, "Format, lint, and test", "cancelled", suite=2, started="2026-11-19T01:05:00Z"),
        ]
        self.assertEqual(verdict(checks=checks), "NOT READY: Rust CI / Format, lint, and test: cancelled")

    def test_same_started_at_falls_back_to_the_higher_id(self):
        checks = [run(11, "job", "success"), run(10, "job", "failure")]
        self.assertEqual(verdict(checks=checks), "READY")

    def test_pending_run_is_not_ready(self):
        checks = [run(1, "Format, lint, and test"), run(2, "Analyze (rust)", status="in_progress")]
        self.assertEqual(verdict(checks=checks), "NOT READY: Rust CI / Analyze (rust): in_progress")

    def test_same_job_name_in_two_workflows_is_kept_apart(self):
        checks = [run(1, "test", "success", suite=1), run(2, "test", "failure", suite=3)]
        self.assertEqual(verdict(checks=checks), "NOT READY: Release marker tests / test: failure")

    def test_skipped_and_neutral_pass(self):
        checks = [run(1, "a", "skipped"), run(2, "b", "neutral")]
        self.assertEqual(verdict(checks=checks), "READY")

    def test_null_mergeable_after_retries_is_not_ready(self):
        result = verdict(p=pull(mergeable=None, merge_state="unknown"), checks=[run(1, "a")], tries=12)
        self.assertEqual(result, "NOT READY: mergeable still unknown after 12 tries")

    def test_conflicting_pr_is_not_ready(self):
        result = verdict(p=pull(mergeable=False, merge_state="dirty"), checks=[run(1, "a")])
        self.assertEqual(result, "NOT READY: not mergeable (mergeable_state=dirty)")

    def test_blocked_state_is_not_ready(self):
        result = verdict(p=pull(merge_state="blocked"), checks=[run(1, "a")])
        self.assertEqual(result, "NOT READY: mergeable_state=blocked")

    def test_main_not_merged_in_is_not_ready(self):
        compare = {"status": "diverged", "ahead_by": 3, "behind_by": 7}
        result = verdict(compare=compare, checks=[run(1, "a")])
        self.assertEqual(result, "NOT READY: origin/main is not merged into the head (behind by 7)")

    def test_no_checks_is_not_ready_unless_allowed(self):
        self.assertEqual(
            verdict(checks=[]), "NOT READY: 0 check names on the head, expected at least 1"
        )
        self.assertEqual(verdict(checks=[], min_checks=0), "READY")

    def test_latest_commit_status_per_context(self):
        statuses = [
            {"id": 1, "context": "Vercel", "state": "pending", "created_at": "2026-11-19T01:00:00Z"},
            {"id": 2, "context": "Vercel", "state": "success", "created_at": "2026-11-19T01:01:00Z"},
        ]
        self.assertEqual(verdict(checks=[run(1, "a")], statuses=statuses), "READY")
        self.assertEqual(
            verdict(checks=[run(1, "a")], statuses=statuses[:1]), "NOT READY: status Vercel: pending"
        )

    def test_closed_draft_pr_lists_every_reason(self):
        result = verdict(p=pull(merge_state="draft", draft=True, state="closed"), checks=[run(1, "a")])
        self.assertIn("PR is closed", result)
        self.assertIn("PR is a draft", result)


FAKE_GH = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json, os, sys
    root = os.environ["FAKE_GH_DIR"]
    with open(os.path.join(root, "calls.log"), "a") as log:
        log.write(json.dumps(sys.argv[1:]) + "\\n")
    endpoint = sys.argv[2]
    def emit(name):
        with open(os.path.join(root, name)) as f:
            sys.stdout.write(f.read())
    if "/pulls/" in endpoint:
        counter = os.path.join(root, "pull-count")
        n = int(open(counter).read()) if os.path.exists(counter) else 0
        open(counter, "w").write(str(n + 1))
        pulls = json.load(open(os.path.join(root, "pulls.json")))
        print(json.dumps(pulls[min(n, len(pulls) - 1)]))
    elif "/compare/" in endpoint:
        emit("compare.json")
    elif "/check-runs" in endpoint:
        emit("check-runs.json")
    elif "/statuses" in endpoint:
        emit("statuses.json")
    elif "/actions/runs" in endpoint:
        emit("runs.json")
    else:
        sys.exit("unexpected endpoint " + endpoint)
    """
)


class ScriptTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        gh = self.dir / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.write("compare.json", AHEAD)
        self.write("check-runs.json", [{"total_count": 1, "check_runs": [run(1, "Format, lint, and test")]}])
        self.write("statuses.json", [[]])
        self.write("runs.json", [{"workflow_runs": WORKFLOWS}])

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name, data):
        (self.dir / name).write_text(json.dumps(data))

    def script(self, *args, tries="3"):
        env = dict(
            os.environ,
            PATH=f"{self.dir}{os.pathsep}{os.environ['PATH']}",
            FAKE_GH_DIR=str(self.dir),
            PR_READY_TRIES=tries,
            PR_READY_DELAY="0",
        )
        return subprocess.run(
            ["bash", str(HERE / "pr-ready.sh"), *args], env=env, capture_output=True, text=True
        )

    def calls(self):
        return [json.loads(line) for line in (self.dir / "calls.log").read_text().splitlines()]

    def test_retries_while_mergeable_is_null_then_ready(self):
        self.write("pulls.json", [pull(None, "unknown"), pull(None, "unknown"), pull()])
        result = self.script("7")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"head:      {HEAD}", result.stdout)
        self.assertIn("mergeable: true (clean) after 3 tries", result.stdout)
        self.assertIn("success          Rust CI / Format, lint, and test", result.stdout)
        self.assertTrue(result.stdout.rstrip().endswith("READY"))

    def test_null_after_every_retry_is_not_ready(self):
        self.write("pulls.json", [pull(None, "unknown")])
        result = self.script("7")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("NOT READY: mergeable still unknown after 3 tries", result.stdout)
        self.assertEqual(sum("/pulls/" in c[1] for c in self.calls()), 3)

    def test_only_reads_through_rest(self):
        self.write("pulls.json", [pull()])
        self.script("7", "--repo", "Example/repo")
        calls = self.calls()
        self.assertTrue(calls)
        for call in calls:
            self.assertEqual(call[0], "api")
            self.assertTrue(call[1].startswith("repos/Example/repo/"), call)
            self.assertNotIn("graphql", call[1])
            self.assertFalse({"-X", "--method", "-f", "-F", "--input"} & set(call), call)

    def test_api_failure_exits_2(self):
        self.write("pulls.json", [pull()])
        (self.dir / "check-runs.json").unlink()
        result = self.script("7")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
