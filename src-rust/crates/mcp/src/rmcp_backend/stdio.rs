//! Stdio transport and its stderr reader have one retirement owner.
use claurst_core::child_diagnostics::DiagnosticReader;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::{TokioChildProcess, Transport};
use rmcp::RoleClient;
use std::future::Future;
use std::io;
use std::process::Stdio;
use tokio::process::Command;
use tokio::sync::watch;

pub(super) struct DiagnosticChild {
    transport: TokioChildProcess,
    stderr: Option<DiagnosticReader>,
    // Keeps the local shutdown channel live for non-plugin connections.
    _lifetime: Option<watch::Sender<bool>>,
}

impl DiagnosticChild {
    pub(super) fn spawn(
        command: Command,
        name: &str,
        lifetime: Option<watch::Sender<bool>>,
    ) -> io::Result<Self> {
        let (transport, stderr) = TokioChildProcess::builder(command)
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self {
            transport,
            stderr: stderr.map(|reader| DiagnosticReader::new(reader, name)),
            _lifetime: lifetime,
        })
    }
}

impl Transport<RoleClient> for DiagnosticChild {
    type Error = io::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        self.transport.send(message)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.transport.receive().await
    }

    async fn close(&mut self) -> io::Result<()> {
        // Keep draining while graceful shutdown waits or kills/reaps the child.
        let child = self.transport.close().await;
        let reader = match &mut self.stderr {
            Some(stderr) => stderr.finish().await,
            None => Ok(()),
        };
        child.and(reader)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::backend::McpClientBackend;
    use crate::rmcp_backend::RmcpClientBackend;
    use claurst_core::config::McpServerConfig;
    use futures::FutureExt;
    use nix::{
        errno::Errno,
        sys::signal::{kill, Signal},
        unistd::Pid,
    };
    use std::collections::HashMap;
    use std::path::Path;
    use tokio::time::{timeout, Duration};

    fn config(script: &str, root: &Path) -> McpServerConfig {
        McpServerConfig {
            name: "stderr-fixture".into(),
            command: Some("python3".into()),
            args: vec![
                "-u".into(),
                "-c".into(),
                script.into(),
                root.display().to_string(),
            ],
            env: HashMap::new(),
            url: None,
            server_type: "stdio".into(),
        }
    }

    async fn wait_file(path: &Path) -> String {
        timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(path).await {
                    if !text.is_empty() {
                        return text;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    async fn reaped(pid: &str) {
        let pid = Pid::from_raw(pid.trim().parse().unwrap());
        timeout(Duration::from_secs(6), async {
            loop {
                // Do not fork a process for every observation: a fork can
                // briefly inherit sibling OAuth tests' live listener FDs.
                match kill(pid, None) {
                    Err(Errno::ESRCH) => break,
                    Ok(()) => {}
                    Err(error) => panic!("cannot inspect owned fixture: {error}"),
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child must be reaped");
    }

    #[tokio::test]
    async fn noisy_stdio_preserves_handshake_rpc_and_joined_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let script = include_str!("../../../../../scripts/fixtures/noisy_mcp.py");
        let backend = timeout(
            Duration::from_secs(5),
            RmcpClientBackend::connect_stdio(&config(script, root.path()), None),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::fs::write(root.path().join("trigger"), "flood")
            .await
            .unwrap();
        wait_file(&root.path().join("noise-flood-sustained")).await;
        // Cross a reporting tick with a continuously full stderr pipe before
        // testing RPC progress. A completed burst is insufficient evidence.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let result = timeout(Duration::from_secs(2), backend.call_tool("ping", None))
            .await
            .unwrap()
            .unwrap();
        assert!(!result.is_error);
        assert!(format!("{result:?}").contains("MCP-PING-OK"));
        tokio::fs::write(root.path().join("trigger"), "")
            .await
            .unwrap();
        wait_file(&root.path().join("noise-flood-done")).await;
        let pid = wait_file(&root.path().join("pid")).await;
        timeout(Duration::from_secs(5), backend.shutdown())
            .await
            .unwrap()
            .unwrap();
        reaped(&pid).await;
        assert!(backend
            .transport_completion
            .as_ref()
            .unwrap()
            .clone()
            .now_or_never()
            .is_some());
    }

    #[tokio::test]
    async fn unexpected_disconnect_joins_reader_and_child() {
        let root = tempfile::tempdir().unwrap();
        let script = include_str!("../../../../../scripts/fixtures/noisy_mcp.py");
        let backend = timeout(
            Duration::from_secs(5),
            RmcpClientBackend::connect_stdio(&config(script, root.path()), None),
        )
        .await
        .unwrap()
        .unwrap();
        let pid = wait_file(&root.path().join("pid")).await;
        tokio::fs::write(root.path().join("trigger"), "disconnect")
            .await
            .unwrap();
        timeout(
            Duration::from_secs(5),
            backend.transport_completion.as_ref().unwrap().clone(),
        )
        .await
        .unwrap()
        .unwrap();
        reaped(&pid).await;
    }

    #[tokio::test]
    async fn dropping_connected_backend_retires_transport_and_reader() {
        let root = tempfile::tempdir().unwrap();
        let script = include_str!("../../../../../scripts/fixtures/noisy_mcp.py");
        let backend = timeout(
            Duration::from_secs(5),
            RmcpClientBackend::connect_stdio(&config(script, root.path()), None),
        )
        .await
        .unwrap()
        .unwrap();
        let pid = wait_file(&root.path().join("pid")).await;
        let completion = backend.transport_completion.as_ref().unwrap().clone();
        drop(backend);
        timeout(Duration::from_secs(5), completion)
            .await
            .unwrap()
            .unwrap();
        reaped(&pid).await;
    }

    #[tokio::test]
    async fn initialization_failure_drains_output_and_reaps_child() {
        let root = tempfile::tempdir().unwrap();
        let script = "import os,pathlib,sys; pathlib.Path(sys.argv[1],'pid').write_text(str(os.getpid())); os.write(2,b'x'*2000000); sys.exit(7)";
        let result = timeout(
            Duration::from_secs(6),
            RmcpClientBackend::connect_stdio(&config(script, root.path()), None),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        reaped(&wait_file(&root.path().join("pid")).await).await;
    }

    #[tokio::test]
    async fn cancelling_initialization_retires_child_and_reader() {
        let root = tempfile::tempdir().unwrap();
        let config = config("import os,pathlib,sys,time; pathlib.Path(sys.argv[1],'pid').write_text(str(os.getpid())); os.write(2,b'partial'); time.sleep(30)", root.path());
        let task =
            tokio::spawn(async move { RmcpClientBackend::connect_stdio(&config, None).await });
        let pid = wait_file(&root.path().join("pid")).await;
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        reaped(&pid).await;
    }

    #[tokio::test]
    async fn withdrawal_during_initialization_joins_retirement() {
        let root = tempfile::tempdir().unwrap();
        let config = config("import os,pathlib,sys,time; pathlib.Path(sys.argv[1],'pid').write_text(str(os.getpid())); time.sleep(30)", root.path());
        let (stop, shutdown) = watch::channel(false);
        let task =
            tokio::spawn(
                async move { RmcpClientBackend::connect_stdio(&config, Some(shutdown)).await },
            );
        let pid = wait_file(&root.path().join("pid")).await;
        stop.send_replace(true);
        assert!(timeout(Duration::from_secs(6), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        reaped(&pid).await;
    }

    #[tokio::test]
    async fn descendant_stderr_does_not_hold_reader_shutdown_open() {
        let root = tempfile::tempdir().unwrap();
        let config = config("import os,pathlib,subprocess,sys; p=subprocess.Popen(['sleep','30'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL); pathlib.Path(sys.argv[1],'descendant').write_text(str(p.pid)); os.write(2,b'final partial')", root.path());
        let mut command = Command::new(config.command.as_ref().unwrap());
        command.args(&config.args);
        let mut transport = DiagnosticChild::spawn(command, "fixture", None).unwrap();
        let pid = wait_file(&root.path().join("descendant")).await;
        let result = timeout(Duration::from_secs(2), transport.close()).await;
        // The test owns the intentionally surviving descendant, not the transport.
        let _ = kill(Pid::from_raw(pid.trim().parse().unwrap()), Signal::SIGTERM);
        result
            .expect("inherited stderr must not extend shutdown")
            .unwrap();
        transport.close().await.unwrap();
    }
}
