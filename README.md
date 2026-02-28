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

Each WebSocket connection is backed by a persistent session (UUID v4). The PTY
and shell process live independently of the WebSocket — closing a tab or losing
connectivity does not kill the shell.

**Reconnect** — the client stores the session ID in `sessionStorage` and passes
it as `?sid=<uuid>` on reconnect. The server replays the scrollback buffer
(last 64 KB of output) and then streams live output — no gaps. From the user's
perspective the terminal picks up where it left off. Reconnection uses
exponential backoff starting at 1 s up to a maximum of 5 s.

**Share a session** — open a second tab with `?sid=<uuid>` in the page URL
(e.g. `http://localhost:9090/?sid=...`). All tabs see the same output and can
send input simultaneously. The session ID is printed to the browser console on
connect.

**View mode** — append `&view` to a session URL
(e.g. `http://localhost:9090/?sid=<uuid>&view`) to connect as a read-only
observer. The terminal output is visible but all keyboard input and resize
events are ignored. Useful for demos, monitoring, and pair-programming.

**Lifecycle** — a session is removed when:

- the shell process exits and no clients are attached (immediately), or
- the shell process exits while clients are still attached (as soon as the
  last client disconnects), or
- no client is attached for 1 minute (orphan timeout).

### Wire protocol

All WebSocket messages are binary frames. The first byte is the command, the
rest is the payload.

| Direction | Cmd | Payload | Description |
|-----------|-----|---------|-------------|
| client → server | `0x00` | raw bytes | Terminal input |
| client → server | `0x01` | rows(u16 BE) + cols(u16 BE) | Resize |
| server → client | `0x00` | raw bytes | Terminal output |
| server → client | `0x10` | UUID string | Session ID |
| server → client | `0x11` | raw bytes | Scrollback snapshot |
| server → client | `0x12` | — | Shell exited |

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
