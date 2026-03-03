# Wire Protocol


All WebSocket messages are **binary frames**. The first byte is the command,
the rest is the payload.

## Commands

| Direction | Cmd | Payload | Description |
|-----------|-----|---------|-------------|
| client → server | `0x00` | raw bytes | Terminal input |
| client → server | `0x01` | rows(u16 BE) + cols(u16 BE) | Resize |
| server → client | `0x00` | raw bytes | Terminal output |
| server → client | `0x10` | UUID string | Session ID |
| server → client | `0x11` | raw bytes | Scrollback snapshot |
| server → client | `0x12` | — | Shell exited |

## Close codes

| Code | Meaning |
|------|---------|
| `4404` | Session not found (invalid or expired `sid`) |

## Handshake sequence

```mermaid
sequenceDiagram
    participant C as Client
    participant S as Server

    C->>S: WS connect (?sid, view)
    Note right of S: resolve / create session
    S->>C: 0x10 Session ID
    S-->>C: 0x11 Scrollback (if non-empty)
    Note over C,S: streaming
    S->>C: 0x00 Output
    C->>S: 0x00 Input
    C->>S: 0x01 Resize
    S->>C: 0x00 Output
    S->>C: 0x12 Shell exited
    Note over C,S: connection closed
```

1. The client opens a WebSocket to `/ws` with an optional `sid` query parameter
   and an optional `view` flag.
2. The server resolves an existing session or creates a new one. If `sid` is
   provided but not found, the connection is closed with code **4404**.
3. The server sends `0x10` with the session UUID.
4. If the scrollback buffer is non-empty, the server sends `0x11` with the
   buffered output. The subscription is established atomically so no messages
   are lost between the snapshot and live streaming.
5. The main loop begins: output is forwarded as `0x00` frames, input and resize
   commands are read from the client. In view mode, client input is ignored.
6. When the shell process exits, the server sends `0x12` and the connection
   closes.
