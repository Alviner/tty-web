mod health;
mod static_files;
mod ws;

use axum::Router;
use axum::routing::get;

pub fn router(shell: String) -> Router {
    Router::new()
        .route("/ws", get(ws::ws_handler).with_state(shell))
        .route("/api/v1/ping", get(health::ping))
        .route("/", get(static_files::index))
        .route("/{*path}", get(static_files::static_file))
}
