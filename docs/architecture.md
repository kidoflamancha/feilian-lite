# Feilian Lite Architecture

## Status

This document records the initial architecture while the upstream command-line
client is being converted into a desktop application. The current executable
still runs the original single-process lifecycle. New code must move toward the
modules below without changing the proven protocol behavior unnecessarily.

## Modules

### Protocol core

The protocol core owns company discovery, authentication, session cookies, VPN
node discovery, route calculation, and preparation of a tunnel specification.
It must not create network interfaces, change routes or DNS, request elevated
privileges, read from stdin, or depend on Tauri.

The public session state is modeled by `SessionState`. Callers submit a
`SessionEvent`; invalid transitions are rejected instead of being represented
as combinations of booleans.

### Tunnel helper

The tunnel helper is the only process allowed to link `libwg` and modify system
network state. It exposes a versioned local interface limited to starting,
stopping, inspecting, and cleaning up one tunnel. It receives an ephemeral
WireGuard private key when starting a tunnel, but does not receive account
passwords or authentication cookies and does not retain the private key in its
supervisor state.

SOCKS5 mode runs without elevation. System split-tunnel mode starts the helper
with platform elevation and keeps the desktop application itself unprivileged.
Linux and macOS currently use an owner-only Unix socket with peer UID checking.
Windows uses a per-launch random named pipe. The pipe rejects remote clients,
allows only one server instance, and carries an explicit DACL granting access
only to the launching user's SID, SYSTEM, and Administrators. The helper also
verifies that every client is the exact desktop process that launched it.

### Desktop application

The Tauri application owns windows, tray behavior, user interaction, and
orchestration. Its Rust controller is the only caller of the protocol core and
helper client. The web frontend must not receive general shell or sidecar
execution permissions.

The initial desktop shell is implemented with Tauri 2 and Vue 3. Its
`AppController` selects a root-owned system helper or user-owned SOCKS5 helper,
then exposes typed status, traffic, stop, and cleanup commands. Helper absence
is represented as a retryable state rather than an application startup error.
The same controller owns a serialized authentication session: it discovers the
company, creates or reloads the local profile, keeps QR polling tokens outside
the WebView, and exposes only the login URL, authentication state, and VPN node
summaries. A selected node is prepared by the protocol core, converted to the
versioned `TunnelSpec`, and sent to a helper only after its identity and version
are verified. Failed helper startup triggers a best-effort server disconnect.

SOCKS5 starts the sibling helper directly. Linux system split-tunnel mode uses
`pkexec`, macOS uses a parameterized AppleScript administrator prompt, and
Windows uses ShellExecute/UAC while retaining the elevated process handle and
verifying the named-pipe server PID before sending tunnel secrets. Every helper
receives the desktop PID and exits through its normal cleanup path after that
parent disappears. macOS requests a dynamically allocated `utun` interface.
Tauri release builds stage the target-specific helper and bundle it as an
external binary next to the desktop executable.

The macOS AppleScript launcher is appropriate for the current beta. A signed
production deployment should replace it with an `SMAppService` privileged
helper and authenticate IPC using code-signing requirements. Windows packages
stage the official signed Wintun DLL after verifying its pinned SHA-256.

On Linux the desktop configures WebKit before Tauri initializes. It preserves
the native GTK backend and disables the DMA-BUF renderer to avoid a reproducible
GDK Wayland protocol failure without forcing the window through Xwayland.
`FEILIAN_GDK_BACKEND` remains the explicit operator backend override.

On Unix the profile directory is mode `0700`; profile and cookie files are mode
`0600`. WireGuard private keys, TOTP secrets, and optional passwords are stored
through the operating-system credential service instead of serialized into the
profile. Linux uses Secret Service, macOS uses Keychain, and Windows uses
Credential Manager. Existing plaintext desktop profiles are migrated on native
startup and rewritten without secret fields. A missing secure credential is a
hard error rather than a reason to generate a different WireGuard identity.

## Invariants

- At most one tunnel may be active in a helper process.
- Tunnel setup is transactional: cleanup runs in reverse order after partial
  failure, normal disconnect, parent exit, or termination signal.
- Private keys, authentication cookies, and TOTP secrets are never logged.
- The frontend receives typed, redacted state and errors rather than raw logs.
- IPC peers are authenticated using operating-system credentials and strict
  filesystem or named-pipe permissions.
- Existing CLI behavior remains a regression path until the desktop flow has
  equivalent integration coverage.

## Initial Delivery Scope

- Windows x86_64, macOS, and Linux x86_64.
- Feishu QR code and OIDC authentication.
- System split-tunnel and local SOCKS5 modes.
- UDP and TCP transports with IPv4 and IPv6 support inherited from upstream.

Password, LDAP, email-code UI, full-tunnel mode, mobile platforms, and a
persistent boot-time service are outside the first release.