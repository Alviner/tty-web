use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::terminal::Terminal;

const SCROLLBACK_LIMIT: usize = 64 * 1024;
const ORPHAN_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

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

    /// Subscribe to live output and get a scrollback snapshot atomically.
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

pub struct SessionStore {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl SessionStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
        })
    }

    pub fn insert(
        self: &Arc<Self>,
        session: Arc<Session>,
    ) -> String {
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
                tokio::time::sleep(std::time::Duration::from_secs(1))
                    .await;
                let Some(store) = store.upgrade() else { return };
                let should_remove = {
                    let sessions = store.sessions.read().unwrap();
                    match sessions.get(&sid) {
                        Some(s) => {
                            s.is_orphaned()
                                || (*closed_rx.borrow()
                                    && s.clients
                                        .load(Ordering::Relaxed)
                                        == 0)
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

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap().get(id).cloned()
    }
}
