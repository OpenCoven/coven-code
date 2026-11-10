//! Connection-local admission. Control notifications never wait behind handlers
//! or output. Exhausting the bounded error lane retires the connection instead
//! of allocating another task or blocking the reader.
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol_schema as acp;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

use crate::connection::{Inbound, ReadEvent};
use crate::AgentServer;

pub(crate) const MAX_HANDLERS: usize = 32;
const MAX_REJECTIONS: usize = 64;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

struct Admitted {
    message: Inbound,
    permit: OwnedSemaphorePermit,
}

pub(crate) async fn run<R>(server: Arc<AgentServer>, reader: R) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let permits = Arc::new(Semaphore::new(MAX_HANDLERS));
    // Every queued or running message owns one of the same 32 permits.
    let (tx, mut rx) = mpsc::unbounded_channel::<Admitted>();
    let (reject_tx, mut reject_rx) = mpsc::channel(MAX_REJECTIONS);
    let routing_server = server.clone();
    let reader = crate::connection::run_reader_with_handler(
        server.connection.clone(),
        reader,
        move |event| {
            let outcome = match event {
                ReadEvent::Message(Inbound::Notification { method, params }) => {
                    routing_server.handle_control_notification(&method, params);
                    Ok(())
                }
                ReadEvent::Message(message) => match permits.clone().try_acquire_owned() {
                    Ok(permit) => tx
                        .send(Admitted { message, permit })
                        .map_err(|_| anyhow::anyhow!("ACP dispatcher closed")),
                    Err(_) => {
                        let Inbound::Request { id, .. } = message else {
                            return std::future::ready(Ok(()));
                        };
                        reject_tx
                            .try_send((
                                id,
                                acp::Error::new(-32000, "ACP request capacity exceeded"),
                            ))
                            .map_err(|_| anyhow::anyhow!("ACP error response capacity exceeded"))
                    }
                },
                ReadEvent::Error { id, error } => reject_tx
                    .try_send((id, error))
                    .map_err(|_| anyhow::anyhow!("ACP error response capacity exceeded")),
            };
            std::future::ready(outcome)
        },
    );
    let dispatch_server = server.clone();
    let dispatch = async move {
        let mut tasks = JoinSet::new();
        while let Some(admitted) = rx.recv().await {
            while let Some(result) = tasks.try_join_next() {
                report_task(result);
            }
            let server = dispatch_server.clone();
            tasks.spawn(async move {
                let _permit = admitted.permit;
                server.handle_message(admitted.message).await;
            });
        }
        let drain = async {
            while let Some(result) = tasks.join_next().await {
                report_task(result);
            }
        };
        if tokio::time::timeout(DRAIN_TIMEOUT, drain).await.is_err() {
            dispatch_server.sessions.cancel_all();
            tasks.shutdown().await;
            anyhow::bail!("ACP handlers exceeded shutdown deadline");
        }
        Ok(())
    };
    let stopped = tokio_util::sync::CancellationToken::new();
    let read_server = server.clone();
    let read_stopped = stopped.clone();
    let reader = async move {
        let result = tokio::select! {
            result = reader => result,
            _ = read_stopped.cancelled() => Err(anyhow::anyhow!("ACP response writer stopped")),
        };
        if result.is_err() {
            read_server.sessions.cancel_all();
        }
        result
    };
    let connection = server.connection.clone();
    let rejection = async {
        while let Some((id, error)) = reject_rx.recv().await {
            // A client which stops reading cannot retain this writer forever.
            tokio::time::timeout(DRAIN_TIMEOUT, connection.send_error_response(id, error))
                .await
                .map_err(|_| anyhow::anyhow!("ACP error response write timed out"))??;
        }
        Ok::<(), anyhow::Error>(())
    };
    let rejection = async {
        let result = rejection.await;
        if result.is_err() {
            stopped.cancel();
            server.sessions.cancel_all();
        }
        result
    };
    // Always join the dispatcher, including on reader/writer failure, so every
    // handler is either drained or cancelled and reaped before returning.
    let (read_result, dispatch_result, rejection_result) =
        tokio::join!(reader, dispatch, rejection);
    read_result?;
    dispatch_result?;
    rejection_result
}

fn report_task(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::warn!(
            panicked = error.is_panic(),
            cancelled = error.is_cancelled(),
            "ACP: dispatch task failed"
        );
    }
}
