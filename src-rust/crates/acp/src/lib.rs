//! Agent Client Protocol (ACP) server for Coven Code.
//!
//! ACP is the open protocol pioneered by Zed for standardizing communication
//! between AI coding agents and editors (Zed, Neovim, JetBrains, VS Code, …).
//! Spec: <https://agentclientprotocol.com>
//!
//! # OpenCoven integration seam
//!
//! This crate is the primary entry point for connecting Coven Code to the
//! broader OpenCoven ecosystem. The ACP server exposes JSON-RPC 2.0 over stdio
//! (`coven-code acp`), which Coven orchestration layers can attach to as an
//! agent worker.
//!
//! To add OpenCoven-specific capabilities:
//! - Extend `AcpServer` with additional RPC methods in this crate.
//! - Implement new capabilities in the `capabilities` module.
//! - Register provider-specific tool overrides in `claurst-tools`. (internal crate)
//! - See `registry-template/agent.json` for the ACP registry entry.
//!
//! This crate turns the local `coven-code` binary into a compliant ACP agent
//! over newline-delimited JSON-RPC 2.0 on stdio. Editors launch `coven-code acp`
//! as a subprocess and drive it through the protocol's standard methods:
//!
//! | Method                       | Direction  | Notes                                       |
//! |------------------------------|------------|---------------------------------------------|
//! | `initialize`                 | C → A      | Capability negotiation                      |
//! | `authenticate`               | C → A      | No-op (Coven Code uses local credentials)      |
//! | `session/new`                | C → A      | Create a session with cwd + MCP roster      |
//! | `session/prompt`             | C → A      | Run a turn; streams `session/update` events |
//! | `session/cancel`             | C → A (no resp) | Cancel an in-flight prompt             |
//! | `session/update`             | A → C (no resp) | Streamed text/tool deltas              |
//! | `session/request_permission` | A → C      | Tool approval dialog                        |
//!
//! Per-session MCP server configs supplied via `session/new` are accepted
//! but currently ignored in favour of the global `settings.json` MCP roster.
//! This will be resolved as part of the planned unified MCP routing work
//! (see `src-rust/plan/migration-todo.md`).

mod admission;
mod connection;
mod permission;
mod prompt;
mod runtime;
mod server;
mod sessions;

use std::sync::Arc;

#[cfg(test)]
use futures::{stream::FuturesUnordered, StreamExt};
#[cfg(test)]
use tokio::sync::mpsc;
use tracing::info;
#[cfg(test)]
use tracing::warn;

pub use connection::Connection;
pub use runtime::AgentRuntime;
pub use server::AgentServer;

/// Run the ACP server on the current process' stdin / stdout. Returns when
/// stdin reaches EOF, reporting runtime initialization or reader transport errors.
/// In-flight dispatch tasks are drained before returning a reader error.
pub async fn run_acp_server() -> anyhow::Result<()> {
    // We must NEVER write to stdout outside the protocol — every byte on
    // stdout is parsed by the client as JSON-RPC. Force logs to stderr.
    install_stderr_tracing();

    let working_dir = std::env::current_dir()?;
    info!(cwd = %working_dir.display(), version = env!("CARGO_PKG_VERSION"), "ACP: starting server");

    let runtime = AgentRuntime::build(working_dir).await?;
    let runtime = Arc::new(runtime);
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        use std::os::unix::fs::FileTypeExt;
        // Tokio's generic stdio uses uncancellable blocking tasks. An editor's
        // full output pipe or open, idle input pipe would outlive our shutdown
        // deadline. Register actual pipes with the reactor instead.
        let output = std::fs::File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
        let connection = if output.metadata()?.file_type().is_fifo() {
            Connection::new(tokio::net::unix::pipe::Sender::from_file(output)?)
        } else {
            Connection::new(tokio::io::stdout())
        };
        let server = AgentServer::new(connection, runtime);
        let input = std::fs::File::from(std::io::stdin().as_fd().try_clone_to_owned()?);
        if input.metadata()?.file_type().is_fifo() {
            run_connection(server, tokio::net::unix::pipe::Receiver::from_file(input)?).await
        } else {
            run_connection(server, tokio::io::stdin()).await
        }
    }
    #[cfg(not(unix))]
    {
        let connection = Connection::new(tokio::io::stdout());
        let server = AgentServer::new(connection, runtime);
        run_connection(server, tokio::io::stdin()).await
    }
}

async fn run_connection<R>(server: Arc<AgentServer>, reader: R) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    admission::run(server, reader)
        .await
        .map_err(|error| error.context("ACP reader loop failed"))
}

fn install_stderr_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_env("COVEN_CODE_ACP_LOG")
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
async fn dispatch_messages(
    server: Arc<AgentServer>,
    mut rx: mpsc::UnboundedReceiver<connection::Inbound>,
) {
    let mut tasks = FuturesUnordered::new();
    let mut accepting = true;
    while accepting || !tasks.is_empty() {
        tokio::select! {
            message = rx.recv(), if accepting => {
                match message {
                    Some(message) => tasks.push(server.dispatch(message)),
                    None => accepting = false,
                }
            }
            Some(result) = tasks.next(), if !tasks.is_empty() => {
                if let Err(error) = result {
                    warn!(panicked = error.is_panic(), cancelled = error.is_cancelled(),
                        "ACP: dispatch task failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod dispatch_tests;
#[cfg(test)]
mod reader_tests;

#[cfg(test)]
mod test_support;
