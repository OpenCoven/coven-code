//! Invocation-local observation of an actual model transport attempt.
//!
//! A round can include transport retries, so this is a latch, not a request
//! counter. Preparation/authentication never marks it. Task-local scoping keeps
//! concurrent queries and nested invocations from sharing accounting state.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

tokio::task_local! {
    static ACTIVE_DISPATCH: Arc<AtomicBool>;
}

#[derive(Default)]
pub struct ModelDispatch {
    attempted: Arc<AtomicBool>,
}

impl ModelDispatch {
    pub async fn scope<F: Future>(&self, future: F) -> F::Output {
        ACTIVE_DISPATCH.scope(self.attempted.clone(), future).await
    }

    pub fn was_attempted(&self) -> bool {
        self.attempted.load(Ordering::Acquire)
    }
}

/// Call immediately before model HTTP send or after successful CLI spawn.
/// Calls outside a query-owned tracking scope leave ordinary API use unchanged.
pub fn mark_attempted() {
    let _ = ACTIVE_DISPATCH.try_with(|attempted| attempted.store(true, Ordering::Release));
}
