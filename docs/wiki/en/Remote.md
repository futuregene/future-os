# Phone remote access

FutureOS Mobile for Android/iOS controls sessions on your desktop. Tools execute
on the **desktop computer**, not inside a phone sandbox. Keep that computer awake,
the desktop app running, and both devices connected to the network.

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

## What you can do

Read streaming replies, thinking and tool activity; send prompts and attachments;
choose the model/thinking level; rename conversations; stop runs; and respond to
approval requests. Image/file previews and downloads are available for supported
formats. Reconnection can refill missed events, but a sleeping/offline desktop
cannot execute new work until it is reachable again.

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
