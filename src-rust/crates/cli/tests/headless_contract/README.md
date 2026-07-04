# Vendored headless-contract fixtures

These four files are **verbatim copies** of the locked headless execution
contract artifacts owned by the `coven-github` repository:

- `session-brief.schema.json`
- `result.schema.json`
- `session-brief.example.json`
- `result.example.json`

Canonical source (single source of truth):
`OpenCoven/coven-github` → `docs/headless-contract.md` + `docs/contracts/`.

**Contract version: `1`.**

`coven-code` is the *consumer* of the session brief and the *producer* of the
result envelope. The conformance tests in `crates/cli/src/headless.rs` round-trip
these golden fixtures through the runtime types and assert that:

- every brief that validates against `session-brief.schema.json` is accepted,
- every `result.json` the runtime emits validates against `result.schema.json`.

If a test here fails, the runtime drifted from the contract **or** the contract
changed. Do not "bless" the fixtures — re-copy them from `coven-github` only as
part of a deliberate, version-bumped contract change (see the contract doc's
Versioning section). A breaking change MUST bump `contract_version` on both
sides.
