mod config;
mod pty;
mod session;
mod terminal;
mod web;

use clap::Parser;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::session::SessionStore;

#[tokio::main]
async fn main() {
    let config = Config::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new(&config.log_level)
            }),
        )
        .init();

    let sessions = SessionStore::new();
    let addr = std::net::SocketAddr::new(config.address, config.port);
    let app = web::router(config.shell, sessions);

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
    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    )
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
