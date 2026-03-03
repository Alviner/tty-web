//! Persistent terminal sessions with scrollback and lifecycle management.
//!
//! A [`Session`] wraps a [`Terminal`] and adds:
//! - a configurable ring-buffer of recent output (scrollback, default 256 KiB),
//! - client attach/detach tracking,
//! - orphan detection (no clients for 60 s → auto-remove).
//!
//! [`SessionStore`] is the global session registry. Each session gets a reaper
//! task that periodically checks for removal conditions.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Instant;

use tokio::sync::{broadcast, watch};

use crate::terminal::Terminal;

/// Time without any attached clients before a session is reaped.
const ORPHAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A persistent terminal session.
///
/// Tracks connected clients, buffers recent output for replay on reconnect,
/// and detects when the session becomes orphaned.
pub struct Session {
    pub terminal: Terminal,
    scrollback: Mutex<VecDeque<u8>>,
    scrollback_limit: usize,
    clients: AtomicUsize,
    detached_at: Mutex<Option<Instant>>,
    window_size: watch::Sender<(u16, u16)>,
}

/// Skip to the first ESC byte (`\x1b`) in a scrollback snapshot to avoid
/// sending partial escape sequences or broken UTF-8 from ring-buffer wrap.
fn sanitize_scrollback_start(buf: &[u8]) -> usize {
    buf.iter().position(|&b| b == 0x1b).unwrap_or(0)
}

impl Session {
    /// Create a new session and spawn a background scrollback collector task.
    pub fn new(terminal: Terminal, output_rx: broadcast::Receiver<Vec<u8>>, scrollback_limit: usize) -> Arc<Self> {
        let (ws_tx, _) = watch::channel((24, 80));
        let session = Arc::new(Self {
            terminal,
            scrollback: Mutex::new(VecDeque::with_capacity(scrollback_limit)),
            scrollback_limit,
            clients: AtomicUsize::new(0),
            detached_at: Mutex::new(None),
            window_size: ws_tx,
        });

        // Scrollback collector
        let weak: Weak<Session> = Arc::downgrade(&session);
        let mut rx = output_rx;
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(data) => {
                        let Some(s) = weak.upgrade() else {
                            break;
                        };
                        let limit = s.scrollback_limit;
                        let mut sb = s.scrollback.lock().unwrap();
                        for &byte in &data {
                            if sb.len() >= limit {
                                sb.pop_front();
                            }
                            sb.push_back(byte);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        session
    }

    /// Attach a client: increment the counter, subscribe to live output, and
    /// return a scrollback snapshot. The subscription and snapshot are taken
    /// under the same lock so no output is lost.
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>, watch::Receiver<(u16, u16)>) {
        self.clients.fetch_add(1, Ordering::Relaxed);
        *self.detached_at.lock().unwrap() = None;
        let sb = self.scrollback.lock().unwrap();
        let rx = self.terminal.subscribe();
        let ws_rx = self.window_size.subscribe();
        let raw: Vec<u8> = sb.iter().copied().collect();
        let skip = sanitize_scrollback_start(&raw);
        (raw[skip..].to_vec(), rx, ws_rx)
    }

    /// Update the current PTY window size (broadcast to viewers).
    pub fn set_window_size(&self, rows: u16, cols: u16) {
        let _ = self.window_size.send((rows, cols));
    }

    /// Detach a client. When the last client detaches, the orphan timer starts.
    pub fn detach(&self) {
        if self.clients.fetch_sub(1, Ordering::Relaxed) == 1 {
            *self.detached_at.lock().unwrap() = Some(Instant::now());
        }
    }

    fn is_orphaned(&self) -> bool {
        if self.clients.load(Ordering::Relaxed) > 0 {
            return false;
        }
        match *self.detached_at.lock().unwrap() {
            Some(t) => t.elapsed() >= ORPHAN_TIMEOUT,
            None => false,
        }
    }
}

/// Thread-safe session registry keyed by UUID.
pub struct SessionStore {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl SessionStore {
    /// Create an empty session store.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
        })
    }

    /// Register a session under a new UUID and spawn a reaper task that
    /// removes it when the shell exits with no clients or the orphan timeout
    /// elapses.
    pub fn insert(self: &Arc<Self>, session: Arc<Session>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.sessions
            .write()
            .unwrap()
            .insert(id.clone(), session.clone());

        // Reaper task: periodically checks for removal conditions
        let store = Arc::downgrade(self);
        let sid = id.clone();
        let closed_rx = session.terminal.closed();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let Some(store) = store.upgrade() else { return };
                let should_remove = {
                    let sessions = store.sessions.read().unwrap();
                    match sessions.get(&sid) {
                        Some(s) => {
                            s.is_orphaned()
                                || (*closed_rx.borrow() && s.clients.load(Ordering::Relaxed) == 0)
                        }
                        None => return,
                    }
                };
                if should_remove {
                    store.sessions.write().unwrap().remove(&sid);
                    tracing::info!("removed session {sid}");
                    return;
                }
            }
        });

        id
    }

    /// Look up a session by ID.
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap().get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SCROLLBACK_LIMIT: usize = 256 * 1024;

    fn spawn_session() -> Arc<Session> {
        let (terminal, output_rx) = Terminal::spawn("/bin/sh", None).expect("spawn /bin/sh");
        Session::new(terminal, output_rx, TEST_SCROLLBACK_LIMIT)
    }

    #[tokio::test]
    async fn test_attach_detach_clients() {
        let session = spawn_session();

        let (_sb1, _rx1, _ws1) = session.attach();
        assert_eq!(session.clients.load(Ordering::Relaxed), 1);

        let (_sb2, _rx2, _ws2) = session.attach();
        assert_eq!(session.clients.load(Ordering::Relaxed), 2);

        session.detach();
        assert_eq!(session.clients.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_not_orphaned_with_clients() {
        let session = spawn_session();
        let (_sb, _rx, _ws) = session.attach();
        assert!(!session.is_orphaned());
    }

    #[tokio::test]
    async fn test_not_orphaned_immediately_after_detach() {
        let session = spawn_session();
        let (_sb, _rx, _ws) = session.attach();
        session.detach();
        // Timeout hasn't elapsed yet
        assert!(!session.is_orphaned());
    }

    #[tokio::test]
    async fn test_orphaned_after_timeout() {
        let session = spawn_session();
        let (_sb, _rx, _ws) = session.attach();
        session.detach();
        // Simulate that detach happened 61 seconds ago
        *session.detached_at.lock().unwrap() =
            Some(Instant::now() - ORPHAN_TIMEOUT - std::time::Duration::from_secs(1));
        assert!(session.is_orphaned());
    }

    #[tokio::test]
    async fn test_scrollback_captures_output() {
        let session = spawn_session();

        session
            .terminal
            .write(b"echo scrollback_test_marker\n".to_vec())
            .await
            .unwrap();

        // Give the shell time to produce output
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let (scrollback, _rx, _ws) = session.attach();
        let text = String::from_utf8_lossy(&scrollback);
        assert!(
            text.contains("scrollback_test_marker"),
            "scrollback should contain marker, got: {text}"
        );
    }

    #[tokio::test]
    async fn test_session_store_insert_and_get() {
        let store = SessionStore::new();
        let session = spawn_session();
        let id = store.insert(session);

        assert!(store.get(&id).is_some());
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_sanitize_clean_start() {
        // Starts with ESC — nothing to skip
        let buf = b"\x1b[32mhello";
        assert_eq!(sanitize_scrollback_start(buf), 0);
    }

    #[test]
    fn test_sanitize_partial_csi() {
        // Tail of \x1b[48;2;30;30;46m — skip to next ESC
        let buf = b"8;2;30;30;46m\x1b[32mok";
        assert_eq!(sanitize_scrollback_start(buf), 13);
    }

    #[test]
    fn test_sanitize_broken_utf8() {
        // UTF-8 continuation bytes before ESC
        let buf = [0x80, 0xBF, 0x1b, b'[', b'0', b'm'];
        assert_eq!(sanitize_scrollback_start(&buf), 2);
    }

    #[test]
    fn test_sanitize_plain_text() {
        // No ESC at all — keep everything
        let buf = b"hello world";
        assert_eq!(sanitize_scrollback_start(buf), 0);
    }

    #[test]
    fn test_sanitize_empty() {
        assert_eq!(sanitize_scrollback_start(b""), 0);
    }
}
