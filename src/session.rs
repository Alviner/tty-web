use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::terminal::Terminal;

const SCROLLBACK_LIMIT: usize = 64 * 1024;
const ORPHAN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(300);

pub struct Session {
    pub terminal: Terminal,
    scrollback: Mutex<VecDeque<u8>>,
    clients: AtomicUsize,
    detached_at: Mutex<Option<Instant>>,
}

impl Session {
    pub fn new(
        terminal: Terminal,
        output_rx: broadcast::Receiver<Vec<u8>>,
    ) -> Arc<Self> {
        let session = Arc::new(Self {
            terminal,
            scrollback: Mutex::new(VecDeque::with_capacity(
                SCROLLBACK_LIMIT,
            )),
            clients: AtomicUsize::new(0),
            detached_at: Mutex::new(None),
        });

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

    /// Subscribe to live output and get a scrollback snapshot atomically.
    /// The subscription is created while holding the scrollback lock,
    /// guaranteeing no gaps or duplicates between scrollback and live stream.
    pub fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Vec<u8>>) {
        self.clients.fetch_add(1, Ordering::Relaxed);
        *self.detached_at.lock().unwrap() = None;
        let sb = self.scrollback.lock().unwrap();
        let rx = self.terminal.subscribe();
        let snapshot = sb.iter().copied().collect();
        (snapshot, rx)
    }

    pub fn detach(&self) {
        if self.clients.fetch_sub(1, Ordering::Relaxed) == 1 {
            *self.detached_at.lock().unwrap() = Some(Instant::now());
        }
    }

    pub fn is_alive(&self) -> bool {
        self.terminal.is_alive()
    }

    pub fn is_orphaned(&self) -> bool {
        if self.clients.load(Ordering::Relaxed) > 0 {
            return false;
        }
        match *self.detached_at.lock().unwrap() {
            Some(t) => t.elapsed() >= ORPHAN_TIMEOUT,
            None => false,
        }
    }
}

pub struct SessionStore {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl SessionStore {
    pub fn new() -> Arc<Self> {
        let store = Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
        });

        let weak = Arc::downgrade(&store);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                let Some(store) = weak.upgrade() else {
                    break;
                };
                store.cleanup();
            }
        });

        store
    }

    pub fn insert(&self, session: Arc<Session>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.sessions.write().unwrap().insert(id.clone(), session);
        id
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap().get(id).cloned()
    }

    fn cleanup(&self) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.retain(|id, session| {
            if !session.is_alive() {
                tracing::info!("removing dead session {id}");
                return false;
            }
            if session.is_orphaned() {
                tracing::info!(
                    "removing orphaned session {id} (no clients for {}s)",
                    ORPHAN_TIMEOUT.as_secs()
                );
                return false;
            }
            true
        });
    }
}
