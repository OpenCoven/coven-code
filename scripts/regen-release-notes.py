#!/usr/bin/env python3
"""Regenerate every Coven Code GitHub Release body with a consistent
template: categorized changelog from git, install instructions pinned
to the release's version, asset table, verify snippet, and a compare
link to the previous tag.

Run with no args from anywhere in the repo. Writes one body file per
tag to /tmp/release-notes/, then `gh release edit <tag> --notes-file`
for each.

Skips no tags; for the very first release (v0.0.1) the changelog is
"all commits up to this tag" since there's no previous tag.

The "0.0.12" lightweight tag (no `v` prefix) is a duplicate of
v0.0.12 from an earlier release flow and gets a short pointer note
rather than a synthetic changelog.
"""

from __future__ import annotations
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

# Repo root — script lives in /tmp, repo path is fixed here for clarity.
REPO = Path(__file__).resolve().parent.parent
OUT = Path("/tmp/release-notes")
OUT.mkdir(parents=True, exist_ok=True)

# Tags to (re)generate, in chronological release order. The "0.0.12"
# lightweight tag is the duplicate.
TAGS = [
    "v0.0.1",
    "v0.0.5",
    "v0.0.6",
    "v0.0.7",
    "v0.0.8",
    "v0.0.9",
    "v0.0.10",
    "v0.0.11",
    "0.0.12",   # duplicate of v0.0.12 from earlier flow
    "v0.0.12",
    "v0.0.13",
    "v0.0.14",
    "v0.0.15",
    "v0.0.16",
    "v0.0.17",
]

# Predecessor map for compare links. None = no predecessor.
PREV = {
    "v0.0.1":  None,
    "v0.0.5":  "v0.0.1",
    "v0.0.6":  "v0.0.5",
    "v0.0.7":  "v0.0.6",
    "v0.0.8":  "v0.0.7",
    "v0.0.9":  "v0.0.8",
    "v0.0.10": "v0.0.9",
    "v0.0.11": "v0.0.10",
    "0.0.12":  "v0.0.11",
    "v0.0.12": "v0.0.11",
    "v0.0.13": "v0.0.12",
    "v0.0.14": "v0.0.13",
    "v0.0.15": "v0.0.14",
    "v0.0.16": "v0.0.15",
    "v0.0.17": "v0.0.16",
}

# Tagline per release. Curated so the release page leads with the
# actual headline of each version.
TAGLINES = {
    "v0.0.1":  "First OpenCoven release. The forked-from-Claurst baseline with the OpenCoven binary name, env vars, and data dirs.",
    "v0.0.5":  "OpenCoven branding, npm package, signed releases, and the first working `curl ... install.sh | bash` path.",
    "v0.0.6":  "Republish of v0.0.5 to recover from an npm sigstore tlog collision. Same binaries, same code.",
    "v0.0.7":  "Cat-shaped Kitty mascot, violet brand palette, and renamed `coven-code` defaults.",
    "v0.0.8":  "`/familiar` slash command, familiar auto-detection, and the F2 switcher popup.",
    "v0.0.9":  "Daemon integration foundation: walking familiar animation, glyph alignment, and the first end-to-end `~/.coven/coven.sock` read.",
    "v0.0.10": "Coven familiars as dynamic agents. `~/.coven/familiars.toml` is now read at session start; each familiar gets an `AgentDefinition`.",
    "v0.0.11": "Redesigned all seven familiar glyphs to match the reference art (kitty, nova, cody, charm, sage, astra, echo).",
    "0.0.12":  "Lightweight tag from an earlier release flow. The maintained release is v0.0.12 with the `v` prefix.",
    "v0.0.12": "Agents-viewer selection + familiar-card image assets. First release with per-familiar PNG/Sixel art support.",
    "v0.0.13": "First-party Anthropic OAuth, ACP permission-context fix, MCP OAuth tokens bound to server URLs, hardened CI release flow, explicit RGB TUI background.",
    "v0.0.14": "Production-quality push: typed daemon errors, structured `/coven` surface, atomic AuthStore, Codex hardening, deuteranopia diff palette, welcome status block, doctor Coven Substrate report.",
    "v0.0.15": "Distribution-channel sync release. Same code as v0.0.14 — only published because npm@0.0.14 was already taken by an older build.",
    "v0.0.16": "Fixes #49: plain `v` no longer hijacks typing in the TUI. Voice push-to-talk is Alt+V only.",
    "v0.0.17": "Fixes #50: welcome screen no longer flickers `Daemon: offline` under daemon load. New `DaemonReachability` enum and `check_reachability(timeout)` API.",
}

CATEGORY_HEADINGS = [
    ("feat",   "✨ Features"),
    ("fix",    "🐛 Fixes"),
    ("perf",   "⚡ Performance"),
    ("refactor", "♻️ Refactors"),
    ("test",   "🧪 Tests"),
    ("docs",   "📚 Docs"),
    ("chore",  "🧹 Chore"),
    ("ci",     "🤖 CI"),
    ("style",  "🎨 Style"),
    ("build",  "🛠 Build"),
    ("other",  "📦 Other"),
]

PREFIX_RE = re.compile(r"^(?P<kind>[a-z]+)(?:\([^)]+\))?!?:\s*(?P<rest>.*)")


def run(cmd: list[str]) -> str:
    return subprocess.check_output(cmd, cwd=REPO, text=True).strip()


def commits_between(prev: str | None, tag: str) -> list[tuple[str, str]]:
    """Return (sha, subject) pairs for commits in (prev, tag] order
    newest-first, excluding merge commits."""
    if prev is None:
        rev = tag
    else:
        rev = f"{prev}..{tag}"
    out = subprocess.run(
        ["git", "log", "--no-merges", "--pretty=%h\t%s", rev],
        cwd=REPO, text=True, capture_output=True,
    )
    if out.returncode != 0:
        return []
    rows: list[tuple[str, str]] = []
    for line in out.stdout.splitlines():
        if "\t" in line:
            sha, sub = line.split("\t", 1)
            rows.append((sha.strip(), sub.strip()))
    return rows


def categorize(sub: str) -> str:
    m = PREFIX_RE.match(sub)
    if not m:
        return "other"
    kind = m.group("kind")
    return kind if kind in {k for k, _ in CATEGORY_HEADINGS} else "other"


def release_date(tag: str) -> str:
    try:
        return run(["git", "log", "-1", "--format=%ad", "--date=short", tag])
    except subprocess.CalledProcessError:
        return ""


def install_section(version_with_v: str) -> str:
    bare = version_with_v.lstrip("v")
    return f"""## Install

**One-line (Linux/macOS):**
```bash
curl -fsSL https://github.com/OpenCoven/coven-code/releases/download/{version_with_v}/install.sh | bash
```

**One-line (Windows PowerShell):**
```powershell
irm https://github.com/OpenCoven/coven-code/releases/download/{version_with_v}/install.ps1 | iex
```

**npm:**
```bash
npm install -g @opencoven/coven-code@{bare}
```

**Already installed?**
```bash
coven-code upgrade --version {bare}
```

**Verify:**
```bash
coven-code --version    # → coven-code {bare}
```
"""


ASSETS = """## What's in the box

| Platform | Archive |
|---|---|
| macOS · Apple Silicon | `coven-code-macos-aarch64.tar.gz` |
| macOS · Intel | `coven-code-macos-x86_64.tar.gz` |
| Linux · x86_64 | `coven-code-linux-x86_64.tar.gz` |
| Linux · aarch64 | `coven-code-linux-aarch64.tar.gz` |
| Windows · x86_64 | `coven-code-windows-x86_64.zip` |

Each archive contains a single `coven-code` (or `coven-code.exe`) binary. Plus `install.sh` (Linux/macOS) and `install.ps1` (Windows).
"""


def build_body(tag: str) -> str:
    prev = PREV[tag]
    date = release_date(tag)
    tagline = TAGLINES.get(tag, "")
    # Special case for the lightweight 0.0.12 duplicate.
    if tag == "0.0.12":
        body = [
            f"# Coven Code {tag}",
            "",
            f"*Tagged {date}*" if date else "",
            "",
            tagline,
            "",
            "Use [`v0.0.12`](https://github.com/OpenCoven/coven-code/releases/tag/v0.0.12) — that's the canonical tag with the proper `v` prefix and a maintained release page.",
            "",
        ]
        return "\n".join(s for s in body if s is not None)

    commits = commits_between(prev, tag)
    # For the initial release the changelog is "everything OpenCoven
    # changed against upstream" which is 200+ commits — useless as a
    # dump. Cap at the most recent 25 and add a "+N upstream" note.
    truncated_count = 0
    if tag == "v0.0.1" and len(commits) > 25:
        truncated_count = len(commits) - 25
        commits = commits[:25]
    groups: dict[str, list[str]] = defaultdict(list)
    for sha, sub in commits:
        kind = categorize(sub)
        commit_url = f"https://github.com/OpenCoven/coven-code/commit/{sha}"
        groups[kind].append(f"- {sub} ([`{sha}`]({commit_url}))")

    lines: list[str] = []
    lines.append(f"# Coven Code {tag}")
    lines.append("")
    if date:
        lines.append(f"*Released {date}*")
        lines.append("")
    if tagline:
        lines.append(tagline)
        lines.append("")

    # Changelog
    lines.append("## Changelog")
    lines.append("")
    if not commits:
        lines.append("_No commits found in this range — see the compare link below._")
        lines.append("")
    else:
        for kind, heading in CATEGORY_HEADINGS:
            items = groups.get(kind, [])
            if not items:
                continue
            lines.append(f"### {heading}")
            lines.append("")
            lines.extend(items)
            lines.append("")

    if truncated_count > 0:
        lines.append(
            f"…plus **{truncated_count} earlier commits** from the OpenCoven rebrand of upstream Claurst. The full history is in the compare link at the bottom of this page."
        )
        lines.append("")

    # Install section
    lines.append(install_section(tag))
    # Assets section
    lines.append(ASSETS)

    # Compare link
    if prev is not None:
        lines.append(
            f"**Full changelog:** https://github.com/OpenCoven/coven-code/compare/{prev}...{tag}"
        )
        lines.append("")
    return "\n".join(lines)


def main() -> int:
    only = sys.argv[1] if len(sys.argv) > 1 else None
    summary = []
    for tag in TAGS:
        if only and tag != only:
            continue
        body = build_body(tag)
        out_path = OUT / f"{tag}.md"
        out_path.write_text(body, encoding="utf-8")
        commits = len(commits_between(PREV[tag], tag)) if PREV[tag] is not None or tag == "v0.0.1" else 0
        summary.append((tag, len(body), commits, out_path))
    print(f"Wrote {len(summary)} release-note bodies to {OUT}:")
    for tag, n, c, p in summary:
        print(f"  {tag:<10}  {c:>4} commits  {n:>6} chars  {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
