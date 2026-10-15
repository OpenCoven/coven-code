//! Interrupt HTTP I/O at withdrawal without detaching rmcp's transport worker.
//! The lifetime channel also waits for rmcp's SSE tasks to release their clients
//! and response streams after the worker has been joined.

use futures::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    model::ClientJsonRpcMessage,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{
        common::client_side_sse::BoxedSseResponse,
        streamable_http_client::{
            StreamableHttpClient, StreamableHttpClientTransportConfig, StreamableHttpError,
            StreamableHttpPostResponse,
        },
        StreamableHttpClientTransport, Transport,
    },
    RoleClient,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, watch};

type HttpError = StreamableHttpError<reqwest::Error>;

#[derive(Clone)]
struct RevocableHttpClient {
    client: reqwest::Client,
    shutdown: watch::Receiver<bool>,
    lifetime: mpsc::Sender<()>,
}

impl RevocableHttpClient {
    async fn guard<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T, HttpError>>,
    ) -> Result<T, HttpError> {
        let mut shutdown = self.shutdown.clone();
        if *shutdown.borrow() {
            return Err(StreamableHttpError::TransportChannelClosed);
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => Err(StreamableHttpError::TransportChannelClosed),
            result = operation => result,
        }
    }

    fn stream(&self, stream: BoxedSseResponse) -> BoxedSseResponse {
        let mut shutdown = self.shutdown.clone();
        let lifetime = self.lifetime.clone();
        Box::pin(
            stream
                .take_until(async move {
                    if !*shutdown.borrow() {
                        let _ = shutdown.changed().await;
                    }
                })
                .inspect(move |_| {
                    let _ = &lifetime;
                }),
        )
    }
}

impl StreamableHttpClient for RevocableHttpClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        match self
            .guard(
                self.client
                    .post_message(uri, message, session_id, auth_header, custom_headers),
            )
            .await?
        {
            StreamableHttpPostResponse::Sse(stream, session) => Ok(
                StreamableHttpPostResponse::Sse(self.stream(stream), session),
            ),
            response => Ok(response),
        }
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxedSseResponse, HttpError> {
        let stream = self
            .guard(self.client.get_stream(
                uri,
                session_id,
                last_event_id,
                auth_header,
                custom_headers,
            ))
            .await?;
        Ok(self.stream(stream))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), HttpError> {
        // This is transport cleanup, not a newly authorized request. Preserve
        // rmcp's session deletion and its bounded cleanup policy on withdrawal.
        self.client
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
    }
}

pub(crate) struct OwnedHttpTransport {
    inner: StreamableHttpClientTransport<RevocableHttpClient>,
    lifetimes: mpsc::Receiver<()>,
}

impl OwnedHttpTransport {
    pub(crate) fn new(
        config: StreamableHttpClientTransportConfig,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        let (lifetime, lifetimes) = mpsc::channel(1);
        let client = RevocableHttpClient {
            client: reqwest::Client::new(),
            shutdown,
            lifetime,
        };
        Self {
            inner: StreamableHttpClientTransport::with_client(client, config),
            lifetimes,
        }
    }
}

impl Transport<RoleClient> for OwnedHttpTransport {
    type Error = HttpError;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        self.inner.receive().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let result = self.inner.close().await;
        // No sender sends values. Closure proves every client/stream owner is
        // gone, including SSE tasks aborted internally by rmcp's JoinSet Drop.
        let _ = self.lifetimes.recv().await;
        result
    }
}
