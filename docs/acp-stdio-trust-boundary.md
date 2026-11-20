# Stdio trust boundary for ACP: a launcher-supplied allowlist

Issue: [#383](https://github.com/OpenCoven/coven-code/issues/383), coordinated
with [epic #406](https://github.com/OpenCoven/coven-code/issues/406). Inspected
2026-11-18. Status: design note. Val chose the option, an orchestrator-supplied
allowlist; the points still open are under "Decisions for the maintainer". This
note changes no behavior and no code.

Sources, cited as `path:line` at these commits:

- coven-code `main` at `3fc38e5cecfd74030869a1158db3eb87a501e0fb`.
- OpenCoven/coven `main` at `29d44cd871175363cf43c937d6918d12c632f119`, for
  citations that start with `coven:`.
- `agent-client-protocol-schema` 0.13.0, the version locked in
  `src-rust/Cargo.lock:12-13`, for citations that start with `schema:`.

A path that starts with a crate name, such as `acp/src/server.rs`, is relative
to `src-rust/crates/`. A bare `server.rs`, `runtime.rs`, `prompt.rs` or
`permission.rs` is in `src-rust/crates/acp/src/`, a bare `claude_cli.rs` or
`claude_cli_policy.rs` is in `src-rust/crates/api/src/providers/`, a bare
`rmcp_backend.rs` is in `src-rust/crates/mcp/src/`, and a bare `main.rs` is
`src-rust/crates/cli/src/main.rs`. Any other bare file name repeats a full path
given earlier in the note.

Read with the [ACP disposition](acp-coverage-disposition.md), which names this
gate as open, and the "Engine contract" section of [COVEN.md](../COVEN.md).
Nothing here was run: every statement about behavior comes from reading the
code, and what was not read is marked.

## The problem

`coven-code acp` serves the Agent Client Protocol as newline-delimited JSON-RPC
on its own stdin and stdout (`run_acp_server`,
`src-rust/crates/acp/src/lib.rs:64-101`). Four parties are involved:

- The **launcher** starts the process. It chooses the arguments, the
  environment, the working directory and what stdin and stdout are connected
  to. It is an editor that read the registry entry
  (`src-rust/crates/acp/registry-template/agent.json`, `args: ["acp"]`), or
  Coven's `coven acp`, or any other program.
- The **client** is whatever writes requests on stdin and answers the engine's
  own requests. It is usually the launcher. It can also be something the
  launcher relays for: a remote front end, another agent, a script.
- The **user** owns the engine home: the settings file and the stored
  credentials.
- The **model** proposes tool calls during a prompt.

The engine does not tell the launcher and the client apart, and it does not
identify the client at all. So the question in #383, "what authorization the
launcher supplies", has one answer today: none that the engine can see. What
the client is then trusted with is listed below.

## What exists today

### Launch

- `acp` is a fast path. `main` compares the first argument and calls
  `run_acp_server()`; arguments after `acp` are not read
  (`src-rust/crates/cli/src/main.rs:507-510`). One exception: `--version` or
  `-V` anywhere on the command line is handled first and prints the version
  instead of starting the server (`main.rs:485-488`). Coven's contract says the
  subcommand "accepts no flags" (`coven:docs/ENGINE-CONTRACT.md:58-61`).
- The engine home, and so the settings file, is chosen by the environment:
  `COVEN_CODE_HOME`, then `COVEN_HOME` or `COVEN_PARENT=coven`, then the home
  directory (`resolve_config_home`,
  `src-rust/crates/core/src/lib.rs:1735-1777`, read from the environment at
  `1788-1794`). A managed home with no data falls back to a legacy
  `~/.coven-code` that has data (`effective_path`,
  `core/src/lib.rs:1720-1731`), and the home migration runs before the fast
  path (`main.rs:475`). ACP loads only that engine-global `settings.json`
  (`AgentRuntime::build`, `src-rust/crates/acp/src/runtime.rs:39-45`;
  `Settings::load_sync`, `core/src/lib.rs:1911-1919`). It does not use the
  hierarchical loader, so no project settings file and no shared
  `~/.coven/settings.json` are merged. Invalid settings stop the process before
  any reply is written (`src-rust/crates/cli/tests/acp_smoke.rs:205-211`).
- The process working directory becomes the runtime's `project_dir` and default
  query directory (`acp/src/lib.rs:69-72`, `runtime.rs:51`, `96-97`).
- Stdin may be a pipe, a named FIFO, a file or a terminal. The engine looks at
  the file type only to pick an I/O strategy (`acp/src/lib.rs:74-100`). It makes
  no peer check. Which other processes can reach the launcher's pipe is decided
  by the launcher and the operating system; this note does not determine that
  per platform.
- `coven acp` is a passthrough. Coven adds `COVEN_PARENT=coven`, forwards
  `COVEN_HOME` when set, appends the user's own arguments after `acp`, and on
  Unix replaces itself with the engine
  (`coven:crates/coven-cli/src/main.rs:890-893`, `2689`, `3243-3252`,
  `3254-3262`, `3272-3282`). It does not clear the environment and it supplies
  no policy for this surface.

### Requests the client can send

| Request | What the engine does with it | Check today |
|---|---|---|
| `initialize` | Stores the client's capabilities and answers with protocol version 1, `loadSession: false` and default prompt and MCP capabilities (`on_initialize`, `acp/src/server.rs:158-180`) | None. It is not required before `session/new`, and a repeat overwrites the stored capabilities (`handle_request`, `server.rs:76-112`; `166`) |
| `authenticate` | Parses the request and returns an empty result (`server.rs:82-87`) | None. It is a no-op by design |
| `session/new` `cwd` | Becomes the session's tool working directory, the query working directory and the directory tool and stop hooks run in (`on_new_session`, `server.rs:186-188`, `acp/src/prompt.rs:59`, `76-77`, `src-rust/crates/query/src/lib.rs:4081-4086`) | Absolute, resolves to an existing directory, UTF-8, no control characters; stored in canonical form (`validate_session_cwd`, `server.rs:220-241`). No containment: any directory the user can reach is accepted |
| `session/new` `mcpServers` | Nothing. A non-empty list logs a warning with the count (`server.rs:191-197`) | Not applicable: ignored |
| `session/load` | Refused with method-not-found (`server.rs:93-97`) | Not applicable |
| `session/prompt` | Runs a model turn with the stored credentials and the built-in tools (`on_prompt`, `server.rs:203-217`; `prompt::handle`, `prompt.rs:95-106`) | The session id must exist, and the prompt must render to non-empty text: text, a resource link or an embedded text resource (`server.rs:207-215`, `prompt.rs:35-38`, `149-167`) |
| Reply to `session/request_permission` | Allows or denies one tool call (`request_from_client`, `acp/src/permission.rs:89-97`) | Bound to the turn that asked (#404): cancellation, unknown selections, client errors and transport errors deny (`permission.rs:41-47`, `94-127`) |
| `session/cancel` | Cancels the session's current turn (`server.rs:129-139`) | None needed |

The schema gives `session/new` a list of MCP servers, and a stdio server is a
`command`, `args` and `env` (`schema:src/v1/agent.rs:946-947`, `2898-2906`). It
also says every agent must support that transport
(`schema:src/v1/agent.rs:2707-2711`). The engine's comments say per-session
servers will be routed later (`acp/src/lib.rs:34-37`, `runtime.rs:82-85`,
`server.rs:191-196`). Once they are, `session/new` is a direct request to start
a named program with chosen arguments and environment.

### What the engine starts in ACP mode

| Launch | Who names the program | When | Check today |
|---|---|---|---|
| Settings-owned MCP stdio servers | The user's engine-global settings, after `${VAR}` expansion from the engine's environment (`src-rust/crates/mcp/src/lib.rs:53-115`) | At startup, before the first request is read (`build_mcp_manager`, `runtime.rs:86`, `115-122`; `McpManager::connect_all`, `mcp/src/lib.rs:1065-1122`) | None. `Command::new(command)` with the configured arguments and added variables; the engine sets no working directory and clears no environment there (`RmcpClientBackend::connect_stdio`, `src-rust/crates/mcp/src/rmcp_backend.rs:114-137`) |
| Per-session MCP servers | The client | Never | Ignored |
| Tool commands, for example Bash | The model | During a prompt | For the tools that ask, a permission round trip to the client (`request_permission_inner`, `src-rust/crates/tools/src/lib.rs:403-416`). For Bash: permission, then the classifier's refusal of Critical commands, then the command in the session directory (`src-rust/crates/tools/src/pty_bash.rs:491-507`; Unix foreground spawn at `266-268`; background and Windows paths not traced). Not every tool asks: see fact 1 |
| Hooks | The user's engine-global settings | Tool hooks before and after a tool call (`query/src/lib.rs:4081-4086`, `4254-4259`), stop hooks (`3595-3600`, and a second, detached launch in `stop_hooks_with_full_behavior`, `1256-1309`), and post-turn hooks after an assistant turn (`fire_post_sampling_hooks`, `1184-1214`, called at `2764` and `3310`) | None. `sh -c <command>`, or `cmd /C` on Windows. Tool and stop hooks run in the session directory (`run_hooks`, `core/src/lib.rs:5557`, `5587-5593`); post-turn hooks set no directory, so they run where the engine was started |
| The `claude` binary | `COVEN_CODE_CLAUDE_BIN`, else the first `claude` file on `PATH`, else the bare name (`resolve_claude_binary`, `src-rust/crates/api/src/providers/claude_cli.rs:137-159`; `binary_or_default`, `695-700`) | Each model call, when the turn's provider is `anthropic` and the engine has no Anthropic credential: the startup client's credential is empty and the auth store holds no Anthropic key (`runtime.rs:54-61`, `query/src/lib.rs:2337-2349`; `runtime_provider_for_with_delegation`, `src-rust/crates/api/src/registry.rs:104-111`) | None from ACP. See fact 3 |

The table lists the launch sites read for this note. Not traced: the other
built-in tools, formatters and LSP servers.

Three facts about this list matter for the boundary.

1. **The permission prompt is answered by the party that sent the prompt.** In
   ACP a tool's permission request goes to the client whatever permission mode
   the settings name: the handler always asks (`AcpPermissionHandler`,
   `permission.rs:24-37`: `check_permission` returns `Ask`,
   `request_permission` forwards), and the engine keeps no record of an answer,
   so "allow always" is one allow (`tools/src/lib.rs:417-418`). Not every tool
   asks. The query loop has no permission gate of its own (`execute_tool`,
   `query/src/lib.rs:4297-4318`), and two tools that were read do not always
   ask. `Config` never does, and it can write `permission_mode` into the
   engine-global settings (`src-rust/crates/tools/src/config_tool.rs:65-173`).
   PowerShell skips the request for a Medium-risk command when the settings
   mode is `acceptEdits` or `bypassPermissions` and the model does not ask for
   confirmation (`src-rust/crates/tools/src/powershell.rs:156-183`). The
   remaining tools were not audited. Where the round trip happens, it protects
   the client's user from the model. It does not protect the engine's user from
   the client. A client that sends a prompt and approves the request that
   follows has the user's shell, in a directory of its choice, and spends the
   user's model credentials.
2. **MCP servers named in settings start because the engine started.** No
   request is needed, and no request field selects them. Their tools are not
   attached to the model's tool list in ACP (`runtime.rs:88-94`); the resource
   tools and `mcp__auth` can still reach the manager (`prompt.rs:68`,
   `src-rust/crates/tools/src/mcp_resources.rs:75-86`,
   `src-rust/crates/tools/src/mcp_auth_tool.rs:61-112`). `http` and `sse`
   servers connect at the same moment with any stored token
   (`mcp/src/lib.rs:1077-1082`).
3. **On the keyless Claude transport the permission bridge is not in the
   path.** The provider reports `tool_calling: false`, so the `claude` child
   runs its own tools and the engine's tool loop stays out
   (`claude_cli.rs:1216-1220`, `query/src/lib.rs:2392-2396`). The child
   receives the permission mode from the settings as `--permission-mode`
   (`query/src/lib.rs:2466`; `build_args`, `claude_cli.rs:304-329`;
   `cli_permission_mode`, `333-340`), after ACP has turned `plan` into
   `default` (`runtime.rs:46-50`), so a `plan` setting is widened on this
   transport. It receives Coven's policy when `COVEN_CLAUDE_POLICY` is set
   (`claude_cli.rs:731-744`). The spawn sets no working directory
   (`claude_cli.rs:741-748`), so the child runs in the directory the engine was
   started in, not the session's `cwd`. This path was read, not exercised.

### What a hostile or confused client can do

- It cannot name a program. No request field that names one is honored.
- When the engine runs the tools, it can run any shell command the classifier
  does not rate Critical, through a prompt and its own approval (fact 1). On
  the keyless transport the `claude` child decides instead (fact 3).
- It can point a session at any existing directory. Tool and stop hooks then
  run there, and the engine's tool calls start there.
- It can spend the stored model credentials with `session/prompt`. No other
  request reaches the provider. Stored MCP tokens are first used when the
  servers connect at startup; during a prompt the resource tools use those
  connections.
- With a shell it can edit the settings file that the next start reads, so the
  settings-owned roster and hooks are within its reach across restarts.
- It can do all of this without `initialize` or `authenticate`.

For an editor this is the intended shape: launcher, client and approver are one
program acting for the person at the keyboard. It stops being intended when the
client is not the launcher, when approvals are answered by a program, or when
per-session MCP servers are routed and a request can name the program to start.

## Options

**A. Status quo, documented.** Record that the stdio peer is trusted as the
user and stop. Costs nothing and breaks nothing. It leaves the session
directory unbounded, starts the user's whole MCP roster under any launcher, and
gives no rule for routing per-session servers, so that feature stays blocked or
ships as an unchecked launch request.

**B. Engine-side prompt for each launch.** Ask before each MCP server start and
each new session root. The only channel the engine has is
`session/request_permission`, which goes to the client: the party being
constrained. An orchestrator client answers it with code. Settings-owned
servers start before any request has been read, so they would have to be
deferred to the first session. A server launch is not a tool call, so the
request would overload `ToolCallUpdate`. Outbound requests are capped at 64 per
connection (`src-rust/crates/acp/src/connection.rs:118`, `182-185`). The engine
has no terminal of its own to ask on: stdout is the protocol and stderr is the
log (`acp/src/lib.rs:65-67`).

**C. A launcher-supplied allowlist.** The launcher states, when it starts the
engine, which directories sessions may use and which MCP programs may be
started. The engine reads it once, before it reads stdin, and requests can only
stay inside it. It needs no round trip, works with no human present, and gives
per-session MCP routing a rule to ship behind. It cannot stop a launcher that
supplies nothing, it does not see the content of an allowed program, and it
does not cover tool commands.

**D. A signed or capability-based grant from Coven.** Coven issues a signed
statement of the same allowlist, bound to a session or familiar, and the engine
verifies it against a key it trusts. It distinguishes "Coven authorized this"
from "something set a variable" and survives being passed through
intermediaries. It needs a trust anchor in the engine, key distribution,
expiry and revocation rules, a Coven release, an engine pin and a contract
addition, and no editor can produce one. A key held in the same user account
adds little against other processes of that user. Coven has an owner-root
signing key for its introduction policy
(`coven:crates/coven-cli/src/main.rs:1271-1277`); how it works, and whether it
could anchor such a grant, was not read. D carries C's document, so C is not
thrown away.

| Case | A | B | C | D |
|---|---|---|---|---|
| T1 the client names an MCP program (once routed) | Unchecked, or the feature stays off | Asked of the client itself | Started only if listed | As C, and the list is attested |
| T2 the client picks a directory outside the intended workspace | Accepted | Asked of the client itself | Refused | As C |
| T3 the client prompts and approves its own shell command | Possible | Possible | Possible | Possible |
| T4 settings-owned MCP servers start under an orchestrator | All start | Deferred, then asked of the client | Only listed ones start | As C |
| T5 no human at the client | No effect | Prompts are answered by code | Unaffected | Unaffected |
| T6 a launcher or engine that predates the mechanism | No effect | Old clients see a prompt shape they do not know | Ignored without an error; see "Default" | Old engines cannot verify; old Coven cannot issue |

No option closes T3. Closing it needs a sandbox around tool commands, which
exists only for headless Chat streams (`--chat-workspace-policy-v1`,
`cli/src/main.rs:282-284`, `633-652`) and is never installed on the `acp` path,
which returns before argument parsing (`main.rs:507-510`).

On the keyless Claude transport (fact 3), C and D cover less than the table
says. The `claude` child is not matched, it runs where the engine was started,
and by this repository's own description it reads its own settings and MCP
configuration unless `COVEN_CLAUDE_POLICY` says otherwise
(`src-rust/crates/api/src/providers/claude_cli_policy.rs:3-8`, `44-46`). T1, T2
and T4 are answered for the engine's own launches only.

## The chosen option: a launcher-supplied allowlist

The decision names an orchestrator. This note says "launcher" because the same
channel serves an editor, and the engine cannot tell the two apart.

### Where it comes from

One environment variable, `COVEN_ACP_ALLOWLIST`, holding a JSON object. The
engine reads it once in `run_acp_server`, before `AgentRuntime::build` (which
starts MCP servers) and before the first byte of stdin.

Why the environment and not a flag: there is a precedent with the same shape.
Coven hands the engine `COVEN_CLAUDE_POLICY`, a versioned JSON object that is
refused when it cannot be applied in full (`claude_cli_policy.rs:23-31`,
`58-83`), and that variable was added to the contract's Environment section
without a change to `contract_version: 1` (`coven:docs/ENGINE-CONTRACT.md:19`,
`73-76`, added in Coven commit `1c7174f3`). A flag would contradict the
contract's statement that `acp` accepts no flags. The name avoids
`COVEN_CODE_*`, which the contract calls engine-owned and says Coven never
overrides (`coven:docs/ENGINE-CONTRACT.md:77`); the variables Coven sets or
passes through are listed there with a `COVEN_` prefix (`68-76`). Any launcher
may set it, not only Coven.

Why this channel is trusted and later requests are not:

- The environment is fixed by whoever executes the engine, before the engine
  reads anything. That party can already run any program as the user, so
  honoring its statement grants nothing it did not have.
- Requests arrive afterwards on a channel the engine cannot attribute. There is
  no peer identity, `authenticate` does nothing, and `initialize` is neither
  required first nor limited to once. A field in `initialize` or `session/new`
  would be supplied by the channel it is meant to constrain, and a launcher that
  relays requests could not vouch for it.
- So the rule is: the launch-time allowlist is the ceiling, and a request can
  only ask for something inside it.

The allowlist is not a defense against a hostile launcher or against another
process of the same user. The value is inherited by the engine's children, so
it must not contain secrets.

### The document

```json
{
  "version": 1,
  "sessionRoots": ["/work/project"],
  "mcpServers": [
    {
      "command": "/opt/tools/bin/example-mcp",
      "args": ["--stdio"],
      "env": ["EXAMPLE_TOKEN"]
    }
  ]
}
```

- `version` is read first. Any value other than 1 makes the document unusable.
- Unknown fields make it unusable, as in `claude_cli_policy.rs:27-31`: a field
  this engine does not know is a rule it cannot apply.
- `sessionRoots` and `mcpServers` are both required. An empty list allows
  nothing of that kind. Each list holds at most 128 entries, the bound the
  headless workspace policy uses
  (`src-rust/crates/tools/src/workspace_scope.rs:63-64`).
- A root is an absolute path to an existing directory. An entry's `command` is
  an absolute path to an existing file. `args` is the exact argument list.
  `env` is a list of variable names.

### What it matches on

**Working directory.** `session/new` runs today's `validate_session_cwd` first.
The canonical `cwd` must then equal a canonical root or lie beneath one,
compared by path component, so `/work/project-two` is not under
`/work/project`. Roots are made canonical once, at startup. A symlink inside a
root that leads outside is refused because the canonical `cwd` is outside; a
symlink elsewhere that leads inside is accepted because it is inside. The roots
bound where a session starts. They are not a sandbox: an approved Bash command
can change directory and file tools accept absolute paths
(`tools/src/lib.rs:368-375`).

**Executable.** Both sides are compared as canonical paths.

- The entry's `command` is made canonical at startup. If it does not resolve,
  the document is unusable.
- A client-supplied command, once routing exists, must be absolute. It is made
  canonical and compared. A relative path or a bare name is refused: there is
  no `PATH` lookup for a client's request.
- A settings-owned command is taken after `${VAR}` expansion. An absolute path
  is made canonical. A bare name is looked up in the engine's own `PATH`, first
  entry that is a file, as `resolve_claude_binary` does
  (`claude_cli.rs:144-158`), with one addition that function does not have:
  empty and relative `PATH` entries are skipped. The result is made canonical.
  A relative path with a separator never matches. Neither does a bare name
  whose settings entry sets its own `PATH`, because the entry changes the
  child's `PATH` (`rmcp_backend.rs:132`) and the lookup might then not be the
  engine's; how the standard library resolves that case was not tested.
- The engine then starts the canonical path it checked, not the string it was
  given, so the check and the launch cannot disagree about `PATH` or a symlink.
  A program that behaves by its invoked name will see a different one; on Unix
  the requested name can be kept as `argv[0]`. Windows lookup rules (`PATHEXT`)
  are not designed here.
- This pins a path, not content. Replacing the file, or allowing a runner such
  as `npx` that fetches its target, is outside what the allowlist can see.

**Arguments.** Exact match: same strings, same order, same count. No patterns
in version 1. Allowing an interpreter with any arguments would allow anything.

**Environment.** Every variable name the request or the settings entry sets
must be in the entry's `env` list. Values are not matched, so the document
never needs to hold a secret. The allowlist does not restrict the variables the
child inherits from the engine; the engine clears none today.

**Scope.** With an allowlist present, the same check applies to settings-owned
stdio servers at startup and to per-session stdio servers once they are routed.
`http` and `sse` servers are not process launches and are not matched. Tool
commands, hooks, the `claude` binary and what it starts, and helper programs
such as `git` are not matched either; facts 1 and 3 above stand, and the trust
statement for this boundary has to say so. One thing the filter does add: a
settings file edited through a tool cannot bring in a server the launcher did
not list.

The check needs a small change in the mcp crate. ACP reaches the spawn through
`McpManager::connect_all`, `McpClient::connect`, `connect_inner` and
`RmcpClientBackend::connect_stdio` (`mcp/src/lib.rs:1082`, `610-631`;
`rmcp_backend.rs:114`). Expansion happens inside `connect_all`
(`mcp/src/lib.rs:1073`), so a filter applied in `build_mcp_manager`
(`runtime.rs:115-122`) would see unexpanded commands, and the manager's failed
list is private with a getter only (`mcp/src/lib.rs:1028`, `1240`). Either
`connect_all` takes the filter, or ACP expands, filters and then connects. The
manager's one other connect site is the plugin path (`mcp/src/lib.rs:1159`),
which ACP startup does not use: it calls `connect_all` directly
(`runtime.rs:119`). No reconnect method was found on `McpManager`. The mcp
crate has two other routes to the same spawn, `McpConnectionManager`
(`src-rust/crates/mcp/src/connection_manager.rs:112-148`) and
`McpClient::connect_stdio` (`mcp/src/lib.rs:707-709`); neither is referenced
from the acp crate. The implementer should confirm that before relying on a
single check.

### Default when no allowlist is supplied

| State of the variable | Behavior |
|---|---|
| Not set | Today's behavior, unchanged: any valid `cwd`, every settings-owned server, `mcpServers` ignored |
| Set and usable | The checks above |
| Set and unusable: empty, not JSON, wrong version, unknown field, a missing required field, a list over the bound, a path that is not absolute or does not resolve | The engine does not start. Non-zero exit, a message on stderr, nothing on stdout, as for invalid settings today |

The honest consequence of each default:

- **Fail open** keeps every existing client working: an editor that launches
  from the registry entry and `coven acp` both supply nothing. It keeps today's
  gap for those launchers (T2 and T4). It opens no new one, provided per-session
  MCP routing is implemented so that it is active only when an allowlist is
  present. Without an allowlist a client-named server is never started, exactly
  as today.
- **Fail closed** (no allowlist, no sessions) closes the gap for everyone and
  breaks every existing editor client and the `coven acp` passthrough until
  each supplies an allowlist. It changes what an existing caller of contract
  surface 10 gets, so it needs a contract version bump.

Recommendation: fail open. The maintainer must decide whether that is the
permanent default or a first step with a stated release for the switch.

Under fail open, an engine that predates the mechanism ignores the variable
without an error. A launcher that depends on enforcement has to gate on the
engine version, which it already has from `coven-code --version` and from
`agentInfo.version` in the `initialize` result (`server.rs:168`).

### How refusals are reported

No new method, notification or result field.

- `session/new` with a `cwd` outside every root: JSON-RPC error `-32602` with
  `data: {"reason": "cwd is outside the launch allowlist"}`. This is the
  envelope and the `data.reason` key that today's `cwd` refusals use
  (`server.rs:221-225`). The reason is a fixed string and the path is not
  echoed, as in today's `cwd` refusals (`server.rs:226-239`).
- `session/new` naming a stdio server that is not allowed, once routing exists:
  the whole request is refused the same way, with reason
  `"mcp server is not in the launch allowlist"`. A session that silently lacks
  a server the client asked for is worse than no session. Until routing exists,
  `mcpServers` stays ignored with today's warning.
- A settings-owned server that does not match: not started, and logged.
  Recording it in the manager's failed list, where an unsupported transport
  goes today (`mcp/src/lib.rs:1108-1118`), with a fixed reason, is possible only
  if the filter lives inside `connect_all`. No request has been read at that
  point, so nothing is sent.
- An unusable document: the process exits before writing to stdout.
- `-32000` is not used. The engine already uses it for two capacity errors
  (`src-rust/crates/acp/src/admission.rs:52`, `prompt.rs:127-130`), and the
  schema names it `AuthRequired` (`schema:src/v1/error.rs:196`).

### Engine contract

COVEN.md says the flags, output formats and exit codes in Coven's contract may
not change without a contract version bump and a coordinated `engine.lock`
update. Measured against that:

- Environment variable, fail open: no listed surface changes. `acp` still takes
  no flags of its own, and a caller that sets nothing gets today's bytes. No
  bump is needed for the engine to accept the variable.
- For Coven to rely on it, Coven has to list the variable in its contract and
  pin an engine that enforces it. That is an addition on Coven's side, as
  `COVEN_CLAUDE_POLICY` was. Whether Coven treats it as a bump is Coven's
  decision; the contract's own rule covers breaking changes only
  (`coven:docs/ENGINE-CONTRACT.md:13-15`).
- A flag instead of a variable changes the text of surface 10.
- Fail closed is a breaking change to surface 10 and needs a bump.

Contract bumps are out of scope for this run, so only the first line can be
implemented under it.

### First slice

1. Engine: parse and validate `COVEN_ACP_ALLOWLIST`; refuse to start when it is
   unusable; check `session/new` `cwd`; filter settings-owned stdio servers.
   Nothing changes when the variable is not set.
2. Later, separately: route per-session stdio servers, active only under an
   allowlist.
3. Coven, in its own repository: decide whether `coven acp` sets the variable,
   and list it in the contract.

## Test plan

The issue asks for a native editor client and an orchestrator run in addition
to automated tests. None of this has been run.

- **Unit.** Document parsing: not set, empty, not JSON, version 2, unknown
  field, relative root, missing root, missing command, more than 128 entries.
  Root containment: equal, child, sibling with a shared prefix, `..`, a symlink
  that leaves a root, a symlink that enters one, a root that is itself a
  symlink. Command matching: symlinked command, bare name through `PATH`,
  empty and relative `PATH` entries, a settings entry that sets its own `PATH`,
  argument order, extra and missing arguments, a variable name outside the
  list. Each new test is shown failing first.
- **Scripted ACP client**, in `src-rust/crates/cli/tests/acp_smoke.rs` against
  the real binary with a scratch home: with the variable not set, the existing
  results are unchanged and the `initialize` result is identical with and
  without it; a `cwd` inside a root returns a session id; a `cwd` outside
  returns `-32602` with the reason; a symlink that leaves a root is refused; a
  settings-owned stdio server that writes a marker file starts only when it is
  listed; an unusable document gives a non-zero exit, a stderr message and an
  empty stdout. The harness fixes the child's environment and takes no
  environment parameter today (`acp_smoke.rs:48-87`), so it needs one. The
  variable joins its removal list and the scrubbed list in
  `src-rust/crates/cli/tests/engine_contract.rs:52-70`.
- **Native editor client.** One editor that launches the engine from the
  registry entry, against a release build. Without the variable, a session
  works as before, which is the compatibility proof for fail open. With the
  variable set in the editor's agent configuration, a session inside a root
  works and one outside shows the editor's error for a refused `session/new`.
  Record the editor and its version, the engine commit and the binary hash.
- **Orchestrator run.** `coven acp` against the same release build, with the
  variable exported by the caller. Coven does not clear the environment, so
  this needs no Coven change. Then again without the variable. Record the Coven
  and engine commits.
- **Platforms.** Canonical paths on Windows are proven on the hosted Windows
  job, not claimed from macOS.

## Decisions for the maintainer

1. Default with no allowlist: **fail open** (recommended; today's behavior, and
   client-named MCP servers stay unrouted), or fail closed, which breaks
   registry-launched editors and `coven acp` and needs a contract bump. If fail
   open is a first step, name the release that switches.
2. Channel and name: **the environment variable `COVEN_ACP_ALLOWLIST`**
   (recommended), or a flag on `acp`, which changes contract surface 10.
3. Settings-owned MCP servers under an allowlist: **start only the ones that
   match** (recommended), or leave them outside the allowlist.
4. Environment matching: **names only** (recommended; keeps secrets out of an
   inherited variable), or names and values.
5. Tool commands and the keyless Claude transport: **leave them to the
   permission round trip and to `COVEN_CLAUDE_POLICY`, and record that the
   stdio peer is trusted as the user** (recommended for version 1), or add a
   tool-name list to the allowlist, or bring the headless workspace scope to
   ACP.
6. Enforcement signal: **none in the protocol; launchers gate on the engine
   version** (recommended under this contract version), or an additive `_meta`
   entry in the `initialize` result.
7. Coven: whether `coven acp` sets an allowlist itself, and when the variable
   is added to Coven's contract. Both are changes in Coven's repository.

## Out of scope

- Any code, and any contract version bump, in this run.
- Routing per-session `mcpServers`. The allowlist is its precondition.
- A sandbox for tool commands, matching shell strings, and extending the
  headless workspace scope to ACP.
- Option D, signed grants.
- An allowlist for `http` and `sse` MCP servers and their stored tokens.
- Peer identity for the stdio channel, and giving `authenticate` a meaning.
- Hooks, formatters, LSP servers, the `claude` binary and helper programs.
- Pinning executable content by hash.
- Findings made while reading, none fixed here:
  - `Config` writes `permission_mode` and other keys to the engine-global
    settings with no permission request (`config_tool.rs:65-173`).
  - On the keyless transport the `claude` child does not run in the session's
    `cwd`, and a `plan` setting reaches it as `default`.
  - `-32000` carries both capacity errors while the schema names it
    `AuthRequired`.
  - `initialize` is not required before `session/new`.
- The native editor and orchestrator runs themselves.
