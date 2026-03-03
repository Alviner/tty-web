# Sessions

Each WebSocket connection is backed by a persistent session identified by a
UUID v4. The PTY and shell process live independently of the WebSocket —
closing a tab or losing connectivity does not kill the shell.

## Reconnect

The client stores the session ID in `sessionStorage` and passes it as
`?sid=<uuid>` on reconnect. The server replays the scrollback buffer (last
64 KB of output) and then streams live output — no gaps. From the user's
perspective the terminal picks up where it left off.

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
Useful for demos, monitoring, and pair-programming.

## Lifecycle

A session is removed when:

- the shell process exits and no clients are attached (immediately), or
- the shell process exits while clients are still attached (as soon as the last
  client disconnects), or
- no client is attached for 60 seconds (orphan timeout).

### Internal constants

| Constant | Value | Description |
|----------|-------|-------------|
| `SCROLLBACK_LIMIT` | 64 KB | Maximum scrollback buffer size |
| `ORPHAN_TIMEOUT` | 60 s | Time before removing a session with no clients |
| Reaper period | 1 s | How often the reaper checks each session |
| `OUTPUT_CHANNEL_SIZE` | 64 | Broadcast channel capacity for output |
| `INPUT_CHANNEL_SIZE` | 256 | Input mpsc channel capacity |
