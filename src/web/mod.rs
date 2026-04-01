//! HTTP and WebSocket server built on [Axum](https://docs.rs/axum).
//!
//! Routes:
//! - `GET /ws` — WebSocket endpoint (terminal I/O)
//! - `GET /api/v1/ping` — health check
//! - `GET /` and `GET /*path` — embedded static frontend

pub mod health;
pub mod static_files;
pub mod ws;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::session::SessionStore;

/// Shared state passed to all request handlers.
#[derive(Clone)]
#[non_exhaustive]
pub struct AppState {
    /// Shell binary path (e.g. `/bin/bash`).
    pub shell: String,
    /// Working directory for new shell sessions.
    pub pwd: Option<PathBuf>,
    /// Scrollback buffer size in bytes.
    pub scrollback_limit: usize,
    /// Global session registry.
    pub sessions: Arc<SessionStore>,
    /// Time without clients before a session is reaped.
    pub orphan_timeout: std::time::Duration,
}

/// Build the Axum router with all routes and shared state.
pub fn router(
    shell: String,
    pwd: Option<PathBuf>,
    scrollback_limit: usize,
    sessions: Arc<SessionStore>,
    orphan_timeout: std::time::Duration,
) -> Router {
    let state = AppState {
        shell,
        pwd,
        scrollback_limit,
        sessions,
        orphan_timeout,
    };
    Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/v1/ping", get(health::ping))
        .route("/", get(static_files::index))
        .route("/{*path}", get(static_files::static_file))
        .with_state(state)
}
