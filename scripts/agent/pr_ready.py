#!/usr/bin/env python3
"""Report whether a pull request is ready to merge. Never merges, pushes or edits.

usage: pr-ready.sh <pr> [--repo OWNER/NAME] [--min-checks N] [--require-workflow NAME]...

Reads the pull request through the REST API (`gh api`) and prints:
- the head SHA;
- whether the PR's base branch on GitHub (main) is an ancestor of the head;
- mergeable and mergeable_state, retried while GitHub reports them as unknown;
- the latest run for each check name, the latest run of each workflow (which
  catches a workflow whose jobs have not started), and the latest commit
  status per context;
then one verdict line, READY or NOT READY with the reasons.

A check name is keyed by its workflow ("Rust CI / Format, lint, and test"), so
same-named jobs in two workflows stay apart, while a cancelled duplicate and a
later rerun of the same job collapse into the later one. Runs from another
trigger (a workflow_dispatch run on the branch) are kept apart from the
pull_request runs, so they cannot stand in for a failed one.

READY covers only the runs that exist. Right after a push GitHub may not have
created every workflow run yet, so pass --require-workflow for the workflows
the change must run (for example "Rust CI" and "TUI Tests" for src-rust/).

Environment: PR_READY_TRIES (default 12) and PR_READY_DELAY (seconds, default
10) bound the wait for a known mergeable state, about two minutes in all.

Exit status: 0 READY, 1 NOT READY, 2 usage or API error.
"""
import argparse
import json
import os
import subprocess
import sys
import time

DEFAULT_REPO = "OpenCoven/coven-code"
PASSING = {"success", "neutral", "skipped"}
# GitHub's mergeable_state values that allow a merge with every check green.
MERGEABLE_OK = {"clean", "has_hooks"}
STATUS_FAILING = {"failure", "error"}


class ApiError(Exception):
    pass


def gh_json(args):
    try:
        out = subprocess.run(
            ["gh", "api", *args], check=True, capture_output=True, text=True
        ).stdout
    except FileNotFoundError as error:
        raise ApiError("gh is not installed") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        raise ApiError(f"gh api {args[0]} failed: {detail}") from error
    try:
        return json.loads(out)
    except json.JSONDecodeError as error:
        raise ApiError(f"gh api {args[0]} returned non-JSON output") from error


def pages(data, key=None):
    """Flatten `gh api --paginate --slurp` output: a list of pages."""
    if not isinstance(data, list):
        data = [data]
    items = []
    for page in data:
        if key is not None and isinstance(page, dict):
            items.extend(page.get(key) or [])
        elif isinstance(page, list):
            items.extend(page)
    return items


def latest(items, key, order):
    chosen = {}
    for item in items:
        k = key(item)
        if k not in chosen or order(item) > order(chosen[k]):
            chosen[k] = item
    return chosen


def check_key(run, workflows):
    suite = (run.get("check_suite") or {}).get("id")
    workflow = workflows.get(suite)
    name = run.get("name") or "?"
    if workflow:
        return f"{workflow} / {name}"
    # Not an Actions job: two apps may use the same check name.
    app = (run.get("app") or {}).get("slug")
    return f"{name} ({app})" if app and app != "github-actions" else name


def workflow_label(run):
    """A workflow run's name, with its trigger when that is not the PR itself."""
    name = run.get("name") or "?"
    event = run.get("event")
    return name if event in (None, "pull_request") else f"{name} ({event})"


def evaluate(
    pull, compare, check_runs, statuses, workflow_runs, tries, min_checks=1, required=()
):
    """Return (report lines, reasons). READY when reasons is empty."""
    reasons = []
    lines = []
    head = (pull.get("head") or {}).get("sha") or "?"
    base = (pull.get("base") or {}).get("ref") or "main"
    lines.append(f"PR #{pull.get('number')}: {pull.get('title', '')}")
    lines.append(f"head:      {head}")

    if pull.get("merged"):
        reasons.append("already merged")
    elif pull.get("state") != "open":
        reasons.append(f"PR is {pull.get('state')}")
    if pull.get("draft"):
        reasons.append("PR is a draft")

    behind = compare.get("behind_by")
    status = compare.get("status")
    contains = status in ("ahead", "identical") and not behind
    lines.append(
        f"base:      origin/{base} is {'' if contains else 'NOT '}an ancestor of the head"
        f" (compare status={status}, ahead_by={compare.get('ahead_by')}, behind_by={behind})"
    )
    if not contains and pull.get("state") == "open":
        reasons.append(f"origin/{base} is not merged into the head (behind by {behind})")

    mergeable = pull.get("mergeable")
    state = pull.get("mergeable_state")
    shown = "null" if mergeable is None else str(mergeable).lower()
    lines.append(f"mergeable: {shown} ({state}) after {tries} tr{'y' if tries == 1 else 'ies'}")
    # A closed PR already has its reason; GitHub stops computing mergeability.
    if pull.get("state") == "open":
        if mergeable is None or state == "unknown":
            reasons.append(f"mergeable still unknown after {tries} tries")
        elif mergeable is False:
            reasons.append(f"not mergeable (mergeable_state={state})")
        elif state not in MERGEABLE_OK:
            reasons.append(f"mergeable_state={state}")

    # A dispatched or pushed run of the same workflow is kept apart from the
    # pull_request run, so it cannot stand in for a failed one.
    workflows = {
        run.get("check_suite_id"): workflow_label(run)
        for run in workflow_runs
        if run.get("check_suite_id") is not None
    }
    # Ids only increase, so the highest id is the newest run, even for a
    # queued rerun that has no started_at yet.
    runs = latest(
        check_runs,
        key=lambda run: check_key(run, workflows),
        order=lambda run: run.get("id") or 0,
    )
    lines.append(f"checks:    {len(runs)} check names ({len(check_runs)} runs on the head)")
    for name in sorted(runs):
        run = runs[name]
        if run.get("status") != "completed":
            result = run.get("status") or "pending"
            reasons.append(f"{name}: {result}")
        else:
            result = run.get("conclusion") or "none"
            if result not in PASSING:
                reasons.append(f"{name}: {result}")
        lines.append(f"  {result:<16} {name}")
    if len(runs) < min_checks:
        reasons.append(f"{len(runs)} check names on the head, expected at least {min_checks}")

    # A workflow run can exist before its jobs have check runs (queued, or a
    # workflow_run trigger), or fail without any job (startup_failure).
    # Keyed by the workflow file, since two workflows can share a display name.
    newest_workflows = latest(
        workflow_runs,
        key=lambda run: (
            run.get("path") or run.get("workflow_id") or run.get("name") or "?",
            run.get("event") or "",
        ),
        order=lambda run: run.get("id") or 0,
    )
    if newest_workflows:
        lines.append(f"workflows: {len(newest_workflows)} workflow runs")
    for key in sorted(newest_workflows, key=str):
        run = newest_workflows[key]
        name = workflow_label(run)
        if run.get("status") != "completed":
            result = run.get("status") or "pending"
        else:
            result = run.get("conclusion") or "none"
        if run.get("status") != "completed" or result not in PASSING:
            reasons.append(f"workflow {name}: {result}")
        lines.append(f"  {result:<16} {name}")
    # Right after a push GitHub may not have created every workflow run yet.
    # Only the PR's own runs count; a dispatched run is not the PR's check.
    present = {run.get("name") for run in workflow_runs if run.get("event") in (None, "pull_request")}
    for name in required:
        if name not in present:
            reasons.append(f"workflow {name}: no run on the head yet")

    contexts = latest(
        statuses,
        key=lambda s: s.get("context") or "?",
        order=lambda s: s.get("id") or 0,
    )
    if contexts:
        lines.append(f"statuses:  {len(contexts)} contexts")
    for name in sorted(contexts):
        result = contexts[name].get("state") or "pending"
        if result == "pending" or result in STATUS_FAILING:
            reasons.append(f"status {name}: {result}")
        lines.append(f"  {result:<16} {name}")
    return lines, reasons


def verdict_line(reasons):
    return "READY" if not reasons else "NOT READY: " + "; ".join(reasons)


def fetch_pull(repo, pr, tries, delay):
    pull = {}
    for attempt in range(1, tries + 1):
        pull = gh_json([f"repos/{repo}/pulls/{pr}"])
        known = pull.get("mergeable") is not None and pull.get("mergeable_state") != "unknown"
        if known or pull.get("state") != "open" or attempt == tries:
            return pull, attempt
        time.sleep(delay)
    return pull, tries


def main(argv=None):
    parser = argparse.ArgumentParser(prog="pr-ready.sh", description=__doc__.splitlines()[0])
    parser.add_argument("pr", type=int)
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument(
        "--min-checks",
        type=int,
        default=1,
        help="NOT READY with fewer check names than this (default 1; 0 allows none)",
    )
    parser.add_argument(
        "--require-workflow",
        action="append",
        default=[],
        metavar="NAME",
        help="NOT READY until a run of this workflow exists on the head (repeatable), "
        "e.g. 'Rust CI' for a Rust change",
    )
    args = parser.parse_args(argv)
    if args.min_checks < 0:
        parser.error("--min-checks must be 0 or more")
    try:
        tries = max(1, int(os.environ.get("PR_READY_TRIES", "12")))
        delay = max(0.0, float(os.environ.get("PR_READY_DELAY", "10")))
    except ValueError:
        print("pr-ready: PR_READY_TRIES and PR_READY_DELAY must be numbers", file=sys.stderr)
        return 2
    repo = args.repo
    try:
        pull, used = fetch_pull(repo, args.pr, tries, delay)
        head = (pull.get("head") or {}).get("sha")
        base = (pull.get("base") or {}).get("ref") or "main"
        if not head:
            raise ApiError(f"PR #{args.pr} has no head SHA")
        compare = gh_json(
            [
                f"repos/{repo}/compare/{base}...{head}",
                "--jq",
                "{status, ahead_by, behind_by}",
            ]
        )
        check_runs = pages(
            gh_json(
                [f"repos/{repo}/commits/{head}/check-runs?per_page=100", "--paginate", "--slurp"]
            ),
            key="check_runs",
        )
        statuses = pages(
            gh_json([f"repos/{repo}/commits/{head}/statuses?per_page=100", "--paginate", "--slurp"])
        )
        workflow_runs = pages(
            gh_json(
                [f"repos/{repo}/actions/runs?head_sha={head}&per_page=100", "--paginate", "--slurp"]
            ),
            key="workflow_runs",
        )
        # A push during the calls above would make the verdict describe an old head.
        now = (gh_json([f"repos/{repo}/pulls/{args.pr}"]).get("head") or {}).get("sha")
    except ApiError as error:
        print(f"pr-ready: {error}", file=sys.stderr)
        return 2
    try:
        lines, reasons = evaluate(
            pull,
            compare,
            check_runs,
            statuses,
            workflow_runs,
            used,
            args.min_checks,
            args.require_workflow,
        )
    except (AttributeError, KeyError, TypeError) as error:
        print(f"pr-ready: unexpected API response: {error!r}", file=sys.stderr)
        return 2
    if now != head:
        reasons.append(f"the head moved to {now} during the check; rerun")
    print("\n".join(lines))
    print(verdict_line(reasons))
    return 0 if not reasons else 1


if __name__ == "__main__":
    sys.exit(main())
