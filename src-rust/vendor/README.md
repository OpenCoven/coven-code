# Patched rmcp transport ownership

`rmcp-1.7.0/` contains the exact 132 files from the crates.io rmcp 1.7.0
archive, except for the cancellation fix below. The published normalized
`Cargo.toml`, original manifest, lockfile, README, and VCS metadata are preserved.
The package is excluded from workspace membership and selected by the workspace
`[patch.crates-io]` entry. Its version and dependency requirements are unchanged.

Archive SHA-256:
`0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`.
Upstream revision: `3529c3675ff64db805bd947ca6ece6090809e43d`, package path
`crates/rmcp` in <https://github.com/modelcontextprotocol/rust-sdk>.
The package omits the repository license file, so `rmcp-1.7.0/LICENSE` is copied
from that exact revision, preserving upstream's licensing transition notice.
The local Cargo installation marker `.cargo-ok` is not included.

The only upstream source changes are in `src/transport/worker.rs`,
`WorkerContext::recv_from_handler` and `WorkerContext::send_to_handler`: select
the existing cancellation token alongside each handler channel wait. On
cancellation, return the existing `WorkerQuitReason::Cancelled` value. This
allows `WorkerTransport::close` to join its worker while initialization waits for
the first protocol message or the initialized notification, including when the
client rejects the initialize response. It also releases a worker blocked sending
to a full handler queue after its owner stops receiving. Normal channel capacity
and backpressure are unchanged. No message, auth, session, retry, or reconnect
protocol is replaced or fabricated.

Coven's separate HTTP client adapter still cancels blocked HTTP I/O. The tests
`plugin_http_withdrawal_before_first_protocol_message_joins_worker` and
`plugin_http_rejected_initialize_joins_registered_owner`, plus the stalled
header/SSE/body fixtures, require actual cleanup completion. The deterministic
`worker_close_cancels_a_pending_handler_send` fixture and loopback
`plugin_http_saturated_receive_joins_worker` require joined closure with a full
handler queue; `plugin_http_drained_receive_joins_worker` preserves the drained
positive control.

Remove this patch and vendor directory when a released upstream version provides
cancellation-aware handler reception and delivery (or an equivalent joinable close) and all
of those regression tests pass against it. A version bump alone is insufficient:
rmcp 3.5.1 retains the affected wait and private sender ownership.
