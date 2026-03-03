//! HTTP and WebSocket server built on [Axum](https://docs.rs/axum).
//!
//! Routes:
//! - `GET /ws` — WebSocket endpoint (terminal I/O)
//! - `GET /api/v1/ping` — health check
//! - `GET /` and `GET /*path` — embedded static frontend

mod health;
mod static_files;
mod ws;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::session::SessionStore;

/// Shared state passed to all request handlers.
#[derive(Clone)]
pub struct AppState {
    /// Shell binary path (e.g. `/bin/bash`).
    pub shell: String,
    /// Working directory for new shell sessions.
    pub pwd: Option<PathBuf>,
    /// Global session registry.
    pub sessions: Arc<SessionStore>,
}

/// Build the Axum router with all routes and shared state.
pub fn router(shell: String, pwd: Option<PathBuf>, sessions: Arc<SessionStore>) -> Router {
    let state = AppState {
        shell,
        pwd,
        sessions,
    };
    Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/v1/ping", get(health::ping))
        .route("/", get(static_files::index))
        .route("/{*path}", get(static_files::static_file))
        .with_state(state)
}
