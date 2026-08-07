//! Lazily-started connection to the elevated helper.
//!
//! The session is created on first privileged use, not at startup — that is the whole
//! point of the broker design. Opening SIO to look at the dashboard must never produce
//! a UAC prompt.
//!
//! Once running, the session is reused for the rest of the process lifetime, so a bulk
//! install of thirty apps prompts once rather than thirty times.

use sio_core::error::{Error, Result};
use sio_core::privileged::PrivilegedOps;
use sio_winsys::broker::{self, Session};
use sio_winsys::elevation;
use sio_winsys::InProcessOps;
use std::sync::Arc;
use tokio::sync::Mutex;

/// How this process performs privileged work.
///
/// Kept as a concrete enum rather than a bare `Arc<dyn PrivilegedOps>` so liveness can
/// actually be checked — a trait object would have to be downcast to ask the session
/// whether its process is still there.
#[derive(Clone)]
enum Route {
    /// The app is already elevated; no helper needed.
    InProcess(Arc<InProcessOps>),
    /// Talking to the elevated helper over a pipe.
    Broker(Arc<Session>),
}

impl Route {
    fn is_alive(&self) -> bool {
        match self {
            Route::InProcess(_) => true,
            Route::Broker(session) => session.is_alive(),
        }
    }

    fn ops(&self) -> Arc<dyn PrivilegedOps> {
        match self {
            Route::InProcess(ops) => Arc::clone(ops) as Arc<dyn PrivilegedOps>,
            Route::Broker(session) => Arc::clone(session) as Arc<dyn PrivilegedOps>,
        }
    }
}

#[derive(Default)]
pub struct BrokerState {
    inner: Mutex<Option<Route>>,
}

impl std::fmt::Debug for BrokerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerState").finish_non_exhaustive()
    }
}

impl BrokerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a privileged-operations handle, starting the helper if needed.
    ///
    /// Shows a UAC prompt the first time. Returns [`Error::ElevationDeclined`] if the
    /// user dismisses it — callers should treat that as a cancellation, not a fault.
    pub async fn get(&self) -> Result<Arc<dyn PrivilegedOps>> {
        let mut guard = self.inner.lock().await;

        // Replace a route whose helper has died, so a crashed broker costs one extra
        // UAC prompt rather than breaking every later operation.
        if let Some(route) = guard.as_ref() {
            if route.is_alive() {
                return Ok(route.ops());
            }
            tracing::warn!("the elevated helper exited; starting a new one");
            *guard = None;
        }

        let route = if elevation::is_elevated().unwrap_or(false) {
            // Already elevated: a helper would be pure overhead and an extra prompt.
            tracing::info!("running elevated; using in-process operations");
            Route::InProcess(Arc::new(InProcessOps::new()))
        } else {
            let exe = broker::broker_path()?;
            if !exe.exists() {
                return Err(Error::Broker {
                    reason: format!("the helper is missing from {}", exe.display()),
                });
            }
            tracing::info!("starting the elevated helper from {}", exe.display());
            Route::Broker(Arc::new(Session::launch(&exe).await?))
        };

        let ops = route.ops();
        *guard = Some(route);
        Ok(ops)
    }

    /// Whether a live privileged route already exists — i.e. whether the next
    /// privileged action will prompt.
    pub async fn is_connected(&self) -> bool {
        self.inner
            .lock()
            .await
            .as_ref()
            .is_some_and(Route::is_alive)
    }
}
