"use strict";

var CMD_INPUT = 0x00;
var CMD_RESIZE = 0x01;

var CMD_OUTPUT = 0x00;
var CMD_SESSION_ID = 0x10;
var CMD_SCROLLBACK = 0x11;
var CMD_SHELL_EXIT = 0x12;

var RECONNECT_BASE_MS = 1000;
var RECONNECT_MAX_MS = 5000;

function buildInputFrame(data) {
  var encoder = new TextEncoder();
  var encoded = typeof data === "string" ? encoder.encode(data) : data;
  var frame = new Uint8Array(1 + encoded.length);
  frame[0] = CMD_INPUT;
  frame.set(encoded, 1);
  return frame;
}

function buildResizeFrame(rows, cols) {
  var frame = new Uint8Array(5);
  frame[0] = CMD_RESIZE;
  frame[1] = (rows >> 8) & 0xff;
  frame[2] = rows & 0xff;
  frame[3] = (cols >> 8) & 0xff;
  frame[4] = cols & 0xff;
  return frame;
}

function main() {
  var term = new Terminal({
    allowProposedApi: true,
    fontFamily:
      "'LigaHack Nerd Font', monospace",
    fontSize: 14,
    theme: {
      background: "#1a1b26",
      foreground: "#c0caf5",
      cursor: "#c0caf5",
      cursorAccent: "#1a1b26",
    },
  });

  var fitAddon = new FitAddon.FitAddon();
  var webLinksAddon = new WebLinksAddon.WebLinksAddon();

  term.loadAddon(fitAddon);
  term.loadAddon(webLinksAddon);
  term.open(document.getElementById("terminal"));
  fitAddon.fit();

  // Ligatures: enable OpenType contextual alternates and register
  // a character joiner so xterm.js draws ligature sequences as a
  // single text run, letting the font apply substitution rules.
  var LIGATURES = [
    "<!--", "<!---", "===", "!==", ">>>", "<<<", "<-->", "-->",
    "<--", "<->", "=>", "->", "<-", ">=", "<=", "!=", "::", "...",
    "/*", "*/", "//", "++", "+++", "||", "&&", "??", "?.", "|>",
    "<|", "<|>", "<*", "<*>", "*>", "<:", ":>", ":=", "=:", "=~",
    "!~", "<<", ">>", "==", "--", "++"
  ].sort(function (a, b) { return b.length - a.length; });

  term.element.style.fontFeatureSettings = '"calt" on';
  term.registerCharacterJoiner(function (text) {
    var ranges = [];
    var i = 0;
    while (i < text.length) {
      var matched = false;
      for (var j = 0; j < LIGATURES.length; j++) {
        var lig = LIGATURES[j];
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

  var readonly = new URLSearchParams(location.search).has("view");

  var ws = null;
  var reconnectDelay = RECONNECT_BASE_MS;
  var resizeSent = false;
  var shellExited = false;
  var replaying = false;

  function sendResize() {
    if (readonly) return;
    if (!resizeSent && ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildResizeFrame(term.rows, term.cols));
      resizeSent = true;
    }
  }

  function getSid() {
    var params = new URLSearchParams(location.search);
    return params.get("sid") || sessionStorage.getItem("tty-web-sid");
  }

  function connect() {
    var protocol = location.protocol === "https:" ? "wss:" : "ws:";
    var wsUrl = protocol + "//" + location.host + "/ws";
    var sid = getSid();
    if (sid) {
      wsUrl += "?sid=" + encodeURIComponent(sid);
    }
    if (readonly) {
      wsUrl += (sid ? "&" : "?") + "view";
    }
    ws = new WebSocket(wsUrl);
    ws.binaryType = "arraybuffer";

    ws.onopen = function () {
      reconnectDelay = RECONNECT_BASE_MS;
      resizeSent = false;
    };

    ws.onmessage = function (event) {
      var data = new Uint8Array(event.data);
      if (data.length < 1) return;

      var cmd = data[0];
      var payload = data.subarray(1);

      switch (cmd) {
        case CMD_OUTPUT:
          term.write(payload);
          break;
        case CMD_SESSION_ID:
          var newSid = new TextDecoder().decode(payload);
          var oldSid = sessionStorage.getItem("tty-web-sid");
          console.log("[tty-web] session_id:", newSid, oldSid === newSid ? "(reattach)" : "(new)");
          if (newSid !== oldSid) {
            term.reset();
            sendResize();
          }
          sessionStorage.setItem("tty-web-sid", newSid);
          break;
        case CMD_SCROLLBACK:
          console.log("[tty-web] scrollback:", payload.length, "bytes");
          replaying = true;
          term.reset();
          term.write(payload, function () {
            term.write("\x1b[?25h");
            replaying = false;
            sendResize();
          });
          break;
        case CMD_SHELL_EXIT:
          shellExited = true;
          sessionStorage.removeItem("tty-web-sid");
          term.write("\r\n\x1b[90m[Shell exited.]\x1b[0m\r\n");
          break;
      }
    };

    ws.onclose = function () {
      if (shellExited) return;
      term.write(
        "\r\n\x1b[33m[Disconnected. Reconnecting in " +
          Math.round(reconnectDelay / 1000) +
          "s...]\x1b[0m\r\n"
      );
      setTimeout(connect, reconnectDelay);
      reconnectDelay = Math.min(
        reconnectDelay * 2,
        RECONNECT_MAX_MS
      );
    };

    ws.onerror = function () {};
  }

  term.onData(function (data) {
    if (readonly || replaying) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildInputFrame(data));
    }
  });

  term.onBinary(function (data) {
    if (readonly || replaying) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      var bytes = new Uint8Array(data.length);
      for (var i = 0; i < data.length; i++) {
        bytes[i] = data.charCodeAt(i);
      }
      ws.send(buildInputFrame(bytes));
    }
  });

  term.onResize(function (size) {
    if (readonly) return;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildResizeFrame(size.rows, size.cols));
    }
  });

  var resizeObserver = new ResizeObserver(function () {
    fitAddon.fit();
  });
  resizeObserver.observe(document.getElementById("terminal"));

  connect();
}

document.addEventListener("DOMContentLoaded", main);
