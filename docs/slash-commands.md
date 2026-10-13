# Slash commands

Generated from the command catalog. Run `cargo run --package claurst-commands --example command_reference` from `src-rust/` to regenerate.

Type `/` for contextual suggestions, or open the command palette with Ctrl+K. Enter opens a command group; choose its Run entry to execute the parent. Use `/help <path>` for a command, including grouped paths. Suggestions use local session state and recent commands.

## Start & Navigate

| Command | Usage | Description |
|---|---|---|
| `/clear` | `/clear` | Clear the conversation history |
| `/exit` | `/exit` | Exit Coven |
| `/help` | `/help [command]` | Show available commands and usage information |
| `/init` | `/init` | Initialize a new project with AGENTS.md |
| `/search` | `/search <query>` | Search across all sessions |
| `/session` | `/session <resume\|fork\|branch\|tag\|add-dir>` | Show or manage conversation sessions |
| `/session rename` | `/session rename <name>` | Rename the current session |
| `/session resume` | `/session resume [id]` | Resume a saved session |
| `/session fork` | `/session fork [message_index]` | Fork the current session |
| `/session branch` | `/session branch [arguments]` | Create a session branch |
| `/session tag` | `/session tag [arguments]` | Tag the current session |
| `/session add-dir` | `/session add-dir <path>` | Add an allowed workspace directory |

## Work & Review

| Command | Usage | Description |
|---|---|---|
| `/commit` | `/commit [message]` | Ask Coven to commit staged changes |
| `/diff` | `/diff` | Show git diff of changes in the working directory |
| `/memory` | `/memory [edit\|clear]` | View, edit, or clear AGENTS.md memory files |
| `/review` | `/review [pr-comments]` | Review code changes via LLM and optionally post to GitHub PR |
| `/review pr-comments` | `/review pr-comments` | Read pull request comments |
| `/review security` | `/review security [path]` | Run a security-focused review |
| `/review ultra` | `/review ultra [path]` | Run an exhaustive review |
| `/rewind` | `/rewind [list\|diff\|last]` | Rewind the conversation or roll back a turn's file changes |
| `/rewind list` | `/rewind list` | List recorded file-change checkpoints |
| `/rewind diff` | `/rewind diff [checkpoint]` | Preview a recorded file-change diff |

## Conversation

| Command | Usage | Description |
|---|---|---|
| `/find` | `/find` | Search the current transcript |
| `/compact` | `/compact` | Compact the conversation to reduce token usage |
| `/export` | `/export [copy [n]\|share] [--format markdown\|json] [--output <file>]` | Export conversation to markdown or JSON |
| `/export share` | `/export share` | Create a shareable session export |
| `/export copy` | `/export copy` | Copy the session export to the clipboard |
| `/plan` | `/plan [prompt]` | Enter plan mode – model outputs a plan for approval before acting |
| `/tasks` | `/tasks` | List and manage background tasks |
| `/thinking` | `/thinking <back>` | Toggle extended thinking mode |
| `/thinking back` | `/thinking back` | View earlier thinking traces |

## Account & Models

| Command | Usage | Description |
|---|---|---|
| `/account` | `/account <login\|logout\|list\|dedupe\|providers\|connect\|switch\|refresh>` | Manage accounts and provider connections |
| `/account login` | `/account login [--console] [--codex] [--label <name>]` | Sign in to a provider |
| `/account logout` | `/account logout` | Sign out of a provider |
| `/account list` | `/account list` | List configured accounts |
| `/account dedupe` | `/account dedupe` | Remove duplicate credentials |
| `/account providers` | `/account providers` | List available providers |
| `/account connect` | `/account connect` | Connect a provider account |
| `/account switch` | `/account switch [name]` | Switch the active account |
| `/account refresh` | `/account refresh` | Refresh provider authentication |
| `/model` | `/model [name]` | Show or change the current model |
| `/permissions` | `/permissions [sandbox]` | View or change tool permission settings |
| `/permissions sandbox` | `/permissions sandbox` | Configure sandbox permissions |
| `/usage` | `/usage [cost\|context\|stats]` | Show API usage, quotas, and rate limit status |
| `/usage cost` | `/usage cost` | Show session cost details |
| `/usage context` | `/usage context` | Show context window usage |
| `/usage stats` | `/usage stats` | Show detailed session statistics |

## Customize

| Command | Usage | Description |
|---|---|---|
| `/vim` | `/vim` | Toggle Vim input mode |
| `/voice` | `/voice` | Toggle voice input |
| `/config` | `/config [show\|get\|set\|unset\|<area>] ...` | Show or modify configuration settings |
| `/config splash` | `/config splash [show\|hide\|toggle]` | Configure the empty-session splash screen |
| `/config theme` | `/config theme [name]` | Select the display theme |
| `/config output-style` | `/config output-style [style]` | Select the response output style |
| `/config keybindings` | `/config keybindings` | Open keybinding configuration |
| `/config import` | `/config import` | Import configuration from another tool |
| `/config advisor` | `/config advisor [model\|off]` | Configure the advisor model |
| `/effort` | `/effort [low\|normal\|high\|fast [on\|off]]` | Set the model's thinking effort (low \| normal \| high) |
| `/effort fast` | `/effort fast [on\|off]` | Enable fast response mode |

## Extend

| Command | Usage | Description |
|---|---|---|
| `/help feedback` | `/help feedback [report]` | Submit feedback about Coven |
| `/integrations` | `/integrations <hooks\|mcp\|chrome>` | Configure external integrations |
| `/integrations hooks` | `/integrations hooks` | Configure lifecycle hooks |
| `/integrations mcp` | `/integrations mcp [arguments]` | Manage MCP servers |
| `/integrations chrome` | `/integrations chrome` | Connect Chrome integration |
| `/learn` | `/learn` | Codify a script or workflow we just built into a reusable skill (Hermes) |
| `/plugin` | `/plugin [reload]` | Manage plugins |
| `/plugin reload` | `/plugin reload` | Reload installed plugins |
| `/skills` | `/skills` | List available skills in .coven-code/commands/ |

## Coven & Advanced

| Command | Usage | Description |
|---|---|---|
| `/coven` | `/coven [status\|sessions\|familiars\|log\|run\|daemon\|goal\|help] ...` | Drive the local Coven daemon (sessions, harness runs, rituals) |
| `/coven goal` | `/coven goal [objective\|status\|pause\|resume\|clear\|complete]` | Manage durable session goals |
| `/familiar` | `/familiar <incant\|whisper\|handoff\|list\|managed>` | Your familiar & agents — switch persona, inspect, and manage |
| `/familiar incant` | `/familiar incant [arguments]` | Run a familiar incantation |
| `/familiar whisper` | `/familiar whisper <message>` | Ask a familiar side question |
| `/familiar handoff` | `/familiar handoff <name>` | Hand work to a familiar |
| `/familiar list` | `/familiar list` | List available familiars |
| `/familiar managed` | `/familiar managed [arguments]` | Manage familiar execution |
| `/stash` | `/stash <link\|attach>` | Store useful links and attachments |
| `/stash link` | `/stash link [arguments]` | Store and manage links |
| `/stash attach` | `/stash attach [arguments]` | Store and manage attachments |
| `/status` | `/status [doctor]` | Show comprehensive system and session status |
| `/status doctor` | `/status doctor` | Run diagnostic checks |
| `/update` | `/update <version\|notes>` | Check for updates and download the latest release |
| `/update version` | `/update version` | Show the installed Coven version |
| `/update notes` | `/update notes` | Show recent Coven release notes |

## Removed paths

These paths no longer execute. Their replacement remains available.

| Removed path | Use instead |
|---|---|
| `/cost` | `/usage cost` |
| `/context` | `/usage context` |
| `/stats` | `/usage stats` |
| `/rename` | `/session rename` |
| `/changes` | `/diff` |
| `/version` | `/update version` |
| `/v` | `/update version` |
| `/release-notes` | `/update notes` |
| `/whats-new` | `/update notes` |
| `/changelog` | `/update notes` |
| `/resume` | `/session resume` |
| `/r` | `/session resume` |
| `/continue` | `/session resume` |
| `/reload-plugins` | `/plugin reload` |
| `/doctor` | `/status doctor` |
| `/login` | `/account login` |
| `/logout` | `/account logout` |
| `/accounts` | `/account list` |
| `/accounts dedupe` | `/account dedupe` |
| `/switch` | `/account switch` |
| `/refresh` | `/account refresh` |
| `/incant` | `/familiar incant` |
| `/handoff` | `/familiar handoff` |
| `/hooks` | `/integrations hooks` |
| `/import-config` | `/config import` |
| `/mcp` | `/integrations mcp` |
| `/fork` | `/session fork` |
| `/theme` | `/config theme` |
| `/output-style` | `/config output-style` |
| `/keybindings` | `/config keybindings` |
| `/splash` | `/config splash` |
| `/welcome-screen` | `/config splash` |
| `/share` | `/export share` |
| `/add-dir` | `/session add-dir` |
| `/agents` | `/familiar list` |
| `/branch` | `/session branch` |
| `/tag` | `/session tag` |
| `/pr-comments` | `/review pr-comments` |
| `/copy` | `/export copy` |
| `/chrome` | `/integrations chrome` |
| `/fast` | `/effort fast` |
| `/speed` | `/effort fast` |
| `/think-back` | `/thinking back` |
| `/thinkback` | `/thinking back` |
| `/whisper` | `/familiar whisper` |
| `/btw` | `/familiar whisper` |
| `/sandbox` | `/permissions sandbox` |
| `/sandbox-toggle` | `/permissions sandbox` |
| `/advisor` | `/config advisor` |
| `/providers` | `/account providers` |
| `/connect` | `/account connect` |
| `/managed-agents` | `/familiar managed` |
| `/goal` | `/coven goal` |
| `/link` | `/stash link` |
| `/links` | `/stash link` |
| `/attach` | `/stash attach` |
| `/attachments` | `/stash attach` |
| `/feedback` | `/help feedback` |
| `/bug` | `/help feedback` |
| `/survey` | `/help feedback` |
