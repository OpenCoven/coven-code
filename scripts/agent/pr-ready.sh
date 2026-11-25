#!/usr/bin/env bash
# usage: scripts/agent/pr-ready.sh <pr> [--repo OWNER/NAME] [--min-checks N] [--require-workflow NAME]...
#
# Prints a pull request's head SHA, whether its base branch on GitHub is an
# ancestor of it, mergeable and mergeable_state (retried for about two
# minutes while GitHub reports them as unknown), the latest run for each check
# name, the latest run of each workflow and the latest commit status per
# context, then READY or NOT READY with the reasons. REST only (`gh api`); it never merges,
# pushes or edits anything. Exit 0 only when READY. See pr_ready.py.
set -u
exec python3 -I "$(dirname "${BASH_SOURCE[0]}")/pr_ready.py" "$@"
