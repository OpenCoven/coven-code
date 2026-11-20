#!/usr/bin/env python3
"""Rewrite an issue's "## Current state: YYYY-MM-DD" section.

usage: replace-state.py <issue> <section-file> [--apply] [--insert]
                        [--repo OWNER/NAME] [--out-dir DIR]

It replaces only a section whose end it can find, and refuses anything else
rather than guess. One limit remains (below): a section with no marker at all.
The old section must:
- be the body's only "## Current state: YYYY-MM-DD" line, after a blank
  line, with no code fence or HTML anywhere before it;
- hold, after its heading, only plain lines (paragraphs, lists, tables) up to
  a single-line "**Next action:**" paragraph, with a blank line before it and
  a blank line or the end of the body after it. A line that starts a heading
  (ATX or setext), a rule or a code fence, also inside a quote or list item
  ("> ## Original report"), HTML anywhere in a line, "]:"
  anywhere (a link or footnote definition, even in a quote or list), or a
  lone carriage return is refused, as is text right after the section that
  starts indented. So are a footnote reference ("[^") and "$$" anywhere,
  and a near-miss marker such as "**Next Action:**" or "Next action:".
The old section ends after that Next action line. One limit: an old section
with no marker at all, or one too mangled to mention a next action, is taken
to run on to the first exact "**Next action:**" line below it, so read the
preview diff before --apply. Everything after it (the
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
section is an error. CRLF line endings in the body become LF, and a leading
byte-order mark is dropped.

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
HEADING = re.compile(r"## Current state: [0-9]{4}-[0-9]{2}-[0-9]{2}[ \t]*")
ANY_STATE_HEADING = re.compile(r"\s{0,3}(#{1,6}\s*|<h[1-6][^>]*>\s*)Current state\b", re.I)
NEXT_ACTION = re.compile(r"\*\*Next action:\*\*")
# A hand-typed variant ("**Next Action:**", "Next action:") before the exact
# marker would let the section run on into the original text.
# Caught: "**Next Action:**", "**Next actions:**", "**Next action (Val):**",
# "Next action: x", "- next-action: x", and any bold text naming a next
# action. Prose such as "the previous next action is done" is fine.
NEAR_NEXT_ACTION = re.compile(
    r"next[\s_*-]*actions?\W{0,4}:"
    r"|^\s*(?:[-*+>]\s*|\d+[.)]\s*)*[*_]*\s*next[\s_*-]*actions?\b[^:\n]{0,40}:"
    r"|\*\*[^*\n]*next[\s_-]*actions?\b[^*\n]*\*\*",
    re.I,
)
# Lines that could start or end a block GitHub renders as structure: ATX and
# setext headings, rules, code fences, HTML, link and footnote definitions.
RISKY = re.compile(
    r"\s{0,3}(#{1,6}(\s|$)|=+\s*$|-+\s*$|([-*_])(\s*\3){2,}\s*$|```|~~~|<|\[[^\]]+\]:)"
)
# Quote and list markers in front of a line: a heading, rule or fence inside
# a quote or list item is structure too ("> ## Original report").
CONTAINER = re.compile(r"(?:[ \t]*(?:>|[-*+](?=[ \t])|[0-9]{1,9}[.)](?=[ \t]|$)))*")
# HTML anywhere in a line: a tag, comment, declaration or processing instruction.
HTML = re.compile(r"<[A-Za-z!/?]|--!?>")
# A link or footnote definition applies to the whole body, even inside a quote
# or list item, so any "]:" is refused. A footnote reference decides whether
# and in what order footnotes further down render, so "[^" is refused too, and
# "$$" since a math block can span blank lines.
DEFINITION = re.compile(r"\]\s*:|\[\^|\$\$")


class SectionError(Exception):
    pass


def lines_of(text):
    """Split on "\n" only; str.splitlines also splits on form feeds and others."""
    if "\r" in text:
        raise SectionError("a lone carriage return is a line break on GitHub; fix it by hand")
    lines = text.split("\n")
    # Only spaces and tabs make a blank line on GitHub; a line of other
    # whitespace (a no-break space, say) is text, but str.strip() empties it.
    for line in lines:
        if line and not line.strip() and line.strip(" \t"):
            raise SectionError(
                f"a line holds only unusual whitespace ({line!r}), which GitHub does not "
                "treat as blank; edit it by hand"
            )
    return lines


def risky(line):
    inner = line[CONTAINER.match(line).end() :].lstrip(" \t")
    return bool(
        RISKY.match(line) or RISKY.match(inner) or HTML.search(line) or DEFINITION.search(line)
    )


def section_lines(lines, start, what):
    """Index of the Next action line of the section whose heading is lines[start]."""
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if NEXT_ACTION.match(line):
            if HTML.search(line) or DEFINITION.search(line):
                raise SectionError(
                    f"{what}'s **Next action:** line holds HTML or a definition; edit it by hand"
                )
            if lines[index - 1].strip():
                # Without a blank line it continues a list item or paragraph,
                # which can then join the text after the section.
                raise SectionError(
                    f"{what}'s **Next action:** line does not follow a blank line; "
                    "edit it by hand"
                )
            following = lines[index + 1] if index + 1 < len(lines) else ""
            if following.strip():
                raise SectionError(
                    f"{what}'s **Next action:** paragraph runs on to the next line "
                    f"({following.strip()[:60]!r}); edit it by hand"
                )
            return index
        if NEAR_NEXT_ACTION.search(line):
            raise SectionError(
                f"{what} has {line.strip()[:60]!r}, which looks like a Next action line "
                "but is not exactly '**Next action:**' at the start; edit it by hand"
            )
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
    """Why the line after prefix_lines might not be a top-level heading, or None.

    Fences and HTML can wrap a heading in ways that are hard to track (list
    items, comments, nested blocks), so any of them before the heading is
    refused. No real Current state section has either before it.
    """
    for line in prefix_lines:
        if "```" in line or "~~~" in line or "$$" in line:
            return "after a code fence or math block"
        if HTML.search(line):
            return "after HTML"
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
    first_text = next((line for line in tail.split("\n") if line.strip()), "")
    if first_text[:1] in (" ", "\t"):
        raise SectionError(
            "the text after the section starts indented, and could render differently "
            "next to a new section; edit it by hand"
        )
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
    original = issue.get("body") or ""
    before = original.replace("\r\n", "\n")
    try:
        after = replace_section(before, section, insert=args.insert)
    except SectionError as error:
        print(f"replace-state: #{args.issue}: {error}", file=sys.stderr)
        return 1
    try:
        args.out_dir.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        print(f"replace-state: cannot create {args.out_dir}: {error}", file=sys.stderr)
        return 2
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    prefix = args.out_dir / f"{args.issue}.{stamp}"
    suffix = 1
    while prefix.with_name(prefix.name + ".before.md").exists():
        suffix += 1
        prefix = args.out_dir / f"{args.issue}.{stamp}-{suffix}"
    before_file = prefix.with_name(prefix.name + ".before.md")
    after_file = prefix.with_name(prefix.name + ".after.md")
    try:
        # The exact original, CRLF and all, so it can be restored byte for byte.
        before_file.write_text(original, encoding="utf-8", newline="")
        after_file.write_text(after, encoding="utf-8", newline="")
    except (OSError, UnicodeError) as error:
        print(f"replace-state: cannot write the copies: {error}", file=sys.stderr)
        return 2
    sys.stdout.writelines(
        difflib.unified_diff(
            before.splitlines(True),
            after.splitlines(True),
            f"#{args.issue} before",
            f"#{args.issue} after",
            n=1,
        )
    )
    # A dropped byte-order mark alone is not worth an edit.
    if after == before.lstrip("\ufeff"):
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
