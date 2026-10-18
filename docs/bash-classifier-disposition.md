# Bash classifier surface disposition

Owner: OpenCoven core and tools maintainers. Reviewed against `origin/main` at
`0f7ae711` on 2026-10-18.

Manifest position 126 (`surface-bash-structured-risk`) is superseded by
[PR #273](https://github.com/OpenCoven/coven-code/pull/273), merged as
`e0a93f52382c7e4faadab1bb844e3b5269f02575`. The historical source adds a
tree-sitter scan over a textual classifier. Current main instead uses a bounded
typed Bash parser in
[`bash_classifier.rs`](../src-rust/crates/core/src/bash_classifier.rs).
It walks executable commands in chains, pipelines, substitutions, assignments,
functions, compound commands, and expandable heredocs. It distinguishes quoted
data from commands, recognizes wrapped shell invocations, and raises uncertain
or over-limit analysis to at least High. Its existing tests cover these cases,
including recursive parser inputs in a bounded child process.

`cargo test --package claurst-core bash_classifier::tests --locked -- --test-threads=1`
passed all 34 classifier tests on that main head. The older
tree-sitter patch would replace the current parser and lose its typed argument
handling, so it must not be ported.

The classifier is advisory. Aliases, runtime variables, sourced files, and
arbitrary interpreters remain outside static analysis; commands beyond its
documented resource limits may remain unanalyzed and receive High rather than
Critical. The core and tools maintainers own those limits and any future
classifier defect. Shell child lifetime and execution policy are separate audit
surfaces and are not closed by this disposition.
