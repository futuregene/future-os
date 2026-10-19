# Remote package manual acceptance

[中文](remote-package-manual-acceptance.zh-CN.md)

This pending checklist covers phases 0–5 of the [development plan](../architecture/remote-package-development.md). It is not a pass record. After automated acceptance passes, freeze a complete commit and build Desktop GUI, headless, its matching CLI and Mobile from that commit. Product source changes require affected automated and manual checks again with a newly recorded candidate. Results from different versions cannot establish one candidate passing.

## Candidate and environment record

Record the complete commit, branch, CI URL/result, host OS/architecture, phone OS/model/channel, platform/relay environment, previous version used for pairing compatibility, operator and time. Use public labels only; never record invitations, tokens, keys or authenticated URLs. For every item record PASS / FAIL / not executed, observations, redacted evidence and associated issue. Not executed does not mean passed.

Build in the isolated worktree. Use a test account, dedicated workspace and nonsensitive files. Do not overwrite daily data or stop an existing Agent. Upgrade compatibility uses old-version data owned by a dedicated test system user, backed up before upgrading. Test fresh pairing separately. Run GUI/headless sequentially for a shared data directory. External-Agent checks use a dedicated FutureOS home and an unused endpoint according to existing Agent instructions.

Source entry points: GUI `make run-desktop`; Android `make run-mobile-android`; iOS `make run-mobile-ios`; headless and matching CLI `make build-desktop-headless`, then execute the resulting `futureos-headless` in an interactive terminal (`.exe` on Windows). Follow the [headless guide](../guide/desktop-headless.md) for login, pairing and exit. Building does not establish runtime success.

## Actions and expectations

| Item | Action | Expected result |
|---|---|---|
| Visible GUI lifecycle | Start an unpaired GUI, enable Remote without adding a device, then disable it | Empty instance can be Enabled without creating an invitation; disabled entry rejects remote operations |
| Fresh pairing | Add a device, scan/paste the complete invitation in Mobile, connect, then reuse the invitation | First pairing succeeds with correct identity; consumed invitation fails; diagnostic logs contain no complete invitations or keys |
| Stored pairing compatibility | Pair on the old version using dedicated data, upgrade it to the candidate, restart both apps | Existing identity, pairing book, sessions and credentials remain usable without repairing or path changes |
| Desktop controller | Connect a second candidate Desktop to the first, read catalog, open a session, send a message, disconnect | Rust roles interoperate; explicit disconnect is not undone by stale recovery |
| Mobile sessions and replay | Create a session and stream text/tool output; switch sessions and page history | Correct ordered content and tools; local/remote identities remain distinct; no lost or duplicate history |
| Network and lifecycle | Disconnect/reconnect network during a session; switch Wi-Fi/cellular, lock and foreground phone | Visible state changes, recovered catalog/events, no repairing, duplicate messages or repeated execution |
| Control ownership | Pair two controllers, let B explicitly take over, then A; inspect both and attempt writes | Timely active/standby/holder updates; standby cannot execute controlled business; catalog/events resume after takeover |
| Endpoint isolation | Connect one controller to two hosts with distinguishable content; switch sessions and stop one | Identities, catalogs, events and files remain separate; other host stays usable; recovery retains ownership |
| Approvals | Trigger an operation known to require approval in the test environment; deny then approve | Host presents approval, denial prevents execution, approval executes only its operation and routes results correctly |
| Sandbox | Access allowed test workspace and then explicitly forbidden paths/operations | Existing authorization permits/rejects correctly through the extracted Server boundary |
| Native files | Upload/download small, multi-chunk and empty files with Unicode/spaced names; compare size/hash; open/share on Mobile | Exact bytes/names and correct native picker/save/open/share behavior; no endpoint mix-up |
| Stop during transfer | Disable Remote during a large transfer, allow network/read completion, reenable and transfer again | Old operation fails/cancels without late writes or false success; new instance works without stale-task interference |
| GUI exit | With connection and active request, close the main window/quit, try another phone request, restart GUI | Exit closes admission; no operation accepted while closed; valid pairing survives restart without stale tasks |
| Headless | After GUI exits, run interactive headless, login/pair or reuse pairing, read/write from phone, Ctrl+C during activity, restart | No GUI window; terminal state visible; Ctrl+C closes entry and returns to shell; valid pairing reusable |
| External Agent ownership | In dedicated environment start independent Agent, attach GUI/headless, exit host | Remote entry closes; independent Agent still serves local clients and is not killed |
| Revoke and repair | Remove one device, reconnect with old credentials, add another; repeat offline removal followed by reconnection | Existing revocation/refresh policy rejects old device, preserves others, drains pending revocations and permits new pairing |
| Renewal/exit race | In controllable renewal window maintain two devices, renew standby slot and exit host during renewal | Correct slot credentials used; late results cannot reinstall bridges or reopen admission. Mark not executed if renewal cannot be controlled |

## Platform coverage and completion

Explicitly record which cases were executed on Windows/macOS/Linux hosts and Android/iOS physical devices. Missing environments remain pending; another platform's pass is not a substitute. TLS/private CA, self-hosted relay authentication and OS behavior require their real environments; the unauthenticated loopback interoperability broker cannot prove them.

For failures record candidate, reproduction, expected and actual behavior before fixing and revalidating. Complete the Goal only after required automated evidence and user confirmation of required manual cases. Performance separately requires matched runner, toolchain, features and cache conditions as defined in the development plan; manual success does not establish performance gains.
