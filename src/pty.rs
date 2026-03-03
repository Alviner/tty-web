//! UNIX pseudo-terminal (PTY) allocation and window-size control.
//!
//! Wraps `openpty(3)` and `ioctl(TIOCSWINSZ)` from the [`nix`] crate into a
//! safe, async-friendly interface.

use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use nix::fcntl;
use nix::libc;
use nix::pty::openpty;

/// Owns the master side of a PTY and the child shell process.
pub struct PtyMaster {
    /// Master file descriptor (non-blocking).
    pub master: OwnedFd,
    /// Child process running the shell.
    pub child: Child,
}

impl PtyMaster {
    /// Allocate a new PTY pair, spawn `shell` on the slave side, and return the
    /// master fd set to non-blocking mode.
    ///
    /// If `pwd` is provided, the shell process starts in that directory.
    pub fn spawn(shell: &str, pwd: Option<&Path>) -> std::io::Result<Self> {
        let pty = openpty(None, None).map_err(std::io::Error::other)?;

        let slave_out = pty.slave.try_clone()?;
        let slave_err = pty.slave.try_clone()?;

        let mut cmd = Command::new(shell);
        cmd.stdin(Stdio::from(pty.slave))
            .stdout(Stdio::from(slave_out))
            .stderr(Stdio::from(slave_err))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor");

        if let Some(dir) = pwd {
            cmd.current_dir(dir);
        }

        // Safety: pre_exec runs in forked child before exec.
        // Only async-signal-safe libc calls are used.
        let child = unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            })
            .spawn()?
        };

        // Set master fd to non-blocking for async I/O
        let flags =
            fcntl::fcntl(&pty.master, fcntl::FcntlArg::F_GETFL).map_err(std::io::Error::other)?;
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
    let ret = unsafe { libc::ioctl(fd.as_fd().as_raw_fd(), libc::TIOCSWINSZ as _, &ws) };
    if ret == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_and_child_alive() {
        let mut pty = PtyMaster::spawn("/bin/sh", None).expect("spawn /bin/sh");
        // Child should still be running
        assert!(
            pty.child.try_wait().unwrap().is_none(),
            "child should be alive"
        );
        // Cleanup
        let _ = pty.child.kill();
        let _ = pty.child.wait();
    }

    #[test]
    fn test_set_window_size() {
        let mut pty = PtyMaster::spawn("/bin/sh", None).expect("spawn /bin/sh");
        set_window_size(&pty.master, 40, 120).expect("set_window_size should succeed");
        let _ = pty.child.kill();
        let _ = pty.child.wait();
    }

    #[test]
    fn test_spawn_with_pwd() {
        let dir = std::env::temp_dir();
        let mut pty = PtyMaster::spawn("/bin/sh", Some(dir.as_path())).expect("spawn with pwd");
        assert!(
            pty.child.try_wait().unwrap().is_none(),
            "child should be alive"
        );
        let _ = pty.child.kill();
        let _ = pty.child.wait();
    }
}
