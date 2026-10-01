# PROVENANCE.md — Coven Code Origin Record

> This document records where the code in this repository came from. It is a public record, not a legal claim.

---

## Origin

**Coven Code** is a derivative of [Claurst](https://github.com/Kuberwastaken/claurst) by Kuber Mehta (`@Kuberwastaken`), used under the GNU General Public License v3.0. See [`ATTRIBUTION.md`](ATTRIBUTION.md) for the full list of changes from upstream.

- Upstream: https://github.com/Kuberwastaken/claurst (GPL-3.0)
- Fork point: upstream commit `d85747b` (2026-05-27), workspace version 0.1.4, 24 commits after the upstream `v0.1.4` tag
- First OpenCoven commit: `ced6d9c` (2026-05-27), "rebrand: OpenCoven/coven-codes fork of Claurst (GPL-3.0)"
- The upstream commit history, including the work of Kuber Mehta and other Claurst contributors, is preserved in this repository's git history.
- Internal Rust crate names (`claurst`, `claurst-*`) are intentionally kept from upstream. See [`ATTRIBUTION.md`](ATTRIBUTION.md).

GitHub does not list a fork parent for this repository because it was created as a standalone repository. It is nevertheless a derivative work of Claurst, not an original work.

---

## License

GPL-3.0. The full license text is in [`LICENSE.md`](LICENSE.md). Upstream copyright notices are preserved, and OpenCoven's changes are distributed under the same license.

---

## What OpenCoven Added Here

The following were added by OpenCoven after the fork point and can be checked with `git log d85747b..HEAD`:

- **Rebrand to Coven Code**: binary name, npm package `@opencoven/coven-code`, data directories, environment variable prefix, mascot, and docs. Itemized in [`ATTRIBUTION.md`](ATTRIBUTION.md).
- **Coven daemon integration**: IPC client for `~/.coven/coven.sock` (`src-rust/crates/core/src/coven_daemon.rs`), session ledger registration (`src-rust/crates/core/src/coven_ledger.rs`), the `/handoff` command (`src-rust/crates/tui/src/handoff.rs`), and daemon status display (`src-rust/crates/tui/src/coven_status.rs`).
- **Coven runtime stream-json mode**: `src-rust/crates/cli/src/stream_mode.rs` and the runtime manifest `spec/runtime-manifest/coven-code.json`, documented in [`docs/coven-runtimes.md`](docs/coven-runtimes.md).
- **Headless execution contract** for `coven-github`: `src-rust/crates/cli/src/headless.rs`, documented in [`docs/headless-contract.md`](docs/headless-contract.md).
- **Claude CLI transport**, which runs Claude turns through the local `claude` binary: `src-rust/crates/api/src/providers/claude_cli.rs`.
- **Familiar presentation in the TUI**: `src-rust/crates/tui/src/familiar_card.rs`, `familiar_theme.rs` and `familiar_image.rs`, documented in [`docs/familiars.md`](docs/familiars.md).
- **Hosted review mode**: `src-rust/crates/core/src/hosted_review.rs`.
- **Engine contract** between Coven Code and the Coven CLI: [`COVEN.md`](COVEN.md).

OpenCoven has also made bug fixes and smaller features to code inherited from Claurst. The git history is the authoritative record of those changes.

---

## Org-Level Concepts

The familiar model, the agent spawn harness and the other OpenCoven architectural concepts originated in `OpenCoven/coven`, not in this repository. They are recorded at https://github.com/OpenCoven/coven/blob/main/PROVENANCE.md.

---

## Maintainer

Maintained by OpenCoven. Maintainer: **Valentina Alexander** ([@BunsDev](https://github.com/BunsDev)).

---

## Contributing to the Record

If you find an inaccuracy in this record, please open an issue. We maintain this document honestly — if something was not original to us, we want to know and update the record accordingly.

---

*Last updated: 2026-10-01*
*This document does not constitute legal advice.*
