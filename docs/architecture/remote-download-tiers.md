# Remote download tiers: a desktop is not a phone

> [中文](remote-download-tiers.zh-CN.md)
>
> Decision date: 2026-10-26. Source baseline: `89a18748d`, plus the wording fix in
> `708959a23` (*name the download limit when it is what blocks a file*).

## The defect

A remote desktop saving a file from a paired host was refused with:

> This file type is not available on mobile; view it on desktop.

The file was a 12.5 MB `app-debug.apk`, and the refusal was wrong twice over: the
type was not the binding limit (the size was, at 10 MiB), and the panel showing
the error *was* the desktop.

One policy was serving two clients. The 10 MiB ceiling and the type allow-list are
a phone's rules, and they are right for a phone: a preview is decoded inside a
10 MiB budget, the mobile reader can only act on a fixed set of types, and a tap
can happen on a metered link. A desktop had none of those reasons — it had asked
to write bytes to a path the user picked — and it was held to them anyway, because
`download_prepare` had no way to say who was asking.

## The rule now

`download_prepare`'s `mode` names the *client's intent*, and the host picks its
limits from it ([`files.rs`](../../host/src/remote_host/files.rs), `DownloadTier`):

| Variant | Who | Ceiling | Type allow-list |
|---|---|---|---|
| `preview` (and an empty mode) | phone, in-app preview | 10 MiB | yes — the host serves exactly what the phone's reader can route |
| `original` | phone, save/share | 10 MiB | yes |
| `download` | desktop, save-as | 512 MiB | no |

Unchanged for the phone, deliberately: `MAX_FILE_BYTES`, `is_mobile_download_allowed`,
the type table in `mobile/src/remote/fileTypes.ts`, and every phone-facing message.
An unknown or misspelled variant is *not* a way to reach the desktop tier — only
the one variant that names a desktop save is, because a phone that sends something
unrecognised must not silently buy 512 MiB.

One refusal survives the desktop tier: the source must be a regular file. A
directory, a device node, a socket or a FIFO has no size to stream and no meaning
as "a copy", so it is refused under both tiers.

### Why 512 MiB

The number is not new and not a second guess: the pulling half already refuses to
assemble more than `MAX_DOWNLOAD_BYTES` = 512 MiB
([`transfer.rs`](../../packages/remote-client/src/transfer.rs)), and the host
re-exports that constant as `DESKTOP_MAX_FILE_BYTES` rather than restating it. A
host tier above the client's would prepare transfers the client must then fail; one
below would refuse files the client could have completed. 512 MiB covers what a
remote desktop is actually asked for — an installer, a log bundle, a database dump
— and still stops a mis-click on a directory-sized artefact or a video. The
ceiling is a power of two and the messages label it `MB`, which is how the file
list already labels a size (`12.5 MB` for 12.5 MiB): the wording matches the row
the user is reading rather than introducing a second convention.

### Two different refusals, two different sentences

| Tier | Message |
|---|---|
| phone, over the ceiling | `The file is over the 10 MB download limit.` |
| phone, type the reader cannot use | `This file type is not available on mobile; view it on desktop.` |
| desktop, over the ceiling | `The file is over the 512 MB desktop download limit.` |

Size is decided before type, so a phone is told the thing it can act on. The phone
matches `download limit` to pick its localised wording, which is why the desktop
sentence contains that phrase too but names the desktop: the two refusals must not
be reported as each other.

## What did not change

- **The phone.** Same threshold, same allow-list, same messages, byte for byte.
  `prepare_preview` (the mobile preview path) still applies `MAX_FILE_BYTES`.
- **Desktop preview.** Still the phone's dialect (`preview`, falling back to
  `original`). A preview renders inside this app and its cache is bounded by entry
  count and TTL, not by size — deliberately not opened up here.
- **The transfer protocol itself.** Chunking, pulls, the hash check and the
  `read_binary_v1` paging lane are untouched; only which limits a `download_prepare`
  is measured against changed.

## Old hosts

The desktop asks for `download` and, if the host answers `UNSUPPORTED_VARIANT`
(a matched string in [`remote-protocol`](../../packages/remote-protocol/src/files.rs)),
asks again for `original` — today's request, today's limits, today's outcome. So a
new desktop against an old host behaves exactly as it does today, at the cost of
one extra request; nothing is refused that the old host would have served.

Only that refusal is retried. Retrying any failure as `original` would replace the
desktop's own limit message with the phone's and report two different problems as
one, so the predicate is a unit test of its own.

## Acceptance, 2026-10-26

Run on macOS arm64, the repository's pinned toolchain (1.97), against an in-process
host bridge over the in-process broker — the host's real `download_prepare`, the
real chunk pull, the client's own hash check:

| Check | Result |
|---|---|
| Desktop save-as of a 12.5 MiB `app-debug.apk` | succeeds, and the written bytes equal the source byte for byte (`a_desktop_save_takes_a_file_the_phone_must_refuse`, `a_desktop_download_of_an_installer_is_not_held_to_the_phones_limits`) |
| Phone mode (`original`) on the same file | still refused, `The file is over the 10 MB download limit.`, unchanged |
| A file over the desktop ceiling | refused with the desktop's own sentence, and refused by the phone's rule under the phone's mode (`a_desktop_download_has_its_own_ceiling_and_says_so`) |
| A directory under either tier | refused |
| Old host (no `download` variant) | falls back to `original`; both halves pinned — the host emits `UNSUPPORTED_VARIANT` exactly, the client retries on exactly that (`only_a_missing_variant_falls_back`) |
| Ceiling single source | `DESKTOP_MAX_FILE_BYTES == MAX_DOWNLOAD_BYTES` (`the_desktop_ceiling_is_the_one_the_puller_enforces`) |

**Not measured here:** two real machines over a real relay with a file near the
512 MiB ceiling. No host was deployed — replacing the serving binary is a separate,
approved step — so the numbers above are one process, not a network. Two costs are
inherent to the existing design and worth knowing before that test: the host stages
a full copy of the file while the transfer runs (released when the client cancels,
which a completed save does; a failed or abandoned transfer waits for the 30-minute
TTL), and the pulling client assembles the whole file in memory before writing it,
so a full-ceiling save costs 512 MiB of RAM on the desktop plus 512 MiB of
temporary disk on the host.

### Across a process boundary and a real broker, 2026-10-26

A second run puts the host bridge in one OS process and the desktop's client half
(`remote_peer::transfer::download_for_desktop` + `write_to_path`, the functions the
Tauri command calls) in another, with a real `nats-server` over TCP between them:
the 12.5 MiB installer saves with a matching `shasum` in 94 ms (release), a 64 MiB
file in 437 ms, 600 MiB and 512 MiB + 1 byte are refused with the desktop's
sentence, the phone's mode and type rule still refuse their cases word for word,
and a host built from `b594ac2bc` really does answer `Unsupported download variant.`
and get asked again. Numbers, the exact strings and what remains unmeasured are in
[`docs/testing/remote-download-tiers-live-2026-10-26.md`](../testing/remote-download-tiers-live-2026-10-26.md).

## Reproducing

```bash
cargo test -p future-host --lib remote_peer::transfer   # the save-as and the fallback
cargo test -p future-host --lib remote_host::files      # the tiers and their sentences
cd desktop/src-tauri && cargo test --lib commands::remote_peer   # the command layer
```
