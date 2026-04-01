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

/// Default time without any attached clients before a session is reaped.
pub const DEFAULT_ORPHAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Return type of [`Session::attach`]: scrollback events, output stream,
/// and window-size watch.
pub type AttachResult = (
    Vec<ScrollbackEvent>,
    broadcast::Receiver<Vec<u8>>,
    watch::Receiver<(u16, u16)>,
);

/// A scrollback event — either terminal output or a window-size change.
///
/// Storing events instead of raw bytes ensures that eviction never splits
/// an escape sequence and that resize history is preserved for replay.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollbackEvent {
    /// Raw terminal output bytes.
    Output(Vec<u8>),
    /// PTY window size changed (rows, cols).
    WindowSize(u16, u16),
}

impl ScrollbackEvent {
    /// Logical byte cost used for eviction accounting.
    fn byte_cost(&self) -> usize {
        match self {
            Self::Output(data) => data.len(),
            Self::WindowSize(_, _) => 4,
        }
    }
}

/// A persistent terminal session.
///
/// Tracks connected clients, buffers recent output for replay on reconnect,
/// and detects when the session becomes orphaned.
pub struct Session {
    pub terminal: Terminal,
    scrollback: Mutex<VecDeque<ScrollbackEvent>>,
    scrollback_bytes: Mutex<usize>,
    scrollback_limit: usize,
    clients: AtomicUsize,
    detached_at: Mutex<Option<Instant>>,
    window_size: watch::Sender<(u16, u16)>,
    orphan_timeout: std::time::Duration,
}

impl Session {
    /// Create a new session.
    ///
    /// `orphan_timeout` controls how long a session with no attached clients
    /// survives before the reaper removes it (default: [`DEFAULT_ORPHAN_TIMEOUT`]).
    pub fn new(
        terminal: Terminal,
        output_rx: broadcast::Receiver<Vec<u8>>,
        scrollback_limit: usize,
        orphan_timeout: std::time::Duration,
    ) -> Arc<Self> {
        let (ws_tx, _) = watch::channel((24, 80));
        let session = Arc::new(Self {
            terminal,
            scrollback: Mutex::new(VecDeque::new()),
            scrollback_bytes: Mutex::new(0),
            scrollback_limit,
            clients: AtomicUsize::new(0),
            detached_at: Mutex::new(None),
            window_size: ws_tx,
            orphan_timeout,
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
                        s.push_scrollback(ScrollbackEvent::Output(data));
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

    /// Push an event into the scrollback ring buffer, evicting old events
    /// when the byte budget is exceeded.
    fn push_scrollback(&self, event: ScrollbackEvent) {
        let cost = event.byte_cost();
        let mut sb = self.scrollback.lock().unwrap();
        let mut bytes = self.scrollback_bytes.lock().unwrap();
        *bytes += cost;
        sb.push_back(event);
        while *bytes > self.scrollback_limit {
            if let Some(old) = sb.pop_front() {
                *bytes -= old.byte_cost();
            } else {
                break;
            }
        }
    }

    /// Attach a client: increment the counter, subscribe to live output, and
    /// return the scrollback event log. The subscription and snapshot are taken
    /// under the same lock so no output is lost.
    pub fn attach(&self) -> AttachResult {
        self.clients.fetch_add(1, Ordering::Relaxed);
        *self.detached_at.lock().unwrap() = None;
        let sb = self.scrollback.lock().unwrap();
        let rx = self.terminal.subscribe();
        let ws_rx = self.window_size.subscribe();
        let events: Vec<ScrollbackEvent> = sb.iter().cloned().collect();
        (events, rx, ws_rx)
    }

    /// Update the current PTY window size (broadcast to viewers) and record
    /// the resize in the scrollback log so replay clients see it too.
    pub fn set_window_size(&self, rows: u16, cols: u16) {
        let _ = self.window_size.send((rows, cols));
        self.push_scrollback(ScrollbackEvent::WindowSize(rows, cols));
    }

    /// Detach a client. When the last client detaches, the orphan timer starts.
    pub fn detach(&self) {
        if self.clients.fetch_sub(1, Ordering::Relaxed) == 1 {
            *self.detached_at.lock().unwrap() = Some(Instant::now());
        }
    }

    /// Number of currently attached clients.
    pub fn client_count(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }

    fn is_orphaned(&self) -> bool {
        self.clients.load(Ordering::Relaxed) == 0
            && self
                .detached_at
                .lock()
                .unwrap()
                .is_some_and(|t| t.elapsed() >= self.orphan_timeout)
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

    /// Returns `true` if there are no active sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.read().unwrap().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SCROLLBACK_LIMIT: usize = 256 * 1024;

    fn spawn_session() -> Arc<Session> {
        let (terminal, output_rx) = Terminal::spawn("/bin/sh", None).expect("spawn /bin/sh");
        Session::new(
            terminal,
            output_rx,
            TEST_SCROLLBACK_LIMIT,
            DEFAULT_ORPHAN_TIMEOUT,
        )
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
        assert!(!session.is_orphaned());
    }

    #[tokio::test]
    async fn test_orphaned_after_timeout() {
        let session = spawn_session();
        let (_sb, _rx, _ws) = session.attach();
        session.detach();
        *session.detached_at.lock().unwrap() =
            Some(Instant::now() - session.orphan_timeout - std::time::Duration::from_secs(1));
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

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let (events, _rx, _ws) = session.attach();
        let has_marker = events.iter().any(|e| match e {
            ScrollbackEvent::Output(data) => {
                String::from_utf8_lossy(data).contains("scrollback_test_marker")
            }
            _ => false,
        });
        assert!(has_marker, "scrollback should contain Output with marker");
    }

    #[tokio::test]
    async fn test_session_store_insert_and_get() {
        let store = SessionStore::new();
        let session = spawn_session();
        let id = store.insert(session);

        assert!(store.get(&id).is_some());
        assert!(store.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_scrollback_eviction_removes_whole_events() {
        let (terminal, output_rx) = Terminal::spawn("/bin/sh", None).expect("spawn");
        let session = Session::new(terminal, output_rx, 10, DEFAULT_ORPHAN_TIMEOUT);

        session.push_scrollback(ScrollbackEvent::Output(b"aaaaa".to_vec())); // 5
        session.push_scrollback(ScrollbackEvent::Output(b"bbbbb".to_vec())); // 5, total 10
        session.push_scrollback(ScrollbackEvent::Output(b"ccc".to_vec())); // 3, total 13 → evict

        let sb = session.scrollback.lock().unwrap();
        let bytes = *session.scrollback_bytes.lock().unwrap();
        assert!(bytes <= 10, "bytes {bytes} should be within limit");
        assert!(
            sb.iter().all(|e| matches!(e, ScrollbackEvent::Output(_))),
            "all events should be Output"
        );
        assert_ne!(
            sb.front(),
            Some(&ScrollbackEvent::Output(b"aaaaa".to_vec())),
            "oldest event should have been evicted"
        );
    }

    #[tokio::test]
    async fn test_set_window_size_records_event() {
        let (terminal, output_rx) = Terminal::spawn("/bin/sh", None).expect("spawn");
        let session = Session::new(
            terminal,
            output_rx,
            TEST_SCROLLBACK_LIMIT,
            DEFAULT_ORPHAN_TIMEOUT,
        );

        session.set_window_size(40, 120);

        let sb = session.scrollback.lock().unwrap();
        let has_ws = sb
            .iter()
            .any(|e| matches!(e, ScrollbackEvent::WindowSize(40, 120)));
        assert!(has_ws, "scrollback should contain WindowSize(40, 120)");
    }
}
