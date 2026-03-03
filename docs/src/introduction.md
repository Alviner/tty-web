# Introduction

**tty-web** is a web-based terminal emulator that opens a real PTY in the browser
over WebSocket.

![demo](images/demo.gif)

## Features

- Real PTY with full job control and signals
- [Persistent sessions](./sessions.md) with configurable scrollback replay
- [Session sharing and view mode](./sessions.md#share-a-session) with window size sync
- [Lightweight binary protocol](./wire-protocol.md)
- Single static binary (frontend embedded via `rust-embed`)
- Multi-arch Docker images (`amd64` / `arm64`) — minimal scratch and playground variants
