# Reverse engine contract CI

The `Coven engine contract` job builds Coven Code and the consumer's `coven-cli`
test binary, then runs exactly these engine-facing consumer tests:

- `tests::contract_version_output_parses`
- `tests::contract_auth_status_json_is_machine_readable`
- `tests::contract_print_flags_are_accepted`

The job is blocking. The runner resolves the test executable from Cargo's JSON
artifact output, requires an executable engine path, verifies the exact test
list, and requires one executed pass per test. Missing tests, skipped execution,
ignored tests, empty success, nonzero exit and timeout all fail. Each invocation
has a 90-second deadline; timeout kills its owned process group, including engine
children. The runner targets POSIX CI hosts (the workflow uses Ubuntu).

Run locally from the engine repository, with a consumer checkout at `coven-main`:

```sh
(cd src-rust && cargo build -p claurst --locked)
(cd coven-main && cargo test -p coven-cli --bin coven --locked --no-run --message-format=json) > /tmp/coven-test-artifacts.jsonl
python3 scripts/test-coven-contract-runner.py
python3 scripts/run-coven-contract.py \
  --engine "$PWD/src-rust/target/debug/coven-code" \
  --cargo-artifacts /tmp/coven-test-artifacts.jsonl
```

The workflow checks out the consumer's default branch, so consumer-side contract
changes are detected. Record its exact SHA when assessing release readiness.
Builds may download dependencies. Runtime tests use disposable engine/Coven state,
an allowlisted environment without inherited credentials, an absent daemon socket,
blocked HTTP proxies, and a system PATH checked for absent provider CLIs. This is
controlled offline testing, not an OS network sandbox.

## Coverage limits

These consumer tests check version parsing, machine-readable authentication
status, and print-flag acceptance. The upstream print test only rejects exit 2;
it does not establish successful inference, valid terminal envelopes, or stream
semantics. The engine's `engine_contract` fixtures and headless schema checks
remain required. Hosted adapter v3 conformance, managed updater compatibility,
native platform checks, and human terminal acceptance are separate release gates.

The adversarial runner tests emulate libtest and exercise the guard's failure
paths. They do not substitute for compiling and running the actual consumer.
