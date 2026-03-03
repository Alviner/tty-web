# Getting Started

## Running

```bash
tty-web --address 127.0.0.1 --port 9090 --shell /bin/zsh
```

Then open <http://127.0.0.1:9090> in a browser.

## CLI Flags

Every flag can also be set via an environment variable.

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--address` | `TTY_WEB_ADDRESS` | `127.0.0.1` | Listen address |
| `--port` | `TTY_WEB_PORT` | `9090` | Listen port |
| `--shell` | `TTY_WEB_SHELL` | `/bin/bash` | Shell to spawn |
| `--log-level` | `TTY_WEB_LOG_LEVEL` | `info` | Log level (`trace`, `debug`, `info`, `warn`, `error`) |
| `--log-format` | `TTY_WEB_LOG_FORMAT` | `text` | Log output format (`text`, `json`) |

## Docker

Pre-built images are available for `linux/amd64` and `linux/arm64`:

```bash
docker run --rm -p 9090:9090 ghcr.io/alviner/tty-web:latest
```

Override the default shell:

```bash
docker run --rm -p 9090:9090 ghcr.io/alviner/tty-web:latest \
  tty-web --shell /bin/sh
```
