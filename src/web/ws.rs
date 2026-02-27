use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use tokio::sync::broadcast::error::RecvError;

use crate::session::Session;
use crate::terminal::Terminal;
use crate::web::AppState;

// Client → Server
const CMD_INPUT: u8 = 0x00;
const CMD_RESIZE: u8 = 0x01;

// Server → Client
const CMD_OUTPUT: u8 = 0x00;
const CMD_SESSION_ID: u8 = 0x10;
const CMD_SCROLLBACK: u8 = 0x11;
const CMD_SHELL_EXIT: u8 = 0x12;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let sid = params.get("sid").cloned();
    ws.on_upgrade(move |socket| handle_socket(socket, state, sid))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    sid: Option<String>,
) {
    let (session, session_id) = match resolve_or_create(&state, sid) {
        Ok(result) => result,
        Err(e) => {
            tracing::error!("failed to create session: {e}");
            return;
        }
    };

    // Send session ID
    let mut sid_frame = Vec::with_capacity(1 + session_id.len());
    sid_frame.push(CMD_SESSION_ID);
    sid_frame.extend_from_slice(session_id.as_bytes());
    if socket
        .send(Message::Binary(sid_frame.into()))
        .await
        .is_err()
    {
        return;
    }

    // Attach: subscribe + scrollback snapshot (atomically, no gaps)
    let (scrollback, mut output_rx) = session.attach();

    // Send scrollback if non-empty
    if !scrollback.is_empty() {
        let mut sb_frame = Vec::with_capacity(1 + scrollback.len());
        sb_frame.push(CMD_SCROLLBACK);
        sb_frame.extend_from_slice(&scrollback);
        if socket
            .send(Message::Binary(sb_frame.into()))
            .await
            .is_err()
        {
            session.detach();
            return;
        }
    }

    // Main loop: bridge WebSocket ↔ session
    let mut closed_rx = session.terminal.closed();
    loop {
        tokio::select! {
            result = output_rx.recv() => {
                match result {
                    Ok(data) => {
                        let mut frame = Vec::with_capacity(1 + data.len());
                        frame.push(CMD_OUTPUT);
                        frame.extend_from_slice(&data);
                        if socket
                            .send(Message::Binary(frame.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::warn!("output lagged {n} messages");
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
                        handle_client_message(
                            &session.terminal, &data,
                        ).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            _ = closed_rx.changed() => {
                // Drain buffered output before sending exit
                while let Ok(data) = output_rx.try_recv() {
                    let mut frame = Vec::with_capacity(1 + data.len());
                    frame.push(CMD_OUTPUT);
                    frame.extend_from_slice(&data);
                    if socket
                        .send(Message::Binary(frame.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                let _ = socket
                    .send(Message::Binary(vec![CMD_SHELL_EXIT].into()))
                    .await;
                break;
            }
        }
    }
    session.detach();
}

fn resolve_or_create(
    state: &AppState,
    sid: Option<String>,
) -> std::io::Result<(Arc<Session>, String)> {
    if let Some(sid) = sid
        && let Some(session) = state.sessions.get(&sid)
    {
        tracing::info!("reattaching to session {sid}");
        return Ok((session, sid));
    }
    let (terminal, output_rx) = Terminal::spawn(&state.shell)?;
    let session = Session::new(terminal, output_rx);
    let id = state.sessions.insert(session.clone());
    tracing::info!("created new session {id}");
    Ok((session, id))
}

async fn handle_client_message(terminal: &Terminal, data: &[u8]) {
    let cmd = data[0];
    let payload = &data[1..];

    match cmd {
        CMD_INPUT => {
            if let Err(e) = terminal.write(payload.to_vec()).await {
                tracing::error!("write to terminal failed: {e}");
            }
        }
        CMD_RESIZE => {
            if payload.len() >= 4 {
                let rows =
                    u16::from_be_bytes([payload[0], payload[1]]);
                let cols =
                    u16::from_be_bytes([payload[2], payload[3]]);
                if let Err(e) = terminal.resize(rows, cols) {
                    tracing::error!("resize failed: {e}");
                }
            }
        }
        _ => {
            tracing::warn!("unknown command: 0x{cmd:02x}");
        }
    }
}
