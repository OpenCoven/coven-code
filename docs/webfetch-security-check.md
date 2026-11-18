# Agent WebFetch security check

Run from the repository root on the candidate checkout:

```sh
python3 scripts/check-webfetch-security.py --output /tmp/webfetch-candidate-unique
```

Requires Python 3.10+, Git, Rust 1.100.0, and the project's native build dependencies.
Use a fresh output directory outside the checkout. Prepare dependencies with
`cargo fetch --locked` from `src-rust` before running; qualification uses Cargo
`--offline --locked`. The runner currently requires a POSIX host for bounded
process-group termination. Windows native qualification remains separate.

On hosts without the optional microphone/ALSA build dependencies, explicitly add
`--no-default-features`. The receipt records this configuration and each command;
a pass in that mode does not establish that the default voice-enabled build passes.

The runner returns nonzero for missing prerequisites, compilation failure,
timeout, ignored tests, zero matching tests, any failed test, or a source change
during execution. It stops on the first failing group. Unexecuted groups are not
passes. Preserve receipt.json and all logs, including failures. Receipts contain
HEAD, dirty status, every tracked/unignored file digest, command, environment
platform/toolchain, exit status and log hashes. A dirty-tree receipt is identified
by its source digest, never represented as evidence for pristine HEAD.

Coverage: authority/deny precedence, forbidden destination zero-contact assertions,
DNS and redirects, body limits, fresh-body cache validation, UTF-8 boundaries,
producer envelope validation, query budgeting/history, Research parsing/rendering,
and WebFetch typing → disk save/load → Research in both renderer modes.

The last check uses a synthetic producer envelope and real key handling, rewind
adoption, history APIs and Research projection. The backend checks separately
exercise local HTTP fixtures. This is compositional source evidence, **not** a
single installed-binary HTTP-to-Research journey. No production private-network
bypass is added. The test environment has isolated homes and no inherited provider
credentials, proxy or endpoint settings; this is not an OS network sandbox.

For failing-first proof, run the same regression against the original behavior in
an isolated checkout, retain its failing receipt, then run the corrected candidate.
Do not weaken assertions, broaden permissions, or retry a failure without retaining
its original output. Run the repository's format/check/Clippy/workspace gates too.
Never merge or publish solely because this focused check passes.

Still separate: published-package acceptance; actual fresh semantic extraction
success/fallback; compressed-response qualification if decoding is enabled; and
native platform/human acceptance. The current reqwest workspace dependency does
not enable automatic compression decoding. Do not claim a compressed-body test
from the ordinary streaming-size test.

Runner self-tests:

```sh
python3 -B -m unittest discover -s scripts -p test_webfetch_security_runner.py
```
