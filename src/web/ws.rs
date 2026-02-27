use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use tokio::sync::broadcast::error::RecvError;

use crate::terminal::Terminal;

const CMD_INPUT: u8 = 0x00;
const CMD_RESIZE: u8 = 0x01;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(shell): State<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, shell))
}

async fn handle_socket(mut socket: WebSocket, shell: String) {
    let (terminal, mut output_rx) = match Terminal::spawn(&shell) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("failed to spawn terminal: {}", e);
            return;
        }
    };

    loop {
        tokio::select! {
            result = output_rx.recv() => {
                match result {
                    Ok(data) => {
                        if socket
                            .send(Message::Binary(data.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!("lagged {} messages", n);
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if data.is_empty() {
                            continue;
                        }
                        handle_binary_message(
                            &terminal, &data,
                        ).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

async fn handle_binary_message(terminal: &Terminal, data: &[u8]) {
    let cmd = data[0];
    let payload = &data[1..];

    match cmd {
        CMD_INPUT => {
            if let Err(e) = terminal.write(payload.to_vec()).await {
                tracing::error!("write to terminal failed: {}", e);
            }
        }
        CMD_RESIZE => {
            if payload.len() >= 4 {
                let rows =
                    u16::from_be_bytes([payload[0], payload[1]]);
                let cols =
                    u16::from_be_bytes([payload[2], payload[3]]);
                if let Err(e) = terminal.resize(rows, cols) {
                    tracing::error!("resize failed: {}", e);
                }
            }
        }
        _ => {
            tracing::warn!("unknown command: 0x{:02x}", cmd);
        }
    }
}
