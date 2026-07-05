# Hosted Review Phase 2 PR Notes

## Linked issues

Fixes #98.
Fixes #99.
Fixes #104.
Fixes #110.

## Summary

- Expands hosted review scope to include tenant id, GitHub App installation id, stable repo id, repo full name, canonical repo identity, and memory domain.
- Moves hosted memory and transcript paths from local-path identity to tenant/installation/repo/domain namespaces.
- Adds canonical GitHub repo identity parsing for HTTPS and SSH remotes plus deterministic local project ids.
- Adds hosted-derived settings sync project keys and hosted team-memory repo keys so hosted callers do not pass arbitrary project ids.
- Splits hosted memory domains for default branch, branch, release, pull request, and security-private review contexts.
- Hardens Windows test isolation for home-derived paths and gates Unix-socket daemon tests to Unix.

## Test evidence

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p claurst-core --lib hosted -- --nocapture`
- `cargo test -p claurst-core --lib local_project_id -- --nocapture`
- `cargo test -p claurst-core --lib sync_keys -- --nocapture`
- `cargo test -p claurst-core --lib team_memory_key -- --nocapture`
- `cargo test -p claurst-commands --lib named_commands::tests::test_agents_reset_removes_saved_roster_state -- --nocapture`
- `cargo test -p claurst-core --lib roster_reset -- --nocapture`
- `cargo test -p claurst-core --lib build_import_preview_maps_settings_and_doc -- --nocapture`
- `cargo test -p claurst-core --lib test_imported_anthropic_cli_token_resolves_without_coven_oauth_client -- --nocapture`

## Full-suite status

- `cargo test --workspace` progressed through CLI, ACP, API, bridge, buddy, commands, and core tests, then Smart App Control blocked an unsigned freshly built test binary.
- Latest block: `C:\dev-cargo-target\coven-code\debug\deps\claurst_tools-f5b8d284b5de1d1b.exe`, OS error 4551.
- Code Integrity event: Smart App Control reported the binary did not meet Enterprise signing level requirements under policy `{0283ac0f-fff1-49ae-ada1-8a933130cad6}`.

## Risk notes

- Local mode pathing remains available through existing local APIs.
- Hosted durable state now requires explicit scope to avoid path-derived cross-tenant collisions.
- Security-private domains are represented and are excluded from public review loading unless explicitly allowed by policy.
