# ACP boundary and dispatch disposition

Audit base: `d875fdc` on 2026-10-18. Owner: Val, with implementation in
`src-rust/crates/acp`.

**Disposition: defer the dispatch limit pending an overload contract.** Current
main has useful failure and lifecycle controls, but it does not bound inbound
messages or active handlers. This is an open availability risk for a client
that sends too many requests. The client is the process connected to stdio,
often an editor but potentially an OpenCoven orchestration layer. ACP
`authenticate` is intentionally a no-op because credentials stay in the local
agent process. This audit does not establish a separate client identity or
authorization boundary.

## Evidence on current main

- [`connection.rs`](../src-rust/crates/acp/src/connection.rs) forwards requests
  and notifications through an unbounded channel. It routes responses to
  pending request IDs and releases pending requests when the reader exits.
- [`lib.rs`](../src-rust/crates/acp/src/lib.rs) passes each inbound message to
  `AgentServer::dispatch` and reaps completed join handles. In
  [`server.rs`](../src-rust/crates/acp/src/server.rs), `dispatch` spawns one
  Tokio task per message. Neither the queue nor active task count has a cap.
  The dispatch tests cover cancellation behind one blocked response and EOF
  draining, but do not test sustained overload.
- `session/new` canonicalizes and validates cwd before storing it (#348), and
  `session/prompt` rejects unknown session IDs. The permission bridge requests
  client approval and maps unknown selections, cancellation, and transport
  errors to Deny. The current 50 ms permission drainer spawns forwarding tasks
  without joining them; #343 proposes ownership, and the later accepted ACP
  turn-permission unit remains to be ported.
- The partial-frame writer (#344) and reader-error propagation (#376) are
  separate open PRs. Their passing focused tests do not close the dispatch
  capacity or permission-turn ownership questions.

## Rationale and next gate

A semaphore around every handler could delay `session/cancel` behind a prompt
that is waiting for that cancellation. A limit needs an explicit response for
excess requests while keeping cancellation and response routing live. After the
ACP dependency chain lands, reproduce a sustained request flood with blocked
handlers and a following cancellation; then implement the chosen overload
contract in its own reviewed PR. Recheck EOF draining and permission denial on
the resulting tree. Val also owns a follow-up review of which local processes
may attach to stdio, which methods use stored credentials, and whether the
launcher boundary supplies the required authorization. Native editor and
orchestrator acceptance remain open.
