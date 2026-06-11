/**
 * Searchable index for the global Cmd+K palette.
 *
 * Each entry: { kind, label, category, desc, href }
 *   kind     — one-word taxonomy for the result row (Command, Key, …)
 *   label    — primary text shown bold (the thing you type to match)
 *   category — sub-grouping shown next to the label
 *   desc     — short description below the label
 *   href     — anchor URL to navigate to (e.g. "#commands")
 *
 * Section + sub-heading entries are appended at runtime by main.js
 * (it reads the rendered DOM and tocBySection map for those).
 */

// Static entries — mirror the data already in demos.js. Drift risk is
// real but bounded: if you add an item to an explorer, mirror it here.

export const STATIC_PALETTE_ITEMS = [
  // ---- Slash commands -------------------------------------------------
  ...slash([
    ['/help',            'Session',         'Show all commands'],
    ['/clear',           'Session',         'Clear the conversation'],
    ['/exit',            'Session',         'Quit the session'],
    ['/resume',          'Session',         'Resume a previous session'],
    ['/session',         'Session',         'List or pick a session'],
    ['/session fork',    'Session',         'Branch the current session into a new one'],
    ['/rename',          'Session',         'Rename the current session'],
    ['/rewind',          'Session',         'Go back to a previous message'],
    ['/compact',         'Session',         'Compress conversation history'],
    ['/model',           'Model',           'Switch model or provider'],
    ['/providers',       'Model',           'List available providers'],
    ['/connect',         'Model',           'Connect to a remote provider endpoint'],
    ['/thinking',        'Model',           'Toggle extended thinking display'],
    ['/effort',          'Model',           'Set extended-thinking effort level'],
    ['/config advisor',  'Model',           'Set a secondary advisor model'],
    ['/effort fast',     'Model',           'Toggle fast mode (smaller, faster model)'],
    ['/config',          'Config',          'Open settings or adjust UI config'],
    ['/config color',    'Config',          'Set the prompt bar color'],
    ['/config statusline','Config',         'Configure the TUI status line'],
    ['/config vim',      'Config',          'Toggle vim mode'],
    ['/config voice',    'Config',          'Voice input mode'],
    ['/config terminal-setup','Config',     'Run terminal capability checks'],
    ['/config keybindings','Config',        'Open the interactive keybinding editor'],
    ['/permissions',     'Config',          'Manage tool permission rules'],
    ['/hooks',           'Config',          'Inspect active hooks'],
    ['/mcp',             'Config',          'Manage MCP servers'],
    ['/config output-style','Config',       'Switch output style'],
    ['/config theme',    'Config',          'Switch visual theme'],
    ['/commit',          'Code & Git',      'Stage and commit current changes'],
    ['/diff',            'Code & Git',      'Show working-tree diff'],
    ['/review',          'Code & Git',      'Review a PR or current changes'],
    ['/security-review', 'Code & Git',      'Security audit of pending changes'],
    ['/init',            'Code & Git',      'Initialise a new AGENTS.md for the project'],
    ['/search',          'Code & Git',      'Codebase search'],
    ['/memory',          'Memory & Cost',   'Browse persistent memory entries'],
    ['/usage context',   'Memory & Cost',   'Show context window usage'],
    ['/usage cost',      'Memory & Cost',   'Token usage and dollar cost for the session'],
    ['/usage stats',     'Memory & Cost',   'Open the interactive session statistics dialog'],
    ['/insights',        'Memory & Cost',   'Session statistics and tool usage report'],
    ['/agent list',      'Agents & Tasks',  'List built-in, custom, and familiar agents'],
    ['/agent',           'Agents & Tasks',  'Switch active agent for this session'],
    ['/tasks',           'Agents & Tasks',  'Show the live task list'],
    ['/goal',            'Agents & Tasks',  'Set an autonomous multi-turn goal'],
    ['/agent managed',   'Agents & Tasks',  'Configure manager-executor agents'],
    ['/plan',            'Agents & Tasks',  'Enter planning mode (read-only)'],
    ['/ultraplan',       'Agents & Tasks',  'Deep planning mode'],
    ['/ultrareview',     'Agents & Tasks',  'Exhaustive multi-dimensional code review'],
    ['/login',           'Auth',            'OAuth login (--codex for ChatGPT)'],
    ['/accounts',        'Auth',            'List stored profiles'],
    ['/login switch',    'Auth',            'Switch active account'],
    ['/logout',          'Auth',            'Clear credentials'],
    ['/login refresh',   'Auth',            'Refresh OAuth tokens'],
    ['/caveman',         'Display',         'Telegraphic speech mode (saves 40–85% tokens)'],
    ['/rocky',           'Display',         'Rocky (Project Hail Mary) speech mode'],
    ['/normal',          'Display',         'Deactivate speech modes'],
    ['/mobile',          'Display',         'Compact mobile-friendly rendering'],
    ['/coven',           'Coven',           'Substrate surface: kill, log, send, familiars'],
    ['/familiar',        'Coven',           'Switch active familiar (also F2)'],
    ['/handoff',         'Coven',           'Hand off a session between familiars'],
    ['/status doctor',   'Diagnostics',     'Environment and substrate health check'],
    ['/version',         'Diagnostics',     'Show version info'],
    ['/update',          'Diagnostics',     'Check for and download updates'],
    ['/export',          'Diagnostics',     'Save session transcript'],
    ['/export copy',     'Diagnostics',     'Copy last response to clipboard'],
    ['/thinking back',   'Diagnostics',     'View thinking traces from previous responses'],
  ], '#commands'),

  // ---- Keybindings ----------------------------------------------------
  ...kb([
    ['Ctrl+C',       'Global',       'Interrupt the current operation'],
    ['Ctrl+D',       'Global',       'Exit Coven Code'],
    ['Ctrl+L',       'Global',       'Redraw the terminal screen'],
    ['Ctrl+R',       'Global',       'Open interactive history search'],
    ['Ctrl+B',       'Global',       'Create a new git branch'],
    ['Alt+H',        'Global',       'Open the help panel'],
    ['F2',           'Global',       'Open familiar switcher popup'],
    ['Enter',        'Chat',         'Submit message'],
    ['Shift+Enter',  'Chat',         'Insert a literal newline'],
    ['Tab',          'Chat',         'Indent or cycle completions'],
    ['Ctrl+A',       'Chat',         'Move cursor to start of line'],
    ['Ctrl+E',       'Chat',         'Move cursor to end of line'],
    ['Ctrl+Shift+A', 'Chat',         'Open the model picker'],
    ['Ctrl+K',       'Chat',         'Open the slash command palette'],
    ['Ctrl+W',       'Chat',         'Delete word before cursor'],
    ['Ctrl+F',       'Chat',         'Find within current conversation'],
    ['Ctrl+Shift+F', 'Chat',         'Open global codebase search'],
    ['F3',           'Chat',         'Jump to next search match'],
    ['@',            'Chat',         'Open file picker for @file injection'],
    ['Y',            'Confirmation', 'Approve the pending action'],
    ['N',            'Confirmation', 'Deny the pending action'],
    ['A',            'Confirmation', 'Approve and add a permanent allow rule'],
    ['Escape',       'Confirmation', 'Cancel and deny'],
  ], '#keybindings'),

  // ---- Providers ------------------------------------------------------
  ...row('Provider', [
    ['anthropic',  'Cloud',      'Default. Claude Opus/Sonnet/Haiku via /v1/messages.'],
    ['openai',     'Cloud',      'GPT-4o, o-series, gpt-4.1.'],
    ['google',     'Cloud',      'Gemini Pro / Flash / Flash-8B.'],
    ['bedrock',    'Cloud',      'AWS region — Claude, Llama, Titan, Mistral.'],
    ['azure',      'Cloud',      'OpenAI models through Azure.'],
    ['groq',       'Cloud',      'Fastest hosted inference — Llama, Mixtral.'],
    ['mistral',    'Cloud',      'Mistral Large/Medium/Small, Codestral, Pixtral.'],
    ['deepseek',   'Cloud',      'V3, R1 — reasoning models with strong math/code.'],
    ['xai',        'Cloud',      'Grok family.'],
    ['cohere',     'Cloud',      'Command R/R+, Aya.'],
    ['perplexity', 'Cloud',      'Sonar models with built-in web search.'],
    ['copilot',    'Cloud',      'GitHub Copilot OAuth.'],
    ['cerebras',   'Cloud',      'Very high TPS on Llama via wafer-scale silicon.'],
    ['openrouter', 'Aggregator', 'Single key, 200+ models from many providers.'],
    ['together',   'Aggregator', 'Llama, Mixtral, DeepSeek, Qwen hosted at Together.'],
    ['ollama',     'Local',      'Local socket. Bring your own GGUF.'],
    ['lmstudio',   'Local',      'Local HTTP — point at LM Studio.'],
    ['llamacpp',   'Local',      'Local HTTP from llama.cpp server.'],
  ], '#providers'),

  // ---- Hook events ----------------------------------------------------
  ...row('Hook', [
    ['PreToolUse',         'Tool',         'Fires before any tool executes. Exit 2 blocks + rewakes the model.'],
    ['PostToolUse',        'Tool',         'Fires after a tool completes successfully.'],
    ['PostToolUseFailure', 'Tool',         'Fires when a tool errors.'],
    ['UserPromptSubmit',   'Turn',         'Fires when the user submits a new prompt.'],
    ['Stop',               'Turn',         'Fires when the model finishes a turn cleanly.'],
    ['StopFailure',        'Turn',         'Fires when a turn ends due to error.'],
    ['Notification',       'Turn',         'Fires for user-facing notifications.'],
    ['SessionStart',       'Session',      'Fires once when the session starts.'],
    ['SessionEnd',         'Session',      'Fires once when the session ends.'],
    ['SubagentStart',      'Subagent',     'Fires when a sub-agent is spawned.'],
    ['SubagentStop',       'Subagent',     'Fires when a sub-agent completes.'],
    ['PreCompact',         'Compaction',   'Fires before context compaction. Exit 2 blocks it.'],
    ['PostCompact',        'Compaction',   'Fires after context compaction.'],
    ['PermissionRequest',  'Permissions',  'Fires when a tool requests permission.'],
    ['PermissionDenied',   'Permissions',  'Fires when the user denies a permission request.'],
    ['TaskCreated',        'Tasks',        'Fires when a task is added.'],
    ['TaskCompleted',      'Tasks',        'Fires when a task is marked completed.'],
    ['Elicitation',        'Elicitation',  'Fires when Coven Code asks a structured question.'],
    ['ElicitationResult',  'Elicitation',  'Fires when the user responds.'],
    ['ConfigChange',       'Other',        'Fires when settings change at runtime.'],
    ['WorktreeCreate',     'Other',        'Fires when a git worktree is created.'],
  ], '#hooks'),

  // ---- Plugin fields --------------------------------------------------
  ...row('Plugin field', [
    ['name',           'Identity', 'Required. Unique plugin identifier.'],
    ['version',        'Identity', 'Required. Semver string.'],
    ['description',    'Metadata', 'One-line summary surfaced in /plugin list.'],
    ['author',         'Metadata', '{ name, email, url } describing the maintainer.'],
    ['marketplace_id', 'Metadata', 'Marketplace listing identifier (owner/name).'],
    ['commands',       'Content',  'Extra slash command markdown files.'],
    ['agents',         'Content',  'Extra agent markdown files.'],
    ['skills',         'Content',  'Extra skill directories.'],
    ['mcp_servers',    'Inline',   'Inline MCP server definitions.'],
    ['lsp_servers',    'Inline',   'Inline LSP server definitions.'],
    ['hooks',          'Inline',   'Inline hook definitions.'],
    ['user_config',    'Config',   'Schema for user-configurable options.'],
    ['capabilities',   'Config',   'Capability grants array (omit to allow all).'],
  ], '#plugins'),

  // ---- MCP config fields ----------------------------------------------
  ...row('MCP field', [
    ['name',    'Identity',  'Required. Unique server identifier in the session.'],
    ['type',    'Transport', '"stdio" (default) or "http".'],
    ['command', 'Transport', 'Required for stdio. Executable to spawn.'],
    ['args',    'Transport', 'Arguments passed to command. Supports ${VAR} expansion.'],
    ['url',     'Transport', 'Required for http. Full SSE endpoint URL.'],
    ['env',     'Runtime',   'Extra env vars for the child process.'],
  ], '#mcp'),

  // ---- Tools ----------------------------------------------------------
  ...row('Tool', [
    ['FileReadTool',   'File',  'Read files (text, images, PDFs, notebooks).'],
    ['FileWriteTool',  'File',  'Create or overwrite a file.'],
    ['FileEditTool',   'File',  'Exact-string replacement.'],
    ['BatchEditTool',  'File',  'Apply many edits in one call.'],
    ['ApplyPatchTool', 'File',  'Apply a unified diff patch.'],
    ['BashTool',       'Shell', 'Shell commands with persistent CWD + env.'],
    ['MonitorTool',    'Shell', 'Tail a background process started by Bash.'],
    ['PtyBashTool',    'Shell', 'Bash in a pseudo-terminal (needs TTY).'],
    ['PowerShellTool', 'Shell', 'PowerShell execution on Windows.'],
    ['ReplTool',       'Shell', 'Persistent REPL session.'],
    ['GlobTool',       'Search', 'Match files by glob pattern.'],
    ['GrepTool',       'Search', 'Regex-search file contents (ripgrep).'],
    ['ToolSearchTool', 'Search', 'Look up tools by name/keyword.'],
    ['WebFetchTool',   'Web',   'Fetch a URL and summarize via small model.'],
    ['WebSearchTool',  'Web',   'Web search with snippet results.'],
    ['TaskCreate',     'Task',  'Create a tracked task.'],
    ['TaskUpdate',     'Task',  'Update task status / fields.'],
    ['TaskList',       'Task',  'Enumerate open/completed tasks.'],
    ['GitCommit',      'Git',   'Stage + create a commit.'],
    ['WorktreeCreate', 'Git',   'Create a git worktree.'],
  ], '#tools'),
];

// ---- helpers ----------------------------------------------------------

function slash(rows, href) {
  return rows.map(([label, category, desc]) => ({
    kind: 'Slash', label, category, desc, href,
  }));
}
function kb(rows, href) {
  return rows.map(([label, category, desc]) => ({
    kind: 'Key', label, category, desc, href,
  }));
}
function row(kind, rows, href) {
  return rows.map(([label, category, desc]) => ({
    kind, label, category, desc, href,
  }));
}
