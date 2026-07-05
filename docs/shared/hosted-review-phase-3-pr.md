# Hosted Review Phase 3 PR Notes

## Linked issues

Fixes #101.
Fixes #107.
Fixes #108.

## Summary

- Adds hosted memory source trust classification with configurable `memorySourceTrust`, `memoryTrustThreshold`, and `allowAutoMemoryPersistence`.
- Routes hosted session memory extraction through a policy gate instead of always appending to durable `.coven-code/AGENTS.md`.
- Writes untrusted or unapproved hosted extractions as reviewable JSON candidates under `.coven-code/memory-candidates/`.
- Adds candidate approval and rejection APIs; approval promotes candidates to durable memory as maintainer-approved entries, while rejection records a reason without durable writes.
- Preserves local mode direct memory persistence.

## Test evidence

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p claurst-core --lib hosted --quiet`
- `cargo test -p claurst-query session_memory --quiet`
- `cargo test --workspace --quiet`

## Risk notes

- Candidate approval/rejection is exposed as Rust API surface in this phase; hosted dashboard or CLI wiring can call it in a later integration PR.
- Hosted direct durable writes remain disabled by default and require both explicit policy and sufficient source trust.
- Candidate artifacts are not loaded into prompts by the existing memory loader, so rejected or pending candidates do not affect future sessions.
