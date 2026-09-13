# Phone remote access

FutureOS Mobile for Android/iOS controls sessions on your desktop. Tools execute
on the **desktop computer**, not inside a phone sandbox. Keep that computer awake,
Desktop running (graphical or explicitly headless), and both devices connected to the network.

## Pair a phone

1. Install the mobile build from the official distribution channel appropriate to
   your platform (Android package or iOS TestFlight invitation when offered).
2. Sign in on the desktop, open **Remote**, and select **Pair & start**.
3. Open FutureOS Mobile, grant camera permission and scan the desktop QR code.
   You can also paste the pairing code manually.
4. The code is valid for **five minutes** and can be used **once**. If expired,
   generate a new one. Do not share or post screenshots of it.
5. Choose an existing desktop conversation or create one from the phone.

Production and test builds must use matching service environments. A code issued
by another environment is rejected; use a matching build rather than editing the
code or endpoint.

## Headless Desktop over SSH

Run the Desktop executable directly in your terminal (`futureos.exe` on Windows):

```bash
./futureos --headless
```

No `--pair --qr` is needed: QR codes and text links appear when required. The process stays in the foreground; it does not daemonize.

1. **Not signed in to the platform:** the terminal shows a login QR, authorization URL and user code. Scan with your phone camera/browser, sign in to Future OS and authorize the server. No browser or inbound callback port is needed on the server.
2. **No phone pairing:** once the entry is ready, a second QR appears. Scan this one with **Add device / Scan in the Future OS app**, or paste the complete pairing link into the app.
3. **Valid login and pairing already saved:** reuse them and connect with the existing phone. Startup does not generate a replacement invitation. Authorization failures and expired invitations are reported explicitly.

Use `--no-qr` for links only; narrow terminals also fall back to links. `--re-pair` explicitly replaces the saved phone pairing; reserve it for changing phones or deliberately re-pairing, not ordinary network outages.

**Ctrl+C closes remote access and exits**, preserving login and completed pairing. An Agent started by this Desktop is stopped too, interrupting its active conversations; an independently running Agent is left alone. Unix also handles SIGTERM/SIGHUP. Exiting is not unpairing.

GUI and headless Desktop cannot own the same data directory concurrently. Initial login/pairing requires an interactive terminal; authorization links are not written to redirected logs. If you later use another user or systemd, keep the same account, HOME and credentials directory. Run as an ordinary user, not root. Continuity after SSH disconnect is not promised; explicitly use tmux or a service manager if you need it.

### Server build without graphical dependencies

The default Desktop build supports `--headless` but still links GUI system libraries. For a Linux server without GTK/WebKit, build the same backend without the GUI feature, from the repository root:

```bash
cargo build --release --no-default-features --manifest-path desktop/src-tauri/Cargo.toml
```

The executable is `desktop/src-tauri/target/release/futureos` (or under your `CARGO_TARGET_DIR`). Place a matching `future` CLI beside it or on PATH so Desktop can start the local Agent, or run `future agent` separately first. A missing Agent is an error, not permission to bypass platform login. Build channels and mobile production/test environments must still match; `--release` optimization alone does not determine the platform channel.

This build needs no window, WebView or X11/Wayland session. It is not a separate remote service product: it uses Desktop's storage, approvals and mobile protocol.

## What you can do

Read streaming replies, thinking and tool activity; send prompts and attachments;
choose the model/thinking level; rename conversations; stop runs; and respond to
approval requests. Image/file previews and downloads are available for supported
formats. Conversations can be pinned, renamed or deleted individually — or in
bulk from multi-select — and a workspace can be deleted with everything in it.
Workspace groups remember whether you folded them. Attachments come from the
system camera, the system photo picker or the system file picker, and text,
images or files can be **shared** into FutureOS from another app: that opens a
new conversation with the content in the composer, which you review before
sending.

Reconnection can refill missed events, but a sleeping/offline desktop cannot
execute new work until it is reachable again.

Remote controls the same desktop sessions and their permissions. Pairing does not
automatically turn on approval or sandbox mode. Review [[Sandbox]] before allowing
remote tasks to access sensitive files.

## Disconnect and revoke access

**Disconnect** stops the current remote connection; it is not equivalent to
revoking a device. Use **Unpair** for a phone that should no longer control the
computer. Reusing that phone afterward requires a new code. Revoke access if the
phone is lost or shared, and protect the desktop account too.

## Privacy and transport

Commands, conversation events and requested file content pass through the
configured NATS relay. Mobile uses per-device credentials, short-lived JWTs and
refresh tokens, with secrets in platform secure storage; it requires `wss://`.
The desktop-to-NATS hop follows the deployment configuration and does not enforce
TLS unconditionally. This is not a promise of end-to-end encryption or local-only
data processing. Only pair devices and use relay deployments you trust.

## Troubleshooting

- **Camera denied:** enable camera permission in phone settings, or paste the code.
- **Invalid/expired code:** generate a fresh code and confirm matching production/test builds.
- **Desktop offline:** check that it is awake, the app is running and Remote is connected.
- **Reconnect/pairing error:** follow the displayed reason/code; expired or revoked
  pairing requires a new pairing, while a temporary network failure can reconnect.
- **Approval pending:** inspect the requested scope and approve or reject explicitly;
  do not switch to Unrestricted merely to dismiss a security prompt.

See [[Installation]], [[Using FutureOS|Using-FutureOS]] and [[Sandbox]]. Mobile
source-build and TestFlight maintainer instructions live in the repository's
`mobile/README.md`.
