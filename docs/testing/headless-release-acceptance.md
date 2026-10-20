# Headless deferred release acceptance checklist

> [中文](headless-release-acceptance.zh-CN.md)
>
> 2026-10-20: implementation merged through [PR #1259](https://github.com/futuregene/future-os/pull/1259); ordinary CI is complete and successful. This checklist records remaining work. No release, signing, upload or real installation was triggered.

## Confirmed state and candidate identity

- Automated candidate: `dac8832f725f7f2813cf2cc453d0f847b60f7cfa`.
- Squash merge: `a5f91752b4e470f90fbe5f4bc303ec0ca814944d`; source trees were checked identical at merge.
- [Final CI 39408277224](https://github.com/futuregene/future-os/actions/runs/39408277224): 19 successful jobs, two optional performance jobs skipped, zero failed, all complete. Windows Host: 1693 passed/0 failed/10 original ignores. Linux/macOS Host: each 1723 passed/0 failed/10 original ignores.
- Windows installer fixtures, macOS notarization recovery logic, native entry/signal, shared-core and dependency-boundary checks passed. These do not establish actual signed-payload, real install/upgrade or notarization-service acceptance.
- The user reported exploratory manual testing complete on older `a0d4d840`; this is not per-case, all-platform signed acceptance of later candidates. See [manual record](headless-manual-acceptance.md) and [final CI evidence](headless-ci-39408277224-2026-10-20.json).

Start deferred builds from the verified merge. If newer source is selected, freeze a new full SHA, perform affected automation and reconcile manual/artifact results on that same candidate. Do not combine moving main, old debug binaries and artifacts from unrelated runs.

## Remaining work and required evidence

| Item | Current status | Entry/scope | Evidence required to close |
|---|---|---|---|
| R01 Version and candidate freeze | Pending | Full SHA, version/channel, toolchain, features, targets, runners and Mobile identity | Candidate manifest, source identity, version JSON, run links, artifact SHA-256 |
| R02 CLI-only and Desktop sidecar | Wired; real packages pending | [Linux](../../.github/workflows/build-linux.yaml), [Windows](../../.github/workflows/build-windows-signed.yml), [macOS](../../.github/workflows/build-macos-signed.yml); independent CLI and matched bundled CLI | Payload listing; CLI-only install runs full `future headless`; no shipped/required `futureos-headless` |
| R03 Linux musl and old systems | Scripts retained; candidate payload pending | x86_64/aarch64 musl CLI, separate GNU GUI, deb/portable and existing networkless old Rocky 8 smoke | Actual ELF static/NEEDED results, old-Linux startup and first noninteractive refusal logs |
| R04 Windows signing | Installer regression passed; actual signing pending | Existing Authenticode flow for staged CLI/sidecar, GUI, NSIS/portable and updater payloads | Signature/timestamp/certificate checks for actual files, versions and hashes; unsigned does not replace signing evidence |
| R05 macOS signing/notarization | Recovery regression passed; actual signing pending | arm64/x86_64 app, bundled CLI, DMG and updater archive; Developer ID, entitlements, notarization/stapling | codesign, notarytool status/diagnostics, stapler/Gatekeeper checks and payload hashes |
| R06 Actual binary GUI exclusion | Cargo graph passed; release binary pending | [Artifact checker](../../scripts/ci/check-headless-artifact.py): ELF, normal/delayed PE imports, Mach-O; [packaged entry checks](../../scripts/ci/check-packaged-entrypoints.py) | Actual packaged CLI dependencies, help/version, isolated-home noninteractive refusal and no sensitive state side effects |
| R07 Real install/upgrade/uninstall | Pending real machine/VM | Existing Windows/macOS/Linux channels; upgrade an old release with original data/pairing, rollback and uninstall | Platform/old-new versions/logs; old pairing survives, no retired entry runtime dependency, external Agent preserved and existing data-removal policy retained |
| R08 Same-candidate manual reconciliation | User exploratory testing complete; final identity pending | Desktop, CLI/headless and Mobile; revisit affected [manual cases](headless-manual-acceptance.md) | Platforms, full SHA, actual App versions, pass/fail/unexecuted cases and evidence; no inferred platform passes |
| R09 Matched performance | Tool wired; skipped in this run | CI `headless-measurement` scope and [observer](../../scripts/ci/headless-measurement.py), identical runner/toolchain/target/features/commands/cache policy | Compile/direct-link/real Store execution, raw size/startup samples, Cargo timings; record order/page-cache/sample limitations, no assumed improvement |
| R10 Distribution and channels | Pending | Existing installer download, CLI install/update and Desktop updater, using only verified candidate payloads | Downloads/checksums, architecture/channel match, upgrade path and final confirmation; no new protocol/service |

## Order and physical-device work

Freeze R01, build/check R02/R03/R06, complete R04/R05 with existing signing facilities, then perform version-identified R07/R08 and R09 before final R10 channel checks. When a user machine is needed, provide platform, exact candidate/version, payload/hash, actions and expected results; no signed package is assumed available now.

[Build Test](../../.github/workflows/build.yml) also publishes test/nightly artifacts and invokes Android/iOS flows. Signed nightly, TestFlight and formal [Release](../../.github/workflows/release.yml) have external publication effects. This inventory does not trigger them. Freeze version/publication parameters before execution; ordinary CI success does not complete release work.

Keep the overall Goal incomplete until applicable evidence, same-candidate manual results and user confirmation are complete.
