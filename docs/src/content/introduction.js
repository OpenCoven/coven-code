export const meta = { title: 'Introduction' };

export function render() {
  return `
    <h1>What Is Coven Code?</h1>
    <p class="lead">Coven Code is a high-performance Rust reimplementation of Claude Code — a terminal-native AI coding agent with streaming responses, 40+ built-in tools, 15+ LLM provider integrations, a full ratatui TUI, and an extensible plugin system.</p>

    <p>You give Coven Code a task in natural language. It plans, reads and writes files, runs shell commands, searches the web, and iterates — all inside your terminal, with every step visible in real time.</p>

    <pre><code data-lang="bash">$ coven run codex "add input validation to the signup form"</code></pre>

    <p>It reads your codebase, implements the change across multiple files, runs your tests, and reports back — without you leaving the terminal.</p>

    <h2>Key Capabilities</h2>

    <h3>Agentic Loop</h3>
    <p>Coven Code runs a multi-turn loop: it streams a response from the model, executes any tool calls (file read, bash, web search, …), feeds the results back, and continues until the task is done or the turn limit is reached.</p>

    <div class="demo" x-data="agenticLoop" x-init="init()">
      <div class="demo-header">
        <span>walkthrough · "add input validation"</span>
        <div class="demo-header-actions">
          <button class="demo-btn" @click="prev()" title="Previous step">‹</button>
          <button class="demo-btn" @click="toggle()" :aria-pressed="playing" x-text="playing ? 'Pause' : 'Play'"></button>
          <button class="demo-btn" @click="next()" title="Next step">›</button>
          <button class="demo-btn" @click="reset()" title="Reset to start">⟲</button>
        </div>
      </div>
      <div class="demo-body">
        <div class="loop-wrap">
          <div class="loop-stages">
            <div class="loop-stage" :data-active="current.actor === 'user'">
              <span class="loop-stage-num">01</span>
              You prompt
            </div>
            <div class="loop-stage" :data-active="current.actor === 'model' && !current.tool">
              <span class="loop-stage-num">02</span>
              Model thinks
            </div>
            <div class="loop-stage" :data-active="current.actor === 'model' && !!current.tool">
              <span class="loop-stage-num">03</span>
              Tool call
            </div>
            <div class="loop-stage" :data-active="current.actor === 'tool'">
              <span class="loop-stage-num">04</span>
              Result returns
            </div>
          </div>
          <div class="loop-message" :class="'loop-actor-' + current.actor">
            <div class="loop-actor">
              <span class="loop-actor-dot"></span>
              <span x-text="current.label"></span>
              <span style="margin-left:auto; color: var(--color-text-dimmer); font-family: var(--font-mono); font-size: 11px;">
                step <span x-text="step + 1"></span> / <span x-text="steps.length"></span>
              </span>
            </div>
            <div class="loop-text" x-text="current.text"></div>
            <template x-if="current.tool">
              <div class="loop-tool">
                <span class="loop-tool-name" x-text="current.tool"></span>(<span class="loop-tool-args" x-text="current.toolArgs"></span>)
              </div>
            </template>
          </div>
        </div>
      </div>
    </div>

    <p>The loop repeats — model emits a tool call, Coven Code runs it, the result feeds back in, the model decides what to do next. It only stops when the model says it's done, when a goal is verified complete, or when the turn limit is hit.</p>

    <h3>40+ Built-In Tools</h3>
    <ul>
      <li><strong>File operations</strong> — read, write, edit, patch, batch-edit</li>
      <li><strong>Shell</strong> — bash with persistent working directory and environment</li>
      <li><strong>Search</strong> — glob file patterns, grep contents, web search, web fetch</li>
      <li><strong>Git</strong> — commit, branch, worktree</li>
      <li><strong>Notebooks</strong> — read and edit Jupyter notebooks</li>
      <li><strong>Desktop automation</strong> — screenshot, click, type (optional feature)</li>
      <li><strong>Task management</strong> — create, track, and complete tasks</li>
    </ul>

    <h3>15+ LLM Providers</h3>
    <p>Anthropic Claude (default), OpenAI, Google Gemini, AWS Bedrock, Azure OpenAI, Ollama, Groq, Mistral, DeepSeek, xAI, Cohere, OpenRouter, Together AI, Perplexity, GitHub Copilot, Cerebras, LM Studio, and LLaMA.cpp.</p>

    <h3>AMOLED Terminal UI</h3>
    <p>A ratatui-based TUI with real-time streaming, syntax-highlighted code blocks, diff viewer, permission dialogs, slash command autocomplete, session browser, and a full keybinding system.</p>

    <h3>Multi-Account Credentials</h3>
    <p>Store multiple named Anthropic (Claude.ai / Console) and Codex (ChatGPT) accounts in one install and switch between them instantly with <code>/switch</code> or <code>coven-code auth switch &lt;id&gt;</code>. Identity is detected from the OAuth JWT, so re-logging-in the same account is idempotent.</p>

    <h3>@file Injection</h3>
    <p>Type <code>@path/to/file</code> anywhere in a prompt to inject the file's contents inline. Typeahead autocomplete suggests paths as you type, with size/binary safety checks before submit.</p>

    <h3>Plugin System</h3>
    <p>Extend Coven Code with TOML-manifest plugins that add custom slash commands, MCP servers, hooks, output styles, and tool overlays.</p>

    <h3>Multi-Agent Orchestration</h3>
    <p>Run named agents (<code>build</code>, <code>plan</code>, <code>explore</code>) or spawn parallel sub-agents in coordinator mode. Agents communicate via a shared task registry and message channels.</p>

    <h3>Goal System</h3>
    <p>Set a durable objective with <code>/goal</code> and Coven Code works autonomously across turns until the goal is verified complete — using the <code>GoalCompleteTool</code> for audited completion rather than just stopping.</p>

    <h3>Speech Modes</h3>
    <p>Activate <code>/caveman</code> or <code>/rocky</code> to compress model responses by 40–85%, saving tokens in long sessions. Deactivate with <code>/normal</code>.</p>
  `;
}
