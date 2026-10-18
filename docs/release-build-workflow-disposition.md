# Shared release build workflow disposition

Owner: Val, OpenCoven release maintainer. Unit 141 was reviewed against
`origin/main` at `f38210d` on 2026-10-18. Decision: retain it as a worthwhile
refactor after the `v0.9.0` release. It is not a release prerequisite.

The [`release.yml`](../.github/workflows/release.yml) and
[`patch-release.yml`](../.github/workflows/patch-release.yml) build jobs each
define the same five targets, Rust toolchain setup, cache, native and cross
build commands, binary staging, and artifact upload. Keeping two copies makes
future target or archive changes easy to apply to only one caller.

The jobs are not behaviorally identical. The regular release build sets
`persist-credentials: false` on checkout; the patch build uses checkout's
default. The patch build installs `libssl-dev` for native Linux and its cross
image, while the regular build installs ALSA and pkg-config without OpenSSL.
These differences must be explicit caller inputs in any reusable workflow.
The existing publication lock, preflight dependency, `build` job identity,
downstream artifact handling, permissions, and checksum publication order must
remain intact. A called workflow must receive `CARGO_TERM_COLOR` explicitly.

Before replacing either job, verify both profiles against the current jobs,
including all five matrix entries, checkout credentials, native packages,
cross image setup, archive names, and artifacts. Run the workflow contract
tests and both profiles through a nonpublishing hosted build before landing.
