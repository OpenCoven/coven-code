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
