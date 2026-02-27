# tty-web

Web-based terminal emulator. Opens a real PTY in the browser over WebSocket.

## Usage

```bash
tty-web --address 127.0.0.1 --port 9090 --shell /bin/zsh
```

| Flag | Env | Default |
|------|-----|---------|
| `--address` | `TTY_WEB_ADDRESS` | `127.0.0.1` |
| `--port` | `TTY_WEB_PORT` | `9090` |
| `--shell` | `TTY_WEB_SHELL` | `/bin/bash` |
| `--log-level` | `TTY_WEB_LOG_LEVEL` | `info` |

## Sessions

Each WebSocket connection is backed by a persistent session. The PTY and shell
process live independently of the WebSocket — closing a tab or losing
connectivity does not kill the shell.

**Reconnect** — the client stores the session ID in `sessionStorage` and passes
it as `?sid=<uuid>` on reconnect. The server replays the scrollback buffer
(last 64 KB of output) and then streams live output. From the user's
perspective the terminal picks up where it left off.

**Share a session** — open a second tab with `?sid=<uuid>` in the page URL
(e.g. `http://localhost:9090/?sid=...`). Both tabs see the same output and can
send input. The session ID is printed to the browser console on connect.

**Lifecycle** — a session is removed when:

- the shell process exits (detected within 5 s), or
- no client is attached for 5 minutes (orphan timeout).

## Build

```bash
make build     # debug
make release   # release
make docker    # docker image
```

## Docker

```bash
docker run --rm -p 9090:9090 ghcr.io/alviner/tty-web:latest
```
