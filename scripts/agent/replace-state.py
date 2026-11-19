#!/usr/bin/env python3
"""Rewrite an issue's "## Current state: YYYY-MM-DD" section.

usage: replace-state.py <issue> <section-file> [--apply] [--insert]
                        [--repo OWNER/NAME] [--out-dir DIR]

It replaces only a section whose end is certain, and refuses anything else,
so it never drops original issue text. The old section must:
- be the body's only "## Current state: YYYY-MM-DD" line, not inside an
  unclosed <!-- comment or <details> block;
- hold, after its heading, only plain lines (paragraphs, lists, tables) up to
  a single-line "**Next action:**" paragraph, followed by a blank line or the
  end of the body. A heading of any kind (including setext and HTML), a rule,
  a code fence, an HTML line or a "-->" before the Next action line is
  refused, as is a Next action paragraph that runs on to a second line.
The old section ends after that Next action line. Everything after it (the
original report, acceptance, prompts) is kept byte for byte.

The section file follows the same rules: its heading, plain lines, and one
single-line "**Next action:**" paragraph last. So every later run finds its
end. A result that a second run would cut differently is refused.

By default it only previews: it prints a unified diff and writes
<issue>.<UTC time>.before.md and .after.md into --out-dir (default: the
current directory, refused when that is inside a git work tree, where the
copies could be committed by accident). Earlier copies are never overwritten.
--apply also writes the new body with `gh api -X PATCH`. It re-reads the body
just before writing and refuses if it changed; GitHub offers no atomic check,
so a concurrent edit in that last second can still be overwritten. --insert
puts the section at the top of a body that has none; without it, a missing
section is an error. CRLF line endings in the body become LF.

Exit status: 0 on success (or no change), 1 when the body or the section's
content cannot be handled, 2 on usage errors, an unreadable section file, a
pull request number, an issue that changed before --apply, or API errors.
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
HEADING = re.compile(r"## Current state: \d{4}-\d{2}-\d{2}[ \t]*")
ANY_STATE_HEADING = re.compile(r"\s{0,3}(#{1,6}\s*|<h[1-6][^>]*>\s*)Current state\b", re.I)
NEXT_ACTION = re.compile(r"\*\*Next action:\*\*")
# Lines that could start or end a block GitHub renders as structure: ATX and
# setext headings, rules, code fences, HTML (headings, <details>, comments).
RISKY = re.compile(r"\s{0,3}(#{1,6}(\s|$)|=+\s*$|-+\s*$|([-*_])(\s*\3){2,}\s*$|```|~~~|<)")
# HTML that hides or reflows what follows it, wherever it appears in a line.
CONTAINERS = ("<!--", "-->", "<details", "</details", "<pre", "</pre", "<textarea",
              "</textarea", "<script", "</script", "<style", "</style")
PAIRS = (("<!--", "-->"), ("<details", "</details"), ("<pre", "</pre"),
         ("<textarea", "</textarea"), ("<script", "</script"), ("<style", "</style"))
FENCE = re.compile(r" {0,3}(`{3,}|~{3,})(.*)$")


class SectionError(Exception):
    pass


def lines_of(text):
    """Split on "\n" only; str.splitlines also splits on form feeds and others."""
    if "\r" in text:
        raise SectionError("a lone carriage return is a line break on GitHub; fix it by hand")
    return text.split("\n")


def risky(line):
    lowered = line.lower()
    return bool(RISKY.match(line)) or any(token in lowered for token in CONTAINERS)


def section_lines(lines, start, what):
    """Index of the Next action line of the section whose heading is lines[start]."""
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if NEXT_ACTION.match(line):
            if any(token in line.lower() for token in CONTAINERS):
                raise SectionError(f"{what}'s **Next action:** line holds HTML; edit it by hand")
            following = lines[index + 1] if index + 1 < len(lines) else ""
            if following.strip():
                raise SectionError(
                    f"{what}'s **Next action:** paragraph runs on to the next line "
                    f"({following.strip()[:60]!r}); edit it by hand"
                )
            return index
        if risky(line):
            raise SectionError(
                f"{what} has {line.strip()[:60]!r} before its **Next action:** line, "
                "so its end is not certain; edit it by hand"
            )
    raise SectionError(f"{what} has no **Next action:** line, so its end is not certain")


def normalize_section(text):
    """Check the new section so that a later run finds exactly where it ends."""
    text = text.replace("\r\n", "\n").lstrip("\ufeff").strip()
    lines = lines_of(text)
    if not HEADING.fullmatch(lines[0]):
        raise SectionError(
            "the section file must start with a '## Current state: YYYY-MM-DD' heading"
        )
    action = section_lines(lines, 0, "the section file")
    if action != len(lines) - 1:
        raise SectionError("the section file must end with its **Next action:** line")
    return text + "\n"


def contained(prefix_lines):
    """Why the line after prefix_lines would not be a top-level heading, or None."""
    fence = None
    for line in prefix_lines:
        match = FENCE.match(line)
        if fence is None:
            if match and not (match.group(1)[0] == "`" and "`" in match.group(2)):
                fence = match.group(1)
        elif match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence) \
                and not match.group(2).strip():
            fence = None
    if fence is not None:
        return "inside a code fence"
    text = "\n".join(prefix_lines).lower()
    for opener, closer in PAIRS:
        position = 0
        while True:
            opened = text.find(opener, position)
            if opened == -1:
                break
            closed = text.find(closer, opened + len(opener))
            if closed == -1:
                return f"inside an unclosed {opener}"
            position = closed + len(closer)
    if prefix_lines and prefix_lines[-1].strip():
        return "not after a blank line"
    return None


def splice(body, section, insert):
    lines = lines_of(body)
    found = [i for i, line in enumerate(lines) if HEADING.fullmatch(line)]
    if len(found) > 1:
        raise SectionError(f"the body holds {len(found)} Current state headings")
    if not found:
        loose = [line.strip() for line in lines if ANY_STATE_HEADING.match(line)]
        if loose:
            raise SectionError(
                f"the body has a Current state heading in another form ({loose[0][:60]!r}); "
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
    reason = contained(lines[:start])
    if reason:
        raise SectionError(f"the Current state heading is {reason}; edit it by hand")
    action = section_lines(lines, start, "the old Current state section")
    head = "\n".join(lines[:start])
    tail = "\n".join(lines[action + 1 :]).lstrip("\n")
    return (head + "\n" if start else "") + section + ("\n" + tail if tail else "")


def replace_section(body, section, insert=False):
    """Return body with its dated Current state section replaced by section."""
    body = (body or "").replace("\r\n", "\n").lstrip("\ufeff")
    section = normalize_section(section)
    out = splice(body, section, insert)
    # The next run must find the same boundary, or it would take original
    # text with it.
    if splice(out, section, False) != out:
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
        section = args.section_file.read_text(encoding="utf-8-sig")
    except (OSError, UnicodeDecodeError) as error:
        print(f"replace-state: cannot read {args.section_file}: {error}", file=sys.stderr)
        return 2
    try:
        issue = json.loads(gh([endpoint], capture_output=True).stdout)
    except (RuntimeError, json.JSONDecodeError) as error:
        print(f"replace-state: {error}", file=sys.stderr)
        return 2
    if "pull_request" in issue:
        print(f"replace-state: #{args.issue} is a pull request, not an issue", file=sys.stderr)
        return 2
    before = (issue.get("body") or "").replace("\r\n", "\n")
    try:
        after = replace_section(before, section, insert=args.insert)
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
