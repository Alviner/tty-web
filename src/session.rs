//! Persistent terminal sessions with scrollback and lifecycle management.
//!
//! A [`Session`] wraps a [`Terminal`] and adds:
//! - a 64 KB ring-buffer of recent output (scrollback),
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

use tokio::sync::broadcast;

use crate::terminal::Terminal;

/// Maximum scrollback buffer size in bytes.
const SCROLLBACK_LIMIT: usize = 64 * 1024;
/// Time without any attached clients before a session is reaped.
const ORPHAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A persistent terminal session.
///
/// Tracks connected clients, buffers recent output for replay on reconnect,
/// and detects when the session becomes orphaned.
pub struct Session {
    pub terminal: Terminal,
    scrollback: Mutex<VecDeque<u8>>,
    clients: AtomicUsize,
    detached_at: Mutex<Option<Instant>>,
}

impl Session {
    /// Create a new session and spawn a background scrollback collector task.
    pub fn new(terminal: Terminal, output_rx: broadcast::Receiver<Vec<u8>>) -> Arc<Self> {
        let session = Arc::new(Self {
            terminal,
            scrollback: Mutex::new(VecDeque::with_capacity(SCROLLBACK_LIMIT)),
            clients: AtomicUsize::new(0),
            detached_at: Mutex::new(None),
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
                        let mut sb = s.scrollback.lock().unwrap();
                        for &byte in &data {
                            if sb.len() >= SCROLLBACK_LIMIT {
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
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        self.clients.fetch_add(1, Ordering::Relaxed);
        *self.detached_at.lock().unwrap() = None;
        let sb = self.scrollback.lock().unwrap();
        let rx = self.terminal.subscribe();
        let snapshot = sb.iter().copied().collect();
        (snapshot, rx)
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

    fn spawn_session() -> Arc<Session> {
        let (terminal, output_rx) = Terminal::spawn("/bin/sh", None).expect("spawn /bin/sh");
        Session::new(terminal, output_rx)
    }

    #[tokio::test]
    async fn test_attach_detach_clients() {
        let session = spawn_session();

        let (_sb1, _rx1) = session.attach();
        assert_eq!(session.clients.load(Ordering::Relaxed), 1);

        let (_sb2, _rx2) = session.attach();
        assert_eq!(session.clients.load(Ordering::Relaxed), 2);

        session.detach();
        assert_eq!(session.clients.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_not_orphaned_with_clients() {
        let session = spawn_session();
        let (_sb, _rx) = session.attach();
        assert!(!session.is_orphaned());
    }

    #[tokio::test]
    async fn test_not_orphaned_immediately_after_detach() {
        let session = spawn_session();
        let (_sb, _rx) = session.attach();
        session.detach();
        // Timeout hasn't elapsed yet
        assert!(!session.is_orphaned());
    }

    #[tokio::test]
    async fn test_orphaned_after_timeout() {
        let session = spawn_session();
        let (_sb, _rx) = session.attach();
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

        let (scrollback, _rx) = session.attach();
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
}
