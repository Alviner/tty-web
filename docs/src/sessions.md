# Sessions

Each WebSocket connection is backed by a persistent session identified by a
UUID v4. The PTY and shell process live independently of the WebSocket —
closing a tab or losing connectivity does not kill the shell.

## Reconnect

On first connect the server assigns a UUID and the client updates the browser
URL to `/?sid=<uuid>` via `history.replaceState`. On reconnect the client
reads `sid` from the URL and passes it as a query parameter. The server
replays the scrollback buffer and then streams live output — no gaps. From the
user's perspective the terminal picks up where it left off.

The scrollback buffer defaults to **256 KiB** and can be changed with
`--scrollback-limit` (in KiB). The buffer stores an event log of output chunks
and window-size changes. When the byte budget is exceeded, entire events are
evicted from the front — escape sequences are never split mid-stream. On
reconnect the server replays the event log as regular `Output` and
`WindowSize` protocol frames followed by a `ReplayEnd` marker.

Reconnection uses exponential backoff starting at 1 s up to a maximum of 5 s.

## Share a session

Open a second tab with `?sid=<uuid>` in the page URL:

```
http://localhost:9090/?sid=<uuid>
```

All tabs see the same output and can send input simultaneously. The session ID
is printed to the browser console on connect.

## View mode

Append `&view` to a session URL to connect as a read-only observer:

```
http://localhost:9090/?sid=<uuid>&view
```

Terminal output is visible but all keyboard input and resize events are ignored.
The viewer's terminal automatically matches the interactive client's window
size — when the interactive client resizes, all viewers receive the updated
dimensions via the `0x13` (Window size) protocol command.

Useful for demos, monitoring, and pair-programming.

## Lifecycle

A session is removed when:

- the shell process exits and no clients are attached (immediately), or
- the shell process exits while clients are still attached (as soon as the last
  client disconnects), or
- no client is attached for 60 seconds (orphan timeout).

For internal constants and implementation details, see the
[API Reference](./api-reference.md).
