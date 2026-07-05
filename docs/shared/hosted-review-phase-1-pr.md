# Hosted Review Phase 1 PR Notes

## Linked issues

Fixes #97.
Fixes #100.

## Summary

- Adds trusted hosted-policy switches under `hostedReview` for user memory, managed rules, write tools, MCP servers, and plugins.
- Wires prompt memory loading through the effective hosted policy so hosted review excludes global user memory and managed rules by default.
- Keeps hosted review on read-only built-in tools by default and skips configured MCP/plugin loading unless explicitly allowed.
- Documents the local-personal versus hosted-review defaults and trusted opt-ins.

## Test evidence

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p claurst-core --lib hosted_review -- --nocapture`
- `cargo test -p claurst --bin coven-code hosted_review -- --nocapture`
- `cargo test --workspace` progressed through unit tests, then failed in `claurst --test acp_smoke` because Windows Application Control blocked spawning the freshly built `coven-code` test binary with OS error 4551.

## Risk notes

- Local mode remains the default and keeps existing user/managed memory behavior.
- Hosted deployments that need shared rules, write tools, MCP, or plugins must opt in explicitly through trusted hosted policy settings.
