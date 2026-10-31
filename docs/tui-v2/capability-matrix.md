# TUI v2 capability matrix

Source inspection: 2026-10-29, for [#407](https://github.com/OpenCoven/coven-code/issues/407).
Read with the [design](../superpowers/specs/2026-10-28-tui-v2-design.md) and
[compatibility matrix](compatibility-matrix.md).

## Evidence and revisions

`S` means implementation inspected and named tests/fixtures located; only the
suites listed in the compatibility matrix ledger are claimed as executed. `P` means proposed presentation or adapter. `U` means unsupported
or unqualified for the specified terminal operation; it does not mean the entire
producer is absent. No row is fresh live-integration acceptance. Owner names are
architectural domains, not newly assigned people.

| Repository | Immutable inspected revision | Inspected authority/interface |
|---|---|---|
| OpenCoven/coven-code | `87239e77acfd42dcbd2492c0ba36817626124549` | [engine/terminal source](https://github.com/OpenCoven/coven-code/blob/87239e77acfd42dcbd2492c0ba36817626124549/AGENTS.md) |
| OpenCoven/coven | `6b5caaecf98fd0a7cd8ee62878979161e697079a` | [coven.daemon.v1; ENGINE-CONTRACT.md v1](https://github.com/OpenCoven/coven/blob/6b5caaecf98fd0a7cd8ee62878979161e697079a/docs/API-CONTRACT.md) |
| OpenCoven/psyche | `1e47b40dbbb9db21962b4884afe49763321f04b0` | [typed CovenPort and canonical lifecycle contracts](https://github.com/OpenCoven/psyche/blob/1e47b40dbbb9db21962b4884afe49763321f04b0/crates/psyche-coven/src/port.rs) |
| OpenCoven/coven-threads | `659beca959d3459d48ff63282bbd9112256fb9c4` | [protected authorization model](https://github.com/OpenCoven/coven-threads/blob/659beca959d3459d48ff63282bbd9112256fb9c4/docs/authority-model.md) |
| OpenCoven/familiar-contract | `93f486cf9e7b66f7f3a49131fbcfd003dd9da8ac` | [current normative RFC-0001 v0.7.0 (Draft), including embodiment_binding.v1 §10.2](https://github.com/OpenCoven/familiar-contract/blob/93f486cf9e7b66f7f3a49131fbcfd003dd9da8ac/rfcs/RFC-0001-familiar-contract.md) |
| OpenCoven/sdk | `89685543a62008533237478c31ecce565798ddf6` | [typed Coven transport/client; no implied universal memory/task API](https://github.com/OpenCoven/sdk/blob/89685543a62008533237478c31ecce565798ddf6/packages/coven/src/client.ts) |
| OpenCoven/coven-memory | `eb542a598c93775f9d411c03de22a894c17e8527` | [read DTO validation; read-only client](https://github.com/OpenCoven/coven-memory/blob/eb542a598c93775f9d411c03de22a894c17e8527/src/server/memory-contract.ts) |
| OpenCoven/brand | `798e2f5f2f1a69f5d156ccc0a9aafc4f7da55fc8` | [stable web profile 1.0.0; canonical role values](https://github.com/OpenCoven/brand/blob/798e2f5f2f1a69f5d156ccc0a9aafc4f7da55fc8/web/profile.json) |
| OpenCoven/ui | `141a407aeeb73a42837524183de0515f87ff3c45` | [portable interaction semantics, not product authority](https://github.com/OpenCoven/ui/blob/141a407aeeb73a42837524183de0515f87ff3c45/contracts/web-interactions.v1.json) |

Coven Code engine baseline: `87239e7`. Unmerged engine PRs are not part of this
baseline. `src-rust/.claude/CLAUDE.md` is referenced by root instructions but absent
from the tracked tree at this revision; root `AGENTS.md` and `CLAUDE.md` apply.
A missing README is not used as evidence of an absent producer: Psyche was
located through its actual crates and `docs/ARCHITECTURE.md`/`docs/CLI.md`.

## Dispatch and evidence keys

- `G`: `commands/tests/slash_command_registry.rs` tests
  `slash_command_registry_matches_golden` and `generated_command_reference_is_current`;
  catalog/routing unit tests include `active_catalog_children_have_a_command_or_tui_route`,
  `canonical_parent_subcommands_delegate_to_existing_handlers`,
  `every_retired_path_and_alias_rejects_execution`. These prove catalog/dispatch shape,
  not every command's side effects. All paths in this paragraph are under `src-rust/crates/`.
- `A`: `tui/src/app.rs` tests `canonical_parent_subcommands_open_their_tui_surfaces`,
  `argument_forms_reach_command_layer`, and `removed_roots_do_not_open_or_mutate_tui_surfaces`.
- `D`: `commands/src/lib.rs::coven_command_integrates_with_daemon_mock` and
  `core/src/coven_daemon.rs` request/error-envelope fixtures. Mock IPC is not a live daemon.
- Specific tests below are additional source evidence. Where a row has only G/A,
  full behavior/PTY coverage is unqualified and required in the owning implementation issue.
  Parent-group behavior tests listed on child rows do not independently qualify
  those children; the applicable G/A routing coverage remains their minimum evidence.

Actual dispatch is catalog preflight -> optional App interception -> command handler
and CLI `CommandResult` handling. Some actions need both UI and command work.
`UserMessage` requests model work; it is not a synchronous successful operation.
`ConfigChange` and `GoalUserMessage` are not permission grants. No removal is proposed.

## Complete current catalog disposition

Every active/advanced path in the checked-in registry golden appears below,
including aliases. All are **preserve; adapt presentation** (`S` implementation,
`P` v2 presentation). Grouped children delegate to retained implementations even
when the old root was removed. Source links name the real dispatch owner; the CLI
and App may additionally process the result. Default build has no steering rows;
feature-gated steering is covered separately below.

| Canonical command | Accepted aliases | Implementation | Test evidence | Authority/effect owner | Proposed v2 presentation |
|---|---|---|---|---|---|
| `/account` | `-` | [AccountCommand](../../src-rust/crates/commands/src/grouped_commands.rs#L27) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account connect` | `-` | [ConnectCommand](../../src-rust/crates/commands/src/lib.rs#L8712) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account dedupe` | `-` | [AccountsCommand](../../src-rust/crates/commands/src/lib.rs#L3104) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account list` | `-` | [AccountsCommand](../../src-rust/crates/commands/src/lib.rs#L3104) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account login` | `-` | [LoginCommand](../../src-rust/crates/commands/src/lib.rs#L2950) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account logout` | `-` | [LogoutCommand](../../src-rust/crates/commands/src/lib.rs#L3017) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account providers` | `-` | [ProvidersCommand](../../src-rust/crates/commands/src/lib.rs#L8643) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account refresh` | `-` | [RefreshCommand](../../src-rust/crates/commands/src/lib.rs#L3238) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/account switch` | `-` | [SwitchCommand](../../src-rust/crates/commands/src/lib.rs#L3186) | G; grouped_parents_delegate_and_explain_unknown_children | engine account/OAuth owner | existing account picker |
| `/clear` | `c,reset,new` | [ClearCommand](../../src-rust/crates/commands/src/lib.rs#L579) | G; test_clear_command_returns_clear_conversation | engine conversation | clear existing session view |
| `/commit` | `-` | [CommitCommand](../../src-rust/crates/commands/src/lib.rs#L6102) | G; llm_prompt_utf8.rs | query/tool permission and Git owner | explicit model-request action |
| `/compact` | `-` | [CompactCommand](../../src-rust/crates/commands/src/lib.rs#L598) | G | query/context owner | transcript compaction boundary |
| `/config` | `settings` | [ConfigCommand](../../src-rust/crates/commands/src/lib.rs#L788) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/config advisor` | `-` | [AdvisorCommand](../../src-rust/crates/commands/src/lib.rs#L7572) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/config import` | `-` | [ImportConfigCommand](../../src-rust/crates/commands/src/lib.rs#L3944) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/config keybindings` | `-` | [KeybindingsCommand](../../src-rust/crates/commands/src/lib.rs#L1282) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/config output-style` | `-` | [OutputStyleCommand](../../src-rust/crates/commands/src/lib.rs#L1217) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/config splash` | `-` | [SplashCommand](../../src-rust/crates/commands/src/lib.rs#L1108) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/config theme` | `-` | [ThemeCommand](../../src-rust/crates/commands/src/lib.rs#L1173) | G/A; ui_settings_mutation.rs | engine settings | existing settings/pickers |
| `/coven` | `-` | [CovenCommand](../../src-rust/crates/commands/src/lib.rs#L10234) | G/D | daemon; local GoalStore for goal | scoped daemon actions/status |
| `/coven goal` | `-` | [GoalCommand](../../src-rust/crates/commands/src/lib.rs#L1649) | G; goal_budget.rs; goal_clear.rs; goal_reads.rs; CLI goal_dispatch_tests.rs | daemon; local GoalStore for goal | scoped daemon actions/status |
| `/diff` | `-` | [DiffCommand](../../src-rust/crates/commands/src/lib.rs#L1552) | G; diff_utf8.rs; tui/tests/diff_viewer.rs | Git/workspace read owner | single diff inspector |
| `/effort` | `-` | [EffortCommand](../../src-rust/crates/commands/src/lib.rs#L6052) | G/A; account_thinking_effort_absorb_their_clusters | engine/provider config | existing effort control |
| `/effort fast` | `-` | [FastCommand](../../src-rust/crates/commands/src/lib.rs#L7678) | G/A; account_thinking_effort_absorb_their_clusters | engine/provider config | existing effort control |
| `/exit` | `quit,q` | [ExitCommand](../../src-rust/crates/commands/src/lib.rs#L717) | G; test_exit_command_returns_exit | CLI/terminal lifetime owner | existing quit path |
| `/export` | `-` | [ExportCommand](../../src-rust/crates/commands/src/lib.rs#L5372) | G; session_and_export_absorb_their_clusters; share_temp.rs | engine export/clipboard; external sharing only on request | explicit export/copy/share |
| `/export copy` | `-` | [CopyCommand](../../src-rust/crates/commands/src/lib.rs#L6271) | G; session_and_export_absorb_their_clusters; share_temp.rs | engine export/clipboard; external sharing only on request | explicit export/copy/share |
| `/export share` | `-` | [ShareCommand](../../src-rust/crates/commands/src/lib.rs#L5534) | G; session_and_export_absorb_their_clusters; share_temp.rs | engine export/clipboard; external sharing only on request | explicit export/copy/share |
| `/familiar` | `familiars,agent` | [FamiliarCommand](../../src-rust/crates/commands/src/lib.rs#L9042) | G/A; familiar_switch_reports_access_tier_and_persists | engine persona/access tier; daemon roster when running | scoped familiar navigation |
| `/familiar handoff` | `-` | [HandoffCommand](../../src-rust/crates/commands/src/lib.rs#L9003) | G/A; familiar_switch_reports_access_tier_and_persists | engine persona/access tier; daemon roster when running | App::intercept_handoff creates a daemon session; command result alone is Silent |
| `/familiar incant` | `-` | [IncantCommand](../../src-rust/crates/commands/src/lib.rs#L3278) | G/A; familiar_switch_reports_access_tier_and_persists | engine persona/access tier; daemon roster when running | scoped familiar navigation |
| `/familiar list` | `-` | [AgentsCommand](../../src-rust/crates/commands/src/named_commands.rs#L49) | G/A; familiar_switch_reports_access_tier_and_persists | engine persona/access tier; daemon roster when running | scoped familiar navigation |
| `/familiar managed` | `-` | [ManagedAgentsCommand](../../src-rust/crates/commands/src/lib.rs#L9216) | G/A; familiar_switch_reports_access_tier_and_persists | engine persona/access tier; daemon roster when running | scoped familiar navigation |
| `/familiar whisper` | `-` | [BtwCommand](../../src-rust/crates/commands/src/lib.rs#L7996) | G/A; familiar_switch_reports_access_tier_and_persists | engine persona/access tier; daemon roster when running | scoped familiar navigation |
| `/find` | `-` | [App::intercept_slash_command_with_args](../../src-rust/crates/tui/src/app.rs#L2109) | A; reader_search_enters_query_and_repeats_to_the_next_match | TUI transcript view | existing transcript search |
| `/help` | `h,?` | [HelpCommand](../../src-rust/crates/commands/src/lib.rs#L504) | G; help_resolves_grouped_paths_and_preserves_feedback | catalog; explicit feedback owner | existing help/palette |
| `/help feedback` | `-` | [BugCommand](../../src-rust/crates/commands/src/lib.rs#L2094) | G; help_resolves_grouped_paths_and_preserves_feedback | catalog; explicit feedback owner | existing help/palette |
| `/init` | `-` | [InitCommand](../../src-rust/crates/commands/src/lib.rs#L3327) | G | workspace instruction-file owner | explicit local initialization |
| `/integrations` | `-` | [IntegrationsCommand](../../src-rust/crates/commands/src/grouped_commands.rs#L73) | G; plugin_mcp_ownership.rs | hooks/MCP/plugin/browser owners | existing integration controls |
| `/integrations chrome` | `-` | [ChromeCommand](../../src-rust/crates/commands/src/lib.rs#L6809) | G; plugin_mcp_ownership.rs | hooks/MCP/plugin/browser owners | existing integration controls |
| `/integrations hooks` | `-` | [HooksCommand](../../src-rust/crates/commands/src/lib.rs#L3970) | G; plugin_mcp_ownership.rs | hooks/MCP/plugin/browser owners | existing integration controls |
| `/integrations mcp` | `-` | [McpCommand](../../src-rust/crates/commands/src/lib.rs#L4017) | G; plugin_mcp_ownership.rs | hooks/MCP/plugin/browser owners | existing integration controls |
| `/learn` | `scribe` | [LearnCommand](../../src-rust/crates/commands/src/lib.rs#L3361) | G; test_learn_resolves_and_emits_skill_prompt | query/skill owner | explicit skill-generation request |
| `/memory` | `-` | [MemoryCommand](../../src-rust/crates/commands/src/lib.rs#L1880) | G; memory_display.rs | engine local AGENTS.md files | label Local instructions |
| `/model` | `-` | [ModelCommand](../../src-rust/crates/commands/src/lib.rs#L736) | G/A; test_model_slash_command_opens_picker | engine/provider owner | existing model picker |
| `/permissions` | `-` | [PermissionsCommand](../../src-rust/crates/commands/src/lib.rs#L4757) | G/A; config_review_status_absorb_their_clusters | engine tool policy/sandbox owner | exact policy settings/review |
| `/permissions sandbox` | `-` | [SandboxToggleCommand](../../src-rust/crates/commands/src/lib.rs#L8039) | G/A; config_review_status_absorb_their_clusters | engine tool policy/sandbox owner | exact policy settings/review |
| `/plan` | `-` | [PlanCommand](../../src-rust/crates/commands/src/lib.rs#L4903) | G | engine permission mode/query owner | plan preview; no new execution authority |
| `/plugin` | `plugins` | [PluginCommand](../../src-rust/crates/commands/src/lib.rs#L2230) | G; plugin_capabilities.rs; plugin_trust.rs; plugin_persistence.rs | plugin trust/config/MCP owner | existing plugin surface |
| `/plugin reload` | `reload,plugin refresh,plugins reload,plugins refresh` | [ReloadPluginsCommand](../../src-rust/crates/commands/src/lib.rs#L2395) | G; plugin_capabilities.rs; plugin_trust.rs; plugin_persistence.rs | plugin trust/config/MCP owner | existing plugin surface |
| `/review` | `-` | [ReviewCommand](../../src-rust/crates/commands/src/lib.rs#L3573) | G; review_citation_validation_flags_unknown_ids; pr_comments.rs | query/review/GitHub owner | review artifacts, scope and citations |
| `/review pr-comments` | `-` | [PrCommentsCommand](../../src-rust/crates/commands/src/named_commands.rs#L682) | G; review_citation_validation_flags_unknown_ids; pr_comments.rs | query/review/GitHub owner | review artifacts, scope and citations |
| `/review security` | `-` | [SecurityReviewCommand](../../src-rust/crates/commands/src/lib.rs#L7408) | G; review_citation_validation_flags_unknown_ids; pr_comments.rs | query/review/GitHub owner | review artifacts, scope and citations |
| `/review ultra` | `-` | [UltrareviewCommand](../../src-rust/crates/commands/src/lib.rs#L8205) | G; review_citation_validation_flags_unknown_ids; pr_comments.rs | query/review/GitHub owner | review artifacts, scope and citations |
| `/rewind` | `-` | [RewindCommand](../../src-rust/crates/commands/src/lib.rs#L5793) | G/A; rewind_with_args_routes_to_file_rollback_not_overlay | engine transcript/checkpoint/filesystem owner | explicit preview and confirmation |
| `/rewind diff` | `-` | [RewindCommand](../../src-rust/crates/commands/src/lib.rs#L5793) | G/A; rewind_with_args_routes_to_file_rollback_not_overlay | engine transcript/checkpoint/filesystem owner | explicit preview and confirmation |
| `/rewind list` | `-` | [RewindCommand](../../src-rust/crates/commands/src/lib.rs#L5793) | G/A; rewind_with_args_routes_to_file_rollback_not_overlay | engine transcript/checkpoint/filesystem owner | explicit preview and confirmation |
| `/search` | `-` | [SearchCommand](../../src-rust/crates/commands/src/lib.rs#L7921) | G; search_date.rs; session_id_display.rs | engine session-history owner | scoped session search |
| `/session` | `remote` | [SessionCommand](../../src-rust/crates/commands/src/lib.rs#L4962) | G/A; session_and_export_absorb_their_clusters | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/session add-dir` | `-` | [AddDirCommand](../../src-rust/crates/commands/src/named_commands.rs#L326) | G; add_dir_save.rs | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/session branch` | `-` | [BranchCommand](../../src-rust/crates/commands/src/named_commands.rs#L396) | G; branch_id_display.rs | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/session fork` | `-` | [ForkCommand](../../src-rust/crates/commands/src/lib.rs#L5101) | G/A; session_and_export_absorb_their_clusters | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/session rename` | `-` | [RenameCommand](../../src-rust/crates/commands/src/lib.rs#L5923) | G/A; session_and_export_absorb_their_clusters | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/session resume` | `-` | [ResumeCommand](../../src-rust/crates/commands/src/lib.rs#L1409) | G/A; session_and_export_absorb_their_clusters | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/session tag` | `-` | [TagCommand](../../src-rust/crates/commands/src/named_commands.rs#L521) | G; tag_reads.rs | engine history/branch/tag/allowed-dir owners | session browser and inspector |
| `/skills` | `skill` | [SkillsCommand](../../src-rust/crates/commands/src/lib.rs#L5677) | G | skill-discovery owner | existing skill picker/list |
| `/stash` | `-` | [StashCommand](../../src-rust/crates/commands/src/grouped_commands.rs#L109) | G; link_and_attach_round_trip_against_temp_home | existing stash owner | links/attachments, not canonical memory |
| `/stash attach` | `-` | [AttachCommand](../../src-rust/crates/commands/src/lib.rs#L10971) | G; link_and_attach_round_trip_against_temp_home | existing stash owner | links/attachments, not canonical memory |
| `/stash link` | `-` | [LinkCommand](../../src-rust/crates/commands/src/lib.rs#L10844) | G; link_and_attach_round_trip_against_temp_home | existing stash owner | links/attachments, not canonical memory |
| `/status` | `-` | [StatusCommand](../../src-rust/crates/commands/src/lib.rs#L1450) | G; test_doctor_command_reports_coven_substrate | engine diagnostics and daemon read owners | status/doctor detail |
| `/status doctor` | `-` | [DoctorCommand](../../src-rust/crates/commands/src/lib.rs#L2520) | G; test_doctor_command_reports_coven_substrate | engine diagnostics and daemon read owners | status/doctor detail |
| `/tasks` | `bashes` | [TasksCommand](../../src-rust/crates/commands/src/lib.rs#L4940) | G/A; core/tests/task_terminal.rs | engine background registry / tools TASK_STORE | TUI background-job overlay; handler additionally requests TaskList |
| `/thinking` | `think` | [ThinkingCommand](../../src-rust/crates/commands/src/lib.rs#L5153) | G/A | engine thinking/query owner | existing thinking/back trace |
| `/thinking back` | `-` | [ThinkBackCommand](../../src-rust/crates/commands/src/lib.rs#L7754) | G/A | engine thinking/query owner | existing thinking/back trace |
| `/update` | `upgrade` | [UpgradeCommand](../../src-rust/crates/commands/src/lib.rs#L7238) | G; upgrade_cli.rs | engine updater/release owner | explicit update/version/notes |
| `/update notes` | `-` | [ReleaseNotesCommand](../../src-rust/crates/commands/src/lib.rs#L1377) | G; upgrade_cli.rs | engine updater/release owner | explicit update/version/notes |
| `/update version` | `-` | [VersionCommand](../../src-rust/crates/commands/src/lib.rs#L1355) | G; upgrade_cli.rs | engine updater/release owner | explicit update/version/notes |
| `/usage` | `-` | [UsageCommand](../../src-rust/crates/commands/src/lib.rs#L2130) | G/A; usage_command_owns_cost_and_context_views | engine cost/context/stats owner | on-demand usage detail |
| `/usage context` | `-` | [ContextCommand](../../src-rust/crates/commands/src/lib.rs#L6207) | G/A; usage_command_owns_cost_and_context_views | engine cost/context/stats owner | on-demand usage detail |
| `/usage cost` | `-` | [CostCommand](../../src-rust/crates/commands/src/lib.rs#L627) | G/A; usage_command_owns_cost_and_context_views | engine cost/context/stats owner | on-demand usage detail |
| `/usage stats` | `-` | [StatsCommand](../../src-rust/crates/commands/src/lib.rs#L5835) | G/A; usage_command_owns_cost_and_context_views | engine cost/context/stats owner | on-demand usage detail |
| `/vim` | `-` | [App::intercept_slash_command_with_args](../../src-rust/crates/tui/src/app.rs#L2109) | A; test_vim_slash_command_toggles_vim | TUI input/settings | preserve configurable editor mode |
| `/voice` | `-` | [App::intercept_slash_command_with_args](../../src-rust/crates/tui/src/app.rs#L2109) | A; tui/src/lib.rs voice opt-out regression | engine voice/microphone consent owner | explicit opt-in voice input |

Coverage: **83 active/advanced canonical paths** from the default-feature
registry golden, plus the following **46 removed canonical paths** and their
aliases. G is a real test-source pointer, not a claim of command-level runtime
coverage. The reference/golden itself is generated and is not modified here.

Removed paths: preserve rejection and exact replacement guidance; never dispatch
the retained old implementation before preflight. Accepted aliases in active rows
remain accepted. Old comments/help snippets that still mention `/resume`, `/goal`
or `/stats` do not override the registry.

| Removed path | Removed aliases | Existing replacement |
|---|---|---|
| `/accounts` | `-` | `/account list` |
| `/accounts dedupe` | `-` | `/account dedupe` |
| `/add-dir` | `-` | `/session add-dir` |
| `/advisor` | `-` | `/config advisor` |
| `/agents` | `-` | `/familiar list` |
| `/attach` | `attachments` | `/stash attach` |
| `/branch` | `-` | `/session branch` |
| `/changes` | `-` | `/diff` |
| `/chrome` | `-` | `/integrations chrome` |
| `/connect` | `-` | `/account connect` |
| `/context` | `-` | `/usage context` |
| `/copy` | `-` | `/export copy` |
| `/cost` | `-` | `/usage cost` |
| `/doctor` | `-` | `/status doctor` |
| `/fast` | `speed` | `/effort fast` |
| `/feedback` | `bug,survey` | `/help feedback` |
| `/fork` | `-` | `/session fork` |
| `/goal` | `-` | `/coven goal` |
| `/handoff` | `-` | `/familiar handoff` |
| `/hooks` | `-` | `/integrations hooks` |
| `/import-config` | `-` | `/config import` |
| `/incant` | `-` | `/familiar incant` |
| `/keybindings` | `-` | `/config keybindings` |
| `/link` | `links` | `/stash link` |
| `/login` | `-` | `/account login` |
| `/logout` | `-` | `/account logout` |
| `/managed-agents` | `-` | `/familiar managed` |
| `/mcp` | `-` | `/integrations mcp` |
| `/output-style` | `-` | `/config output-style` |
| `/pr-comments` | `-` | `/review pr-comments` |
| `/providers` | `-` | `/account providers` |
| `/refresh` | `-` | `/account refresh` |
| `/release-notes` | `whats-new,changelog` | `/update notes` |
| `/reload-plugins` | `-` | `/plugin reload` |
| `/rename` | `-` | `/session rename` |
| `/resume` | `r,continue` | `/session resume` |
| `/sandbox` | `sandbox-toggle` | `/permissions sandbox` |
| `/share` | `-` | `/export share` |
| `/splash` | `welcome-screen` | `/config splash` |
| `/stats` | `-` | `/usage stats` |
| `/switch` | `-` | `/account switch` |
| `/tag` | `-` | `/session tag` |
| `/theme` | `-` | `/config theme` |
| `/think-back` | `thinkback` | `/thinking back` |
| `/version` | `v` | `/update version` |
| `/whisper` | `btw` | `/familiar whisper` |

## Implemented argument surfaces beyond generated catalog rows

The catalog does not enumerate every accepted argument. Preserve the full handlers:

| Existing surface | Actual implementation / tests | Owner and v2 disposition |
|---|---|---|
| `/config color`/prompt-color; vim/vi/editor; voice; statusline; terminal-setup and documented underscore/hyphen synonyms; show/get/set/unset | `ConfigCommand` delegates to Color/Vim/Voice/Statusline/TerminalSetup commands; G plus `ui_settings_mutation.rs`, `advisor_settings.rs` | Engine settings; preserve safe customization through same commands |
| `/session list`; branch/tag suboperations and add-dir | `SessionCommand`, named Branch/Tag/AddDir handlers; `branch_id_display.rs`, `tag_reads.rs`, `add_dir_save.rs` | Session/workspace owners; preserve exact ID and save-error behavior |
| `/familiar <name>`, info/status/clear/reset/auto; create/edit/delete; standalone new/rename/remove; confirmed wipe-roster/reset-roster | `FamiliarCommand`, `named_commands::AgentsCommand`; familiar roster/access-tier/F2 tests in commands and App | Existing persona/agent/roster owners; preserve confirmation and daemon-owned roster refusal; never claim identity verification |
| `/familiar managed` status/presets/preset/setup/configure/budget/enable/disable/reset | `ManagedAgentsCommand`, `agent_command_absorbs_agents_and_managed` (routing only) | Engine manager/executor config; not Psyche lane/lease support |
| `/coven goal` status/objective/--tokens/pause/resume/clear/complete | `GoalCommand`, core `GoalStore`, CLI `goal_dispatch_tests`; command goal fixtures | Durable engine-session goal; separate from canonical Psyche graph |
| `/coven` sessions/familiars/log/send/kill/capabilities/runtimes/info/events/actions/run/attach/summon/archive/sacrifice/doctor/daemon/patch/pc/calls/claim/hooks-install/hooks/adapter/logs/wt/help/status | `CovenCommand` and helpers; D plus `test_coven_*` argument validation/registry fixtures | Direct IPC for supported native verbs; validated `coven` subprocess delegation for others. Preserve current routes; accepted/ok does not prove durable effect/worker exit |
| `/memory edit` and `/memory clear` (global/project selection) | `MemoryCommand`; `memory_display.rs` covers display/error paths, not every editor/clear side effect | Local instruction files; no alias to governed-memory edits |
| `/rewind list/diff/last`, export formats/output/copy/share, review variants | Existing retained handlers; `rewind_with_args_routes_to_file_rollback_not_overlay`, `share_temp.rs`, `pr_comments.rs`, `llm_prompt_utf8.rs` | Preserve effect and confirmation boundaries; external sharing/posting remains explicit |
| Feature `steer`: `/thinking steer` and catalog-declared legacy steering path disposition | `catalog.rs` cfg branches, `SteerCommand`, CLI queue path | Preserve feature build semantics; not enabled by default or a new runtime permission |
| User templates, discovered skills, plugin commands | `execute_command`, `composed_catalog`, `compose_catalogs`; collision/presentation/plugin metadata tests | Built-ins and reserved removed/TUI identifiers win; then user, skill, plugin. Reuse one palette and refresh installed catalog |

## Local state is not canonical orchestration or memory

| State | Current source-backed behavior | Missing guarantee / presentation consequence |
|---|---|---|
| Background execution registry | `core/src/lib.rs::tasks::TaskRegistry`, process-global DashMap; running/completed/failed/cancelled, output/PID and runtime-only cancellation token; `tasks_overlay.rs` renders it | No durable Psyche graph, leases or restart recovery. A registry cancellation signals immediately; no exit/cleanup proof |
| Model task tools | `tools/src/tasks.rs::TASK_STORE`, separate process-global DashMap; TaskCreate/Get/Update/List plus TaskStop/Output. Dependencies and owner fields already exist | These fields are not a canonical scheduler, durable lease or authority. Do not merge with background registry by matching names |
| `/tasks` | App opens `TasksOverlay`; `TasksCommand::execute` returns a user message asking for TaskList; CLI dispatch can also process command result | Preserve both paths deliberately in #408; avoid double effects or labeling model response a direct authoritative task read |
| Session goal | `core/src/goal.rs::GoalStore`, captured `GoalTurn`/store path and session identity; query goal loop/audit | Existing durable scope and budget semantics remain. Goal completion request is not verified completion; `COVEN_CODE_GOALS=0` remains an opt-out |
| Local instructions | `/memory` reads/edits/clears AGENTS.md locations; `memdir.rs`/claudemd/session-memory paths are engine-owned | Not canonical familiar-memory CRUD, cryptographic identity or a protected-write bypass |
| Transcript/context | CLI messages, session JSON/history/transcripts and query context | Not governed identity; selecting private memory for a model is a separate deliberate operation |
| Familiar selection | Saved roster/config and runtime agent/access-tier activation | Persona/access tier is not signed Familiar Contract verification or an authorization receipt |

## Producer support and precise gaps

All new actions below are proposed. Existing producer artifacts are **S**, terminal
integration is **U** until qualified against exact versions and synthetic runtime
fixtures. No blanket missing-product claim is made.

| Intended v2 action | Canonical producer / inspected interface | Supported boundary and required gate |
|---|---|---|
| Negotiate daemon session/event/action support | [health and same-instance negotiation](https://github.com/OpenCoven/coven/blob/6b5caaecf98fd0a7cd8ee62878979161e697079a/docs/daemon/capabilities-handshake.md) | Daemon advertises capabilities and structured errors. Engine core DaemonHealth drops capability fields; The #412 adapter (not #408) must retain/check exact required fields and IPC peer identity. No UI grants. |
| Inspect/create/send/cancel daemon sessions | [DaemonClient methods](https://github.com/OpenCoven/coven-code/blob/87239e77acfd42dcbd2492c0ba36817626124549/src-rust/crates/core/src/coven_daemon.rs) | Legacy engine client supports these over Unix IPC, with non-Unix unavailability. Bound/adopted Psyche requests require their own proof contracts, not the legacy request shape. |
| Canonical task create/plan/start/pause/cancel/retry/resume, dependency and lane inspection | [CovenPort](https://github.com/OpenCoven/psyche/blob/1e47b40dbbb9db21962b4884afe49763321f04b0/crates/psyche-coven/src/port.rs) | Typed negotiate/adopt/lookup/reconcile/inspect/events/result/terminate exists; core graph/node/execution and durable store exist. This is a boundary toward Coven, not a discovered terminal-facing task CRUD API. Runtime composition alone does not establish scheduler/lane service or TUI adapter. #413 blocked on Psyche #11/#12 and exact consumer profile; #19 is a different process-attempt consumer, not automatically this terminal. |
| Research execution and resumable cited artifacts | [WebSearch and WebFetch](https://github.com/OpenCoven/coven-code/blob/87239e77acfd42dcbd2492c0ba36817626124549/src-rust/crates/tools/src/web_search.rs) | Existing web tools, stash links/attachments and review memory-citation validation are reusable. No qualified canonical research task/artifact API was identified in the inspected terminal/producer interfaces. #414 inherits #413; do not infer claim-to-web-source validation from memory-ID citation tests. |
| List/overview/detail familiar memory | [GET /api/v1/memory, /overview, /:id](https://github.com/OpenCoven/coven/blob/6b5caaecf98fd0a7cd8ee62878979161e697079a/docs/API-CONTRACT.md) | Existing daemon reads; UUID detail, 4 MiB UTF-8 cap, source metadata, nullable privacy and unknown verification. Missing/no-follow target is 404; content failure 503; oversize 413; invalid text 422. Existing engine DaemonClient has no typed memory methods: #415 adapter gap, not absent producer. |
| Unknown/private reveal and exact context selection | [read DTOs and privacy fields](https://github.com/OpenCoven/coven-memory/blob/eb542a598c93775f9d411c03de22a894c17e8527/src/server/memory-contract.ts) | Classification null and reveal_required null require explicit reveal. Overview says verification/attestation/history/mutations false. updated_at and opaque ID do not provide an immutable content revision/admission grant. Qualify revision-bound selection and context admission with Coven owner before claiming complete #415 acceptance; read browsing may be a narrower staged subset. |
| Device/mobile memory access | [Memory client transport boundaries](https://github.com/OpenCoven/coven-memory/blob/eb542a598c93775f9d411c03de22a894c17e8527/README.md) | Mobile signed/enrolled-device routes and local daemon dashboard reads have distinct contracts. Do not repurpose device enrollment or silently open remote listeners for terminal launch. Coven Memory stays read-only; canonical storage/index/path resolution stay in Coven. |
| Memory correction, delete/retention, promotion and protected identity changes | [operation-specific producer contracts](https://github.com/OpenCoven/coven/blob/6b5caaecf98fd0a7cd8ee62878979161e697079a/docs/API-CONTRACT.md) | Existing Ward/proposal/control routes do not prove a generic governed memory API. #416 must qualify EACH operation, immutable review/commit binding, authority, replay lookup and durable receipt. No read-to-write fallback. Existing Coven #1619 audit durability and #1618 decision-reservation issue dispositions must be checked for any affected operation, not silently treated as fixed. |
| Protected authorization and governed identity | [Threads authority model](https://github.com/OpenCoven/coven-threads/blob/659beca959d3459d48ff63282bbd9112256fb9c4/docs/authority-model.md) | Threads controls protected authorization; Familiar Contract RFC-0001 v0.7.0 (Draft) and its normative familiar.embodiment_binding.v1 §10.2 define identity/binding semantics; SPEC.md is historical and superseded. Render producer evidence; never convert model output, goal text, palette availability or local persona into permission. Current end-to-end identity enforcement remains independently qualified. #1137 requires independently trusted issuer/ledger observations and authorized sources; schema/fixture signatures alone cannot establish trusted dispatch. |
| SDK client consumption | [CovenClient and transport](https://github.com/OpenCoven/sdk/blob/89685543a62008533237478c31ecce565798ddf6/packages/coven/src/client.ts) | SDK has experimental source bindings (private/unpublished packages; no first-release security review), including typed discovery, health/transport and domain-specific automation/mission bindings. This client file does not establish general Psyche task or memory CRUD. Pin the exact consumer profile; avoid a speculative SDK API or Node dependency. |
| Brand and interaction roles | [profile 1.0.0](https://github.com/OpenCoven/brand/blob/798e2f5f2f1a69f5d156ccc0a9aafc4f7da55fc8/web/profile.json) | Canonical anchor #8E3DFF and theme-specific action/focus/presence/state roles. Terminal translation is #409; accessible alternatives survive. UI contracts define portable semantics, not engine authority. |

Gap ownership: #413 consumes Psyche [#11](https://github.com/OpenCoven/psyche/issues/11)
and [#12](https://github.com/OpenCoven/psyche/issues/12); #414 defines its research
consumer requirements after that lifecycle is qualified. Context admission already
has Coven [#1137](https://github.com/OpenCoven/coven/issues/1137), and main-session
priming [#1182](https://github.com/OpenCoven/coven/issues/1182) is a distinct consumer.
Do not create a duplicate context-admission service or call priming complete TUI
selection. The immutable source-byte/revision gap is already owned by #1137
(Authorized source material); #415 must consume that qualified producer work
before enabling admitted context selection. No duplicate producer issue is needed. #416 must record narrower per-operation
upstream gaps after inspecting the chosen operation; this baseline deliberately
leaves unsupported writes unavailable rather than proposing an umbrella mutation API.

## Qualification still required

The registry and implementation inventory are source baselines. G/A/D, retained
Classic cell goldens, and producer conformance source are not fresh passes. Record
actual engine/consumer/producer heads, platform, terminal, executed test counts,
skips and failure evidence using the compatibility matrix. Human visual acceptance,
native Windows/macOS/Linux PTY runs, real producer integration and privacy canaries
remain separate from documentation/source review.
