# Headless manual acceptance record

> [中文](headless-manual-acceptance.zh-CN.md)
>
> Status: on 2026-10-20 the user reported manual testing complete and requested cleanup and a PR. Individual platform/case evidence was not separately supplied; exploratory results are not transferred to later source candidates.

## Latest status and historical build

[PR #1259](https://github.com/futuregene/future-os/pull/1259) merged as `a5f91752b4e470f90fbe5f4bc303ec0ca814944d`. CI 39408277224 on candidate `dac8832f` is fully complete and successful, including the final Windows Host. See [final evidence](headless-ci-39408277224-2026-10-20.json) and [deferred release checklist](headless-release-acceptance.md). This closes the automation observation, without transferring old exploratory manual results to new source.

The user subsequently requested local branch/worktree cleanup. Primary returned to `main`, the temporary test/implementation branches were removed, and the existing skills checkout was preserved. The identities and commands below are a historical handoff. Existing target/debug files do not become a new candidate when source switches; check `future version --json` before use. Deferred manual checks must use actual version-identified payloads.

## Candidate and order

During exploratory testing, the primary workspace `/Volumes/ZHITAI/Workspace/future-os` used `test/headless-primary` at `a0d4d8402064dab1b82b0ad39bbd645837446236`. macOS ARM64 debug CLI/GUI were built; CLI reports `0.0.2-a0d4d840+local.dirty`, with dirty state coming from the preserved skills submodule checkout. See the [handoff record](headless-primary-handoff-2026-10-20.json). Keep this source and its runnable artifacts unchanged while the user tests; implementation continues in its separate worktree.

Implementation may receive further fixes/upstream changes, each with its own full SHA. Primary-workspace observations do not automatically validate a later candidate. The automated candidate SHA/run are recorded above; final release Desktop/CLI payload identities, Mobile build, platforms and per-case manual evidence remain to be filled.

The user's latest sequence puts exploratory/manual functional testing before actual release packaging, signing and install/upgrade acceptance. Existing automated installer regressions must still pass. Unexecuted/unreported cases remain unconfirmed. Source changes require affected checks to be repeated against the new candidate; do not combine different candidates' results.

## Historical macOS exploratory build

From the primary workspace:

```bash
cd /Volumes/ZHITAI/Workspace/future-os
BUILD_CLI=0 make run-desktop
```

Quit GUI normally before running headless in a real interactive terminal:

```bash
cd /Volumes/ZHITAI/Workspace/future-os
./target/debug/future headless
```

Use a second terminal for device management:

```bash
cd /Volumes/ZHITAI/Workspace/future-os
./target/debug/future remote devices
```

Verify the full commit with `./target/debug/future version --json`. Final same-candidate Mobile builds and other host-platform versions/launch paths are not yet assigned. No Android, iOS, Windows or Linux manual pass is claimed. An existing phone app can first exercise old pairing compatibility; record its actual build and OS.

## Cases and expected outcomes

| ID | Platform/preparation | Actions | Expected | Result |
|---|---|---|---|---|
| M01 | macOS GUI and existing phone/data | Quit old host normally; record workspaces, conversations and pairs. Open the same home with this GUI, then headless, reconnect phone | Original database/workspaces/identity/book/control pointer remain, no re-pairing, conversation continues | Awaiting user report |
| M02 | Interactive terminal, phone, isolated empty Future home | Launch with `FUTURE_HOME=<temporary absolute path>`; complete login and phone invitation, send messages and restart | Functional pairing/event sync, original layout, credentials reused after restart | Awaiting user report |
| M03 | Isolated empty home, redirected stdin | Run `future headless </dev/null`, record output/exit | Exit 1 and terminal requirement; no authorization output, app.db, pairing or credential creation | Awaiting user report |
| M04 | User's self-hosted relay and phone | In isolated home launch with `--relay <URL>` and existing private `--relay-ca <PEM>` if needed; pair and restart | Selected relay in invitation, persisted pairing, original origin/TLS/CA boundaries | Awaiting user report |
| M05 | Two or more paired clients | List devices, add/pair another, use its ID/index to transfer control, restart and inspect | Existing pairs stay, book/control pointer agree, original controller restrictions | Awaiting user report |
| M06 | Isolated M05 test pairing | Remove one device, confirm another still works; exercise revoke while network fails, restart and restore network | Exact target revoked, others preserved, original pending-revoke recovery | Awaiting user report |
| M07 | Phone and GUI separately, temporary workspace | Trigger read/write/command approvals, deny and approve, save a workspace rule and repeat | Decisions actually apply, states sync, atomic persistence/live injection, no privilege expansion | Awaiting user report |
| M08 | Host with OS sandbox support | Access allowed temporary workspace and outside-boundary test file under current sandbox policy; deny escalation | Original allow/deny boundary and visible errors; no bypass after entry/package change | Awaiting user report |
| M09 | Phone, temporary files/workspace | Preview/transfer/edit test files; create and execute a task, inspect conversation/events/results/review | Correct contents/results and event sync, no duplicate business actions, path boundary preserved | Awaiting user report |
| M10 | GUI and headless separately with active work | Exit during generation, transfer, task or pending approval; Ctrl+C headless; restart/reconnect | Admission closes before bounded cleanup, owned Agent stops, no late writes/resurrection, data/pairs preserved | Awaiting user report |
| M11 | macOS, same Future home | Start headless while GUI runs, then reverse after quitting GUI; use separate terminal for device requests | Second host refuses without stealing/deleting lock; live host and local control continue | Awaiting user report |
| M12 | Unix/macOS, only the newly owned child PID | Send SIGTERM/SIGHUP during login, pairing and normal service; restart each time | Bounded exit at each stage, credentials/pair/control pointer preserved, no owned Agent leak | Awaiting user report |
| M13 | Isolated Future home, fresh unused port, external Agent | Start `future agent --grpc-addr 127.0.0.1:<fresh port>` in another terminal against that home. Start headless with the same home, explicit endpoint and external mode. Start sustained work from an independent CLI session, then exit headless | External Agent PID and other-client work survive; only this host's submitted runs cleaned under original ownership. Finally stop only this newly owned Agent normally | Awaiting user report |
| M14 | GUI/headless separately, platform connectivity | Exercise model refresh, login reuse and built-in skill initialization; use an installed skill in a conversation | Models/skills work and persist, maintenance uses the same CLI, no separate headless/Desktop executable required | Awaiting user report |

Use isolated test homes for M02/M03/M04/M06/M13. Do not modify or terminate an existing external Agent. Windows Ctrl+C/console-close and sandbox behavior need native automation and an explicitly identified Windows manual build; macOS M12 does not establish a Windows pass.

## Recording and deferred acceptance

For each result record full candidate SHA, host OS/architecture, actual Desktop/CLI versions, phone OS/app build, data root, actions/expectations, observed result, evidence references and retest status. Credential values are not public evidence. Results without evidence stay unconfirmed.

The user reported this manual round complete and authorized PR preparation, with no remaining failure reported. The exploratory commit is `a0d4d8402064dab1b82b0ad39bbd645837446236`. The table retains per-case feedback status so this aggregate report is not represented as independent evidence for every platform/case. Later main synchronization and CI fixture repairs are outside that tested revision. Signed payloads, real install/upgrade/uninstall, old-Linux static smoke, matched measurements and final same-candidate results remain to be consolidated. Opening or merging the PR does not establish final Goal acceptance.
