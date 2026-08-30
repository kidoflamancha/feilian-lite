# Helper Protocol

The helper protocol is defined by the `feilian-ipc` crate. It is transport
neutral so Unix sockets and Windows named pipes use the same messages and
lifecycle rules.

## Framing

Each transport message is a four-byte unsigned big-endian length followed by a
UTF-8 JSON document. A request or response may not exceed 64 KiB. Each request
contains a protocol version and request ID; the response repeats the request ID.
Malformed frames use request ID `0` when no trusted ID can be decoded.

## Commands

- `hello`: negotiate the protocol and report the helper version.
- `start_tunnel`: start one system split tunnel or SOCKS5 tunnel.
- `stop_tunnel`: stop the active tunnel; stopping while idle succeeds.
- `status`: return the lifecycle state and a redacted tunnel summary.
- `stats`: return cumulative WireGuard transmit and receive bytes.
- `cleanup`: force cleanup after an interrupted or partially failed operation.

Starting the same tunnel twice is idempotent. Starting a different tunnel while
one is active returns `already_running`. The supervisor stores a SHA-256
fingerprint and redacted summary after startup, not the private key or SOCKS5
password.

## Unix Security

The helper accepts a socket path, owner UID, and owner GID. The parent directory
must be owned by that UID and have no group or other permissions. The socket is
created as mode `0600`, assigned to that UID/GID without following symlinks, and
each accepted peer is checked with operating-system peer credentials.

The helper also requires the desktop parent PID. It checks that process once per
second and terminates through normal Rust destruction when the parent exits, so
the libwg backend can stop its active tunnel even when the helper was elevated.

The `feilian-helper-client` crate performs the reverse identity check before it
sends a request: system-tunnel callers require the server process UID to be
root, while SOCKS5 callers require the current user UID. It also verifies the
protocol version and request ID on every response and exposes typed methods
instead of raw JSON to the desktop controller.

## Windows Security

The Windows transport uses a random per-desktop-process named pipe under the
local `\\.\pipe\feilian-lite-*` namespace. The server enables first-instance
protection and rejects remote clients. Its DACL grants full access only to the
launching user's SID, SYSTEM, and Administrators. After accepting a connection
it queries the client PID from the pipe handle and rejects every process except
the exact desktop PID supplied when the helper was launched. The high-entropy
pipe name prevents an unrelated process from pre-creating a predictable
endpoint.

The system helper is launched through UAC, while the SOCKS5 helper stays at the
desktop user's privilege level. Both monitor the desktop process handle and
exit through normal cleanup when it terminates. TCP loopback is not used as an
IPC substitute.