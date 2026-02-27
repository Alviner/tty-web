"use strict";

var CMD_INPUT = 0x00;
var CMD_RESIZE = 0x01;
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
    cursorBlink: true,
    fontFamily:
      "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Menlo, monospace",
    fontSize: 14,
    theme: {
      background: "#1a1b26",
      foreground: "#c0caf5",
      cursor: "#c0caf5",
    },
  });

  var fitAddon = new FitAddon.FitAddon();
  var webLinksAddon = new WebLinksAddon.WebLinksAddon();

  term.loadAddon(fitAddon);
  term.loadAddon(webLinksAddon);
  term.open(document.getElementById("terminal"));
  fitAddon.fit();

  var ws = null;
  var reconnectDelay = RECONNECT_BASE_MS;

  function connect() {
    var protocol = location.protocol === "https:" ? "wss:" : "ws:";
    var wsUrl = protocol + "//" + location.host + "/ws";
    ws = new WebSocket(wsUrl);
    ws.binaryType = "arraybuffer";

    ws.onopen = function () {
      reconnectDelay = RECONNECT_BASE_MS;
      term.clear();
      ws.send(buildResizeFrame(term.rows, term.cols));
    };

    ws.onmessage = function (event) {
      term.write(new Uint8Array(event.data));
    };

    ws.onclose = function () {
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
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(buildInputFrame(data));
    }
  });

  term.onBinary(function (data) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      var bytes = new Uint8Array(data.length);
      for (var i = 0; i < data.length; i++) {
        bytes[i] = data.charCodeAt(i);
      }
      ws.send(buildInputFrame(bytes));
    }
  });

  term.onResize(function (size) {
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
