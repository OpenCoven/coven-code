#!/usr/bin/env python3
"""Rewrite an issue's "## Current state: YYYY-MM-DD" section.

usage: replace-state.py <issue> <section-file> [--apply] [--insert] [--to-end]
                        [--repo OWNER/NAME] [--out-dir DIR]

The section file must start with its own "## Current state: YYYY-MM-DD"
heading and end with one paragraph that starts with "**Next action:**"; it
may hold "###" or deeper headings but no "#"/"##" heading, "<details" or
"---" line. That keeps its end findable, so every later run replaces exactly
it. A result that a second run would cut differently is refused.

The old section starts at its heading and ends:
- after its first "**Next action:**" paragraph, if it has one before the next
  "#" or "##" heading, "<details" line or "---" rule;
- otherwise at the first of those, or at a "###" to "######" heading;
- otherwise, with --to-end only, at the end of the body, taking any plain
  text after it too. Without --to-end that is an error, so unmarked text
  after an old section is never lost by accident.
Lines inside fenced code blocks are not markers. Text after the section (the
original report, acceptance, prompts) is kept.

By default it only previews: it prints a unified diff and writes
<issue>.<UTC time>.before.md and .after.md into --out-dir (default: the
current directory, refused when that is inside a git work tree, where the
copies could be committed by accident). Earlier copies are never overwritten.
--apply also writes the new body with `gh api -X PATCH`, after checking that
the body has not changed since it was read. --insert puts the section at the top of a body
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
ANY_STATE_HEADING = re.compile(r"#{1,6}\s*Current state\b", re.I)


class SectionError(Exception):
    pass


def normalize_section(text):
    """Check the new section so that a later run finds exactly where it ends."""
    text = text.replace("\r\n", "\n").strip("\n")
    first, _, body = text.partition("\n")
    if not HEADING.fullmatch(first):
        raise SectionError(
            "the section file must start with a '## Current state: YYYY-MM-DD' heading"
        )
    actions = []
    for offset, line in markers(body):
        if HARD_STOP.match(line) or ANY_STATE_HEADING.match(line):
            raise SectionError(
                "the section file may hold only ### or deeper headings, and no <details> "
                f"or --- line: {line.strip()!r}"
            )
        if NEXT_ACTION.match(line):
            actions.append(offset)
    if len(actions) != 1 or "\n\n" in body[actions[0]:]:
        raise SectionError(
            "the section file must end with exactly one paragraph that starts with "
            "**Next action:**, so that a later run knows where the section ends"
        )
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


def splice(body, section, insert, to_end):
    found = [offset for offset, line in markers(body) if HEADING.fullmatch(line.rstrip("\n"))]
    if len(found) > 1:
        raise SectionError(f"the body holds {len(found)} Current state headings")
    if not found:
        loose = [line.strip() for _, line in markers(body) if ANY_STATE_HEADING.match(line)]
        if loose:
            raise SectionError(
                f"the body has a Current state heading in another form ({loose[0]!r}); "
                "fix it by hand to '## Current state: YYYY-MM-DD' first"
            )
        if not insert:
            raise SectionError(
                "the body has no '## Current state: YYYY-MM-DD' section; "
                "pass --insert to add one at the top"
            )
        rest = body.lstrip("\n")
        return section + ("\n" + rest if rest else "")
    start = found[0]
    after_heading = body.index("\n", start) + 1 if "\n" in body[start:] else len(body)
    rest = body[after_heading:]
    tail = rest[section_end(rest, to_end):].lstrip("\n")
    return body[:start] + section + ("\n" + tail if tail else "")


def replace_section(body, section, insert=False, to_end=False):
    """Return body with its dated Current state section replaced by section."""
    body = (body or "").replace("\r\n", "\n")
    section = normalize_section(section)
    out = splice(body, section, insert, to_end)
    # The next run must find the same boundary, or it would take original
    # text with it.
    if splice(out, section, False, False) != out:
        raise SectionError("the result would not survive a second rewrite; refusing")
    return out


def gh(args, **kwargs):
    try:
        return subprocess.run(["gh", "api", *args], check=True, text=True, **kwargs)
    except FileNotFoundError as error:
        raise RuntimeError("gh is not installed") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        target = next((a for a in args if a.startswith("repos/")), args[0])
        raise RuntimeError(f"gh api {target} failed: {detail}") from error


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
    parser.add_argument("--out-dir", type=pathlib.Path, help="where to keep the copies")
    args = parser.parse_args(argv)
    if args.out_dir is None:
        try:
            inside = subprocess.run(
                ["git", "rev-parse", "--is-inside-work-tree"], capture_output=True, text=True
            ).stdout.strip()
        except FileNotFoundError:
            inside = ""
        if inside == "true":
            print(
                "replace-state: the current directory is inside a git work tree; "
                "pass --out-dir for the before/after copies",
                file=sys.stderr,
            )
            return 2
        args.out_dir = pathlib.Path.cwd()
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
        if (current.get("body") or "").replace("\r\n", "\n") != before:
            print(
                f"replace-state: #{args.issue} changed while it was being rewritten; rerun",
                file=sys.stderr,
            )
            return 2
        done = gh(
            ["-X", "PATCH", endpoint, "-F", f"body=@{after_file}", "--jq", ".html_url"],
            capture_output=True,
        )
        print(f"#{args.issue}: updated {done.stdout.strip()}")
    except (RuntimeError, json.JSONDecodeError) as error:
        print(f"replace-state: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
