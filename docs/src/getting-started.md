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
| `--pwd` | `TTY_WEB_PWD` | *inherited* | Working directory for new shell sessions |
| `--scrollback-limit` | `TTY_WEB_SCROLLBACK_LIMIT` | `256` | Scrollback buffer size in KiB |

## Docker

Pre-built images are available for `linux/amd64` and `linux/arm64` in two variants:

| Variant | Tags | Description |
|---------|------|-------------|
| minimal | `latest`, `<version>` | Single static binary (~5 MB), ideal for `COPY --from` |
| playground | `playground`, `<version>-playground` | Ubuntu with Python, Node, Go, Rust, Neovim |

### Minimal (default)

Scratch-based image with a single static binary. Use as a source for `COPY --from`:

```dockerfile
COPY --from=ghcr.io/alviner/tty-web:latest /tty-web /usr/local/bin/tty-web
```

### Playground

```bash
docker run --rm -p 9090:9090 ghcr.io/alviner/tty-web:playground
```

Override the default shell:

```bash
docker run --rm -p 9090:9090 ghcr.io/alviner/tty-web:playground \
  tty-web --shell /bin/sh
```
