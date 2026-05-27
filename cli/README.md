# Future OS CLI

Minimal CLI for Future OS account authentication.

## Commands

```bash
npm run dev -- auth login
npm run dev -- auth status
npm run dev -- auth logout
npm run dev -- agent start
npm run dev -- agent stop
npm run dev -- agent restart
npm run dev -- agent status
npm run dev -- tui
```

`auth login` starts a device flow against Future API:

1. Requests a device code from Future API `/oauth/device/code`.
2. Opens the platform console verification URL returned by Future API in the browser.
3. Polls Future API `/oauth/device/token`.
4. Saves the returned API Key to the `future` entry in `~/.future/agent/auth.json`.
5. Fetches `GET /openai/v1/models` with the saved API Key.
6. Writes a `future` provider with those models to `~/.future/agent/models.json`.

`auth logout` removes the saved Future API Key from the `future` entry in
`~/.future/agent/auth.json` while leaving non-secret settings such as `base_url` intact.
It also removes the `future` provider from `~/.future/agent/models.json`.

By default, the CLI connects to Future API at `http://api.westlakefuturegene.com`.
Set `future.base_url` in `~/.future/agent/auth.json` to use another API URL.

The saved key is intended for future `future-server/api` requests and is also readable by the
agent as the `future` provider key.

`agent` commands control the installed local Future agent service through the host service system:

- macOS: `launchctl`, default label `com.future.agent`. `agent start` auto-writes
  `~/Library/LaunchAgents/com.future.agent.plist` and bootstraps it when needed.
- Linux: `systemctl --user`, default unit `future-agent.service`.
- Windows: `sc.exe`, default service name `FutureAgent`.

Override the service name with `FUTURE_AGENT_SERVICE_NAME`, or use the platform-specific
`FUTURE_AGENT_LAUNCHD_LABEL`, `FUTURE_AGENT_SYSTEMD_UNIT`, or `FUTURE_AGENT_WINDOWS_SERVICE`.
On macOS, override the agent binary with `FUTURE_AGENT_BIN` and the gRPC bind address with
`FUTURE_AGENT_GRPC_ADDR`.

`tui` starts the TypeScript TUI and forwards all remaining arguments to it. For example:

```bash
npm run dev -- tui --grpc-addr 127.0.0.1:50051
```

By default it runs `../tui/dist/index.js` relative to the CLI package. Override that path with
`FUTURE_TUI_ENTRY` when using a custom install layout.
