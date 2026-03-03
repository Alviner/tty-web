# Introduction

**tty-web** is a web-based terminal emulator that opens a real PTY in the browser
over WebSocket.

![demo](images/demo.gif)

## Features

- **Real PTY** — runs an actual shell process (`bash`, `zsh`, etc.) with full
  job control, signals, and terminal capabilities.
- **Persistent sessions** — closing a tab does not kill the shell. Reconnect
  with the same session ID and pick up where you left off.
- **Scrollback replay** — on reconnect the server replays the last 64 KB of
  output so there are no gaps.
- **Session sharing** — multiple tabs (or users) can attach to the same session
  simultaneously.
- **View mode** — read-only observers can watch a session without sending input.
- **Lightweight binary protocol** — a single-byte command prefix keeps overhead
  minimal.
- **Single static binary** — frontend assets are embedded at compile time via
  `rust-embed`.
- **Docker image** — multi-arch (`amd64` / `arm64`) images published to
  `ghcr.io`.

## Links

- [GitHub repository](https://github.com/Alviner/tty-web)
- [Container registry](https://ghcr.io/alviner/tty-web)
- License: **MIT**
