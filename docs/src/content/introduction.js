export const meta = { title: 'Introduction' };

export function render() {
  return `
    <h1>What is Coven Code?</h1>
    <p class="lead">Coven Code is a high-performance Rust reimplementation of Claude Code — a terminal-native AI coding agent with streaming responses, 40+ built-in tools, 15+ LLM provider integrations, a full ratatui TUI, and an extensible plugin system.</p>

    <p>You give Coven Code a task in natural language. It plans, reads and writes files, runs shell commands, searches the web, and iterates — all inside your terminal, with every step visible in real time.</p>

    <pre><code data-lang="bash">$ coven-code "add input validation to the signup form"</code></pre>

    <p>It reads your codebase, implements the change across multiple files, runs your tests, and reports back — without you leaving the terminal.</p>

    <h2>Key capabilities</h2>

    <h3>Agentic loop</h3>
    <p>Coven Code runs a multi-turn loop: it streams a response from the model, executes any tool calls (file read, bash, web search, …), feeds the results back, and continues until the task is done or the turn limit is reached.</p>

    <h3>40+ built-in tools</h3>
    <ul>
      <li><strong>File operations</strong> — read, write, edit, patch, batch-edit</li>
      <li><strong>Shell</strong> — bash with persistent working directory and environment</li>
      <li><strong>Search</strong> — glob file patterns, grep contents, web search, web fetch</li>
      <li><strong>Git</strong> — commit, branch, worktree</li>
      <li><strong>Notebooks</strong> — read and edit Jupyter notebooks</li>
      <li><strong>Desktop automation</strong> — screenshot, click, type (optional feature)</li>
      <li><strong>Task management</strong> — create, track, and complete tasks</li>
    </ul>

    <h3>15+ LLM providers</h3>
    <p>Anthropic Claude (default), OpenAI, Google Gemini, AWS Bedrock, Azure OpenAI, Ollama, Groq, Mistral, DeepSeek, xAI, Cohere, OpenRouter, Together AI, Perplexity, GitHub Copilot, Cerebras, LM Studio, and LLaMA.cpp.</p>

    <h3>AMOLED terminal UI</h3>
    <p>A ratatui-based TUI with real-time streaming, syntax-highlighted code blocks, diff viewer, permission dialogs, slash command autocomplete, session browser, and a full keybinding system.</p>

    <h3>Multi-account credentials</h3>
    <p>Store multiple named Anthropic (Claude.ai / Console) and Codex (ChatGPT) accounts in one install and switch between them instantly with <code>/switch</code> or <code>coven-code auth switch &lt;id&gt;</code>. Identity is detected from the OAuth JWT, so re-logging-in the same account is idempotent.</p>

    <h3>@file injection</h3>
    <p>Type <code>@path/to/file</code> anywhere in a prompt to inject the file's contents inline. Typeahead autocomplete suggests paths as you type, with size/binary safety checks before submit.</p>

    <h3>Plugin system</h3>
    <p>Extend Coven Code with TOML-manifest plugins that add custom slash commands, MCP servers, hooks, output styles, and tool overlays.</p>

    <h3>Multi-agent orchestration</h3>
    <p>Run named agents (<code>build</code>, <code>plan</code>, <code>explore</code>) or spawn parallel sub-agents in coordinator mode. Agents communicate via a shared task registry and message channels.</p>

    <h3>Goal system</h3>
    <p>Set a durable objective with <code>/goal</code> and Coven Code works autonomously across turns until the goal is verified complete — using the <code>GoalCompleteTool</code> for audited completion rather than just stopping.</p>

    <h3>Speech modes</h3>
    <p>Activate <code>/caveman</code> or <code>/rocky</code> to compress model responses by 40–85%, saving tokens in long sessions. Deactivate with <code>/normal</code>.</p>
  `;
}
