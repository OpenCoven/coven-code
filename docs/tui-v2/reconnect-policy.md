# Reconnect snapshot and cursor policy for same-peer resume

Issue: [#412](https://github.com/OpenCoven/coven-code/issues/412), under
[epic #406](https://github.com/OpenCoven/coven-code/issues/406). Inspected
2026-11-16. Status: decided on 2026-11-18 (policy B, see
[Decision](#decision-2026-11-18)). Its first part is implemented, see
[Implemented](#implemented-epoch-comparison-unknown-and-registration-at-the-next-session-start);
the rest is listed under [Deferred](#deferred).

The sections "The problem" to "Decision for Val", and "First slice once B is
chosen", are the analysis as written on 2026-11-16. Their citations and their
account of coven-code describe the code before the implemented part.

Sources, cited as `path:line` at these commits:

- coven-code `main` at `04d6c0f8f8013d0ab6d8d0242221ff38c88d9e6a`.
- OpenCoven/coven `main` at `3dc537fb0323fb291ca7fabfff3771bd6bf6c9bc`, unless a
  citation names tag `v0.5.0` (`d9b9d6a8b9f4775ad7fa0a2d95a9a78004c6d92c`), the
  release of the locally installed `coven` CLI.

Read with the [design baseline](../superpowers/specs/2026-10-28-tui-v2-design.md)
(state taxonomy and same-instance negotiation) and the
[capability matrix](capability-matrix.md).

## The problem

A same-peer resume is the moment the terminal uses the Coven daemon again after
a gap: the engine resumes a session (`/session resume`, `--resume`, a restart),
or a daemon read fails and a later one succeeds. The terminal must then decide
which daemon-derived views are still true for the daemon it is now talking to,
and for the session, familiar, account and project now selected.

What a resume has to restore:

- Engine-local state: the transcript and goal from the engine's own session
  files, and the per-session draft and reading position.
- Daemon-derived views: the ledger ownership label, the Next Memory page rows,
  familiar statuses, the welcome panel's daemon indicator (probed at most every
  10 s, `src-rust/crates/tui/src/coven_status.rs:22`, `46-86`), and any `/coven`
  session or event read.
- The outcome of daemon writes: a ledger registration or a `/familiar handoff`
  session create whose answer was lost may still have been applied.
- The identity context those views were read under: familiar, account,
  project, branch or worktree, and producer revision.

What can go wrong:

| Case | Failure |
|---|---|
| F1 missed events | A resumed event read skips events that exist or existed |
| F2 duplicate events | A resumed read returns events already applied, and the view appends them twice |
| F3 daemon replaced | A restarted daemon, or another daemon on the same socket path or store, answers as if nothing changed |
| F4 identity change | A view read for one account, familiar or project stays on screen after a switch |
| F5 revocation or moved revision | A view or pending action stays bound to a familiar revision that was revoked or superseded, or to an older daemon build |
| F6 branch or worktree change | Diffs and pending approvals describe a tree that is no longer checked out |
| F7 sensitive content | Private rows or text from the previous identity or project remain visible or selectable |
| F8 protected replay | A write or approval sent before the gap is sent again without a fresh decision |

## What exists today

### coven-code

- There is no persistent daemon connection. Every call opens a new Unix socket
  and sends one HTTP/1.0 request (`src-rust/crates/core/src/coven_daemon.rs:383-402`).
  The socket is always `$COVEN_HOME/coven.sock`, else `~/.coven/coven.sock`
  (`coven_daemon.rs:328-343`, `src-rust/crates/core/src/coven_shared.rs:28-37`).
  A replaced daemon answers the next request with no signal to the client (F3).
- `DaemonHealth` keeps `apiVersion`, `covenVersion`, `pid`, `socket` and
  `startedAt`, and drops the `capabilities` object (`coven_daemon.rs:51-59`,
  `631-660`). Nothing checks the peer's identity. `/coven` (status) and `/doctor`
  compare `apiVersion` with `coven.daemon.v1` and only warn on a mismatch
  (`src-rust/crates/commands/src/lib.rs:9650`, `9685-9690`, `2899-2920`).
- A write whose answer is lost is reported as failed. After the request is
  written, a read timeout is a transport error and a connection closed without
  an answer is a malformed response (`coven_daemon.rs:411-419`); a handoff
  then shows "Handoff failed" (`src-rust/crates/tui/src/app.rs:2328-2333`), and a
  registration becomes "ledger unavailable ... the daemon does not know this
  session" (`main.rs:3621-3633`, `application/mod.rs:152-155`), although the
  daemon may have applied either.
- `DaemonClient::session_events` sends an `afterSeq` cursor and returns
  `nextCursor.afterSeq` and `hasMore` (`coven_daemon.rs:688-762`). Its only caller
  is the one-shot `/coven events <session-id> [--after N] [--limit M]` command
  (`src-rust/crates/commands/src/lib.rs:10488-10528`). No view follows daemon
  events, and no cursor is stored.
- Ledger registration is opt-in and best effort. It is sent once at startup
  (`src-rust/crates/cli/src/main.rs:3205-3225`, `src-rust/crates/core/src/coven_ledger.rs:16-45`),
  and its answer sets the ownership label once (`main.rs:3618-3635`,
  `src-rust/crates/tui/src/application/mod.rs:120-166`). It is not re-checked
  after a daemon restart.
- `SessionScope` tags query-worker events with a generation and discards those
  from an older one (`application/mod.rs:23-63`, test at `252-297`). The loop
  advances it on clear, rewind and resume only (`main.rs:3929`, `3937`, `3971`,
  `4756`). A familiar switch, from `/familiar <id>` (`main.rs:4146-4165`) or the
  in-app switcher, goes through `App::apply_familiar_agent_mode`
  (`app.rs:6262-6280`); the loop then swaps the agent definition and its tool
  tier (`main.rs:4733-4753`). `/account switch` saves the active account and
  returns a message (`commands/src/lib.rs:3231-3237`,
  `src-rust/crates/core/src/accounts.rs:182-195`). Neither advances a scope or
  clears a view (F4).
- In Next, `enter_session_scope` saves the outgoing draft and position and clears
  tool, selection, Work and Research views; in Classic it only records the id
  (`src-rust/crates/tui/src/app.rs:3440-3476`). The Memory snapshot is dropped
  only when its page closes (`main.rs:2843-2849`), not on a session, familiar or
  account switch (F7). Its titles and excerpts are already withheld unless the
  producer classified the entry as public with no reveal required
  (`coven_daemon.rs:913-924`).
- A persisted session records `working_dir` and `model`, not the familiar,
  account, branch or daemon it ran against (`src-rust/crates/core/src/lib.rs:4966-5004`).
  The transcript format has a `gitBranch` field, always written as `None`
  (`src-rust/crates/core/src/session_storage.rs:136-138`, `712`, `756`).

### Coven daemon

- Health advertises `apiVersion: coven.daemon.v1`, `capabilities.eventCursor:
  "sequence"` and the daemon's `pid`, `startedAt` and `socket`
  (`crates/coven-cli/src/api_health.rs:49-96`, `160-175`;
  `crates/coven-cli/src/daemon.rs:39-49`). The handshake guide says local
  clients should fingerprint the connected Unix peer and socket, and must send no
  dependent bytes to a replaced endpoint and never replay a mutation
  automatically (`docs/daemon/capabilities-handshake.md:19-25`). Coven's own
  Rust client compares socket device and inode and peer pid, uid and gid on
  every request after negotiation, returns `DaemonInstanceChanged` and
  drops its negotiation (`crates/coven-client/src/transport/mod.rs:24-49`,
  `crates/coven-client/src/transport/unix.rs:51-65`, `296-300`,
  `crates/coven-client/src/http.rs:131-133`). coven-code does not use that crate.
- Session events: `GET /api/v1/sessions/:id/events` and `GET /api/v1/events`
  accept `afterSeq`, `afterEventId` and `limit` (`crates/coven-cli/src/api.rs:1505-1509`,
  `1527-1538`, `5144-5208`). `seq` is the SQLite `rowid` of one events table shared by
  all sessions, whose primary key is a text `id` (`crates/coven-cli/src/store.rs:4372`,
  `crates/coven-cli/src/store/schema.rs:208-218`). A session's seq values are
  therefore not contiguous, and a client cannot detect a gap from them.
- An empty page returns `nextCursor: null` (`api.rs:5332-5337`). An `afterEventId`
  that no longer resolves returns the session's events from the start
  (`store.rs:4250-4268`, `api.rs:5196`), a silent rewind (F2).
- The daemon's maintenance tick prunes events older than `log_retention_days`
  (`daemon.rs:5629`, `store.rs:4674-4688`, `4561-4587`). A cursor older than the
  pruned range gets later events with no gap signal (F1). For these routes the
  API contract says a persisted `afterSeq` "always picks up where it stopped"
  (`docs/API-CONTRACT.md:3411`).
- Rowids can also move or repeat. An explicit operator vacuum exists
  (`api.rs:1353`, `store.rs:3558-3593`), and
  [SQLite documents](https://www.sqlite.org/lang_vacuum.html) that `VACUUM` may
  change rowids in a table without an explicit `INTEGER PRIMARY KEY`. Deleting a
  session cascades to its events (`store.rs:3235-3238`, `schema.rs:217`), and
  without `AUTOINCREMENT` SQLite
  [may reuse](https://www.sqlite.org/autoinc.html) the rowids of deleted rows.
- The main-session event stream, on the same table and seq, already refuses a
  cursor whose row is gone with `409 event_cursor_unavailable`
  (`crates/coven-cli/src/main_session_events.rs:119-132`), and its contract says
  seq values are global with gaps and that retention can remove history
  (`docs/API-CONTRACT.md:1636-1637`, `1661-1667`). That check finds a pruned
  cursor, not a renumbered or reused rowid.
- External registration inserts a session row with `external: true` and
  `familiar_id` null and writes no event; repeating it for the same store returns
  the existing row (`api.rs:3670-3754`). Completion updates status only
  (`api.rs:3756-3798`). Orphan recovery skips external sessions
  (`docs/SESSION-LIFECYCLE.md:226`). The daemon therefore holds no event history
  for an engine-local session.
- Coven already has a validated cursor design elsewhere. The Automations event
  feed keeps `earliest_sequence` per stream and returns `CURSOR_EXPIRED` instead of
  rewinding (`crates/coven-cli/src/automations/contract/events.rs:137`, `650-662`;
  `docs/architecture/coven-automations-v1.md:421`, `437`).
- The familiar root and revision ledger is on main and in v0.5.0. A root is
  `familiar:<32 hex>`, not the roster id. Revisions are `active`, `superseded`,
  `retired` or `revoked`, and each change bumps a generation
  (`crates/coven-cli/src/familiar_ledger.rs:1-41`). It is read with the
  `coven.familiars.ledger.get.v1` action over owner-local IPC only
  (`familiar_ledger.rs:62-68`, `586-596`, `1249-1283`). Health does not advertise
  it, and coven-code does not read it.

### Missing

Peer and socket fingerprinting in coven-code, and an `apiVersion` check that
refuses rather than warns; any daemon-view invalidation rule; a session-event
cursor that reports expiry, renumbering or store replacement; an engine record
of the account, familiar root and revision, and branch a view or session was
bound to; and a re-check of ledger registration after a daemon restart.

## Options

**A. Engine-local only.** On resume, restore engine state and drop every
daemon-derived view to "not read" until it is read again. No cursor, no
fingerprint. Needs nothing from Coven. Smallest change, but a reconnect that is
not a resume goes unnoticed: the Memory page and the welcome indicator keep
reading every 10 s (`main.rs:2811`, `coven_status.rs:22`), a replaced daemon
answers them (F3), and late reads from the old one still land.

**B. Fresh snapshot per same-peer epoch.** Negotiate health on first use, on
resume and after any failed daemon request. The epoch is the socket's device
and inode, the peer uid and pid read from the connection, health `pid` and
`startedAt`, `apiVersion`, `covenVersion`, and the capabilities each operation
requires. Tag every daemon-derived view with the epoch and the engine identity
(session, project root, familiar, active accounts) it was read under, as
`SessionScope` already does for query events. On any change, show the view as
stale, drop private rows, discard late reads from the old epoch and read again.
A daemon write in flight or with an unknown outcome at the change keeps that
state until a read settles it, and is never resent. Event views, if one is
added, are re-read from the start, never appended. Needs nothing new from Coven.
Cost: a re-read after every change, and a full event re-read pages from the
oldest retained event because there is no newest-first read. B treats the
advertised `eventCursor: "sequence"` as unfit for resume, for the reasons above.

**C. Snapshot plus today's `afterSeq`, with fallback.** B, plus a stored
`(epoch, session id, last seq)`. On a same-epoch reconnect, read with `afterSeq`;
otherwise fall back to a snapshot. Needs nothing new. Cost: B plus cursor
bookkeeping, and it cannot be made honest: pruning, vacuum and deletes happen
within one daemon lifetime, so the epoch does not change when events disappear
or rowids move, and seq gaps are normal. It would claim replay completeness that
the producer does not give.

**D. Snapshot plus a producer-validated cursor.** B, plus a session-event cursor
that Coven validates: the main-session stream's `event_cursor_unavailable` check
applied to session events, an unknown `afterEventId` rejected instead of
rewound, and a cursor that cannot be renumbered or reused. The Automations feed
gets the last by design: an `AUTOINCREMENT` feed position, a unique per-stream
`sequence` and a no-delete trigger (`automations/contract/events.rs:29-40`,
`70`). For session events that means either such a sequence column or a store
epoch that changes on vacuum and on session deletion. All of it sits behind a
new capability. Refused or mismatched cursors fall back to a snapshot. Needs a
Coven change, a release and an engine pin. Reuses all of B.

| Case | A | B | C | D |
|---|---|---|---|---|
| F1 missed | No daemon events shown | Snapshot shows what is retained | Silent gap after pruning or rowid reuse | Cursor refused, then snapshot |
| F2 duplicate | n/a | View replaced, not appended | Safe only if `afterEventId` is never used and events are deduplicated by id | Cursor refused, then snapshot |
| F3 replaced | Not detected | Epoch change, drop and re-read | As B | As B, plus the cursor's own store check |
| F4 identity | Dropped on resume only | Identity in the tag, cleared on switch | As B | As B |
| F5 revocation, revision | Not handled | `covenVersion` in the epoch; familiar revision needs a ledger read (later slice) | As B | As B |
| F6 branch, worktree | Not handled | Engine adds branch and HEAD to the tag (engine-side) | As B | As B |
| F7 sensitive | Dropped on resume only | Cleared on any tag change | As B | As B |
| F8 replay | None sent | None sent with decision 2(a) below | As B | As B |

## Recommendation

Adopt **B** now. Do not adopt C. Ask Coven for **D** when a terminal view first
needs to follow daemon events.

- The acceptance criterion says that without a supported cursor the terminal uses
  "an explicit safe refresh/reconciliation flow, not invented replay
  guarantees". Coven advertises the session cursor, but it cannot report
  pruning, renumbering, reuse or an unresolved `afterEventId`, so C would invent
  that guarantee.
- No terminal view follows daemon events today, and the daemon has no events for
  engine-local sessions, so a cursor adds nothing yet.
- B's epoch, with the peer check in the first slice, is the same-peer
  negotiation #412 lists as missing and the IPC peer check the capability matrix
  assigns to #412 (`capability-matrix.md:246`). Its tagging reuses the
  `SessionScope` pattern already tested on main. D builds on it without
  discarding anything.

## Decision for Val

1. Reconnect policy for daemon-derived views: **B** (fresh snapshot per same-peer
   epoch, no daemon event cursor), or A, C or D instead.
2. Ledger registration after a daemon restart or replacement: **(a)** show
   "ledger state unknown" and register again only at the next session start or
   on an explicit action (recommended), or **(b)** register again automatically.
   If the first registration was stored, re-registering on the same store
   changes nothing: the row exists and orphan recovery skips external rows. It
   writes a new row when the first one failed or its answer was lost, or when
   the store was replaced; either way it is an automatic retry of a write, which
   the handshake guide's no-replay rule argues against.
3. Coven: defer the request for D's validated cursor until a following view is
   planned (recommended), or file it now. Either way, the
   `docs/API-CONTRACT.md:3411` sentence conflicts with Coven's own main-session
   contract and code and could be raised with Coven.

## Decision (2026-11-18)

Val decided, in the maintainer's decision sheet:

1. Policy **B**.
2. **(a)**: after a daemon restart or replacement, show the ledger state as
   unknown and register again only at the next session start.
3. Defer the request for D's validated cursor.

## Implemented: epoch comparison, "unknown", and registration at the next session start

This is the part of B that concerns the ledger registration and the daemon
indicator. It adds no daemon event cursor and needs nothing new from Coven.

**What identifies a daemon instance.** Coven v0.5.0's health response names
the serving process: `daemon.pid` and `daemon.startedAt`. The engine keeps
those two, `apiVersion`, `covenVersion`, and the device and inode of the
socket file read before connecting (`DaemonInstance` in
`src-rust/crates/core/src/coven_daemon.rs`). Two readings that differ in any
of them are different instances. A health answer that names neither a pid nor
a start time is not an instance: it could not be told from its successor.

**One look at the daemon** (`DaemonClient::sight`) is one of:

- gone: no socket file, a connection that is refused or otherwise fails, or
  one that fails or closes before any byte of an answer;
- unidentified: a connection was made (or was still waiting to be accepted
  when the budget ended) but gave no complete health answer within the
  budget, answered with an error status, or named no process;
- an instance.

The background reachability probe behind the daemon indicator (at most one
every 10 s, as before) now reads `GET /api/v1/health` to the end instead of
the first byte of `GET /api/v1/familiars`. Its 2 s budget now covers the whole
read once connected, so a peer that keeps sending bytes cannot hold it. Since
#580 the connect shares that budget: on a Unix socket it returns at once
unless the daemon's listen queue is full, which on Linux waits for room (now
until the budget ends) and on macOS refuses it. "Online" still means anything
but gone, so a busy daemon is not shown as offline.

**Registration is tied to an instance.** With `daemonLedger` on, the
registration reads health before and after its `POST /api/v1/sessions/external`
(2 s budget each, off the UI thread). If both name the same instance, that
instance holds the registration. If the daemon accepted the registration but
the two readings differ or either names no instance, the registration is
unknown from the start. A health read that takes longer than its budget names
no instance, so a daemon that is slow at that moment starts as unknown
without having restarted.

**Comparison.** Once a second the interactive loop compares the last
completed probe with the instance that holds the registration:

| The probe found | Registration | Indicator |
|---|---|---|
| The same instance | kept | online |
| A different instance | unknown, and it stays unknown | unknown |
| Gone | kept | offline |
| Gone, then the same instance again | kept | online |
| Gone, then a different instance | unknown | unknown |
| Unidentified (busy, error status, no process named) | kept: no evidence either way | online |

A probe that began before the registration was recorded is ignored, since it
may show the daemon from before a restart. A restart is therefore noticed at
the first probe that begins after it and finds the new daemon: within about
14 s (10 s cache, 2 s budget, and a loop tick each to start the probe and to
read it). A probe that lands between the stop and the start reads gone and
adds one more cycle.

**What "unknown" looks like.**

- Next: the header and the Session page read `engine-local, ledger unknown`
  instead of `engine-local, ledger-registered`.
- Classic, and every frame Next hands to Classic (the command palette, for
  one): the welcome line reads `unknown` instead of `online`, and the footer
  shows `◌ coven (unknown)` instead of `✦ coven`. While nothing answers, both
  read offline, as before.

**Registration at the next session start.** Nothing is sent when the state
turns unknown, and no probe, turn or command sends anything later. A session
start does:

- a new process (a fresh launch or `--resume`) registers the session it
  binds, as it always did, and now records the instance;
- inside a running process, `/session resume` registers the session it binds
  when, and only when, the state is unknown. It is the only place a running
  process binds a session: `/clear` and rewinding keep the session and its id.

Until a named instance has accepted the new registration, the indicator keeps
reading unknown. If the new registration is refused, lost, or accepted by a
daemon that names no process, the state stays unknown and the session start
after that tries again. A first registration that failed is not retried, as
before.

At exit the engine sends one completion for every session id it sent a
registration for, to whichever daemon answers then, including after the state
became unknown. A registration still in flight gets up to 2 s to finish
first. That is best effort: against a daemon slower than that, a completion
can still arrive ahead of its registration and leave the row `running`.
Since #580 the exit then waits at most 2 s for the completions, which run
side by side on threads of their own. Past that the exit goes on without the
answer: whether the daemon still records a completion depends on it
finishing before the connection closes, and a row it never records stays
`running`, as against a daemon that is down. Neither a registration nor a
completion still waiting keeps the process alive after that.

Until #580 the registration and completion requests had no total deadline:
each read waited up to 5 s and that timer restarted with every byte, and the
connect was not timed. Every request now has one 5 s deadline over the
connect, the request and the whole answer. A registration therefore gives up
within 9 s at worst (2 s health, 5 s request, 2 s health); against a daemon
that never answers it takes 7 s, because the second health read is skipped
once the request has failed.

**Sessions that never registered** (`daemonLedger` off, the default) remember
no instance. Their indicator reads online or offline as before, and nothing
is registered at any time.

**Not changed.** `--print`, stream-json and ACP still never contact the
daemon.

**What this does not prove.**

- `pid` and `startedAt` are what the daemon says about itself. The engine
  does not yet read the peer's credentials from the connection, so a different
  process that answered with the same values would not be told apart.
- A health answer that names a pid and no start time still counts as an
  instance. A restart that got the same pid would then be noticed only
  through the socket file.
- The socket file's device and inode are enough to read as changed, and not
  enough to prove the same daemon: some file systems reuse an inode number at
  once.
- A daemon whose health never names a process reads unknown from the first
  registration on, and each `/session resume` sends another registration.
- "Unknown" is about which daemon holds the registration. The engine does not
  ask the new daemon whether the session row exists; with the same store it
  usually does, and registering again returns that row.
- A store replaced or vacuumed under a daemon that keeps running is not
  noticed. That is option D's store check.
- Between a restart and the next probe (up to about 14 s) the old label is
  still shown.
- While nothing answers, Next's header keeps the last registration label;
  Next's own frames have no daemon indicator.
- The indicator and Next's label answer different questions. The indicator is
  about the registration this process sent, whichever session is bound.
  Next's label is about the bound session: after `/session resume` to a
  session this process did not register, it reads plain `engine-local`, while
  Classic's indicator still reads unknown after a restart.
- A registration whose answer was lost (a timeout after the request was
  written) is still shown as failed, not as unknown.
- A daemon replaced by one that never names a process (or always answers with
  an error status) reads unidentified on every probe, so the registration
  stays registered, even when the socket file's inode shows a different file.
- A second `/session resume` while a registration sent again is still waiting
  for its answer sends nothing: once that answer lands the state reads
  registered for the session sent first, and the session bound last is not
  registered until a later session start finds the state unknown.

**Observed on 2026-11-18** with Coven v0.5.0 in a disposable `HOME` and
`COVEN_HOME`, `daemonLedger` on, Classic and Next at 120x40:

| Step | Classic | Next |
|---|---|---|
| Launch | `online`, `✦ coven` | `ledger-registered` |
| `coven daemon stop`, then `start` (new pid and start time) | `unknown`, `◌ coven (unknown)` after 9.8 s | `ledger unknown` after 10.7 s |
| 25 s later, then after a turn | still unknown; the daemon's session row is untouched | the same |
| `/session resume <id>` | `online`, `✦ coven` at once | `ledger-registered` at once |
| The daemon stopped with `SIGSTOP` for 26 s, then continued | online throughout | registered throughout |
| `SIGKILL` (socket file left behind) | `offline`, `○ coven` | still `ledger-registered` |
| `coven daemon start` | `unknown`, `◌ coven (unknown)` | `ledger unknown` |
| Quit | completion reached the new daemon; the row reads `completed` | the same |

The session row survived each restart, because the store was the same.

**A daemon that accepts and never answers** (a listener on the socket that
reads and stays silent), same date and sizes. Every wait measured here ended
within the request timeout:

- Launch is not delayed. The registration gives up after 7 s on its own task
  (2 s health, 5 s request) and Next reads `ledger unavailable`; Classic reads
  online, since something accepted the connection.
- `/coven sessions`, `/coven familiars` and `/familiar list` each freeze input
  and drawing for 5 s: they read the daemon on the event loop with the 5 s
  request timeout. `/coven status` and `/status doctor` take 0.3 s.
- Quitting waits 5 s for the completion request. The completions now run
  side by side, so this stays one timeout when two ids were registered.

Those 5 s waits were there before this change and are not changed by it. A
quit within the first seconds after a launch or a `/session resume` waits up
to 2 s for the registration still in flight before it sends the completions,
and the process then still waits for that registration's own task to end
(at most its 7 s against a silent daemon).

Since #580 (tmux case `37_silent_daemon_keeps_input.sh`): the three listings
ask the daemon on a thread of their own, one at a time. Input and drawing go
on; after 300 ms a notice says the daemon is being waited for, and the
listing's result (here, the 5 s deadline's error) lands when it arrives; one
that arrives during a turn waits for the turn to end. A second listing typed
meanwhile is not sent, and a toast says `Not sent; waiting on` the listing
still waiting. Quitting
waits at most 2 s for the completion, plus up to 2 s for a registration still
in flight, and the process then ends at once.

## Deferred

- Decision 3: the request to Coven for a validated session-event cursor (D).
- From the first slice below: the peer credential and socket check on every
  request with an "instance changed" error; refusing dependent operations on
  another `apiVersion`; keeping typed `capabilities` and gating session and
  event reads on them; reporting a lost answer to a registration or handoff
  as "outcome unknown".
- Tagging the Memory snapshot and familiar statuses with the daemon instance
  and the engine identity, and clearing them on a change, a familiar switch
  or an account switch. Today an open Memory page still shows rows read from
  the previous daemon until its next 10 s refresh.
- An explicit action to register again without a session start.
- The later slices: branch and HEAD in the identity tag, and familiar root
  and revision from the ledger.

## First slice once B is chosen

- Core: keep typed `capabilities` in `DaemonHealth`; add the daemon epoch as
  defined in B; on every request, check socket identity before connecting and
  peer credentials after (getpeereid and `LOCAL_PEERPID` on macOS, `SO_PEERCRED`
  on Linux, as coven-client does), and return an "instance changed" error
  without sending. Refuse dependent operations when `apiVersion` is not
  `coven.daemon.v1`, reusing `COVEN_DAEMON_EXPECTED_API`. Gate session and event
  reads on `capabilities.sessions` and `capabilities.events`. Health has no flag
  for memory or familiar reads: memory keeps its 404 and shape handling
  (`application/memory.rs:36-56`), and familiar statuses keep degrading to an
  empty map held in a process-wide 2 s cache
  (`src-rust/crates/tui/src/agents_view.rs:718`, `737-750`), which an epoch
  change must also clear. For registration and handoff, report a transport or
  framing error after the request was written (no complete response,
  `coven_daemon.rs:411-430`) as "outcome unknown", not failed; a complete error
  response stays a refusal (`BadStatus`, `coven_daemon.rs:434-458`).
- TUI and CLI: a daemon scope next to `SessionScope` that tags the Memory
  snapshot, ownership label, familiar statuses and welcome indicator with the
  epoch and engine identity; advance it on resume, familiar switch and account
  switch; clear the Memory snapshot on any change, not only on page close; keep
  an unknown registration outcome as unknown across a change.
- Tests: fake-socket fixtures in the existing `serve_once` style
  (`coven_daemon.rs:1470-1489`) for missing, false or malformed capabilities
  (nothing dependent sent), a socket replaced between negotiation and a read
  (instance changed, no bytes sent), and a new `startedAt` (new epoch); a
  late Memory read from an old epoch discarded, mirroring
  `stale_worker_output_does_not_reach_a_replaced_conversation`; familiar and
  account switches clearing the Memory snapshot; a registration whose answer is
  lost (the fake daemon reads the request, then closes) shown as unknown, not
  failed, and not resent after an epoch change; a resumed draft shown but not
  sent and no Memory rows restored; one tmux case in `scripts/tui-tests`
  restarting a fake daemon mid-session.
- Later slices: branch and HEAD in the identity tag (the unused `gitBranch`
  transcript field could record the branch); familiar root and revision
  from the ledger, so a revoked or superseded revision invalidates bound views
  and pending actions (#411's protected actions stay unavailable until their
  producer contracts exist).

## Real-daemon recovery journey

Run on 2026-11-18 against Coven v0.5.0, with the implemented part in place.
The results are in the table under
[Implemented](#implemented-epoch-comparison-unknown-and-registration-at-the-next-session-start).

- The daemon binds `$COVEN_HOME/coven.sock` and refuses a socket path longer than
  103 bytes on macOS, after resolving symlinks (`crates/coven-cli/src/daemon.rs:2385-2421`
  at v0.5.0). coven-code also connects only to `$COVEN_HOME/coven.sock`. A
  disposable home inside `.git/maintenance-audits/` gives a 126-byte socket
  path, which the daemon refuses, so the journey used a disposable `HOME` and
  `COVEN_HOME` under the system temporary directory (a 32-byte socket path),
  removed afterwards.
- Val's daemon processes and the `~/.coven` listing were recorded before and
  after: the same processes, and the same entries and sizes.
- The behavior before the change is recorded on #412: on 2026-11-17 a
  restarted daemon was used silently, the label stayed `ledger-registered`,
  and nothing registered again.
