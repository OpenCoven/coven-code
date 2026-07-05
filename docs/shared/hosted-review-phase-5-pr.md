# Hosted Review Phase 5 PR Notes

## Linked issues

Fixes #102.
Fixes #103.
Fixes #111.
Fixes #109.

## Summary

- Enforces high-confidence secret scanning before hosted session-memory persistence or candidate creation, settings sync upload, team-memory upload, and team-memory pull apply.
- Blocks secret-bearing memory by default and records only scanner labels/reason codes, never matched secret values.
- Adds structured hosted team-memory sync scope with tenant id, installation id, repo id, repo full name, and domain metadata.
- Replaces server-wins team-memory pull application with conflict-aware handling that preserves local changes and writes conflict records for both-changed cases.
- Adds lifecycle metadata support for `retention_class`, `redacted_at`, and `deleted_at`.
- Excludes deleted hosted memory, redacts hosted prompt content for redacted entries, and adds helpers for deleting a hosted memory scope or redacting a memory file.
- Documents retention, redaction, secret scanning, and conflict workflows.

## Test evidence

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p claurst-core --lib claudemd --quiet`
- `cargo test -p claurst-core --lib team_memory_sync --quiet`
- `cargo test -p claurst-core --lib settings_sync --quiet`
- `cargo test -p claurst-core --lib memdir --quiet`
- `cargo test -p claurst-query session_memory --quiet`
- `cargo test -p claurst-core --lib --quiet`
- `cargo test --workspace --quiet`

## Risk notes

- Hosted server authorization still must be enforced server-side; the client now sends structured scope metadata but does not treat client-side path construction as an authorization boundary.
- Conflict records preserve local and remote memory text for operator review, so downstream tooling should apply the same access controls as memory storage.
- Secret scanning is intentionally high-confidence and blocks by default; false positives require operator edit/redaction before retry.
