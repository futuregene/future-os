# Headless dev unsigned build results

> [中文](headless-unsigned-results.zh-CN.md)

All three workflows and five architecture jobs completed successfully. No source repair or repeated rerun was used to assemble this round.

- Full candidate: `a5f91752b4e470f90fbe5f4bc303ec0ca814944d`; pinned ref: `test/headless-unsigned-20261020`.
- Program version: `0.0.2-a5f9175+dev`; file version: `0.0.2-a5f9175`.
- macOS [39418462362](https://github.com/futuregene/future-os/actions/runs/39418462362): Apple Silicon and Intel successful.
- Windows [39418473647](https://github.com/futuregene/future-os/actions/runs/39418473647): x64 unsigned successful.
- Linux [39418482527](https://github.com/futuregene/future-os/actions/runs/39418482527): x86_64 and aarch64 successful.
- Public URLs come from actual successful job logs. All 15 files returned HTTP 200 and were fully streamed for SHA-256; eight Windows/Linux packages matched their CI archive hashes.
- [Structured evidence](headless-unsigned-results-2026-10-20.json), [SHA-256 manifest](headless-unsigned-2026-10-20.sha256). Compare downloaded files with `shasum -a 256 -c` or a platform equivalent; check only the entries for files you downloaded.

## Public downloads

These are public dev URLs without a GitHub login requirement. Separate GitHub artifact downloads may require login and do not replace the checks of these public URLs.

| File | MiB | Public URL | SHA-256 prefix |
|---|---:|---|---|
| FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz | 46.62 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz) | `8887d0d5173183f8…` |
| FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz.sig | 0.00 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz.sig) | `094d39214644a8b5…` |
| FutureOS_0.0.2-a5f9175_aarch64-unsigned.dmg | 50.41 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_aarch64-unsigned.dmg) | `fc3519be124bb339…` |
| FutureOS_0.0.2-a5f9175_amd64.deb | 49.14 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_amd64.deb) | `f42b49991cf20541…` |
| FutureOS_0.0.2-a5f9175_arm64.deb | 46.95 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_arm64.deb) | `1b35d505fcbf41ff…` |
| FutureOS_0.0.2-a5f9175_linux_aarch64-cli.tar.gz | 22.12 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_aarch64-cli.tar.gz) | `5c6255ce30f0d22e…` |
| FutureOS_0.0.2-a5f9175_linux_aarch64-portable.tar.gz | 45.26 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_aarch64-portable.tar.gz) | `4eb7ce2ea92c7cc7…` |
| FutureOS_0.0.2-a5f9175_linux_x86_64-cli.tar.gz | 23.75 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_x86_64-cli.tar.gz) | `2c4eef8b826ae282…` |
| FutureOS_0.0.2-a5f9175_linux_x86_64-portable.tar.gz | 47.90 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_x86_64-portable.tar.gz) | `fc2893039fb7d9d6…` |
| FutureOS_0.0.2-a5f9175_windows_x64-portable-unsigned.zip | 46.13 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_windows_x64-portable-unsigned.zip) | `2f1a28960b14ac24…` |
| FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe | 32.13 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe) | `db41a858ee04f240…` |
| FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe.sig | 0.00 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe.sig) | `c22a9cf60c91f84d…` |
| FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz | 49.63 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz) | `8339f6213584ff9d…` |
| FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz.sig | 0.00 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz.sig) | `ff28692700c6ee43…` |
| FutureOS_0.0.2-a5f9175_x64-unsigned.dmg | 54.73 | [Download](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-unsigned.dmg) | `89a4d52143fb2968…` |

## Verified payload behavior

| Scope | Actual result |
|---|---|
| Windows NSIS / portable | Extracted real CLI version, headless/Agent help and Desktop help passed without initialization side effects; normal/delayed PE imports contain no prohibited GUI frameworks; CLI hashes match across both packages |
| Linux deb / portable / CLI-only, both architectures | Actual payload entry/version checks passed; no ELF NEEDED on CLI; CLI hashes match across each architecture's three payloads; executable modes retained, retired standalone headless absent |
| Old Linux | The same CLI from deb/portable/CLI-only passed help and first noninteractive refusal in networkless Rocky Linux 8 without GUI, using isolated home; no authorization material, database/pairing or valid credentials produced |
| macOS bundled CLI, both architectures | Native CI ran headless help and first noninteractive refusal; Mach-O has no Tauri/WebView GUI frameworks; DMG structure/checksum passed |
| Public macOS updater archives | Public tar.gz files were inspected read-only, without installation/execution; embedded executable CLI hashes match the native-CI binaries, and the retired standalone entry is absent |
| Public integrity | All 15 files fully read/hashed with byte counts matching HTTP lengths; eight Windows/Linux package hashes also match CI records |

Windows CLI legitimately uses system APIs such as USER32/shell32. GUI exclusion here means no product GUI frameworks such as Tauri/WebView/GTK, not absence of all OS libraries. Linux has separate CLI-only tarballs; this round's Windows/macOS workflows provide CLI inside GUI/portable payloads, with no separate CLI-only archive. Independent CLI installation channels on those two platforms are not accepted by this round.

## Remaining work

- The user will later test Windows Authenticode and macOS Developer ID/notarization/timestamps/Gatekeeper. This round did not use those certificates/services; build-required ad-hoc and existing updater .sig files do not replace platform signing acceptance. Signature files were hashed, without independent local updater public-key verification.
- Real installation, upgrade/rollback/uninstall from an old release and final same-candidate Desktop/CLI/Mobile functional manual evidence remain. NSIS extraction smoke is not a real machine installation.
- Controlled performance comparison has not run; these build durations establish no extraction speedup.
- This round freezes `a5f91752`, excluding later main startup-wait/relay changes. Testing newer source requires a newly identified and validated candidate.
- See the [release acceptance checklist](headless-release-acceptance.md). Unsigned success does not complete the overall Goal.

Retain the pinned ref for later signing tests. Any physical-device task still needs exact candidate/payload/hash and per-case expectations; do not overwrite everyday data or stop an external Agent.
