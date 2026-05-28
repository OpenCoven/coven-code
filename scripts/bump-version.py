#!/usr/bin/env python3
"""Stamp a new Coven Code version into every canonical source.

Usage: scripts/bump-version.py vMAJOR.MINOR.PATCH

Touches:
  - src-rust/Cargo.toml                 workspace.package.version
  - src-rust/Cargo.lock                 12 coven-code* workspace package entries
  - npm/package.json                    version field
  - README.md                           shields.io badge (text + alt) + Beta callout
  - docs/index.md                       **Version:** line
  - docs/installation.md                "coven-code X.Y.Z" sample output
  - src-rust/crates/acp/registry-template/agent.json
                                        version field + 5 release download URLs

Fails loudly if any expected pattern is missing — that means the file shape
changed and the script needs updating, not silently producing a half-stamped
release.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


def replace(path: Path, pattern: str, repl: str, *, count: int = 0, flags: int = re.MULTILINE) -> None:
    text = path.read_text(encoding="utf-8")
    new, n = re.subn(pattern, repl, text, count=count, flags=flags)
    if n == 0:
        die(f"no matches for {pattern!r} in {path.relative_to(ROOT)}")
    if new != text:
        path.write_text(new, encoding="utf-8")
    print(f"  {path.relative_to(ROOT)}: {n} replacement(s)")


def bump_cargo_lock(version: str) -> None:
    """Rewrite every workspace [[package]] block (those without a `source = ` line)."""
    path = ROOT / "src-rust" / "Cargo.lock"
    text = path.read_text(encoding="utf-8")

    blocks = re.split(r"(?=^\[\[package\]\]$)", text, flags=re.MULTILINE)
    touched = 0
    for i, block in enumerate(blocks):
        if not block.startswith("[[package]]"):
            continue
        if re.search(r"^source = ", block, flags=re.MULTILINE):
            continue  # registry / git dep — leave alone
        name_match = re.search(r'^name = "([^"]+)"', block, flags=re.MULTILINE)
        if not name_match:
            continue
        # Workspace crates are named "claurst" or "claurst-*" (internal crate
        # names are preserved from upstream for merge-friendliness; the compiled
        # binary and user-facing package name are "coven-code").
        crate_name = name_match.group(1)
        if crate_name != "claurst" and not crate_name.startswith("claurst-"):
            continue
        new_block, n = re.subn(
            r'^version = "[^"]+"$',
            f'version = "{version}"',
            block,
            count=1,
            flags=re.MULTILINE,
        )
        if n != 1:
            die(f"Cargo.lock: workspace block for {crate_name} had no version line")
        blocks[i] = new_block
        touched += 1

    if touched == 0:
        die("Cargo.lock: found zero workspace package blocks — file shape changed?")
    path.write_text("".join(blocks), encoding="utf-8")
    print(f"  src-rust/Cargo.lock: {touched} workspace package(s)")


def main() -> None:
    if len(sys.argv) != 2:
        die("usage: bump-version.py vMAJOR.MINOR.PATCH")

    tag = sys.argv[1]
    m = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", tag)
    if not m:
        die(f"invalid tag {tag!r} — expected vMAJOR.MINOR.PATCH")
    version = f"{m.group(1)}.{m.group(2)}.{m.group(3)}"

    print(f"Stamping version {version} ({tag}):")

    # 1. Cargo.toml (workspace.package.version — first `version = "..."` line)
    replace(
        ROOT / "src-rust" / "Cargo.toml",
        r'^version = "\d+\.\d+\.\d+"$',
        f'version = "{version}"',
        count=1,
    )

    # 2. Cargo.lock — every coven-code* workspace package
    bump_cargo_lock(version)

    # 3. npm/package.json
    pkg_path = ROOT / "npm" / "package.json"
    pkg = json.loads(pkg_path.read_text(encoding="utf-8"))
    pkg["version"] = version
    pkg_path.write_text(json.dumps(pkg, indent=2) + "\n", encoding="utf-8")
    print(f"  npm/package.json")

    # 4. README.md — Beta callout + attribution version line
    readme = ROOT / "README.md"
    replace(readme, r"Beta \(v\d+\.\d+\.\d+\)\.", f"Beta (v{version}).", count=1)
    replace(readme, r"Claurst v\d+\.\d+\.\d+", f"Claurst v{version}", count=1)

    # 5. docs/index.md
    replace(
        ROOT / "docs" / "index.md",
        r"\*\*Version:\*\* \d+\.\d+\.\d+",
        f"**Version:** {version}",
        count=1,
    )

    # 6. docs/installation.md — sample output line ("coven-code X.Y.Z")
    replace(
        ROOT / "docs" / "installation.md",
        r"^coven-code \d+\.\d+\.\d+$",
        f"coven-code {version}",
        count=1,
    )

    # 7. ACP registry template — version field + 5 release download URLs
    agent = ROOT / "src-rust" / "crates" / "acp" / "registry-template" / "agent.json"
    text = agent.read_text(encoding="utf-8")
    text, n_v = re.subn(r'"version": "\d+\.\d+\.\d+"', f'"version": "{version}"', text, count=1)
    text, n_u = re.subn(r"/releases/download/v\d+\.\d+\.\d+/", f"/releases/download/v{version}/", text)
    if n_v != 1:
        die("agent.json: version field not found")
    if n_u == 0:
        die("agent.json: no /releases/download/vX.Y.Z/ URLs found")
    agent.write_text(text, encoding="utf-8")
    print(f"  src-rust/crates/acp/registry-template/agent.json: 1 version + {n_u} URL(s)")

    print(f"\nStamped {version}.")


if __name__ == "__main__":
    main()
