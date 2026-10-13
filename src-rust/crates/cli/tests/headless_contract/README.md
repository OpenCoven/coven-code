# Runtime headless-contract fixtures

These fixtures describe Coven Code's version 3 headless interface:

- `session-brief.schema.json`
- `result.schema.json`
- `session-brief.example.json`
- `result.example.json`

Run the JSON Schema regression tests from the repository root:

```sh
uv run scripts/test-headless-schemas.py
```

The script pins its validator dependency and tests both golden fixtures,
required version declarations, review evidence, and result exit reasons.
Rust tests in `crates/cli/src/headless.rs` separately exercise runtime parsing,
serialization, and selected schema properties.

The [upstream adapter contract](https://github.com/OpenCoven/coven-github/blob/main/docs/headless-contract.md)
and its `docs/contracts/` fixtures currently use version 2. These runtime
fixtures use version 3 and include `review.memory` provenance. They are not
verbatim copies. The runtime still accepts v2 briefs and echoes that version,
but its extra review provenance needs reconciliation with the strict adapter-v2
schema before claiming cross-repository conformance.

See the [runtime contract](../../../../../docs/headless-contract.md) for the
normative v3 behavior. Coordinate breaking changes and version bumps with
both repositories; do not overwrite these fixtures with an older upstream
schema.
