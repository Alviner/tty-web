use std::process::Child;
use std::sync::{Arc, Mutex};

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;
use tokio::sync::{broadcast, mpsc, watch};

use crate::pty::PtyMaster;

const OUTPUT_CHANNEL_SIZE: usize = 64;
const INPUT_CHANNEL_SIZE: usize = 256;
const READ_BUF_SIZE: usize = 4096;

pub struct Terminal {
    input_tx: mpsc::Sender<Vec<u8>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    fd: Arc<AsyncFd<std::os::fd::OwnedFd>>,
    child: Mutex<Option<Child>>,
    closed_rx: watch::Receiver<bool>,
}

impl Terminal {
    pub fn spawn(
        shell: &str,
    ) -> std::io::Result<(Self, broadcast::Receiver<Vec<u8>>)> {
        let PtyMaster {
            master,
            mut child,
        } = PtyMaster::spawn(shell)?;

        let async_fd = match AsyncFd::with_interest(
            master,
            Interest::READABLE | Interest::WRITABLE,
        ) {
            Ok(fd) => fd,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let fd = Arc::new(async_fd);

        let (input_tx, input_rx) = mpsc::channel(INPUT_CHANNEL_SIZE);
        let (output_tx, output_rx) =
            broadcast::channel(OUTPUT_CHANNEL_SIZE);
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

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    /// Returns a watch receiver that becomes `true` when the PTY read
    /// loop exits (shell died / PTY closed).
    pub fn closed(&self) -> watch::Receiver<bool> {
        self.closed_rx.clone()
    }

    pub async fn write(&self, data: Vec<u8>) -> Result<(), String> {
        self.input_tx
            .send(data)
            .await
            .map_err(|e| e.to_string())
    }

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

async fn read_loop(
    fd: Arc<AsyncFd<std::os::fd::OwnedFd>>,
    tx: broadcast::Sender<Vec<u8>>,
) {
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

async fn write_loop(
    fd: Arc<AsyncFd<std::os::fd::OwnedFd>>,
    mut rx: mpsc::Receiver<Vec<u8>>,
) {
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
