# ACP boundary and dispatch disposition

Audit base: `31e1511` on 2026-11-14 (v0.9.1 plus #475). Owner: Val, with
implementation in `src-rust/crates/acp`. The previous audit, on `d875fdc`
(2026-10-18), deferred a dispatch limit until an overload contract existed;
that contract has since shipped.

**Disposition: dispatch is bounded; the stdio trust boundary is still open.**
Inbound requests and active handlers now have a limit, and excess requests get
an explicit error while cancellation and replies stay live. ACP `authenticate`
is still intentionally a no-op because credentials stay in the local agent
process. This audit does not establish a separate client identity or
authorization boundary. The client is the process connected to stdio, often
an editor but potentially an OpenCoven orchestration layer.

## Evidence on current main

- Admission ([`admission.rs`](../src-rust/crates/acp/src/admission.rs), from
  #461): queued and running messages share 32 permits. A request that arrives
  with none free gets JSON-RPC error `-32000` ("ACP request capacity
  exceeded"). Error replies go through a 64-entry lane; if that lane fills,
  the connection is retired rather than blocking the reader or allocating more
  work.
- Control notifications such as `session/cancel` are handled in the reader and
  never wait for a permit. Responses to the agent's own requests, including
  permission replies, are routed in the reader before admission
  ([`connection.rs`](../src-rust/crates/acp/src/connection.rs)), and pending
  outbound requests are capped at 64.
- At EOF, admitted work drains for up to 2 seconds; anything still running is
  then cancelled.
- Permission requests are bound to the turn that asked for them (#404).
  Unknown selections, cancellation and transport errors still map to Deny.
  Forwarding tasks are owned (#343).
- The partial-frame writer (#344) and reader-error propagation (#376) are
  merged.
- `session/new` canonicalizes and validates cwd before storing it (#348), and
  `session/prompt` rejects unknown session IDs.

## Remaining gate

The overload policy landed inside the #461 release bundle rather than in its
own reviewed PR. Its tests cover admission, cancellation behind blocked
handlers and EOF draining. Val owns the review of which local processes may
attach to stdio, which methods use stored credentials, and whether the
launcher boundary supplies the required authorization. Native editor and
orchestrator acceptance remain open (#383).
