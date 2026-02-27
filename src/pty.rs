use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use nix::fcntl;
use nix::libc;
use nix::pty::openpty;

pub struct PtyMaster {
    pub master: OwnedFd,
    pub child: Child,
}

impl PtyMaster {
    pub fn spawn(shell: &str) -> std::io::Result<Self> {
        let pty = openpty(None, None).map_err(std::io::Error::other)?;

        let slave_out = pty.slave.try_clone()?;
        let slave_err = pty.slave.try_clone()?;

        // Safety: pre_exec runs in forked child before exec.
        // Only async-signal-safe libc calls are used.
        let child = unsafe {
            Command::new(shell)
                .stdin(Stdio::from(pty.slave))
                .stdout(Stdio::from(slave_out))
                .stderr(Stdio::from(slave_err))
                .env("TERM", "xterm-256color")
                .env("COLORTERM", "truecolor")
                .pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                })
                .spawn()?
        };

        // Set master fd to non-blocking for async I/O
        let flags = fcntl::fcntl(&pty.master, fcntl::FcntlArg::F_GETFL)
            .map_err(std::io::Error::other)?;
        let mut flags = fcntl::OFlag::from_bits_truncate(flags);
        flags.insert(fcntl::OFlag::O_NONBLOCK);
        fcntl::fcntl(&pty.master, fcntl::FcntlArg::F_SETFL(flags))
            .map_err(std::io::Error::other)?;

        Ok(PtyMaster {
            master: pty.master,
            child,
        })
    }
}

/// Set the terminal window size on a PTY file descriptor.
///
/// Safe wrapper around `ioctl(TIOCSWINSZ)`. The caller must
/// pass an fd that refers to a valid PTY master or slave.
pub fn set_window_size(fd: impl AsFd, rows: u16, cols: u16) -> std::io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // Safety: fd is a valid file descriptor (enforced by AsFd),
    // ws is a valid winsize struct on the stack.
    let ret = unsafe { libc::ioctl(fd.as_fd().as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
