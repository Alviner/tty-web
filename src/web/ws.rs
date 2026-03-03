//! WebSocket handler implementing the tty-web binary protocol.
//!
//! # Wire protocol
//!
//! All WebSocket messages are **binary frames**. The first byte is the command,
//! the rest is the payload.
//!
//! | Direction | Cmd | Payload | Description |
//! |-----------|------|---------|-------------|
//! | client → server | `0x00` | raw bytes | Terminal input |
//! | client → server | `0x01` | rows(u16 BE) + cols(u16 BE) | Resize |
//! | server → client | `0x00` | raw bytes | Terminal output |
//! | server → client | `0x10` | UUID string | Session ID |
//! | server → client | `0x11` | raw bytes | Scrollback snapshot |
//! | server → client | `0x12` | — | Shell exited |
//! | server → client | `0x13` | rows(u16 BE) + cols(u16 BE) | Window size |

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use tokio::sync::broadcast::error::RecvError;

use crate::session::Session;
use crate::terminal::Terminal;
use crate::web::AppState;

enum ResolveError {
    NotFound(String),
    Io(std::io::Error),
}

/// Client → Server: terminal input.
const CMD_INPUT: u8 = 0x00;
/// Client → Server: resize (4-byte payload: rows u16 BE, cols u16 BE).
const CMD_RESIZE: u8 = 0x01;

/// Server → Client: terminal output.
const CMD_OUTPUT: u8 = 0x00;
/// Server → Client: session UUID string.
const CMD_SESSION_ID: u8 = 0x10;
/// Server → Client: scrollback snapshot on reconnect.
const CMD_SCROLLBACK: u8 = 0x11;
/// Server → Client: shell process exited.
const CMD_SHELL_EXIT: u8 = 0x12;
/// Server → Client: current PTY window size (4-byte payload: rows u16 BE, cols u16 BE).
const CMD_WINDOW_SIZE: u8 = 0x13;

/// WebSocket close code: requested session not found.
const CLOSE_SESSION_NOT_FOUND: u16 = 4404;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let sid = params.get("sid").cloned();
    let readonly = params.contains_key("view");
    ws.on_upgrade(move |socket| handle_socket(socket, state, sid, readonly))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    sid: Option<String>,
    readonly: bool,
) {
    let (session, session_id) = match resolve_session(&state, sid.as_deref()) {
        Ok(result) => result,
        Err(ResolveError::NotFound(id)) => {
            tracing::warn!("session {id} not found");
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: CLOSE_SESSION_NOT_FOUND,
                    reason: "session not found".into(),
                })))
                .await;
            return;
        }
        Err(ResolveError::Io(e)) => {
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
    let (scrollback, mut output_rx, mut window_size_rx) = session.attach();

    // Send scrollback if non-empty
    if !scrollback.is_empty() {
        let mut sb_frame = Vec::with_capacity(1 + scrollback.len());
        sb_frame.push(CMD_SCROLLBACK);
        sb_frame.extend_from_slice(&scrollback);
        if socket.send(Message::Binary(sb_frame.into())).await.is_err() {
            session.detach();
            return;
        }
    }

    // Send current window size (viewers need it to match the PTY)
    {
        let (rows, cols) = *window_size_rx.borrow_and_update();
        if socket.send(Message::Binary(build_window_size_frame(rows, cols).into())).await.is_err() {
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
                        if readonly || data.is_empty() {
                            continue;
                        }
                        handle_client_message(
                            &*session, &data,
                        ).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            Ok(()) = window_size_rx.changed() => {
                let (rows, cols) = *window_size_rx.borrow_and_update();
                if socket.send(Message::Binary(build_window_size_frame(rows, cols).into())).await.is_err() {
                    break;
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

fn resolve_session(
    state: &AppState,
    sid: Option<&str>,
) -> Result<(Arc<Session>, String), ResolveError> {
    if let Some(sid) = sid {
        return state
            .sessions
            .get(sid)
            .map(|session| {
                tracing::info!("reattaching to session {sid}");
                (session, sid.to_owned())
            })
            .ok_or_else(|| ResolveError::NotFound(sid.to_owned()));
    }
    let (terminal, output_rx) =
        Terminal::spawn(&state.shell, state.pwd.as_deref()).map_err(ResolveError::Io)?;
    let session = Session::new(terminal, output_rx, state.scrollback_limit);
    let id = state.sessions.insert(session.clone());
    tracing::info!("created new session {id}");
    Ok((session, id))
}

#[derive(Debug, PartialEq)]
enum ClientCommand<'a> {
    Input(&'a [u8]),
    Resize { rows: u16, cols: u16 },
    Unknown(u8),
}

fn parse_client_message(data: &[u8]) -> Option<ClientCommand<'_>> {
    let (&cmd, payload) = data.split_first()?;
    match cmd {
        CMD_INPUT => Some(ClientCommand::Input(payload)),
        CMD_RESIZE if payload.len() >= 4 => {
            let rows = u16::from_be_bytes([payload[0], payload[1]]);
            let cols = u16::from_be_bytes([payload[2], payload[3]]);
            Some(ClientCommand::Resize { rows, cols })
        }
        CMD_RESIZE => None,
        other => Some(ClientCommand::Unknown(other)),
    }
}

fn build_window_size_frame(rows: u16, cols: u16) -> Vec<u8> {
    vec![
        CMD_WINDOW_SIZE,
        (rows >> 8) as u8, rows as u8,
        (cols >> 8) as u8, cols as u8,
    ]
}

async fn handle_client_message(session: &Session, data: &[u8]) {
    match parse_client_message(data) {
        Some(ClientCommand::Input(payload)) => {
            if let Err(e) = session.terminal.write(payload.to_vec()).await {
                tracing::error!("write to terminal failed: {e}");
            }
        }
        Some(ClientCommand::Resize { rows, cols }) => {
            if let Err(e) = session.terminal.resize(rows, cols) {
                tracing::error!("resize failed: {e}");
            }
            session.set_window_size(rows, cols);
        }
        Some(ClientCommand::Unknown(cmd)) => {
            tracing::warn!("unknown command: 0x{cmd:02x}");
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_input() {
        let data = [0x00, b'h', b'i'];
        assert_eq!(
            parse_client_message(&data),
            Some(ClientCommand::Input(b"hi"))
        );
    }

    #[test]
    fn test_parse_resize() {
        let data = [0x01, 0, 24, 0, 80];
        assert_eq!(
            parse_client_message(&data),
            Some(ClientCommand::Resize { rows: 24, cols: 80 })
        );
    }

    #[test]
    fn test_parse_resize_too_short() {
        let data = [0x01, 0, 24];
        assert_eq!(parse_client_message(&data), None);
    }

    #[test]
    fn test_parse_empty() {
        assert_eq!(parse_client_message(&[]), None);
    }

    #[test]
    fn test_parse_unknown() {
        let data = [0xFF, 1];
        assert_eq!(
            parse_client_message(&data),
            Some(ClientCommand::Unknown(0xFF))
        );
    }
}
