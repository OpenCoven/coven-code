# coven-code — Security & Performance Audit + Remediation Backlog

Generated 2026-06-15. Scope: full `src-rust` workspace (12 crates, ~146K LOC), shell installers, npm packaging.
Method: 8 parallel code-tied auditors (3 security domains, 3 performance domains, 2 ACP/permission deep-dives). Every finding cites real `file:line` evidence.

Severity legend: **C** Critical · **H** High · **M** Medium · **L** Low. Effort: S/M/L.

---

## Executive summary

The four issues that should be fixed before anything else:

1. **Local RCE on opening a repo** (`SEC-SUPPLY-1`, C) — project-local `.coven-code/plugins/*` are auto-enabled and their hooks run via `sh -c` with no consent.
2. **Session write-amplification** (`PERF-MEM-1/9`, C) — every `/tag`, `/rename`, branch op re-serializes the *entire* session (incl. per-checkpoint full message snapshots) and rewrites the whole file.
3. **Session list/search loads everything** (`PERF-MEM-2`, C) — listing/searching deserializes every message of every session just to read titles/dates.
4. **TUI re-renders the whole transcript every frame while streaming** (`PERF-TUI-1/2`, C) — full markdown re-parse + deep clone of all message content, caches bypassed, at ~60fps.

Cross-cutting themes: (a) the Bash safety classifier and the capability sandbox are *advisory* and bypassable; (b) credentials are written world-readable and a key is logged; (c) full-file `read_to_string` and full re-serialization where streaming/range/partial-write would do; (d) per-turn rebuilds of expensive objects (reqwest client, 1.9MB model registry); (e) no dirty-flag in the render loop.

---

## SECURITY FINDINGS

### A. Command execution & the permission boundary

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| SEC-EXEC-1 | H | Bash Critical-classifier bypassed by `;`/`&&`/`\|`/`$()` chaining & `VAR=val` prefixes (`echo hi; rm -rf ~` → Safe) | `core/src/bash_classifier.rs:42-56,107-168`; `tools/src/pty_bash.rs:559-565` |
| SEC-EXEC-2 | H | `EnterWorktree.post_create_command` = arbitrary `sh -c`, skips classifier, prompt only says "Create a git worktree" | `tools/src/worktree.rs:109,210-223` |
| SEC-EXEC-3 | H | `REPL` tool runs arbitrary bash/python/node; code hidden from approval prompt, no classifier | `tools/src/repl_tool.rs:236-258` |
| SEC-EXEC-4 | M | Bash prompt shows model-controlled `description`, discards computed High/Med risk (deceptive approval). PowerShell tool does it right | `tools/src/pty_bash.rs:540-556`; cf. `powershell.rs:126-205` |
| SEC-EXEC-5 | M | Plugin `ShellCommand` slash cmd concatenates user args into `sh -c` unescaped (`/cmd $(rm -rf ~)`) | `commands/src/lib.rs:2336-2353` |
| SEC-EXEC-6 | L | `CLAUDE_STATUS_COMMAND` run via `sh -c` on 500ms loop (env-controlled, not model-reachable) | `cli/src/main.rs:2313-2338` |
| SEC-PATH-1 | M | Write/Edit/ApplyPatch pass `path=None` to permission engine → no workspace containment; blanket allow writes `~/.ssh` etc. | `tools/src/{file_write.rs:60,file_edit.rs:78,apply_patch.rs:296-305}` |
| SEC-PATH-2 | M | `ApplyPatch` writes to paths from diff `+++` header (`../../etc/...`), prompt shows only file count | `tools/src/apply_patch.rs:68-74,298-318` |
| SEC-PATH-3 | L | Grep follows symlinks (`follow_links(true)`); mostly mitigated by canonicalized boundary check | `tools/src/grep_tool.rs:198-199` |

### B. Credentials, auth, network

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| SEC-CRED-1 | H | `GOOGLE_API_KEY` placed in request URL **and** logged verbatim at `debug` (3 sites); redundant with `x-goog-api-key` header | `api/src/providers/google.rs:69-82,613,662,911` |
| SEC-CRED-2 | H | OAuth tokens + derived `sk-ant-` key + provider key store written without `0600` (world-readable on shared/CI hosts). `set_user_only_perms` exists but used only for non-secret registry | `core/src/lib.rs:3951-3958`, `core/src/oauth_config.rs:317-324`, `core/src/auth_store.rs:66-103`, `mcp/src/oauth.rs:61-68`; helper at `core/src/accounts.rs:291-302` |
| SEC-CRED-3 | M | MCP OAuth flow omits `state` and passes `expected_state=None` → CSRF/code-injection on loopback callback | `mcp/src/oauth.rs:139-147,294-299` |
| SEC-CRED-4 | M | Loopback OAuth callback: unbounded read, no `Host` validation, open 180s; compounds CRED-3 | `mcp/src/oauth.rs:230-292` |
| SEC-CRED-5 | L | PKCE verifier/state built from 2×UUIDv4 (~244 bits) instead of CSPRNG; MCP path does it right | `core/src/oauth_config.rs:178-186`, `core/src/lib.rs:4083-4108` |
| SEC-CRED-6 | L | Secret structs derive `Debug` (`OAuthTokens`,`AuthMethod`,`CodexTokens`,`StoredCredential`,`BridgeConfig`) → one `{:?}` from leaking | `api/src/provider_types.rs:239`, `core/src/lib.rs:3869`, `core/src/oauth_config.rs:194,297`, `core/src/auth_store.rs:11`, `bridge/src/lib.rs:159` |
| SEC-CRED-7 | L | OAuth refresh/exchange surface raw server error bodies into anyhow/logs | `api/src/providers/codex.rs:147-176`, `mcp/src/oauth.rs:482-485,560-563` |

**Verified-clean:** no `danger_accept_invalid_certs` anywhere; MCP tokens bound to normalized server URL; bridge IDs regex-validated; official-MCP registry fails closed; bridge JWT decode documented unverified & unused for authz.

### C. Supply chain & plugins

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| SEC-SUPPLY-1 | **C** | Project-local plugins auto-enabled; hooks run `sh -c <command>` with no consent → open a malicious repo = RCE | `cli/src/main.rs:1041,1045`; `plugins/src/lib.rs:224-227`; `plugins/src/loader.rs:158-159`; `plugins/src/hooks.rs:165-180` |
| SEC-SUPPLY-2 | H | Capability gate (`read_files`/`shell`/`network`/…) enforced only for slash commands; hooks/MCP/LSP spawn regardless | `plugins/src/lib.rs:44-67`; only call site `commands/src/lib.rs:2308` |
| SEC-SUPPLY-3 | H | `install.sh` / `install.ps1` download binary with **no** checksum/signature, then chmod+PATH (npm path *does* verify — asymmetry) | `install.sh:108-112`, `install.ps1:129,141` |
| SEC-SUPPLY-4 | H | Marketplace install: optional hash, HTTP URLs allowed, extracted into auto-loading dir (latent — not yet wired to a command) | `plugins/src/marketplace.rs:110-178,270-276` |
| SEC-SUPPLY-5 | L | `install.sh` rc-file PATH edit uses loose substring grep; `$SHELL`/`--install-dir` trusted unvalidated | `install.sh:116-128` |
| SEC-SUPPLY-6 | L | npm checksum manifest generated by re-hashing the published release (no independent build verification) | `.github/workflows/npm-publish.yml:157-198`; `npm/install.js:104-133` |
| SEC-SUPPLY-7 | L | `install.js` extracts zip via PowerShell `-Command` string interpolation (path not user-controlled today) | `npm/install.js:136-140` |

**Verified-clean:** `zip` crate has built-in zip-slip protection; npm installer refuses non-HTTPS, verifies SHA-256 before extract.

### D. ACP / IPC (stdio transport — no network surface)

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| SEC-ACP-1 | M | Session `cwd` from peer validated only for `is_absolute()` — no canonicalization/containment; becomes root for all file/shell tools | `acp/src/server.rs:156-163`; `acp/src/prompt.rs:55` |
| SEC-ACP-2 | M | Permission prompt title/content built from unsanitized model/tool strings → approval spoofing | `acp/src/permission.rs:122-180`; `acp/src/prompt.rs:313-323` |
| SEC-ACP-3 | M | `ManagedInteractivePermissionHandler` falls back to **Allow** on a poisoned lock (fail-open) | `core/src/lib.rs:2954-2972` (esp. 2965-2966) |
| SEC-ACP-4 | L | Permission option-ID mismatch: offered `reject_once` falls to `_ => Deny`; handled `reject_always` never offered (fail-safe but dead) | `acp/src/permission.rs:70-105` |

Note: ACP's `PermissionManager`/allowlist is never consulted on the ACP path (handler always returns `Ask`, fail-closed to client) — by design, but the `permission_manager` field is dead code there.

---

## PERFORMANCE FINDINGS

### E. TUI render & event loop (shared `render_app` runs in the live `cli/main.rs` loop)

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| PERF-TUI-1 | **C** | Full transcript markdown re-parsed every frame while streaming (both caches bypassed when `streaming`) | `render.rs:1437,1556-1558`; `messages/markdown.rs:23` |
| PERF-TUI-2 | **C** | `content_blocks()` deep-clones all message content; called per-message per-frame | `core/src/lib.rs:464-469`; `render.rs:1845`; `messages/mod.rs:369,385,423,651,861,1725` |
| PERF-TUI-3 | H | Two full-screen-buffer walks per frame (OSC8 URL regex scan + selectable-row String cache), even at idle | `osc8.rs:121-170`; `render.rs:798,811-842` |
| PERF-TUI-4 | H | No dirty flag → unconditional redraw. **Live loop polls at 16ms (~60fps)** burning idle CPU | `cli/src/main.rs:2497,2518` (dead `app.rs:7140,7158` has 50ms) |
| PERF-TUI-5 | H | Whole `Vec<RenderedLineItem>` deep-cloned out of cache on every cache hit | `render.rs:1455,1543,1565` |
| PERF-TUI-6 | M | `wrap_line` allocates `Vec<String>` twice (count + render) per frame | `prompt_input.rs:3473-3527`; `render.rs:501` |
| PERF-TUI-7 | M | `build_tool_names` + `build_transcript_turns` rebuilt from scratch every streaming frame | `render.rs:1468-1474`; `transcript_turn.rs:79-173` |
| PERF-TUI-8 | L | `app.diff_viewer.clone()` (full state) every frame while overlay open | `render.rs:577-578` |
| PERF-TUI-9 | L | Per-frame HashMap rebuild for mouse hit-test row maps | `render.rs:1158-1177` |

### F. Async, concurrency, networking

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| PERF-ASYNC-1 | H | Per-turn provider rebuild: blocking `auth.json` read + new `reqwest::Client` (drops TLS pool); provider even built twice (non-Anthropic path) | `query/src/lib.rs:1134-1161`; `api/src/registry.rs:156-322`; `core/src/auth_store.rs:40-46` |
| PERF-ASYNC-2 | H | 1.9MB bundled model snapshot re-parsed on each `ModelRegistry::new()` — per turn (temp reg) and per subagent | `api/src/model_registry.rs:591-620`; `query/src/lib.rs:1098-1106`; `query/src/agent_tool.rs:93-100` |
| PERF-ASYNC-3 | M | SSE parsing allocates `String` per chunk + `Vec` per chunk; MCP path `buffer.drain` is O(n²) | `api/src/lib.rs:913-944`; `providers/openai_compat.rs:818`; `mcp/src/lib.rs:516-533` |
| PERF-ASYNC-4 | M | Full `messages` deep-clone every turn to placeholder unsupported modalities | `query/src/lib.rs:1194-1225` |
| PERF-ASYNC-5 | M | Throwaway `reqwest::Client::new()` per request (Codex + ~7 core/mcp call sites) — no pool reuse | `api/src/lib.rs:635`; `mcp/src/rmcp_backend.rs:228,902,1010`; `core/{team_memory_sync,remote_session,device_code,oauth}.rs` |
| PERF-ASYNC-6 | M | Whole-body buffer + synchronous `serde_json` parse of full responses on runtime thread | `api/src/lib.rs:617-654,788`; `providers/openai_compat.rs:446,566,653` |
| PERF-ASYNC-7 | L | Background models.dev refresh uses blocking `std::fs::write` (1.9MB) + `std::fs::metadata` in async | `api/src/model_registry.rs:920-923` |
| PERF-ASYNC-8 | L | Per-turn session-memory `AnthropicClient` rebuild (gated, detached) | `query/src/lib.rs:1868-1876` |

**Verified-clean:** no lock-held-across-`.await` in mcp connection_manager / codex; `block_on` uses are on dedicated runtimes; live unbounded channels carry only low-volume control msgs.

### G. Memory & algorithms (storage / file tools)

| ID | Sev | Title | Location |
|----|-----|-------|----------|
| PERF-MEM-1 | **C** | `save_session` re-serializes entire session (`to_string_pretty`) incl. per-checkpoint full snapshots on every metadata mutation | `core/src/lib.rs:3359-3366,3410-3439` |
| PERF-MEM-2 | **C** | `list_sessions`/`search_sessions` deserialize every full session (all messages + snapshots) to read titles/dates | `core/src/lib.rs:3376-3398,3478-3499` |
| PERF-MEM-9 | M→(C) | `create_checkpoint` deep-clones entire message history into each snapshot (compounds MEM-1) | `core/src/lib.rs:3322-3332` |
| PERF-MEM-3 | H | Grep reads whole file + `Vec<&str>` of all lines per file, synchronously in async; `head_limit` checked after full scan | `tools/src/grep_tool.rs:252-264,333-347` |
| PERF-MEM-4 | H | `FileRead` loads entire (uncapped) file + collects all lines even for a small `offset/limit` window (OOM risk) | `tools/src/file_read.rs:113-153` |
| PERF-MEM-5 | H | `stats --all-projects` fully deserializes every transcript (two passes, full `Message`) to sum tokens/cost | `commands/src/stats.rs:282-336,518-557` |
| PERF-MEM-6 | M | `get_history` re-reads/parses entire `history.jsonl` + clones pending/skip sets per call; file unbounded | `core/src/prompt_history.rs:416-482` |
| PERF-MEM-7 | M | `expand_pasted_text_refs` rebuilds full string per ref via `format!` (O(R×size)) | `core/src/prompt_history.rs:533-553` |
| PERF-MEM-8 | M | `load_transcript`/tail scan do `.contains()` substring scans per line before parse; then clone every `Message` | `core/src/session_storage.rs:308-325,503-534,601-609` |
| PERF-MEM-10 | L | Glob `stat`s every match (blocking) for mtime sort, even beyond 250-cap | `tools/src/glob_tool.rs:119-127` |
| PERF-MEM-11 | L | ToolSearch recomputes `to_lowercase` of catalog per query | `tools/src/tool_search.rs:302-341` |
| PERF-MEM-12 | L | `stats::truncate` collects `Vec<char>` of whole string even when no truncation | `commands/src/stats.rs:613-624` |

---

## REMEDIATION BACKLOG (long-running goal, sequenced subprompts)

Each `SP-n` is a self-contained subprompt: run it, let it land with its verification gate, then move to the next. Ordered by (severity × leverage × dependency). Phases 0–2 are the "stop the bleeding" core; later phases are mop-up. Check boxes as completed.

### Phase 0 — Safety nets (do first; everything else rides on these)
- [x] **SP-0.1** Add a `write_secret_file(path, bytes)` helper in `core` that writes then `chmod 0600` (and `0700` dirs), reusing `accounts::set_user_only_perms`. Add unit tests asserting mode on Unix. *(enables SEC-CRED-2)* — `core/src/secret_file.rs` (sync + async, 3 tests).
- [x] **SP-0.2** Add regression tests for the Bash classifier covering chaining/prefix bypasses (`echo hi; rm -rf ~`, `true && rm -rf /`, `FOO=1 rm -rf ~`, `$(rm -rf ~)`) — they should FAIL now, documenting SEC-EXEC-1. *(TDD anchor for SP-3.1)* — 6 `#[ignore]`d anchor tests in `bash_classifier.rs`; 5 fail under `--ignored` today. SP-3.1 un-ignores them.
- [x] **SP-0.3** Add a `SessionMeta` golden test + a benchmark/asserting test that `list_sessions` does not deserialize message bodies (e.g. via a large fixture session + timing or a deserialize-counter). *(TDD anchor for SP-2.2)* — golden tests in `lib.rs::history::session_meta_golden_tests`: full parse fails on unreadable `messages`, slim parse recovers metadata. (`config_dir()` has no env override, so a slim-parse contract test was used instead of an fs/`$HOME`-mutating `list_sessions` test.)

### Phase 1 — Critical security: plugin trust model
- [ ] **SP-1.1** (SEC-SUPPLY-1) Stop auto-enabling project-local plugins. Add a per-project "trust this folder's plugins?" prompt (persist decision) or an explicit `settings.enabled_plugins` allow-list; gate hook execution behind it. Tests: untrusted project → hooks do not run.
- [ ] **SP-1.2** (SEC-SUPPLY-2) Enforce `check_plugin_capability` before running hooks (`shell`), spawning plugin MCP servers (`mcp`/`shell`), and LSP launch — deny-by-default. Tests: `capabilities:[]` plugin cannot run a hook/MCP.
- [ ] **SP-1.3** (SEC-EXEC-5) Plugin `ShellCommand`: pass user args as positional `argv` (or shell-quote each), never string-concat into `sh -c`. Test: `/cmd $(touch pwned)` does not create the file.

### Phase 2 — Critical performance: storage + streaming render
- [ ] **SP-2.1** (PERF-MEM-1 + MEM-9) Separate mutable session metadata (title/tags) from message body; stop persisting per-checkpoint `snapshot` (store `message_idx`, reconstruct on restore); use `to_string` not `to_string_pretty`. Verify `/tag` no longer rewrites the message body.
- [ ] **SP-2.2** (PERF-MEM-2) Introduce a slim `SessionMeta` deserialize (id/title/tags/timestamps/count/branch_from) for `list_sessions`/`search_sessions`/`/branch list`. Verify against SP-0.3 test.
- [ ] **SP-2.3** (PERF-TUI-1 + TUI-2) Cache completed-message rendered lines independently of streaming; only rebuild the live (last) turn each frame. Add `content_blocks_ref()` borrowing accessor and switch render-path callers off the cloning `content_blocks()`.
- [ ] **SP-2.4** (PERF-TUI-5) Return `Rc<[RenderedLineItem]>` (or `Rc<Vec<…>>`) from `render_message_items`; borrow instead of cloning the whole transcript per frame.

### Phase 3 — High security: exec safety + credentials + installers
- [ ] **SP-3.1** (SEC-EXEC-1) Rewrite `split_command`/`classify_bash_command` to split on `;`,`&&`,`||`,`|`,newline and command-substitution, strip leading `VAR=val`, classify each segment and take the max. Make SP-0.2 tests pass.
- [ ] **SP-3.2** (SEC-EXEC-2, SEC-EXEC-3) Route `EnterWorktree.post_create_command` and `REPL` bash/python through the classifier; show the literal command/code in the approval `details`.
- [ ] **SP-3.3** (SEC-CRED-2) Replace every secret-bearing `fs::write` (OAuth tokens, codex tokens, auth_store, mcp tokens) with `write_secret_file` from SP-0.1; `0700` the credential dirs.
- [ ] **SP-3.4** (SEC-CRED-1) Remove `?key=` from Google URLs (header already authenticates); ensure no full URL with a key is ever logged (redact `key=`).
- [ ] **SP-3.5** (SEC-SUPPLY-3) Publish signed `SHA256SUMS` per release; make `install.sh`/`install.ps1` download + verify hash (and signature) before extract/chmod. Reuse the npm workflow's manifest.

### Phase 4 — High performance: per-turn rebuilds + file tools + render cadence
- [ ] **SP-4.1** (PERF-ASYNC-2) Parse the bundled model snapshot once into a `OnceLock<Arc<…>>`; always thread `config.model_registry` to subagents; never build a temp registry inside the turn loop.
- [ ] **SP-4.2** (PERF-ASYNC-1 + ASYNC-5) Resolve the provider once per `run_query_loop` (cache by provider/base/key-hash); reuse `self.http`/a shared `OnceLock<reqwest::Client>` instead of `Client::new()` per request/turn.
- [ ] **SP-4.3** (PERF-MEM-4) `FileRead`: when `offset/limit` set, stream via `BufReader::lines()` and take the window; add a max-byte guard; `write!` instead of per-line `format!`.
- [ ] **SP-4.4** (PERF-MEM-3) `Grep`: buffered line reader, stop-at-first-match for `files_with_matches`, incremental `head_limit`, size cap, and run the walk in `spawn_blocking`.
- [ ] **SP-4.5** (PERF-TUI-4 + TUI-3) Add a `needs_redraw` dirty flag to the **live `cli/main.rs` loop**; only `draw()` when dirty; use the tight poll cadence only while streaming/animating; gate OSC8 scan + selectable-row cache behind dirty/lazy.

### Phase 5 — Medium security: containment, OAuth state, ACP
- [ ] **SP-5.1** (SEC-PATH-1 + PATH-2) Make Write/Edit/ApplyPatch resolve+canonicalize paths, pass them to `check_permission_for_path`, enforce workspace containment, and list real target paths in `details`.
- [ ] **SP-5.2** (SEC-EXEC-4) Bash permission dialog: surface classifier risk + the literal command (mirror the PowerShell tool); stop relying on the model-supplied `description`.
- [ ] **SP-5.3** (SEC-CRED-3 + CRED-4) Add `state` to MCP OAuth, validate it on callback; cap callback read size and validate `Host`.
- [ ] **SP-5.4** (SEC-ACP-1) Canonicalize + existence-check the ACP session `cwd`; ideally confine under an allowed root.
- [ ] **SP-5.5** (SEC-ACP-2, ACP-3, ACP-4) Sanitize/length-bound permission-prompt strings; make `ManagedInteractivePermissionHandler` fail **closed** on poisoned lock; fix the `reject_*` option-ID mismatch.

### Phase 6 — Medium performance: streaming, cloning, stats, history
- [ ] **SP-6.1** (PERF-ASYNC-3) Rework SSE parsers to a persistent byte accumulator with cursor advance (no per-chunk `String`/`Vec`, no `drain`).
- [ ] **SP-6.2** (PERF-ASYNC-4) Only clone messages that contain unsupported modality blocks; consider `Arc<Message>`/`Cow` for history.
- [ ] **SP-6.3** (PERF-ASYNC-6 + ASYNC-7) Offload large `serde_json` parses to `spawn_blocking`; switch models.dev cache write/metadata to `tokio::fs`.
- [ ] **SP-6.4** (PERF-MEM-5) `stats`: stream JSONL line-by-line into the accumulator with a slim struct; drop retained `Vec<TranscriptEntry>`.
- [ ] **SP-6.5** (PERF-MEM-6 + MEM-7 + MEM-8) `get_history`: iterate `.lines().rev()` lazily with early cap; single-pass `load_transcript` (parse once, filter tombstones in memory, `into_iter` not clone); single-buffer `expand_pasted_text_refs`. Consider history.jsonl rotation.
- [ ] **SP-6.6** (PERF-TUI-6 + TUI-7) Add allocation-free `wrap_line_count`; version-gate `build_tool_names`/`build_transcript_turns` cache; borrow `tool_names` instead of per-message clone.

### Phase 7 — Low / hardening / cleanup
- [ ] **SP-7.1** (SEC-CRED-6 + CRED-7) Manual redacting `Debug` (or `Redacted<T>`) for secret structs; OAuth error bodies only at `trace`.
- [ ] **SP-7.2** (SEC-CRED-5) Replace UUID-based PKCE/state with `getrandom`.
- [ ] **SP-7.3** (SEC-SUPPLY-4) Before exposing marketplace install: require HTTPS, mandatory hash, registry-host pin, install-time consent.
- [ ] **SP-7.4** (SEC-SUPPLY-5/6/7, SEC-PATH-3, SEC-EXEC-6) Installer rc-edit hardening + anchored PATH match; npm checksums from build artifacts/provenance; `install.js` unzip without `-Command` interpolation; Grep `follow_links(false)` (or canonicalize first); document `CLAUDE_STATUS_COMMAND` trust boundary.
- [ ] **SP-7.5** (PERF-TUI-8/9, PERF-MEM-10/11/12) Borrow diff-viewer state; reuse hit-test maps; Glob `stat` in `spawn_blocking`; precompute ToolSearch lowercase; byte-length fast-path in `stats::truncate`.
- [ ] **SP-7.6** Cleanup: delete the dead `app.rs::run()` loop (and its now-misleading 50ms/OSC8 analysis target) once SP-4.5 lands, so there's one event loop to reason about.

### Execution gate (every SP)
Run `cargo build` + `cargo test` for touched crates; for security SPs add/extend a test proving the exploit is closed; for perf SPs note the before/after (alloc count, file bytes written, or frame cost) in the commit. Commit signed (`-S`), one SP per commit, never on `main`.
