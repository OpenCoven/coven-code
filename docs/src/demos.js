/**
 * Alpine.data() factories for the interactive demos embedded in content modules.
 * Registered before Alpine.start() so directives bind correctly.
 */

export function registerDemos(Alpine) {
  // ----- Demo 1: TUI welcome-screen mockup -------------------------------
  Alpine.data('welcomeMockup', () => ({
    field: null, // hovered field key
    small: false, // small-terminal fallback toggle
    explain: {
      model: {
        title: 'Model',
        body: "Active model ID. Falls back to the provider's default (e.g. claude-sonnet-4-6) when unset. Change with /model or the 'model' field in settings.json.",
      },
      provider: {
        title: 'Provider',
        body: "Active LLM provider — anthropic when unset. Override with /model picker, --provider flag, or 'provider' in settings.json.",
      },
      daemon: {
        title: 'Daemon',
        body: 'Coven daemon online/offline status. A cheap socket check — no RPC. Install @opencoven/coven to bring it online; familiars and skills light up when it is.',
      },
      familiar: {
        title: 'Familiar',
        body: 'Current familiar identity. F2 opens the switcher popup. The portrait in the left column animates with the matching glyph.',
      },
      goal: {
        title: 'Goal',
        body: 'Active autonomous goal — only shown when one is set with /goal <objective>. Coven Code works across turns until GoalCompleteTool verifies completion.',
      },
    },
  }));

  // ----- Demo 2: Agentic loop animation ----------------------------------
  Alpine.data('agenticLoop', () => ({
    playing: true,
    step: 0,
    steps: [
      {
        actor: 'user',
        label: 'You',
        text: 'Add input validation to the signup form',
      },
      {
        actor: 'model',
        label: 'Model',
        text: 'I need to find the signup form first.',
        tool: 'GrepTool',
        toolArgs: '"<form" --type tsx',
      },
      {
        actor: 'tool',
        label: 'Tool result',
        text: 'src/auth/SignupForm.tsx:14',
      },
      {
        actor: 'model',
        label: 'Model',
        text: 'Found it. Reading the file to see what fields need validation.',
        tool: 'FileReadTool',
        toolArgs: 'src/auth/SignupForm.tsx',
      },
      {
        actor: 'tool',
        label: 'Tool result',
        text: '47 lines — email, password, confirmPassword fields, no validation.',
      },
      {
        actor: 'model',
        label: 'Model',
        text: 'Adding zod schema validation with helpful error messages.',
        tool: 'FileEditTool',
        toolArgs: 'src/auth/SignupForm.tsx',
      },
      {
        actor: 'tool',
        label: 'Tool result',
        text: '+ import { z } from "zod"\n+ const SignupSchema = z.object({ ... })\n+ const result = SignupSchema.safeParse(...)',
      },
      {
        actor: 'model',
        label: 'Model',
        text: 'Done. Signup form now validates email format, password length (8+), and password confirmation before submit.',
      },
    ],
    timer: null,
    init() {
      this.startTimer();
    },
    startTimer() {
      this.stopTimer();
      this.timer = setInterval(() => {
        if (!this.playing) return;
        this.step = (this.step + 1) % this.steps.length;
      }, 2500);
    },
    stopTimer() {
      if (this.timer) clearInterval(this.timer);
      this.timer = null;
    },
    toggle() {
      this.playing = !this.playing;
    },
    next() {
      this.playing = false;
      this.step = (this.step + 1) % this.steps.length;
    },
    prev() {
      this.playing = false;
      this.step = (this.step - 1 + this.steps.length) % this.steps.length;
    },
    reset() {
      this.step = 0;
      this.playing = true;
    },
    get current() {
      return this.steps[this.step];
    },
  }));

  // ----- Demo 3: Permission visualizer -----------------------------------
  // Modes × levels → ✓ allow / 🛡 prompt / ✗ block
  const MATRIX = {
    default:            { None: 'allow', ReadOnly: 'allow',  Write: 'prompt', Execute: 'prompt', Dangerous: 'prompt' },
    plan:               { None: 'allow', ReadOnly: 'allow',  Write: 'block',  Execute: 'block',  Dangerous: 'block'  },
    auto:               { None: 'allow', ReadOnly: 'allow',  Write: 'allow',  Execute: 'prompt', Dangerous: 'prompt' },
    acceptEdits:        { None: 'allow', ReadOnly: 'allow',  Write: 'allow',  Execute: 'prompt', Dangerous: 'prompt' },
    bypassPermissions:  { None: 'allow', ReadOnly: 'allow',  Write: 'allow',  Execute: 'allow',  Dangerous: 'allow'  },
  };
  const MODE_BLURBS = {
    default: 'Asks before any write or execute.',
    plan: 'Read-only: writes and shell are blocked entirely.',
    auto: 'Non-destructive tools run silently; destructive still prompt.',
    acceptEdits: 'File edits auto-approved; shell still prompts.',
    bypassPermissions: 'Headless / CI mode — no prompts at all.',
  };
  const LEVEL_EXAMPLES = {
    None: ['SleepTool'],
    ReadOnly: ['FileReadTool', 'GlobTool', 'GrepTool', 'WebFetchTool'],
    Write: ['FileWriteTool', 'FileEditTool', 'ApplyPatchTool', 'ConfigTool'],
    Execute: ['BashTool', 'PtyBashTool', 'TaskCreate', 'SendMessageTool'],
    Dangerous: ['ComputerUseTool'],
  };
  Alpine.data('permissionViz', () => ({
    mode: 'default',
    modes: Object.keys(MATRIX),
    levels: Object.keys(LEVEL_EXAMPLES),
    blurbs: MODE_BLURBS,
    examples: LEVEL_EXAMPLES,
    cell(level) {
      return MATRIX[this.mode][level];
    },
    cellLabel(level) {
      return { allow: 'Allowed', prompt: 'Prompts user', block: 'Blocked' }[this.cell(level)];
    },
    cellMark(level) {
      return { allow: '✓', prompt: '?', block: '✗' }[this.cell(level)];
    },
  }));

  // ----- Shared: Searchable explorer factory ------------------------------
  // Used by both commandExplorer and keybindingExplorer below. Filters items
  // by selected category (null = all) AND by free-text query against the
  // configured search fields. Returns up to `limit` results.
  function makeExplorer({ items, categories, search = ['id', 'desc'], limit = null }) {
    return () => ({
      query: '',
      category: null,
      items,
      categories,
      pick(cat) {
        this.category = this.category === cat ? null : cat;
      },
      clear() {
        this.query = '';
        this.category = null;
      },
      get filtered() {
        const q = this.query.trim().toLowerCase();
        let out = this.items;
        if (this.category) out = out.filter((it) => it.category === this.category);
        if (q) {
          out = out.filter((it) =>
            search.some((f) => String(it[f] || '').toLowerCase().includes(q))
          );
        }
        return limit ? out.slice(0, limit) : out;
      },
      get count() {
        return this.filtered.length;
      },
      get total() {
        return this.items.length;
      },
      countIn(cat) {
        return this.items.filter((it) => it.category === cat).length;
      },
    });
  }

  // ----- Slash command explorer ------------------------------------------
  Alpine.data(
    'commandExplorer',
    makeExplorer({
      categories: [
        'Session', 'Model', 'Config', 'Code & Git', 'Memory & Cost',
        'Agents & Tasks', 'Auth', 'Display', 'Coven', 'Diagnostics',
      ],
      search: ['id', 'desc', 'keywords'],
      items: [
        // Session
        { id: '/help', category: 'Session', desc: 'Show all commands', keywords: 'h ?' },
        { id: '/clear', category: 'Session', desc: 'Clear the conversation' },
        { id: '/exit', category: 'Session', desc: 'Quit the session', keywords: 'quit q' },
        { id: '/resume', category: 'Session', desc: 'Resume a previous session' },
        { id: '/session', category: 'Session', desc: 'List or pick a session' },
        { id: '/fork', category: 'Session', desc: 'Branch the current session into a new one' },
        { id: '/rename', category: 'Session', desc: 'Rename the current session' },
        { id: '/rewind', category: 'Session', desc: 'Go back to a previous message' },
        { id: '/compact', category: 'Session', desc: 'Compress conversation history' },
        // Model
        { id: '/model', category: 'Model', desc: 'Switch model or provider' },
        { id: '/providers', category: 'Model', desc: 'List available providers' },
        { id: '/connect', category: 'Model', desc: 'Connect to a remote provider endpoint' },
        { id: '/thinking', category: 'Model', desc: 'Toggle extended thinking display' },
        { id: '/effort', category: 'Model', desc: 'Set extended-thinking effort level', keywords: 'low medium high max' },
        { id: '/advisor', category: 'Model', desc: 'Set a secondary advisor model' },
        { id: '/fast', category: 'Model', desc: 'Toggle fast mode (smaller, faster model)' },
        // Config
        { id: '/config', category: 'Config', desc: 'Open the settings editor' },
        { id: '/keybindings', category: 'Config', desc: 'Open the interactive keybinding editor' },
        { id: '/permissions', category: 'Config', desc: 'Manage tool permission rules' },
        { id: '/hooks', category: 'Config', desc: 'Inspect active hooks' },
        { id: '/mcp', category: 'Config', desc: 'Manage MCP servers' },
        { id: '/output-style', category: 'Config', desc: 'Switch output style' },
        { id: '/theme', category: 'Config', desc: 'Switch visual theme' },
        { id: '/statusline', category: 'Config', desc: 'Configure status line' },
        { id: '/vim', category: 'Config', desc: 'Toggle vim mode' },
        { id: '/voice', category: 'Config', desc: 'Voice input mode' },
        // Code & Git
        { id: '/commit', category: 'Code & Git', desc: 'Stage and commit current changes', keywords: 'git' },
        { id: '/diff', category: 'Code & Git', desc: 'Show working-tree diff', keywords: 'git' },
        { id: '/undo', category: 'Code & Git', desc: 'Undo the last file edit (uses snapshot)' },
        { id: '/review', category: 'Code & Git', desc: 'Review a PR or current changes' },
        { id: '/security-review', category: 'Code & Git', desc: 'Security audit of pending changes' },
        { id: '/init', category: 'Code & Git', desc: 'Initialise a new AGENTS.md for the project' },
        { id: '/search', category: 'Code & Git', desc: 'Codebase search', keywords: 'grep find' },
        // Memory & Cost
        { id: '/memory', category: 'Memory & Cost', desc: 'Browse persistent memory entries' },
        { id: '/context', category: 'Memory & Cost', desc: 'Show context window usage' },
        { id: '/cost', category: 'Memory & Cost', desc: 'Token usage and dollar cost for the session', keywords: 'tokens money' },
        { id: '/usage', category: 'Memory & Cost', desc: 'Session usage statistics' },
        { id: '/stats', category: 'Memory & Cost', desc: 'Session statistics' },
        { id: '/insights', category: 'Memory & Cost', desc: 'Session statistics and tool usage report' },
        { id: '/status', category: 'Memory & Cost', desc: 'Connection and daemon status' },
        // Agents & Tasks
        { id: '/agents', category: 'Agents & Tasks', desc: 'List built-in, custom, and familiar agents' },
        { id: '/agent', category: 'Agents & Tasks', desc: 'Switch active agent for this session' },
        { id: '/tasks', category: 'Agents & Tasks', desc: 'Show the live task list' },
        { id: '/goal', category: 'Agents & Tasks', desc: 'Set an autonomous multi-turn goal' },
        { id: '/managed-agents', category: 'Agents & Tasks', desc: 'Configure manager-executor agents' },
        { id: '/plan', category: 'Agents & Tasks', desc: 'Enter planning mode (read-only)' },
        { id: '/ultraplan', category: 'Agents & Tasks', desc: 'Deep planning mode' },
        { id: '/ultrareview', category: 'Agents & Tasks', desc: 'Exhaustive multi-dimensional code review' },
        // Auth
        { id: '/login', category: 'Auth', desc: 'OAuth login (--codex for ChatGPT, --label to name)' },
        { id: '/accounts', category: 'Auth', desc: 'List stored profiles' },
        { id: '/switch', category: 'Auth', desc: 'Switch active account' },
        { id: '/logout', category: 'Auth', desc: 'Clear credentials' },
        { id: '/refresh', category: 'Auth', desc: 'Refresh OAuth tokens' },
        // Display
        { id: '/caveman', category: 'Display', desc: 'Telegraphic speech mode (saves 40–85% tokens)' },
        { id: '/rocky', category: 'Display', desc: 'Rocky (Project Hail Mary) speech mode' },
        { id: '/normal', category: 'Display', desc: 'Deactivate speech modes' },
        { id: '/mobile', category: 'Display', desc: 'Compact mobile-friendly rendering' },
        { id: '/color', category: 'Display', desc: 'Adjust colour palette at runtime' },
        { id: '/stickers', category: 'Display', desc: 'Toggle sticker rendering' },
        // Coven
        { id: '/coven', category: 'Coven', desc: 'Substrate surface: kill, log, send, familiars, etc.' },
        { id: '/familiar', category: 'Coven', desc: 'Switch active familiar (also F2)' },
        { id: '/handoff', category: 'Coven', desc: 'Hand off a session between familiars' },
        // Diagnostics
        { id: '/doctor', category: 'Diagnostics', desc: 'Environment and substrate health check' },
        { id: '/version', category: 'Diagnostics', desc: 'Show version info' },
        { id: '/update', category: 'Diagnostics', desc: 'Check for and download updates' },
        { id: '/export', category: 'Diagnostics', desc: 'Save session transcript' },
        { id: '/copy', category: 'Diagnostics', desc: 'Copy last response to clipboard', keywords: 'clipboard' },
        { id: '/think-back', category: 'Diagnostics', desc: 'View thinking traces from previous responses' },
        { id: '/sandbox-toggle', category: 'Diagnostics', desc: 'Toggle sandboxed shell execution' },
      ],
    })
  );

  // ----- Keybinding explorer ---------------------------------------------
  Alpine.data(
    'keybindingExplorer',
    makeExplorer({
      categories: ['Global', 'Chat', 'Confirmation'],
      search: ['id', 'desc', 'keys'],
      items: [
        // Global
        { id: 'Ctrl+C',  keys: 'ctrl c interrupt cancel', category: 'Global', desc: 'Interrupt the current operation (non-rebindable)' },
        { id: 'Ctrl+D',  keys: 'ctrl d exit quit',        category: 'Global', desc: 'Exit Coven Code (non-rebindable)' },
        { id: 'Ctrl+L',  keys: 'ctrl l redraw refresh',   category: 'Global', desc: 'Redraw the terminal screen' },
        { id: 'Ctrl+R',  keys: 'ctrl r history search',   category: 'Global', desc: 'Open interactive history search' },
        { id: 'Ctrl+B',  keys: 'ctrl b git branch',       category: 'Global', desc: 'Create a new git branch' },
        { id: 'Alt+H',   keys: 'alt h help',              category: 'Global', desc: 'Open the help panel' },
        { id: 'F2',      keys: 'f2 familiar switcher',    category: 'Global', desc: 'Open familiar switcher popup' },
        // Chat
        { id: 'Enter',           keys: 'enter return submit',         category: 'Chat', desc: 'Submit message' },
        { id: 'Shift+Enter',     keys: 'shift enter newline',         category: 'Chat', desc: 'Insert a literal newline (also Ctrl+J)' },
        { id: 'Up',              keys: 'up arrow history previous',   category: 'Chat', desc: 'Previous in input history (or Ctrl+O)' },
        { id: 'Down',            keys: 'down arrow history next',     category: 'Chat', desc: 'Next in input history (or Ctrl+I)' },
        { id: 'Tab',              keys: 'tab indent completion',      category: 'Chat', desc: 'Indent (or cycle completions if open)' },
        { id: 'Page Up / Down',   keys: 'page up down scroll',        category: 'Chat', desc: 'Scroll conversation by one page' },
        { id: 'Ctrl+A',           keys: 'ctrl a line start',           category: 'Chat', desc: 'Move cursor to start of line (Emacs-style)' },
        { id: 'Ctrl+E',           keys: 'ctrl e line end',             category: 'Chat', desc: 'Move cursor to end of line' },
        { id: 'Ctrl+Left',        keys: 'ctrl left word backward',     category: 'Chat', desc: 'Move one word left' },
        { id: 'Ctrl+Right',       keys: 'ctrl right word forward',     category: 'Chat', desc: 'Move one word right' },
        { id: 'Alt+Left',         keys: 'alt left previous message',   category: 'Chat', desc: 'Jump to previous message' },
        { id: 'Alt+Right',        keys: 'alt right next message',      category: 'Chat', desc: 'Jump to next message' },
        { id: 'Ctrl+Shift+A',     keys: 'ctrl shift a model picker',   category: 'Chat', desc: 'Open the model picker' },
        { id: 'Ctrl+K',           keys: 'ctrl k palette command',      category: 'Chat', desc: 'Open the slash command palette' },
        { id: 'Ctrl+U',           keys: 'ctrl u kill line',            category: 'Chat', desc: 'Kill from cursor to start of line' },
        { id: 'Ctrl+W',           keys: 'ctrl w kill word',            category: 'Chat', desc: 'Delete word before cursor (or Alt+Backspace)' },
        { id: 'Alt+D',            keys: 'alt d delete word',           category: 'Chat', desc: 'Delete word after cursor' },
        { id: 'Ctrl+F',           keys: 'ctrl f find in conversation', category: 'Chat', desc: 'Find within current conversation' },
        { id: 'Ctrl+Shift+F',     keys: 'ctrl shift f global search',  category: 'Chat', desc: 'Open global codebase search' },
        { id: 'F3',               keys: 'f3 next match',               category: 'Chat', desc: 'Jump to next search match (Shift+F3 for prev)' },
        { id: 'Ctrl+G',           keys: 'ctrl g go to line',           category: 'Chat', desc: 'Jump to a specific line' },
        { id: 'Ctrl+.',           keys: 'ctrl dot error',              category: 'Chat', desc: 'Jump to next error / issue' },
        { id: '@',                keys: 'at file injection',           category: 'Chat', desc: 'Open file picker (typeahead injects file contents)' },
        // Confirmation
        { id: 'Y',       keys: 'y yes approve',           category: 'Confirmation', desc: 'Approve the pending action' },
        { id: 'N',       keys: 'n no deny',               category: 'Confirmation', desc: 'Deny the pending action' },
        { id: 'A',       keys: 'a always allow',          category: 'Confirmation', desc: 'Approve and add a permanent allow rule' },
        { id: 'Enter',   keys: 'enter default',           category: 'Confirmation', desc: 'Accept the highlighted default option' },
        { id: 'Escape',  keys: 'escape cancel',           category: 'Confirmation', desc: 'Cancel and deny' },
      ],
    })
  );

  // ----- Provider explorer -----------------------------------------------
  Alpine.data(
    'providerExplorer',
    makeExplorer({
      categories: ['Cloud', 'Aggregator', 'Local'],
      search: ['id', 'desc', 'keywords'],
      items: [
        { id: 'anthropic',  category: 'Cloud',      desc: 'Default. ANTHROPIC_API_KEY or OAuth. /v1/messages streaming. Default model: claude-sonnet-4-6.', keywords: 'claude opus sonnet haiku' },
        { id: 'openai',     category: 'Cloud',      desc: 'OPENAI_API_KEY. Chat Completions + Responses API. GPT-4o, o-series, gpt-4.1.' },
        { id: 'google',     category: 'Cloud',      desc: 'GOOGLE_API_KEY. Gemini 1.5/2.0/2.5 — Pro, Flash, Flash-8B.',                                  keywords: 'gemini palm' },
        { id: 'bedrock',    category: 'Cloud',      desc: 'AWS credentials chain (env, profile, IAM). Claude, Llama, Titan, Mistral via AWS region.', keywords: 'aws amazon' },
        { id: 'azure',      category: 'Cloud',      desc: 'AZURE_OPENAI_API_KEY + endpoint URL + deployment id. OpenAI models through Azure.', keywords: 'microsoft openai' },
        { id: 'groq',       category: 'Cloud',      desc: 'GROQ_API_KEY. Fastest hosted inference — Llama, Mixtral, Whisper.' },
        { id: 'mistral',    category: 'Cloud',      desc: 'MISTRAL_API_KEY. Mistral Large/Medium/Small, Codestral, Pixtral.' },
        { id: 'deepseek',   category: 'Cloud',      desc: 'DEEPSEEK_API_KEY. V3, R1 — reasoning models with strong math/code.', keywords: 'r1' },
        { id: 'xai',        category: 'Cloud',      desc: 'XAI_API_KEY. Grok family.',                                                                  keywords: 'grok x' },
        { id: 'cohere',     category: 'Cloud',      desc: 'COHERE_API_KEY. Command R/R+, Aya.' },
        { id: 'perplexity', category: 'Cloud',      desc: 'PERPLEXITY_API_KEY. Sonar models with built-in web search.' },
        { id: 'copilot',    category: 'Cloud',      desc: 'GitHub Copilot OAuth (run /login --copilot). Uses your GitHub subscription.', keywords: 'github' },
        { id: 'cerebras',   category: 'Cloud',      desc: 'CEREBRAS_API_KEY. Very high TPS on Llama via wafer-scale silicon.' },
        { id: 'openrouter', category: 'Aggregator', desc: 'OPENROUTER_API_KEY. Single key, 200+ models from many providers.' },
        { id: 'together',   category: 'Aggregator', desc: 'TOGETHER_API_KEY. Llama, Mixtral, DeepSeek, Qwen — hosted at TogetherAI.' },
        { id: 'ollama',     category: 'Local',      desc: 'Local socket — no auth. Set base_url (default http://localhost:11434). Bring your own GGUF.' },
        { id: 'lmstudio',   category: 'Local',      desc: 'Local HTTP server. Point base_url at LM Studio (default http://localhost:1234).', keywords: 'lm studio' },
        { id: 'llamacpp',   category: 'Local',      desc: 'Local HTTP from llama.cpp server. Set base_url to your server endpoint.', keywords: 'llama.cpp' },
      ],
    })
  );

  // ----- Hook event explorer ---------------------------------------------
  Alpine.data(
    'hookEventExplorer',
    makeExplorer({
      categories: ['Tool', 'Turn', 'Session', 'Subagent', 'Compaction', 'Permissions', 'Tasks', 'Elicitation', 'Other'],
      search: ['id', 'desc', 'keywords'],
      items: [
        // Tool lifecycle
        { id: 'PreToolUse',         category: 'Tool',         desc: 'Fires before any tool executes. Matcher compares against tool_name. Exit 0 = allow, 1 = block + report, 2 = block and rewake the model with stderr.', keywords: 'before pre tool' },
        { id: 'PostToolUse',        category: 'Tool',         desc: 'Fires after a tool completes successfully. Receives tool_input + tool_response.',                                                                       keywords: 'after post tool success' },
        { id: 'PostToolUseFailure', category: 'Tool',         desc: 'Fires when a tool errors. Receives tool_input + error message.',                                                                                       keywords: 'after post tool error' },
        // Turn lifecycle
        { id: 'UserPromptSubmit',   category: 'Turn',         desc: 'Fires when the user submits a new prompt. Exit 2 returns stderr as a system message instead of running the prompt.',                                  keywords: 'prompt user input' },
        { id: 'Stop',               category: 'Turn',         desc: 'Fires when the model finishes a turn cleanly.',                                                                                                        keywords: 'turn end finish' },
        { id: 'StopFailure',        category: 'Turn',         desc: 'Fires when a turn ends due to error (rate limit, max turns, network).',                                                                                keywords: 'turn end failure error' },
        { id: 'Notification',       category: 'Turn',         desc: 'Fires for user-facing notifications (e.g., waiting on permission, idle).',                                                                              keywords: 'notify alert' },
        // Session lifecycle
        { id: 'SessionStart',       category: 'Session',      desc: 'Fires once when the session starts.',                                                                                                                  keywords: 'init begin' },
        { id: 'SessionEnd',         category: 'Session',      desc: 'Fires once when the session ends (exit, crash, signal).',                                                                                              keywords: 'shutdown close exit' },
        // Subagent lifecycle
        { id: 'SubagentStart',      category: 'Subagent',     desc: 'Fires when a sub-agent is spawned (e.g., coordinator worker, /agents launch).',                                                                         keywords: 'agent spawn worker' },
        { id: 'SubagentStop',       category: 'Subagent',     desc: 'Fires when a sub-agent completes. Receives the final result payload.',                                                                                  keywords: 'agent stop worker' },
        // Compaction
        { id: 'PreCompact',         category: 'Compaction',   desc: 'Fires before context compaction. Exit 2 blocks the compaction.',                                                                                       keywords: 'before compact summary' },
        { id: 'PostCompact',        category: 'Compaction',   desc: 'Fires after context compaction. Receives the new summary text.',                                                                                       keywords: 'after compact summary' },
        // Permissions
        { id: 'PermissionRequest',  category: 'Permissions',  desc: 'Fires when a tool requests permission. Lets you auto-approve or deny via custom logic.',                                                                keywords: 'permission ask prompt' },
        { id: 'PermissionDenied',   category: 'Permissions',  desc: 'Fires when the user denies a permission request.',                                                                                                     keywords: 'permission deny reject' },
        // Tasks
        { id: 'TaskCreated',        category: 'Tasks',        desc: 'Fires when a task is added via TaskCreate or /tasks.',                                                                                                  keywords: 'task new' },
        { id: 'TaskCompleted',      category: 'Tasks',        desc: 'Fires when a task is marked completed.',                                                                                                                keywords: 'task done finish' },
        // Elicitation
        { id: 'Elicitation',        category: 'Elicitation',  desc: 'Fires when Coven Code asks the user a structured question (via AskUserQuestion / MCP elicitation).',                                                    keywords: 'ask question prompt' },
        { id: 'ElicitationResult',  category: 'Elicitation',  desc: 'Fires when the user responds to an elicitation. Receives the answers.',                                                                                  keywords: 'answer reply' },
        // Other
        { id: 'ConfigChange',       category: 'Other',        desc: 'Fires when settings change at runtime (e.g., /config edit, model switch).',                                                                            keywords: 'settings change' },
        { id: 'WorktreeCreate',     category: 'Other',        desc: 'Fires when a git worktree is created via the WorktreeCreate tool.',                                                                                    keywords: 'git worktree' },
      ],
    })
  );

  // ----- Plugin manifest field explorer ----------------------------------
  Alpine.data(
    'pluginFieldExplorer',
    makeExplorer({
      categories: ['Identity', 'Metadata', 'Content', 'Inline', 'Config'],
      search: ['id', 'desc', 'keywords'],
      items: [
        { id: 'name',           category: 'Identity', desc: 'Required. Unique plugin identifier — must be set.',                                       keywords: 'required id' },
        { id: 'version',        category: 'Identity', desc: 'Required. Semver string, e.g. "1.0.0".',                                                  keywords: 'required semver' },
        { id: 'description',    category: 'Metadata', desc: 'Optional one-line summary surfaced in /plugin list and /plugin info.',                    keywords: 'optional summary' },
        { id: 'author',         category: 'Metadata', desc: 'Optional. Object: { name, email, url } describing the maintainer.',                       keywords: 'optional maintainer' },
        { id: 'marketplace_id', category: 'Metadata', desc: 'Optional. Marketplace listing identifier in owner/name form (e.g. "you/my-plugin").',     keywords: 'optional marketplace' },
        { id: 'commands',       category: 'Content',  desc: 'Optional. Extra slash command markdown files beyond the conventional commands/ dir.',     keywords: 'optional slash extra' },
        { id: 'agents',         category: 'Content',  desc: 'Optional. Extra agent markdown files beyond the conventional agents/ dir.',                keywords: 'optional extra' },
        { id: 'skills',         category: 'Content',  desc: 'Optional. Extra skill directories beyond the conventional skills/ dir.',                   keywords: 'optional extra' },
        { id: 'mcp_servers',    category: 'Inline',   desc: 'Optional array. Inline MCP server definitions merged into the global MCP config on load.', keywords: 'optional inline' },
        { id: 'lsp_servers',    category: 'Inline',   desc: 'Optional array. Inline LSP server definitions for in-editor language tooling.',            keywords: 'optional inline language' },
        { id: 'hooks',          category: 'Inline',   desc: 'Optional. Inline hook definitions — also discoverable via a plugin\'s hooks/ directory.',  keywords: 'optional inline events' },
        { id: 'user_config',    category: 'Config',   desc: 'Optional. Schema for user-configurable options — surfaced in /plugin info as form fields.', keywords: 'optional settings' },
        { id: 'capabilities',   category: 'Config',   desc: 'Optional. Capability grants array (e.g. ["read_files","network","shell"]). Omit to allow all.', keywords: 'optional grants permissions' },
      ],
    })
  );

  // ----- Demo 4: Tools grid ----------------------------------------------
  Alpine.data('toolsGrid', () => ({
    expanded: null,
    toggle(name) {
      this.expanded = this.expanded === name ? null : name;
    },
    categories: [
      {
        name: 'File',
        tools: [
          { name: 'FileReadTool', level: 'ReadOnly', desc: 'Read files (text, images, PDF, notebooks)', params: ['file_path', 'offset?', 'limit?'], example: 'FileReadTool(file_path="/abs/path/to/file.rs")' },
          { name: 'FileWriteTool', level: 'Write', desc: 'Create or overwrite a file', params: ['file_path', 'content'], example: 'FileWriteTool(file_path="/abs/foo.txt", content="...")' },
          { name: 'FileEditTool', level: 'Write', desc: 'Exact string replacement', params: ['file_path', 'old_string', 'new_string', 'replace_all?'], example: 'FileEditTool(file_path=..., old_string="foo", new_string="bar")' },
          { name: 'BatchEditTool', level: 'Write', desc: 'Apply many edits atomically', params: ['edits[]'], example: 'BatchEditTool(edits=[{file_path, old_string, new_string}, ...])' },
          { name: 'ApplyPatchTool', level: 'Write', desc: 'Apply a unified diff', params: ['patch'], example: 'ApplyPatchTool(patch="--- a/file\\n+++ b/file\\n@@ ...")' },
        ],
      },
      {
        name: 'Shell',
        tools: [
          { name: 'BashTool', level: 'Execute', desc: 'Shell with persistent CWD + env', params: ['command', 'description', 'run_in_background?'], example: 'BashTool(command="cargo test", description="Run test suite")' },
          { name: 'MonitorTool', level: 'ReadOnly', desc: 'Tail a backgrounded Bash process', params: ['bash_id'], example: 'MonitorTool(bash_id="abc123")' },
          { name: 'PtyBashTool', level: 'Execute', desc: 'Bash in a pseudo-terminal (TTY)', params: ['command'], example: 'PtyBashTool(command="vim foo.txt")' },
          { name: 'PowerShellTool', level: 'Execute', desc: 'PowerShell on Windows', params: ['command'], example: 'PowerShellTool(command="Get-Process")' },
          { name: 'ReplTool', level: 'Execute', desc: 'Persistent REPL session', params: ['language', 'code'], example: 'ReplTool(language="python", code="print(1+1)")' },
        ],
      },
      {
        name: 'Search',
        tools: [
          { name: 'GlobTool', level: 'ReadOnly', desc: 'Match files by glob pattern', params: ['pattern'], example: 'GlobTool(pattern="src/**/*.rs")' },
          { name: 'GrepTool', level: 'ReadOnly', desc: 'Regex-search file contents (ripgrep)', params: ['pattern', 'path?', 'type?'], example: 'GrepTool(pattern="async fn", type="rs")' },
          { name: 'ToolSearchTool', level: 'ReadOnly', desc: 'Look up tools by name/keyword', params: ['query'], example: 'ToolSearchTool(query="bash")' },
        ],
      },
      {
        name: 'Web',
        tools: [
          { name: 'WebFetchTool', level: 'ReadOnly', desc: 'Fetch a URL and summarize via small model', params: ['url', 'prompt'], example: 'WebFetchTool(url="https://...", prompt="extract X")' },
          { name: 'WebSearchTool', level: 'ReadOnly', desc: 'Web search with snippet results', params: ['query'], example: 'WebSearchTool(query="rustls 0.23 migration")' },
        ],
      },
      {
        name: 'Task',
        tools: [
          { name: 'TaskCreate', level: 'Execute', desc: 'Create a tracked task', params: ['subject', 'description'], example: 'TaskCreate(subject="Fix login bug", description="...")' },
          { name: 'TaskUpdate', level: 'Execute', desc: 'Update task status / fields', params: ['taskId', 'status?', 'subject?'], example: 'TaskUpdate(taskId="1", status="completed")' },
          { name: 'TaskList', level: 'ReadOnly', desc: 'Enumerate open/completed tasks', params: [], example: 'TaskList()' },
        ],
      },
      {
        name: 'Git',
        tools: [
          { name: 'GitCommit', level: 'Execute', desc: 'Stage + create a commit', params: ['message'], example: 'GitCommit(message="fix: ...")' },
          { name: 'GitBranch', level: 'Execute', desc: 'Create or switch branches', params: ['name', 'create?'], example: 'GitBranch(name="feat/x", create=true)' },
          { name: 'WorktreeCreate', level: 'Execute', desc: 'Create a git worktree', params: ['branch', 'path'], example: 'WorktreeCreate(branch="feat/x", path="../wt-x")' },
        ],
      },
    ],
  }));
}
