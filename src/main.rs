//! **tty-web** — web-based terminal emulator.
//!
//! Opens a real PTY in the browser over WebSocket. Each connection is backed by
//! a persistent session that survives tab closes and reconnects.

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use tty_web::config::{Config, LogFormat};
use tty_web::session::SessionStore;

#[tokio::main]
async fn main() {
    let config = Config::parse();

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    match config.log_format {
        LogFormat::Text => tracing_subscriber::fmt().with_env_filter(filter).init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init(),
    };

    let sessions = SessionStore::new();
    let addr = std::net::SocketAddr::new(config.address, config.port);
    let orphan_timeout = std::time::Duration::from_secs(config.orphan_timeout);
    let app = tty_web::web::router(
        config.shell,
        config.pwd,
        config.scrollback_limit * 1024,
        sessions,
        orphan_timeout,
    );

    let listener = TcpListener::bind(addr).await.unwrap_or_else(|e| {
        tracing::error!("failed to bind to {}: {}", addr, e);
        std::process::exit(1);
    });

    tracing::info!("listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap_or_else(|e| {
            tracing::error!("server error: {}", e);
            std::process::exit(1);
        });
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received Ctrl+C, shutting down");
        }
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM, shutting down");
        }
    }
}
