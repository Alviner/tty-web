# Development

## Build

```bash
make build     # debug build
make release   # release build (LTO + strip, musl target)
make docker    # build Docker image
```

All available Make targets:

| Target | Description |
|--------|-------------|
| `build` | Debug build via `cargo build` |
| `run` | Run the binary via `cargo run` |
| `release` | Release build with LTO and symbol stripping |
| `clean` | `cargo clean` |
| `fmt` | Format code with `cargo fmt` |
| `lint` | Lint with `cargo clippy -- -D warnings` |
| `check` | `cargo check` |
| `docker` | Build release binary and Docker image |

## Architecture

The codebase is split into focused modules:

```
src/
├── main.rs          Entry point, signal handling, server startup
├── config.rs        CLI flags (clap) and env var mapping
├── pty.rs           PTY allocation, read/write, window size (ioctl)
├── terminal.rs      Async I/O loops, broadcast/mpsc channels, exit signal
├── session.rs       Session store, attach/detach, scrollback, reaper
└── web/
    ├── mod.rs       Axum router, AppState
    ├── ws.rs        WebSocket handler, binary protocol, session resolve
    ├── health.rs    /api/v1/ping health check
    └── static_files.rs   rust-embed static file serving
```

### Module responsibilities

- **config** — parses `--address`, `--port`, `--shell`, `--log-level` via clap
  with env fallbacks (`TTY_WEB_*`).
- **pty** — wraps UNIX PTY syscalls (`openpty`, `TIOCSWINSZ`) using the `nix`
  crate. Provides `PtyMaster` with async-safe read/write.
- **terminal** — owns the PTY and shell child process. Exposes a broadcast
  channel for output (multiple subscribers), an mpsc channel for input, and a
  watch channel for the exit signal. Sends `SIGHUP` on drop.
- **session** — `SessionStore` manages sessions by UUID. Each `Session` tracks
  an attached-client counter, a scrollback `VecDeque` (capped at 64 KB), and
  spawns a per-session reaper task that runs every 1 s.
- **web** — Axum-based HTTP/WebSocket server. Static assets are embedded at
  compile time. The WebSocket handler implements the binary protocol and
  resolves sessions from the `sid` query parameter.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `axum` 0.8 | HTTP framework + WebSocket support |
| `tokio` 1 | Async runtime (multi-threaded) |
| `nix` 0.31 | UNIX PTY and signal syscalls |
| `rust-embed` 8 | Embed static assets into the binary |
| `clap` 4 | CLI argument parsing with env support |
| `tracing` / `tracing-subscriber` | Structured logging |
| `serde` 1 | JSON serialization |
| `mime_guess` 2 | MIME type detection for static files |
| `uuid` 1 | UUID v4 session identifiers |

## Release profile

The release build enables LTO and strips symbols for a small static binary:

```toml
[profile.release]
lto = true
strip = true
```

Release builds target `*-unknown-linux-musl` for fully static binaries.
