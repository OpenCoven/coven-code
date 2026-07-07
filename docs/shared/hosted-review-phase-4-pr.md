# Hosted Review Phase 4 PR Notes

## Linked issues

Fixes #105.
Fixes #106.
Fixes #112.

## Summary

- Extends AGENTS.md frontmatter with enforceable hosted metadata: stable id, trust, visibility, source, source_ref, expiry, created_at, created_by, session_id, transcript_ref, and confidence.
- Enforces hosted memory metadata during loading: missing trust is lowest trust, expired memory is ignored, security-private memory is excluded from public hosted review, and trust must meet the configured threshold.
- Renders hosted memory as tagged entries with stable ids, trust, visibility, source, source_ref, and session metadata so review output can cite memory refs.
- Threads session-scoped provenance into hosted auto-extracted memory candidates.
- Adds structured review-output parsing and validation helpers for `memory_refs` on memory-dependent findings.
- Documents the hosted frontmatter contract and citation behavior.

## Test evidence

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p claurst-core --lib claudemd --quiet`
- `cargo test -p claurst-core --lib system_prompt --quiet`
- `cargo test -p claurst-core --lib --quiet`
- `cargo test -p claurst-query session_memory --quiet`
- `cargo test -p claurst-commands structured_review_output --quiet`
- `cargo test --workspace --quiet`

## Risk notes

- The slash `/review` command remains markdown-first; the structured review parser is available for hosted integrations that request JSON review artifacts.
- Hosted memory without trust metadata is intentionally ignored unless policy lowers the trust threshold.
- Provenance stores references and ids, not secret values.
