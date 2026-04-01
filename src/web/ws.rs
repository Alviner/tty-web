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
//! | server → client | `0x12` | — | Shell exited |
//! | server → client | `0x13` | rows(u16 BE) + cols(u16 BE) | Window size |
//! | server → client | `0x14` | — | Replay end |

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use tokio::sync::broadcast::error::RecvError;

use crate::session::{ScrollbackEvent, Session};
use crate::terminal::Terminal;
use crate::web::AppState;

/// Client → Server: terminal input.
const CMD_INPUT: u8 = 0x00;
/// Client → Server: resize (4-byte payload: rows u16 BE, cols u16 BE).
const CMD_RESIZE: u8 = 0x01;

/// Server → Client: terminal output.
const CMD_OUTPUT: u8 = 0x00;
/// Server → Client: session UUID string.
const CMD_SESSION_ID: u8 = 0x10;
/// Server → Client: shell process exited.
const CMD_SHELL_EXIT: u8 = 0x12;
/// Server → Client: current PTY window size (4-byte payload: rows u16 BE, cols u16 BE).
const CMD_WINDOW_SIZE: u8 = 0x13;
/// Server → Client: end of scrollback replay.
const CMD_REPLAY_END: u8 = 0x14;

/// WebSocket close code: requested session not found.
const CLOSE_SESSION_NOT_FOUND: u16 = 4404;

/// Send a protocol frame (command byte + payload) over the WebSocket.
async fn send_frame(socket: &mut WebSocket, cmd: u8, payload: &[u8]) -> Result<(), ()> {
    let mut frame = Vec::with_capacity(1 + payload.len());
    frame.push(cmd);
    frame.extend_from_slice(payload);
    socket
        .send(Message::Binary(frame.into()))
        .await
        .map_err(|_| ())
}

/// Encode a window size as 4 big-endian bytes (rows, cols).
fn encode_window_size(rows: u16, cols: u16) -> [u8; 4] {
    let r = rows.to_be_bytes();
    let c = cols.to_be_bytes();
    [r[0], r[1], c[0], c[1]]
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let sid = params.get("sid").cloned();
    let readonly = params.contains_key("view");
    ws.on_upgrade(move |socket| handle_socket(socket, state, sid, readonly))
}

enum ResolveError {
    NotFound(String),
    Io(std::io::Error),
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    sid: Option<String>,
    readonly: bool,
) {
    // Resolve or create session
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

    // Handshake: session ID → window size → replay events → replay end
    if send_frame(&mut socket, CMD_SESSION_ID, session_id.as_bytes())
        .await
        .is_err()
    {
        return;
    }

    let (events, mut output_rx, mut window_size_rx) = session.attach();

    let (rows, cols) = *window_size_rx.borrow_and_update();
    if send_frame(
        &mut socket,
        CMD_WINDOW_SIZE,
        &encode_window_size(rows, cols),
    )
    .await
    .is_err()
    {
        session.detach();
        return;
    }

    // Replay scrollback events
    for event in &events {
        let ok = match event {
            ScrollbackEvent::Output(data) => {
                send_frame(&mut socket, CMD_OUTPUT, data).await.is_ok()
            }
            ScrollbackEvent::WindowSize(r, c) => {
                send_frame(&mut socket, CMD_WINDOW_SIZE, &encode_window_size(*r, *c))
                    .await
                    .is_ok()
            }
        };
        if !ok {
            session.detach();
            return;
        }
    }

    if send_frame(&mut socket, CMD_REPLAY_END, &[]).await.is_err() {
        session.detach();
        return;
    }

    // Main loop: bridge WebSocket ↔ session
    let mut closed_rx = session.terminal.closed();
    loop {
        tokio::select! {
            result = output_rx.recv() => {
                match result {
                    Ok(data) => {
                        if send_frame(&mut socket, CMD_OUTPUT, &data).await.is_err() {
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
                        handle_client_message(&session, &data).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            Ok(()) = window_size_rx.changed() => {
                let (rows, cols) = *window_size_rx.borrow_and_update();
                if send_frame(&mut socket, CMD_WINDOW_SIZE, &encode_window_size(rows, cols)).await.is_err() {
                    break;
                }
            }
            _ = closed_rx.changed() => {
                // Drain buffered output before sending exit
                while let Ok(data) = output_rx.try_recv() {
                    if send_frame(&mut socket, CMD_OUTPUT, &data).await.is_err() {
                        break;
                    }
                }
                let _ = send_frame(&mut socket, CMD_SHELL_EXIT, &[]).await;
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
    let session = Session::new(terminal, output_rx, state.scrollback_limit, state.orphan_timeout);
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
