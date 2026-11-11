# Coven terminal v2: product and contract baseline

Issue: [#407](https://github.com/OpenCoven/coven-code/issues/407), under
[epic #406](https://github.com/OpenCoven/coven-code/issues/406).
Inspected 2026-10-29. The filename follows #407's requested deliverable name;
it is not an October 28 execution claim. Status: accepted #407 design baseline (2026-10-31), with the executable Classic
baseline recorded in the compatibility matrix ledger.

Read with the [capability matrix](../../tui-v2/capability-matrix.md) and
[compatibility matrix](../../tui-v2/compatibility-matrix.md). Those documents
identify immutable producer revisions, implementations, tests and evidence limits.
Normative language below describes the proposed v2 implementation contract, not
features delivered by this documentation PR.

## Intent and scope

One Coven terminal serves coding, orchestration, research and memory inspection.
The default remains a transcript and composer. Detail is available deliberately,
without competing dashboards. Rust/Ratatui/Crossterm and the separately launched
engine remain. Internal `claurst-*` identifiers and the engine contract stay intact.

| Stage | Usable scope | Evidence required before exposure |
|---|---|---|
| M1 | Coding, existing command navigation, tools/diffs/permission requests, session/familiar navigation and honest recovery | #408-#412; applicable #417/#418 M1 terminal, compatibility and rollback evidence |
| M2 | Psyche-backed Work, cited research, private read-only familiar-memory inspection | Separately qualified real producers for #413-#415; no fixture-only integration claims |
| M3 | Operation-specific governed memory proposals | #416 producer authorization, immutable review/commit binding, idempotency and receipts; identity promotion separately governed |

Classic remains the default. There is no default-switch, release, provider
expansion, Cave redesign, new task/memory database or client-side authorization in
this work. Existing #370 release holds are unchanged. #420 owns competitive
research; this baseline neither duplicates it nor uses unverified competitor claims.

## Approach and decisions

The selected baseline follows the already specified #406 approach: extract one
typed presentation seam, retain Classic, then add Next as a consumer of the same
runtime intents/events. This is a recommendation grounded in existing contracts,
not a claim of additional owner approval.

| Approach | Tradeoff | Disposition |
|---|---|---|
| Shared application seam, Classic and Next projections | Requires staged extraction from `App` and the CLI loop; supports event/command parity and rollback | Adopt for #408 |
| Rewrite engine, state stores and presentation together | Couples UI rollout to protocol, migration and authority changes | Reject: outside #406/#407 scope |
| Add independent task/research/memory dashboards and a second palette | Fast mock screens but conflicting state, navigation and lifecycle ownership | Reject: violates one-model and producer-ownership constraints |

Current `App` owns presentation state and some effects; this is not already a
pure reducer architecture. `main.rs` holds the authoritative interactive message
vector, command dispatch, query worker/cancellation and persistence coordination.
`CommandContext` carries session/config/cost/workspace/MCP context; `CommandResult`
includes messages, configuration changes, session changes, UI requests and
captured `GoalUserMessage` authority. #408 must preserve these meanings rather
than manufacture a second execution loop.

The shared model needs typed intents, producer events, operation correlation and
view projections. Names and module details are left to #408. Presentation state
may include draft input, focus, selected object, scroll offsets, filters, inspector
kind and reveal consent. It must not contain a new authoritative task graph,
permission grant, durable memory copy, lease, or synthetic commit receipt.

Effects flow through the existing command/query/daemon adapters. Keep exactly one
owner for each effect. Preserve the existing command preflight before any widget
interception: removed aliases must not trigger UI mutations. Replay-safe reducers
may coalesce cosmetic updates, never permission, tool-result, terminal or receipt
events. Closing an inspector or changing a layout does not restart a run.

## Information architecture and terminal cells

Focus, Work and Inspect are layouts of one session/navigation model. Work selects
an origin-labeled task list; Inspect selects one contextual tool/diff/session/task/
memory detail surface. Research and Familiar memory are destinations in the same
navigation model and existing palette, exposed only at their qualified stage.
The initial M1 navigation may explain unavailable integrations but must not render
invented live tasks or memories.

These are initial #410 design thresholds, to be verified with #417 cell tests:

| Available cells | Layout rule | Reason |
|---|---|---|
| Width < 80 or height < 24 | Focus only (Work and Inspect render as a full-screen replacement of the main pane); retain draft and return position | Avoid a sidebar starving transcript/composer; 40x12 is an existing safety fixture |
| Width >= 80 and height >= 24, with width < 120 or height < 32 | Focus or Work main pane; inspector replaces main pane | 80 columns cannot reliably contain 72-column text plus useful detail |
| Width >= 120 and height >= 32 | Optional 72-column transcript + 1-column separator + at least 40-column inspector, with remaining cells for padding | Both regions retain useful content; never open a second inspector automatically |

At smaller sizes use a bounded warning/return path without panics or invisible
approval choices. The composer and critical authorization prompt take precedence
over optional detail. Resize preserves draft, selection and operation identity.
Do not choose breakpoints in browser pixels or infer font metrics from concept art.

Reuse `/` suggestions, Ctrl+K, catalog categories, drill-in/Run-parent behavior,
recent commands, dynamic command precedence and configurable bindings. Never
recreate the palette or revive retired slash roots. Command buttons are alternate
presentations of the same actions, not alternative execution endpoints.

## Ownership and capability discovery

| Domain | Owner | Terminal responsibility |
|---|---|---|
| Engine-local tasks, transcript, session goals and instruction files | Coven Code's existing core/tools/query/session paths | Preserve current operations and identify their local origin |
| Canonical execution, persistence, IPC and capability advertising | Coven daemon | Negotiate named contracts, validate typed responses, present exact scope |
| Task graph, lane, lease, retry/recovery and verification semantics | Psyche | Consume a qualified producer adapter; no local scheduler or DB |
| Protected decision/commit authorization | Coven/Threads | Render pending decision and receipt; do not derive permission from UI state |
| Governed identity | Familiar Contract with its implementing producers | Present verified bindings and unknowns; a selected persona is not cryptographic verification |
| Continuity | SPAR projection | Treat as projection, never as an authority or storage replacement |
| Visual roles / portable interaction semantics | Brand / UI | Translate roles into cells; no React or DOM dependency is required |

For new daemon integrations start with `GET /api/v1/health`, require
`apiVersion: coven.daemon.v1` and the exact operation's advertised capability.
Ignore unknown additive capabilities; missing/false/malformed required fields mean
unavailable. Bind negotiation to the same authenticated IPC peer/instance; after
replacement discard it and renegotiate before sending dependent bytes. Capability
advertising is not permission. The current engine's older `DaemonHealth` DTO does
not retain the capabilities object: richer negotiation is an explicit adapter gap,
not evidence that the daemon lacks capabilities. Preserve existing Classic behavior.

Do not infer Psyche support from the daemon catalog or memory mutation from a
read endpoint. Familiar Contract RFC-0001 v0.7.0 is the current normative draft;
its embodiment binding records exact root/revision/target and grants no protected
action authority. SPEC.md is historical. A self-consistent signed bundle without
independently trusted signer/current-ledger observations cannot qualify dispatch. SDK TypeScript bindings are references for semantics, not a reason
to introduce a Node runtime to the Rust terminal. Use only qualified operations
in the producer table in the capability matrix.

## State and recovery taxonomy

Keep independent dimensions: connection/freshness, operation outcome, execution
lifetime, authorization and content verification. One green badge cannot flatten
them into success. Preserve raw producer state and observation time alongside the
user-facing explanation.

| Visible state | Evidence / meaning | Safe next action |
|---|---|---|
| Loading | First read pending, no current authoritative result | Wait or cancel the read; do not display an empty successful list |
| Stale | Prior read exists but freshness/scope is no longer trusted | Show scope/time for nonsensitive metadata; redact private detail and re-read |
| Disconnected | Transport absent/lost; independent from authorization status | Restore connection deliberately; no automatic mutation replay |
| Reconciling | Correlated lookup/recovery in progress after uncertainty | Wait for authoritative disposition; preserve operation/request identity |
| Degraded | Some optional evidence is unavailable; required evidence still gates effects | Explain missing fields; disable only affected operations |
| Unavailable / update required | Unsupported contract, missing capability or producer adapter | Explain owner/dependency; no local store or filesystem fallback |
| Pending authorization | Exact operation awaits producer-authorized decision | Review scope, target, digest/revision, expiry and authority; no optimistic commit |
| Outcome unknown | Request may have crossed an effect boundary, but response/receipt is absent | Reconcile original request; timeout does not authorize retry |
| Partial | Truncation/token limit, incomplete output or missing required evidence | Keep partial artifact; a separate bounded continuation needs its own valid scope |
| Completed | Producer reports the scoped operation complete with applicable evidence | Show source and limits; completion is not necessarily verified success |
| Rejected / failed / expired | Authoritative refusal/failure for the exact operation | Show safe reason; re-review changed input/authority rather than silent replay |

Cancellation has three separate observations: request accepted, execution exited,
and cleanup acknowledged (including unsuccessful cleanup). Existing local
`TaskRegistry::cancel` marks `Cancelled` and signals a token synchronously; it does
not wait for worker exit. Next must describe that as cancellation requested unless
it has stronger evidence. Existing Classic labels remain a known limitation, not a
contract guarantee. #373 and #374/PR #397 retain their process-tree and AutoDream
ownership responsibilities. A terminal-frame event, model assertion or UI timer
cannot stand in for cleanup or verified goal completion.

Psyche's typed `AdoptionDisposition` distinguishes `Adopted`,
`ProvenNotAdopted`, and `Unknown`; reconciliation distinguishes `Returned`,
`Fenced`, and `Unresolved`. Only producer-proven non-adoption or a qualified fence
can enable a new attempt under producer policy. `TerminationDisposition` requires
acknowledgement evidence or durable unresolved evidence. Do not promote a
socket timeout or a missing row into proof of non-adoption/termination.

For protected changes keep submission, pending approval, rejection/expiry,
committing, unknown outcome and committed-with-receipt distinct. A submitted
proposal is not committed content. New selection, familiar switch, late response,
revocation or stale review revision invalidates the affected review/reveal state.

## Golden journeys

These are acceptance scripts for downstream implementation, not executed v2 tests.

| Journey | Steps and expected boundary | Existing evidence / remaining work |
|---|---|---|
| Fresh unconfigured launch | Disposable home; show no chosen familiar/provider credentials; prompt for explicit setup only when requested; quit cleanly | Saved-roster/onboarding tests; #410/#417 actual terminal journey required |
| Coding/tool/diff/approval | Submit synthetic turn; stream text; open one tool/diff detail; review exact permission; deny then permit a different request; inspect result | Existing App/query/dialogs and engine fixtures; #411 real effect/denial parity |
| Resume | Save session; quit; resume exact ID; restore transcript/goal linkage; report load/persistence failure without substituting a new session | Existing engine-contract resume fixture; #403 remains separate open persistence fix |
| Familiar/project switch | Retain draft deliberately; establish new scoped generation; invalidate old detail/reveal and late reads; reapply engine access tier | Existing F2/familiar tests; #412 must qualify daemon identity and reconnect |
| Task dependency/recovery | Show local origins separately; canonical producer rejects missing/cyclic dependencies; disconnect after start; lookup before retry; await termination evidence | Local stores exist; Psyche/Coven consumer qualification required for #413 |
| Cited research | Start through supported task lifecycle; preserve source URL/retrieval time/content reference and claim-to-evidence mapping; interrupted artifact remains partial | Web tools, stash and review memory citations exist; #414 research lifecycle/citation artifact adapter remains unqualified |
| Private memory inspection | Negotiate read support; show no unknown-classification previews; explicit reveal; separate exact context selection with destination/provider; scope switch redacts; browsing sends no model input | Coven read contracts exist; #415 adapter/privacy/revision evidence required |
| Unavailable protected mutation | Inspect read-only context; edit/delete/promote are unavailable unless exact operation is qualified; explain owner; no write request sent | #416 remains blocked by operation-specific producer evidence |

Private/unknown content must not appear in default titles, snippets, toasts,
clipboard, exports, logs or CI captures. Selecting context is distinct from
browsing/reveal. It must show exact allowed content/reference, source version,
destination session/provider and retention consequences. Read consent is not
consent to model submission. Treat retrieved instructions as data, never authority.
After intentional rendering, terminal/tmux scrollback and screenshots can retain
text; UI clearing cannot promise erasure of those copies.

## Brand reconciliation

Canonical Brand profile `1.0.0` is pinned in the capability matrix. Its anchor is
`#8E3DFF`; `theme_colors.rs` and `COVEN.md` describe historical `#8B5CF6` as the
brand default. This baseline resolves semantic precedence in favor of the canonical
profile. It does not recolor Classic or edit generated artifacts.

| Terminal role | Canonical dark / light reference | #409 implementation rule |
|---|---|---|
| Identity accent | `#8E3DFF` brand anchor | Restrained accents; not universal body-text or selection foreground |
| Action | `oc-action` `#8E3DFF` / `#7A22EE` | Use role-appropriate foreground/background pairs; keep contrast-tested alternatives |
| Focus | `#C9A7FF` / `#6E19DA` | Visible marker and text/bold/underline fallback; not color alone |
| Presence | `#B991FF` / `#5E2A8A` | Label connected/live; no implied authorization |
| Text and hierarchy | `oc-text`, muted, subtle and surface roles | Legible neutrals; preserve terminal default background option |
| Verified/pending/blocked/evidence | Profile semantic state pairs | Text/shape labels; unknown remains explicit neutral evidence state |

The existing palette also has action `#A78BFA`, info `#C4B5FD`, deuteranopia,
light/dark and named themes. Do not globally replace violet literals or remove
accessibility palettes. Web contrast pairs assume known RGB backgrounds; terminal
ANSI/256-color palettes and transparency require separate validation. Preserve
terminal-owned fonts. The canonical mark is SVG-only (`web/assets/mark.svg`), and no terminal
graphics protocol is qualified, so cell renderers use plain `Coven` text for cell/ASCII fallback, not an invented or retired glyph.

#409 must wire semantic roles through actual renderers; merely changing the
palette file is insufficient. NO_COLOR (currently read nowhere), ASCII/Unicode fallback, reduced motion (wire the
existing persisted `reduceMotion` setting, which no renderer reads today; do not add a second setting),
configurable shortcuts and focus cues are required v2 work with #417 evidence,
not certified current behavior. The inspected render/App/figures paths do not
establish complete support. Translate UI contract focus/failure/progress semantics;
DOM hooks and CSS token values are not a terminal implementation.

## Dependency readiness and review limits

#408 and #409 are the next implementation candidates after acceptance of these
three source baselines. File ownership: #408 owns `tui/src/application/`, App/CLI event and effect
plumbing in `tui/src/app.rs` and `cli/src/main.rs`, and the renderer selector. #409
owns `tui/src/next/` styling/primitives, `theme_colors.rs` and presentation-only
appearance settings in core. Shared edits to `app.rs`, settings or `keybindings.rs`
land in #408 first; #409 rebases onto it and consumes its view model.
They do not wait on #420 or M3. #417 harness planning can begin alongside them;
its execution depends on stable seam/primitives and later M1 journeys.

#410 still requires #408/#409; #411/#412 remain sequenced as in #406. #413 requires
Psyche protocol/consumer qualification; #414 inherits that lifecycle dependency.
#415 can qualify only the exact safe subset of existing memory reads and must
resolve immutable selection/privacy gaps; #416 stays operation-gated. #418 and
all default-switch/release decisions require exact packaged/native evidence.
Issue closure alone does not satisfy any gate.

The Classic executable baseline (targeted suites, real-binary tmux journeys, cold
start and RSS samples) is recorded in the compatibility matrix ledger. Next does
not exist at this baseline, so Classic/Next comparisons begin with #408. Real-daemon
journeys and native-platform acceptance remain #412/#417/#418 work. Accepting this
baseline does not complete the epic or any downstream gate.
