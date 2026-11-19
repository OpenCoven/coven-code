#!/usr/bin/env python3
"""Rewrite an issue's "## Current state: YYYY-MM-DD" section.

usage: replace-state.py <issue> <section-file> [--apply] [--insert]
                        [--repo OWNER/NAME] [--out-dir DIR]

The section file must start with its own "## Current state: YYYY-MM-DD"
heading. The old section runs from its heading to the end of its first
"**Next action:**" paragraph; without one, to the next "#", "##" or "###"
heading or "<details>" line, or to the end of the body. Text after it (the
original report, acceptance, prompts) is kept.

By default it only previews: it writes <issue>.before.md and <issue>.after.md
into --out-dir (default: the current directory) and prints a unified diff.
--apply also writes the new body with `gh api -X PATCH`. --insert puts the
section at the top of a body that has none; without it, a missing section is
an error.

Exit status: 0 on success (or no change), 1 when the body or section file
cannot be handled, 2 on usage or API errors.
"""
import argparse
import difflib
import json
import pathlib
import re
import subprocess
import sys

DEFAULT_REPO = "OpenCoven/coven-code"
HEADING = re.compile(r"^## Current state: \d{4}-\d{2}-\d{2}[ \t]*$", re.M)
NEXT_ACTION = re.compile(r"^\*\*Next action:\*\*", re.M)
STOP = re.compile(r"^(#{1,3} |<details>)", re.M)


class SectionError(Exception):
    pass


def normalize_section(text):
    text = text.replace("\r\n", "\n").strip("\n")
    first = text.split("\n", 1)[0]
    if not HEADING.fullmatch(first):
        raise SectionError(
            "the section file must start with a '## Current state: YYYY-MM-DD' heading"
        )
    if len(HEADING.findall(text)) != 1:
        raise SectionError("the section file holds more than one Current state heading")
    return text + "\n"


def replace_section(body, section, insert=False):
    """Return body with its dated Current state section replaced by section."""
    body = (body or "").replace("\r\n", "\n")
    section = normalize_section(section)
    found = list(HEADING.finditer(body))
    if len(found) > 1:
        raise SectionError(f"the body holds {len(found)} Current state headings")
    if not found:
        if not insert:
            raise SectionError(
                "the body has no '## Current state: YYYY-MM-DD' section; "
                "pass --insert to add one at the top"
            )
        rest = body.lstrip("\n")
        return section + ("\n" + rest if rest else "")
    start = found[0].start()
    after_heading = found[0].end()
    rest = body[after_heading:]
    stop = STOP.search(rest)
    stop_at = stop.start() if stop else len(rest)
    action = NEXT_ACTION.search(rest, 0, stop_at)
    if action:
        paragraph_end = rest.find("\n\n", action.start(), stop_at)
        end = paragraph_end if paragraph_end != -1 else stop_at
    else:
        end = stop_at
    tail = rest[end:].lstrip("\n")
    return body[:start] + section + ("\n" + tail if tail else "")


def gh(args, **kwargs):
    try:
        return subprocess.run(["gh", "api", *args], check=True, text=True, **kwargs)
    except FileNotFoundError as error:
        raise RuntimeError("gh is not installed") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        raise RuntimeError(f"gh api {args[0]} failed: {detail}") from error


def main(argv=None):
    parser = argparse.ArgumentParser(prog="replace-state.py", description=__doc__.splitlines()[0])
    parser.add_argument("issue", type=int)
    parser.add_argument("section_file", type=pathlib.Path)
    parser.add_argument("--apply", action="store_true", help="write the new body to GitHub")
    parser.add_argument("--insert", action="store_true", help="add the section when none exists")
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument("--out-dir", type=pathlib.Path, default=pathlib.Path.cwd())
    args = parser.parse_args(argv)
    endpoint = f"repos/{args.repo}/issues/{args.issue}"
    try:
        section = args.section_file.read_text(encoding="utf-8")
    except OSError as error:
        print(f"replace-state: cannot read {args.section_file}: {error}", file=sys.stderr)
        return 2
    try:
        issue = json.loads(gh([endpoint], capture_output=True).stdout)
    except (RuntimeError, json.JSONDecodeError) as error:
        print(f"replace-state: {error}", file=sys.stderr)
        return 2
    before = (issue.get("body") or "").replace("\r\n", "\n")
    try:
        after = replace_section(before, section, insert=args.insert)
    except SectionError as error:
        print(f"replace-state: #{args.issue}: {error}", file=sys.stderr)
        return 1
    args.out_dir.mkdir(parents=True, exist_ok=True)
    before_file = args.out_dir / f"{args.issue}.before.md"
    after_file = args.out_dir / f"{args.issue}.after.md"
    before_file.write_text(before, encoding="utf-8")
    after_file.write_text(after, encoding="utf-8")
    sys.stdout.writelines(
        difflib.unified_diff(
            before.splitlines(True),
            after.splitlines(True),
            f"#{args.issue} before",
            f"#{args.issue} after",
            n=1,
        )
    )
    if after == before:
        print(f"#{args.issue}: no change")
        return 0
    if not args.apply:
        print(f"#{args.issue}: preview only; wrote {before_file} and {after_file}; rerun with --apply")
        return 0
    try:
        gh(["-X", "PATCH", endpoint, "-F", f"body=@{after_file}", "--jq", ".html_url"])
    except RuntimeError as error:
        print(f"replace-state: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
