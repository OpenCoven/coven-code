//! Keeps the concrete transport owned through initialization, cancellation, and
//! close. Dropping rmcp's initialization future cannot detach child cleanup.

use futures::{
    future::{BoxFuture, FutureExt, Shared},
    stream::{FuturesUnordered, StreamExt},
};
use rmcp::{
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
    RoleClient,
};
use tokio::sync::{mpsc, oneshot, watch};

pub(crate) type Completion = Shared<BoxFuture<'static, Result<(), String>>>;
#[derive(Debug, thiserror::Error)]
#[error("MCP transport retirement failed: {0}")]
pub(crate) struct RetirementError(pub(crate) String);

type Outgoing = (
    TxJsonRpcMessage<RoleClient>,
    oneshot::Sender<Result<(), String>>,
);

pub(crate) struct OwnedTransport {
    outgoing: mpsc::Sender<Outgoing>,
    incoming: mpsc::Receiver<RxJsonRpcMessage<RoleClient>>,
    close: watch::Sender<bool>,
    pub(crate) completion: Completion,
}

impl OwnedTransport {
    pub(crate) fn new<T>(mut transport: T, mut shutdown: watch::Receiver<bool>) -> Self
    where
        T: Transport<RoleClient> + 'static,
    {
        let (outgoing, mut requests) = mpsc::channel::<Outgoing>(32);
        let (messages, incoming) = mpsc::channel(100);
        let (close, mut closed) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut sends = FuturesUnordered::<BoxFuture<'static, ()>>::new();
            loop {
                if *shutdown.borrow() || *closed.borrow() {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    _ = closed.changed() => break,
                    _ = sends.next(), if !sends.is_empty() => {},
                    request = requests.recv() => {
                        let Some((message, responder)) = request else { break; };
                        let send = transport.send(message);
                        sends.push(async move {
                            let _ = responder.send(send.await.map_err(|error| error.to_string()));
                        }.boxed());
                    },
                    message = transport.receive() => {
                        let Some(message) = message else { break; };
                        tokio::select! {
                            biased;
                            _ = shutdown.changed() => break,
                            _ = closed.changed() => break,
                            result = messages.send(message) => if result.is_err() { break; },
                        }
                    },
                }
            }
            // Cancel pending sends before closing the underlying transport. Its
            // close owns the actual subprocess wait/kill/reap or HTTP worker join.
            drop(sends);
            drop(requests);
            let result = transport.close().await.map_err(|error| error.to_string());
            drop(messages);
            result
        });
        let completion = async move { task.await.map_err(|error| error.to_string())? }
            .boxed()
            .shared();
        Self {
            outgoing,
            incoming,
            close,
            completion,
        }
    }
}

impl Transport<RoleClient> for OwnedTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let outgoing = self.outgoing.clone();
        async move {
            let (sender, receiver) = oneshot::channel();
            outgoing
                .send((message, sender))
                .await
                .map_err(|_| std::io::Error::other("MCP transport closed"))?;
            receiver
                .await
                .map_err(|_| std::io::Error::other("MCP transport closed"))?
                .map_err(std::io::Error::other)
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.incoming.recv().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.close.send_replace(true);
        self.completion.clone().await.map_err(std::io::Error::other)
    }
}
