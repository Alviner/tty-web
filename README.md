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
