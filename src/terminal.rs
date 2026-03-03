//! High-level terminal abstraction over a PTY.
//!
//! [`Terminal`] owns a [`PtyMaster`] and drives async
//! read/write loops via tokio. Output is fanned out through a broadcast channel
//! so multiple subscribers (WebSocket clients) can receive the same stream.

use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex};

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::{broadcast, mpsc, watch};

use crate::pty::PtyMaster;

const OUTPUT_CHANNEL_SIZE: usize = 64;
const INPUT_CHANNEL_SIZE: usize = 256;
const READ_BUF_SIZE: usize = 4096;

/// Async terminal backed by a real PTY.
///
/// Spawns two background tasks (read loop and write loop) that bridge the PTY
/// fd with tokio channels. Sends `SIGHUP` to the child process on drop.
pub struct Terminal {
    input_tx: mpsc::Sender<Vec<u8>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    fd: Arc<AsyncFd<std::os::fd::OwnedFd>>,
    child: Mutex<Option<Child>>,
    closed_rx: watch::Receiver<bool>,
}

impl Terminal {
    /// Spawn a shell process and return the terminal plus an initial output
    /// receiver.
    ///
    /// If `pwd` is provided, the shell starts in that directory.
    pub fn spawn(
        shell: &str,
        pwd: Option<&Path>,
    ) -> std::io::Result<(Self, broadcast::Receiver<Vec<u8>>)> {
        let PtyMaster { master, mut child } = PtyMaster::spawn(shell, pwd)?;

        let async_fd = match AsyncFd::with_interest(master, Interest::READABLE | Interest::WRITABLE)
        {
            Ok(fd) => fd,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let fd = Arc::new(async_fd);

        let (input_tx, input_rx) = mpsc::channel(INPUT_CHANNEL_SIZE);
        let (output_tx, output_rx) = broadcast::channel(OUTPUT_CHANNEL_SIZE);
        let (closed_tx, closed_rx) = watch::channel(false);

        let read_fd = fd.clone();
        let read_tx = output_tx.clone();
        tokio::spawn(async move {
            read_loop(read_fd, read_tx).await;
            let _ = closed_tx.send(true);
        });

        let write_fd = fd.clone();
        tokio::spawn(async move {
            write_loop(write_fd, input_rx).await;
        });

        let terminal = Terminal {
            input_tx,
            output_tx,
            fd,
            child: Mutex::new(Some(child)),
            closed_rx,
        };
        Ok((terminal, output_rx))
    }

    /// Subscribe to the terminal output broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    /// Returns a watch receiver that becomes `true` when the PTY read
    /// loop exits (shell died / PTY closed).
    pub fn closed(&self) -> watch::Receiver<bool> {
        self.closed_rx.clone()
    }

    /// Queue bytes to be written to the PTY.
    pub async fn write(&self, data: Vec<u8>) -> Result<(), String> {
        self.input_tx.send(data).await.map_err(|e| e.to_string())
    }

    /// Set the PTY window size (rows x cols).
    pub fn resize(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        crate::pty::set_window_size(&*self.fd, rows, cols)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.get_mut().unwrap().take() {
            let pid = Pid::from_raw(child.id() as i32);
            let _ = signal::kill(pid, Signal::SIGHUP);
            let _ = child.wait();
        }
    }
}

async fn read_loop(fd: Arc<AsyncFd<std::os::fd::OwnedFd>>, tx: broadcast::Sender<Vec<u8>>) {
    let mut buf = [0u8; READ_BUF_SIZE];
    loop {
        let mut ready = match fd.readable().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("pty read await error: {}", e);
                break;
            }
        };

        match nix::unistd::read(&*fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
                ready.retain_ready();
            }
            Err(nix::errno::Errno::EAGAIN) => {
                ready.clear_ready();
            }
            Err(e) => {
                tracing::debug!("pty read error: {}", e);
                break;
            }
        }
    }
}

async fn write_loop(fd: Arc<AsyncFd<std::os::fd::OwnedFd>>, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(data) = rx.recv().await {
        let mut written = 0;
        while written < data.len() {
            let mut ready = match fd.writable().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("pty write await error: {}", e);
                    return;
                }
            };
            match nix::unistd::write(&*fd, &data[written..]) {
                Ok(n) => {
                    written += n;
                    ready.retain_ready();
                }
                Err(nix::errno::Errno::EAGAIN) => {
                    ready.clear_ready();
                }
                Err(e) => {
                    tracing::debug!("pty write error: {}", e);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn test_write_and_read_output() {
        let (terminal, mut rx) = Terminal::spawn("/bin/sh", None).expect("spawn /bin/sh");

        terminal
            .write(b"echo hello_test_marker\n".to_vec())
            .await
            .unwrap();

        let mut collected = String::new();
        let deadline = Duration::from_secs(3);
        let _ = timeout(deadline, async {
            while let Ok(data) = rx.recv().await {
                collected.push_str(&String::from_utf8_lossy(&data));
                if collected.contains("hello_test_marker") {
                    break;
                }
            }
        })
        .await;

        assert!(
            collected.contains("hello_test_marker"),
            "expected output to contain marker, got: {collected}"
        );
    }

    #[tokio::test]
    async fn test_resize() {
        let (terminal, _rx) = Terminal::spawn("/bin/sh", None).expect("spawn /bin/sh");
        terminal.resize(50, 132).expect("resize should succeed");
    }

    #[tokio::test]
    async fn test_closed_on_exit() {
        let (terminal, _rx) = Terminal::spawn("/bin/sh", None).expect("spawn /bin/sh");
        let mut closed = terminal.closed();

        terminal.write(b"exit\n".to_vec()).await.unwrap();

        let deadline = Duration::from_secs(3);
        let result = timeout(deadline, closed.wait_for(|&v| v)).await;
        assert!(
            result.is_ok(),
            "closed signal should be received after exit"
        );
    }
}
