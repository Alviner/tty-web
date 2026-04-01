//! **tty-web** — web-based terminal emulator library.
//!
//! Opens a real PTY in the browser over WebSocket. Can be used as a standalone
//! binary or embedded as a library into other applications.

pub mod config;
pub(crate) mod pty;
pub mod session;
pub mod terminal;
pub mod web;
