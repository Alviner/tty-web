"use strict";

// ── Protocol ──────────────────────────────────────────────────────────

const CMD_INPUT = 0x00;
const CMD_RESIZE = 0x01;

const CMD_OUTPUT = 0x00;
const CMD_SESSION_ID = 0x10;
const CMD_SHELL_EXIT = 0x12;
const CMD_WINDOW_SIZE = 0x13;
const CMD_REPLAY_END = 0x14;

// WebSocket close codes (4000–4999: application-specific)
const CLOSE_SESSION_NOT_FOUND = 4404;

const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 5000;

const buildInputFrame = (data) => {
  const encoder = new TextEncoder();
  const encoded = typeof data === "string" ? encoder.encode(data) : data;
  const frame = new Uint8Array(1 + encoded.length);
  frame[0] = CMD_INPUT;
  frame.set(encoded, 1);
  return frame;
};

const buildResizeFrame = (rows, cols) => {
  const frame = new Uint8Array(5);
  frame[0] = CMD_RESIZE;
  frame[1] = (rows >> 8) & 0xff;
  frame[2] = rows & 0xff;
  frame[3] = (cols >> 8) & 0xff;
  frame[4] = cols & 0xff;
  return frame;
};

// ── Logger ────────────────────────────────────────────────────────────

const createLogger = () => {
  const PREFIX = "[tty-web]";

  const fmt = (ctx) => {
    if (!ctx) return PREFIX;
    const parts = [PREFIX];
    for (const k in ctx) parts.push(`${k}=${ctx[k]}`);
    return parts.join(" ");
  };

  const create = (ctx) => {
    const tag = fmt(ctx);
    return {
      info:  (...args) => console.log(tag, ...args),
      warn:  (...args) => console.warn(tag, ...args),
      error: (...args) => console.error(tag, ...args),
      debug: (...args) => console.debug(tag, ...args),
      child: (extra) => create({ ...ctx, ...extra }),
    };
  };

  return create();
};

// ── Terminal ──────────────────────────────────────────────────────────

const LIGATURES = [
  "<!--", "<!---", "===", "!==", ">>>", "<<<", "<-->", "-->",
  "<--", "<->", "=>", "->", "<-", ">=", "<=", "!=", "::", "...",
  "/*", "*/", "//", "++", "+++", "||", "&&", "??", "?.", "|>",
  "<|", "<|>", "<*", "<*>", "*>", "<:", ":>", ":=", "=:", "=~",
  "!~", "<<", ">>", "==", "--", "++"
].sort((a, b) => b.length - a.length);

const setupLigatures = (term) => {
  term.element.style.fontFeatureSettings = '"calt" on';
  term.registerCharacterJoiner((text) => {
    const ranges = [];
    let i = 0;
    while (i < text.length) {
      let matched = false;
      for (const lig of LIGATURES) {
        if (text.substring(i, i + lig.length) === lig) {
          ranges.push([i, i + lig.length]);
          i += lig.length;
          matched = true;
          break;
        }
      }
      if (!matched) i++;
    }
    return ranges;
  });
};

const createTerminal = () => {
  const term = new Terminal({
    allowProposedApi: true,
    fontFamily: "'LigaHack Nerd Font', monospace",
    fontSize: 14,
    cursorBlink: true,
    theme: {
      background: "#1a1b26",
      foreground: "#c0caf5",
      cursor: "#c0caf5",
      cursorAccent: "#1a1b26",
    },
  });

  const fitAddon = new FitAddon.FitAddon();
  const webLinksAddon = new WebLinksAddon.WebLinksAddon();

  term.loadAddon(fitAddon);
  term.loadAddon(webLinksAddon);
  term.open(document.getElementById("terminal"));
  term.focus();
  fitAddon.fit();

  setupLigatures(term);

  return { term, fitAddon };
};

// ── Status Bar ────────────────────────────────────────────────────────

const STATUS_ICONS = { green: "\uF00C", yellow: "\uF252", red: "\uF00D" };

const createStatusBar = (readonly) => {
  const sbSid = document.getElementById("sb-sid");
  const sbMode = document.getElementById("sb-mode");
  const sbStatus = document.getElementById("sb-status");
  const sbCopy = document.getElementById("sb-copy");
  const sbView = document.getElementById("sb-view");
  const sbNew = document.getElementById("sb-new");

  sbMode.textContent = readonly ? "\uF06E view" : "\uF11C interactive";

  const setStatus = (label, color) => {
    sbStatus.innerHTML = `<span class="sb-${color}">${STATUS_ICONS[color] || ""}</span> ${label}`;
  };

  const setSid = (sid) => {
    sbSid.textContent = `\uF489 ${sid.substring(0, 8)}`;
    sbCopy.disabled = false;
    sbView.disabled = false;
  };

  return { setStatus, setSid, sbCopy, sbView, sbNew };
};

// ── Connection ────────────────────────────────────────────────────────

const connect = (ctx) => {
  const { term, statusBar, log, readonly } = ctx;

  let ws = null;
  let reconnectDelay = RECONNECT_BASE_MS;
  let resizeSent = false;
  let shellExited = false;
  let replaying = false;
  let currentSid = new URLSearchParams(location.search).get("sid");
  let wsLog = log;

  const sendResize = () => {
    if (readonly) return;
    if (!resizeSent && ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildResizeFrame(term.rows, term.cols));
      resizeSent = true;
    }
  };

  const openWs = () => {
    const protocol = location.protocol === "https:" ? "wss:" : "ws:";
    let wsUrl = `${protocol}//${location.host}/ws`;
    const sid = new URLSearchParams(location.search).get("sid");
    if (sid) {
      wsUrl += `?sid=${encodeURIComponent(sid)}`;
    }
    if (readonly) {
      wsUrl += (sid ? "&" : "?") + "view";
    }
    ws = new WebSocket(wsUrl);
    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      reconnectDelay = RECONNECT_BASE_MS;
      resizeSent = false;
      statusBar.setStatus("connected", "green");
      wsLog.info("connected");
    };

    ws.onmessage = (event) => {
      const data = new Uint8Array(event.data);
      if (data.length < 1) return;

      const cmd = data[0];
      const payload = data.subarray(1);

      switch (cmd) {
        case CMD_OUTPUT:
          term.write(payload);
          break;
        case CMD_SESSION_ID: {
          const newSid = new TextDecoder().decode(payload);
          const isReattach = newSid === currentSid;
          wsLog = log.child({ sid: newSid.substring(0, 8) });
          wsLog.info("session", isReattach ? "(reattach)" : "(new)");
          replaying = true;
          term.reset();
          currentSid = newSid;
          history.replaceState(null, "", `/?sid=${newSid}${readonly ? "&view" : ""}`);
          statusBar.setSid(newSid);
          break;
        }
        case CMD_REPLAY_END:
          wsLog.info("replay end");
          term.write("", () => {
            replaying = false;
            term.write("\x1b[?25h");
            sendResize();
          });
          break;
        case CMD_WINDOW_SIZE:
          if (readonly && payload.length >= 4) {
            const rows = (payload[0] << 8) | payload[1];
            const cols = (payload[2] << 8) | payload[3];
            term.resize(cols, rows);
          }
          break;
        case CMD_SHELL_EXIT:
          shellExited = true;
          wsLog.info("shell exited");
          term.write("\r\n\x1b[90m[Shell exited.]\x1b[0m\r\n");
          statusBar.setStatus("exited", "red");
          break;
      }
    };

    ws.onclose = (ev) => {
      if (ev.code === CLOSE_SESSION_NOT_FOUND) {
        wsLog.warn("session not found, code:", ev.code);
        term.write("\r\n\x1b[90m[Session not found.]\x1b[0m\r\n");
        statusBar.setStatus("no session", "red");
        return;
      }
      if (shellExited) return;
      wsLog.info("disconnected, code:", ev.code);
      statusBar.setStatus("reconnecting", "yellow");
      term.write(
        `\r\n\x1b[33m[Disconnected. Reconnecting in ${Math.round(reconnectDelay / 1000)}s...]\x1b[0m\r\n`
      );
      setTimeout(openWs, reconnectDelay);
      reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
    };

    ws.onerror = () => { wsLog.error("websocket error"); };
  };

  // Terminal input handlers
  term.onData((data) => {
    if (readonly || replaying) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildInputFrame(data));
    }
  });

  term.onBinary((data) => {
    if (readonly || replaying) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      const bytes = new Uint8Array(data.length);
      for (let i = 0; i < data.length; i++) {
        bytes[i] = data.charCodeAt(i);
      }
      ws.send(buildInputFrame(bytes));
    }
  });

  term.onResize((size) => {
    if (readonly) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildResizeFrame(size.rows, size.cols));
    }
  });

  // Status bar button handlers
  const flashButton = (btn, original) => {
    btn.textContent = "Copied!";
    setTimeout(() => { btn.textContent = original; }, 1500);
  };

  statusBar.sbCopy.addEventListener("click", async () => {
    if (!currentSid) return;
    await navigator.clipboard.writeText(`${location.origin}/?sid=${currentSid}`);
    flashButton(statusBar.sbCopy, "\uF0C1 Copy link");
  });

  statusBar.sbView.addEventListener("click", async () => {
    if (!currentSid) return;
    await navigator.clipboard.writeText(`${location.origin}/?sid=${currentSid}&view`);
    flashButton(statusBar.sbView, "\uF06E View link");
  });

  statusBar.sbNew.addEventListener("click", () => {
    location.href = `${location.origin}/`;
  });

  openWs();
};

// ── Main ──────────────────────────────────────────────────────────────

const main = () => {
  const log = createLogger();
  const readonly = new URLSearchParams(location.search).has("view");
  const { term, fitAddon } = createTerminal();
  const statusBar = createStatusBar(readonly);

  statusBar.setStatus("connecting", "yellow");

  if (!readonly) {
    new ResizeObserver(() => fitAddon.fit()).observe(document.getElementById("terminal"));
  }

  connect({ term, statusBar, log, readonly });
};

document.addEventListener("DOMContentLoaded", main);
