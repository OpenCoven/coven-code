#!/usr/bin/env python3
"""Prepend a patch bullet to a GitHub release's body, idempotent across runs.

Required env vars:
  REPO          owner/name (e.g. OpenCoven/coven-codes)
  TAG           release tag to amend (e.g. v0.1.1)
  PATCH_BULLET  the markdown line to insert, e.g.
                  - fix(tui): scroll regression ([`abc1234`](https://github.com/.../commit/abc1234567))

Behaviour:
  - If the release body already starts with `## 🩹 Patches`, the new bullet is
    appended to that section's bullet list (so successive patches stack at the
    top in order).
  - Otherwise a new `## 🩹 Patches` section is prepended, leaving the rest of
    the existing body (categories, contributors, full-changelog footer)
    completely untouched.

Reads the current body via `gh release view --json body` and writes the new
body via `gh release edit --notes-file`.  No other release metadata is touched.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

HEADING = "## 🩹 Patches"


def die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


def fetch_body(repo: str, tag: str) -> str:
    out = subprocess.run(
        [
            "gh", "release", "view", tag,
            "--repo", repo,
            "--json", "body",
        ],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out).get("body") or ""


def splice(body: str, bullet: str) -> str:
    # Normalise line endings; gh returns \n already but be defensive.
    body = body.replace("\r\n", "\n").rstrip("\n")
    bullet = bullet.rstrip("\n")

    lines = body.split("\n") if body else []

    # Locate a leading "## 🩹 Patches" heading, tolerating leading blank lines.
    head_idx = 0
    while head_idx < len(lines) and lines[head_idx].strip() == "":
        head_idx += 1

    if head_idx < len(lines) and lines[head_idx].strip() == HEADING:
        # Append to existing patch list — walk past the heading and any
        # immediate blank line, then past the existing `- …` bullets,
        # then insert the new bullet right after the last one so order
        # preserves chronology (oldest patch on top, newest below it).
        j = head_idx + 1
        while j < len(lines) and lines[j].strip() == "":
            j += 1
        while j < len(lines) and lines[j].startswith("- "):
            j += 1
        lines.insert(j, bullet)
        return "\n".join(lines) + "\n"

    # No existing patch section — prepend one without disturbing anything else.
    new_section = [HEADING, "", bullet]
    if body:
        new_section.append("")  # blank line between our section and the rest
        new_section.append(body)
    return "\n".join(new_section) + "\n"


def main() -> None:
    repo = os.environ.get("REPO") or die("REPO env var is required")
    tag = os.environ.get("TAG") or die("TAG env var is required")
    bullet = os.environ.get("PATCH_BULLET") or die("PATCH_BULLET env var is required")

    if not bullet.lstrip().startswith("- "):
        die(f"PATCH_BULLET must start with '- ' (got: {bullet!r})")

    body = fetch_body(repo, tag)
    new_body = splice(body, bullet)

    out_path = Path("new-release-body.md")
    out_path.write_text(new_body, encoding="utf-8")

    subprocess.run(
        [
            "gh", "release", "edit", tag,
            "--repo", repo,
            "--notes-file", str(out_path),
        ],
        check=True,
    )
    print(f"Appended patch note to {tag}.")


if __name__ == "__main__":
    main()
