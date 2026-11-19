#!/usr/bin/env python3
"""Rewrite an issue's "## Current state: YYYY-MM-DD" section.

usage: replace-state.py <issue> <section-file> [--apply] [--insert] [--to-end]
                        [--repo OWNER/NAME] [--out-dir DIR]

The section file must start with its own "## Current state: YYYY-MM-DD"
heading. The old section starts at its heading and ends:
- after its first "**Next action:**" paragraph, if it has one before the next
  "#" or "##" heading, "<details" line or "---" rule;
- otherwise at the first of those, or at a "###" to "######" heading;
- otherwise, with --to-end only, at the end of the body. Without --to-end
  that is an error, so text with no marker after the section is never lost.
Lines inside fenced code blocks are not markers. Text after the section (the
original report, acceptance, prompts) is kept.

By default it only previews: it prints a unified diff and writes
<issue>.<UTC time>.before.md and .after.md into --out-dir (default: the
current directory), so earlier copies are never overwritten. --apply also
writes the new body with `gh api -X PATCH`, after checking that the issue has
not changed since it was read. --insert puts the section at the top of a body
that has none; without it, a missing section is an error.

Exit status: 0 on success (or no change), 1 when the body or the section's
content cannot be handled, 2 on usage errors, an unreadable section file, an
issue that changed before --apply, or API errors.
"""
import argparse
import datetime
import difflib
import json
import pathlib
import re
import subprocess
import sys

DEFAULT_REPO = "OpenCoven/coven-code"
HEADING = re.compile(r"^## Current state: \d{4}-\d{2}-\d{2}[ \t]*$", re.M)
NEXT_ACTION = re.compile(r"\*\*Next action:\*\*")
HARD_STOP = re.compile(r"#{1,2}\s|<details\b|(-{3,}|\*{3,}|_{3,})\s*$")
SOFT_STOP = re.compile(r"#{3,6}\s")
FENCE = re.compile(r" {0,3}(```|~~~)")


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


def markers(text):
    """Yield (offset, line) for each line of text outside fenced code blocks."""
    offset = 0
    fenced = False
    for line in text.splitlines(True):
        if FENCE.match(line):
            fenced = not fenced
        elif not fenced:
            yield offset, line
        offset += len(line)


def section_end(rest, to_end):
    """Offset in rest (the text after the heading line) where the old section ends."""
    hard = soft = action = None
    for offset, line in markers(rest):
        if HARD_STOP.match(line):
            hard = offset
            break
        if soft is None and SOFT_STOP.match(line):
            soft = offset
        if action is None and NEXT_ACTION.match(line):
            action = offset
    limit = len(rest) if hard is None else hard
    if action is not None:
        paragraph_end = rest.find("\n\n", action, limit)
        return limit if paragraph_end == -1 else paragraph_end
    for stop in (soft, hard):
        if stop is not None:
            return stop
    if to_end:
        return len(rest)
    raise SectionError(
        "the Current state section has no **Next action:** paragraph and no heading, "
        "<details> or --- after it, so its end is unclear; pass --to-end to replace "
        "it through the end of the body"
    )


def replace_section(body, section, insert=False, to_end=False):
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
    tail = rest[section_end(rest, to_end):].lstrip("\n")
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
    parser.add_argument(
        "--to-end",
        action="store_true",
        help="let a section with no Next action and nothing after it run to the end of the body",
    )
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
        after = replace_section(before, section, insert=args.insert, to_end=args.to_end)
    except SectionError as error:
        print(f"replace-state: #{args.issue}: {error}", file=sys.stderr)
        return 1
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    prefix = args.out_dir / f"{args.issue}.{stamp}"
    suffix = 1
    while prefix.with_name(prefix.name + ".before.md").exists():
        suffix += 1
        prefix = args.out_dir / f"{args.issue}.{stamp}-{suffix}"
    before_file = prefix.with_name(prefix.name + ".before.md")
    after_file = prefix.with_name(prefix.name + ".after.md")
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
        current = json.loads(gh([endpoint], capture_output=True).stdout)
        if current.get("updated_at") != issue.get("updated_at"):
            print(
                f"replace-state: #{args.issue} changed while it was being rewritten; rerun",
                file=sys.stderr,
            )
            return 2
        gh(["-X", "PATCH", endpoint, "-F", f"body=@{after_file}", "--jq", ".html_url"])
    except (RuntimeError, json.JSONDecodeError) as error:
        print(f"replace-state: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
