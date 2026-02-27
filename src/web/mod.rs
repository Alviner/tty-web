mod health;
mod static_files;
mod ws;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::session::SessionStore;

#[derive(Clone)]
pub struct AppState {
    pub shell: String,
    pub sessions: Arc<SessionStore>,
}

pub fn router(shell: String, sessions: Arc<SessionStore>) -> Router {
    let state = AppState { shell, sessions };
    Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/v1/ping", get(health::ping))
        .route("/", get(static_files::index))
        .route("/{*path}", get(static_files::static_file))
        .with_state(state)
}
