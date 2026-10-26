# A desktop save-as, measured on a real link — 2026-10-26

## The question, and the answer

The download-tier change (`feat/dl-tiers`, merged as `4981f4e99`) had only ever
run inside one process: a real host `download_prepare`, a real chunk pull and the
client's own hash check, but all in one address space over the in-process broker
(`docs/architecture/remote-download-tiers.md`, "Acceptance"). Nobody had saved a
large file from a host on the other side of a socket. This page does that.

**Answer: the reported defect is gone on a real link, and nothing on the phone
side moved.**

- A 12.5 MiB installer (`app-debug.apk`, 13 107 200 bytes, 25 chunks) is saved by
  the desktop with `shasum -a 256` **identical to the source**, in **94 ms**
  (release build; ~2.0 s in a debug build, §5.3). A 64 MiB file (128 chunks) is
  saved too, in 437 ms.
- A 600 MiB file and a 512 MiB + 1 byte file are **refused**, and the sentence is
  the desktop's own: `The file is over the 512 MB desktop download limit.`
- The **same** 12.5 MiB file through the phone's mode is still refused with the
  unchanged phone sentence: `The file is over the 10 MB download limit.`
- The phone's *type* rule is lifted for the desktop as well: a 1 MiB `.bin` is
  saved by the desktop and refused by the phone with
  `This file type is not available on mobile; view it on desktop.`
- An **old host** (built from `b594ac2bc`, before the variant existed) costs
  exactly one extra request: it answers `Unsupported download variant.`, the
  desktop asks again for `original`, and the download completes with a matching
  hash. The installer on that host draws today's phone sentence — today's
  behaviour, verbatim.

One documentation sentence was wrong and is corrected in
`docs/architecture/remote-download-tiers.md`: the host's staging copy is **not**
held for the 30-minute TTL when a download finishes — the client cancels on
completion and the host deletes it (§5.1). One user-visible inconsistency is
recorded as open rather than fixed (§5.2): the *preview* control still answers
the same installer with the phone's 10 MB sentence, by design.

## 1. Run identity

| item | value |
| --- | --- |
| repository | `future-os`, worktree `.worktrees/dl-tiers-accept` |
| branch | `test/dl-tiers-accept`, cut from `origin/dev-huichen` (`ed012b3a9`) |
| host code under test | `dev-huichen` = `b594ac2bc` + the `feat/dl-tiers` merge (`4981f4e99`) |
| old host | worktree `.worktrees/dl-tiers-oldhost`, detached at `b594ac2bc` (`4981f4e99^1`), one test-harness line changed (§4) |
| machine | Mac Studio, Apple M3 Ultra, 96 GB, macOS 26.6.2 (25G83) |
| toolchain | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, the channel `rust-toolchain.toml` pins — invoked through `~/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin`, because the Homebrew `cargo`/`rustc` on `PATH` (1.96.0) shadows rustup and does not read the pin |
| broker | `/opt/homebrew/bin/nats-server` **v2.15.0**, client `127.0.0.1:64881`, monitoring `127.0.0.1:64882`, its own store and log under `/tmp/dltiers` |
| processes | host child, client child, `nats-server`; each run uses `HomeGuard` temp homes, so nothing under the operator's `~/.future` is read or written |
| build dirs | `/tmp/dl-tiers-target` (new code), `/tmp/dl-tiers-old-target` (old code) |

The harness is `host/src/remote/download_tiers_network.rs` (new, `#[cfg(test)]`,
`#[ignore]`d). It is a second entry in the shape of `remote/real_network.rs`: the
test re-executes itself as a child process with `DLTIERS_ROLE=host|client`, so the
two ends are separate PIDs and the only channel between them is the broker.

```bash
# broker (its own store, its own ports)
nats-server -a 127.0.0.1 -p 64881 -m 64882 -sd /tmp/dltiers/nats -l /tmp/dltiers/logs/nats.log &

# new host + desktop client, one command (the orchestrator spawns both children)
cd .worktrees/dl-tiers-accept
export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"
CARGO_TARGET_DIR=/tmp/dl-tiers-target HOME=/tmp/dltiers/home \
  DLTIERS_DIR=/tmp/dltiers/new DLTIERS_NATS_URL=nats://127.0.0.1:64881 \
  cargo test -p future-host --lib --release -- --ignored --exact --nocapture \
    --test-threads=1 remote::download_tiers_network::desktop_download_tiers_over_the_network
```

Sources are created by the orchestrator, not by a fixture: a 12.5 MiB
`app-debug.apk` of deterministic pseudo-random bytes, a 64 MiB
`database-dump.bin` of the same kind, a 1 MiB `.txt` and a 1 MiB `.bin`, a
600 MiB sparse `disk-image.img` and a 512 MiB + 1 byte sparse `edge.bin`. The
sparse files only ever need their *declared* length, because the host refuses
them before copying anything.

## 2. What is real, and what is not

| | |
| --- | --- |
| **real** | `nats-server` process and TCP; the production host bridge (`remote::install_host` → the shipping supervisor, command loop and transfer loop); the real `download_prepare` command and therefore the real tier check, path check and regular-file check; the real chunk pull; the client's own hash check; the desktop's own `write_to_path` save; real files on disk; two separate OS processes |
| **real, but not the GUI** | the client half is `remote_peer::transfer::download_for_desktop` and `write_to_path` in a process of its own — the two functions the Desktop's `remote_peer_download_file` command calls. The Tauri command wrapper itself (argument types, `AppError`) is covered by its own in-process tests (`cargo test --lib commands::remote_peer`), and the panel that picks the destination is covered by `desktop/src/features/remote-peer/*.test.tsx`; **the webview and the save dialog were not driven** |
| **stubbed** | the agent behind the host is the harness's scripted mock agent; its availability is advertised (`agentAvailable: true`) but no conversation is involved in a download. The pairing platform HTTP hop is bypassed the same way `real_network` bypasses it: the v2 invitation is minted locally |
| **not covered here** | two physical machines, the public relay, TLS/WebSocket transport, the phone app itself (§6) |

The client reported 20 advertised host features including `e2ee_v2`,
`file_transfer_v1`, `file_download_v2` and `session_files_v1`.

## 3. The downloads

Release build, one run, the report the harness wrote
(`/tmp/dltiers/final/client.json`):

| # | request | source | result | bytes on disk | chunks | client wall clock | SHA-256 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | desktop save-as, mode `download` | `app-debug.apk` | **saved** | 13 107 200 | 25 | **94 ms** | source = saved (`448a2be1…`) |
| 2 | desktop save-as, mode `download` | `database-dump.bin` | **saved** | 67 108 864 | 128 | **437 ms** | equal |
| 3 | desktop save-as, mode `download` | `disk-image.img` (600 MiB) | **refused** | — | — | — | — |
| 4 | desktop save-as, mode `download` | `edge.bin` (512 MiB + 1 B) | **refused** | — | — | — | — |
| 5 | phone, mode `original` | `app-debug.apk` | **refused** | — | — | — | — |
| 6 | phone, mode `original` | `toolchain.bin` (1 MiB) | **refused (type)** | — | — | — | — |
| 7 | desktop save-as, mode `download` | `toolchain.bin` (1 MiB) | **saved** | 1 048 576 | 2 | — | equal |
| 8 | desktop *preview*, mode `preview` | `app-debug.apk` | **refused** | — | — | — | — |

The sentences, quoted from the client's side of the wire (the client puts the
host's `error` string through unchanged — `check_reply_success`):

| check | host's answer |
| --- | --- |
| 3 | `The file is over the 512 MB desktop download limit.` |
| 4 | `The file is over the 512 MB desktop download limit.` |
| 5 | `The file is over the 10 MB download limit.` |
| 6 | `This file type is not available on mobile; view it on desktop.` |
| 8 | `The file is over the 10 MB download limit.` |

And the saved copy verified from outside the harness, because a harness that
hashes its own bytes can only prove it is self-consistent:

```bash
$ shasum -a 256 /tmp/dltiers/final/source/app-debug.apk /tmp/dltiers/final/saved/app-debug.apk
448a2be182a8c7ed2bb6314eda39d6735318d6aa9d78d2b22d8910e601950a71  .../source/app-debug.apk
448a2be182a8c7ed2bb6314eda39d6735318d6aa9d78d2b22d8910e601950a71  .../saved/app-debug.apk
```

Check 8 is *observed*, not asserted: it is the by-design phone dialect for
previews (§5.2), recorded because a reader of the save-as success should know the
two controls now differ for that file.

Reading the numbers: 13.1 MB in 94 ms and 67.1 MB in 437 ms are both ≈139–154 MB/s on
loopback, so the transfer is not what the ceiling is about — the 512 MiB limit is
about a mis-click, and at that rate a full-ceiling pull would be a few seconds of
bytes plus 512 MiB of client RAM (§5.4). No network with a real RTT or loss was
involved (§6).

## 4. The old host

The host half here is a **separate build of the previous code**: worktree
`.worktrees/dl-tiers-oldhost`, detached at `b594ac2bc`, built with
`CARGO_TARGET_DIR=/tmp/dl-tiers-old-target`. One line of its *test harness* was
changed — `install_host(broker, "realnet-host", true)` → `false` — so its host
role installs the scripted mock agent instead of requiring a real `future-agent`
process. No product file in that worktree was edited.

It was started as its own process (its `REALNET_ROLE=host` half), and the new
desktop client was pointed at it with `DLTIERS_EXTERNAL_HOST=1 DLTIERS_OLD_HOST=1`
(the orchestrator then waits for the host's `host.json` instead of spawning a
host, and never writes into the directory it does not own).

| check | observed |
| --- | --- |
| mode `download` on a 1 MiB `.txt` | refused, `Unsupported download variant.` |
| the desktop's own path for the same file | **saved**, 1 048 576 bytes, SHA-256 equal to source, **122 ms** (one extra prepare on top) |
| mode `download` on the 12.5 MiB installer | refused, `The file is over the 10 MB download limit.` — today's phone sentence, i.e. today's outcome for a file the old host would have refused anyway |

So the fallback is not just a unit-tested predicate: an old host really does cost
one extra request and then serve the file, and a file it cannot serve still fails
with the sentence it used to.

## 5. Findings

### 5.1 The staging copy is released when the download completes (documentation was wrong)

`docs/architecture/remote-download-tiers.md` said the host "copies the file into
its transfer directory for the transfer's lifetime (30-minute TTL)". Measured, the
TTL is the *failure* path, not the normal one: `download_cancel` deletes the
staged copy, and the client sends `download_cancel` after a successful pull
(`Transfer::with_release`). After eight successful downloads on this machine the
staging directory held **no** `original_*` file and nothing over 1 MB — before
the runs it held only old preview files. The corrected sentence is now in the
architecture page: the host holds a copy for the transfer, releases it on cancel
(which a completed save does), and a failed or abandoned transfer waits for the
30-minute TTL.

This bounds the host-side disk cost of the feature better than the old sentence
implied, and it is worth stating because "a 12.5 MB copy per save for half an
hour" would have been a reason not to do the change.

### 5.2 Open: the preview control still answers the same installer with the phone's sentence

Clicking that installer in the remote file panel does not save it, it *previews*
it, and `remote_peer_preview_file` sends `preview` — the phone's tier, by design
(a preview is decoded and cached inside a budget, and the page's decision was not
to open that up). The result is that one file now behaves differently through two
controls: `Save as…` succeeds, opening it says
`The file is over the 10 MB download limit.`

This is not a regression (it was the outcome before the change too) and it is not
fixed here: changing the preview tier is a separate decision about memory and
cache, not about a save. It is recorded so that the next report of "it still says
10 MB" is read against the right control.

### 5.3 The build profile dominates the clock, not the link

The same 12.5 MiB download:

| build | samples (ms) |
| --- | --- |
| `--release` | 97, 93, 89, 94 |
| debug (`cargo test` default) | 2019, 1976, 2022, 2154 |

12.5 MiB in 94 ms is ≈139 MB/s; the debug figure is ≈6.5 MB/s. The gap is the
SHA-256 passes (host: the staged copy is hashed before it is served, and the
staged copy itself is a full `std::fs::copy`; client: the assembled file is
hashed once) plus the per-record AEAD, all unoptimised in a debug build. Loopback
bytes at 13 MiB are not the boundary either way — but a timing quoted without its
profile would misrepresent the feature by 20×, so both are printed.

### 5.4 The two costs the design already names, now measured from the other side

- **Client memory**: the pulling half assembles the whole file in a `Vec` before
  `write_to_path` (`transfer.rs::fetch`). At the ceiling that is 512 MiB of RAM in
  the desktop process; this run proves it works, not that it is cheap, and the
  largest file actually pulled here was 64 MiB.
- **Host disk**: a save stages a full copy (§5.1) — 512 MiB of temporary disk at
  the ceiling, released on completion.
- **Host CPU**: the staged copy is hashed in full before the transfer starts, and
  the file is read again to serve it, so a ceiling-sized save reads the file twice
  on the host.

None of these is new to this change; the phone tier never reached sizes where they
mattered.

### 5.5 What did *not* change, verified rather than assumed

- No file under `mobile/` is touched by the tier commit (`git diff
  b594ac2bc..4981f4e99 -- mobile/` is empty), `MAX_FILE_BYTES` is still
  `10 * 1024 * 1024` in both `host/src/remote_host/files.rs` and
  `mobile/src/remote/files.ts`, and the phone's two sentences came back word for
  word in checks 5 and 6 above.
- The desktop panel's save-as calls `downloadRemoteFile` **without** a `variant`
  in both of its call sites (`useRemoteFiles.ts`, `RemoteFilesDialog.tsx`), which
  is what selects the desktop tier; there is no size or type wording left in the
  desktop remote-files feature.
- An absolute path is still only barred from FutureOS credential files
  (`files::ensure_path_allowed`); the tier widens *what may travel*, not *what may
  be read* — a paired device could already read those files through the agent.
  The harness exercises exactly that: an absolute `/tmp/...` path with no session
  attachment, which is the markdown-link shape the host already allowed.

## 6. Not measured, and why

| not covered | why |
| --- | --- |
| **Two physical machines over the public relay** | needs a relay deployment and, in production, a signed-in account; the local run is two processes on one host over loopback TCP |
| **TLS / WebSocket NATS transport** | both ends call `require_tls(true)` only under `#[cfg(not(test))]`; a test binary speaks plaintext |
| **A file at or near 512 MiB** | the refusal boundary is measured (512 MiB + 1 byte), the largest successful pull is 64 MiB; a full-ceiling pull is 512 MiB of client RAM and 512 MiB of host staging disk, deliberately left to a two-machine run |
| **The phone app** | the phone's dialect is verified by mode, not on a device; no mobile threshold or wording was touched |
| **The Desktop webview, save dialog and panel** | the command's two functions were driven, not its GUI shell (§2) |
| **Packet loss / latency / NAT rebinding** | loopback only, as in `real-network.md` §7 |

## 7. Gates run

| gate | result |
| --- | --- |
| new harness, release, new host | pass — one test, 1.36 s, report quoted in §3 |
| new harness, release, old host | pass — one test, 0.53 s, report quoted in §4 |
| `cargo test -p future-host --lib remote_host::files` | pass (44) |
| `cargo test -p future-host --lib remote_peer::transfer` | pass (34) |
| `cargo fmt -p future-host --check` | clean |
| `cargo clippy -p future-host --all-targets -- -D warnings` | clean |
| `make check-docs` | see the merge report; the pair is written to the same shape as `remote-page-encoding-2026-10-26` |

The release build prints one warning of its own, unrelated to this change:
`stripping debug info with rust-objcopy failed … Library not loaded: @rpath/libLLVM.dylib`
— the pinned toolchain's `rust-objcopy` needs an LLVM dylib that is not shipped
beside it. The build finishes; only the debug-info strip is skipped.

## 8. Re-running the old-host arm

The old host has to be a build of the previous commit, so it is driven from its
own worktree:

```bash
# 1. the old host, in its own process (one harness line patched: real_agent=false)
git -C /Users/geilige/future-os worktree add --detach .worktrees/dl-tiers-oldhost 4981f4e99^1
#    host/src/remote/real_network.rs: install_host(…, "realnet-host", true) → false
cd .worktrees/dl-tiers-oldhost
export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"
CARGO_TARGET_DIR=/tmp/dl-tiers-old-target HOME=/tmp/dltiers/home \
  REALNET_ROLE=host REALNET_DIR=/tmp/dltiers/old REALNET_NATS_URL=nats://127.0.0.1:64881 \
  REALNET_AGENT_ADDR=127.0.0.1:1 REALNET_LOG_DIR=/tmp/dltiers/logs \
  cargo test -p future-host --lib -- --ignored --exact --nocapture --test-threads=1 \
    remote::real_network::real_network_two_process_host_and_client &

# 2. the new client against it
cd .worktrees/dl-tiers-accept
CARGO_TARGET_DIR=/tmp/dl-tiers-target HOME=/tmp/dltiers/home \
  DLTIERS_DIR=/tmp/dltiers/old DLTIERS_NATS_URL=nats://127.0.0.1:64881 \
  DLTIERS_EXTERNAL_HOST=1 DLTIERS_OLD_HOST=1 \
  cargo test -p future-host --lib --release -- --ignored --exact --nocapture \
    --test-threads=1 remote::download_tiers_network::desktop_download_tiers_over_the_network
```

The old host exits as soon as the orchestrator writes `client.done` (which it does
on success as well as on failure), or after its own 300 s deadline.
