#!/usr/bin/env python3
"""Repair the one malformed Rust newline literal on PR #182, fail closed."""

from __future__ import annotations

import hashlib
from pathlib import Path


PATH = Path("src-rust/crates/tui/src/app.rs")
EXPECTED_BLOB = "b42577066024e6ff19966e4725dce359ccb36acc"


def git_blob_sha(data: bytes) -> str:
    header = f"blob {len(data)}\0".encode("ascii")
    return hashlib.sha1(header + data).hexdigest()


def main() -> None:
    data = PATH.read_bytes()
    actual = git_blob_sha(data)
    if actual != EXPECTED_BLOB:
        raise SystemExit(f"source drift: expected {EXPECTED_BLOB}, found {actual}")

    # Build the byte sequences explicitly so no Python/JSON escape layer can
    # turn the intended Rust `\\n` escape back into a physical newline.
    malformed = b"buffer.push('" + bytes([10]) + b"');"
    corrected = b"buffer.push('" + bytes([92, 110]) + b"');"
    count = data.count(malformed)
    if count != 1:
        raise SystemExit(f"expected one malformed newline literal, found {count}")

    updated = data.replace(malformed, corrected, 1)
    PATH.write_bytes(updated)
    print("corrected generated Rust newline literal")


if __name__ == "__main__":
    main()
