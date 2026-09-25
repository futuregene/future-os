# scripts/

Repository automation scripts, organized by purpose. Cross-cutting helpers stay
at the root of this directory because the Makefile and the CI workflows invoke
them from there:

- `version.mjs` — single source of truth for the build version. `node scripts/version.mjs`
  prints it, `--set-bundle` injects it into bundle metadata, and the Rust `build.rs`
  files mirror its resolution logic.
- `npm-install-if-needed.mjs` — checks the workspace install stamp and runs `npm install`
  only when the dependency tree changed.

## build/

Desktop packaging scripts that mirror the CI pipelines.

- `build-desktop-linux.sh` — builds the Linux `.deb` and portable tarball and copies them to `--out-dir`.
- `build-desktop-macos.sh` — builds the macOS DMG, auto-signing when a unique Developer ID
  Application identity is available, with Apple notarization options.
- `build-desktop-windows-installer.ps1` — builds the NSIS installer through `tauri build`,
  optionally Authenticode-signing via `../release/sign-file.ps1`.
- `build-desktop-windows-portable.ps1` — builds the portable Windows zip with the sidecar binaries.

## dev/

Local development sessions that build and start the agent plus one frontend.

- `start-desktop-linux.sh` — Linux Tauri desktop dev session against a locally built agent.
- `start-desktop-macos.sh` — macOS equivalent of the above.
- `start-desktop-windows.bat` — Windows equivalent of the above.
- `start-mobile-android.sh` — Expo/Android dev session (SDK checks, prebuild, Gradle or Metro).
- `start-mobile-ios.sh` — Expo/iOS dev session (CocoaPods, prebuild, Xcode build or Metro).

## release/

Release installers, signing and upload helpers.

- `install.sh` / `install.ps1` — one-line installers served from the download host;
  auto-detect the OS, verify the SHA-256 and install the release.
- `install-future-loop.sh` — installs the `future-loop` CLI into `~/.local/bin` and the
  loop skill into the agent skills directory.
- `sign-file.ps1` — Authenticode signing callback used as the Tauri `signCommand`.
- `setup-ossutil.sh` — installs Aliyun ossutil v2 and derives `OSS_REGION` for CI uploads.
- `lib/windows-signing.ps1` — shared signing helpers dot-sourced by the scripts above.

## measure/

Coverage, profiling and performance measurement tooling.

- `coverage.sh` — workspace test coverage via `cargo llvm-cov`; `--check` enforces the ratchet.
- `coverage_ratchet.py` — aggregates llvm-cov JSON exports into per-crate floors (`emit`)
  and enforces them (`check`).
- `coverage-baseline.json` — per-crate line-coverage floors; edit only through the approved flow.
- `chan-cov.sh` — per-line coverage for the `future-channel` crate plus a missed-lines report.
- `chan-missed.py` — prints uncovered channels lines, grouped by file.
- `show-lines.py` — prints source lines by number with context.
- `agent-profile-bench.sh` / `agent-profile-bench.ps1` — sample the agent under load and
  write a flamegraph SVG.
- `profile-isolated.py` — runs a profiling command under a disposable user home so the live
  agent is never disturbed.
- `profile-quick.ps1` — quick Windows agent profiling entry (used by `make profile-quick`).
- `measure-live-lane.py` — real-traffic lane measurement driven against the desktop publisher.
- `measure-lean-history.py` — measures the lean history page (reasoning bodies, tool output,
  unused call arguments) against real sessions, replayed through the shipping Rust trim.
  `--pages N` measures the actual backward pages of N user exchanges the phone reads,
  newest first, instead of whole sessions.
- `measure-mobile-performance.mjs` — builds an offline browser A/B probe (baseline ref vs working tree).
- `measure-mobile-performance.ts` — probe entry that imports the production mobile sync code.
- `serve-mobile-performance.py` — read-only loopback playback server for the probe.
- `measure-sync-browser.py` — isolated launcher: SQLite snapshot copy, isolated agent, loopback probe.
- `measure-sync-browser.html` — metrics-only browser shell for the sync probe.
- `measure-sync-browser.ts` — browser entry importing production Mobile sync/projection code.
- `measure-sync-snapshot.ts` — A/B entry: legacy raw replay vs snapshot bootstrap.
- `measure-sync-warm.ts` — historical trace playback entry for warm-sync measurements.

## docs/

Scripts that generate or verify the documentation itself.

- `check-docs.py` — structural documentation check: placement, bilingual pairing,
  local links, wiki targets and fences.
- `check-channel-matrix.py` — verifies the provider matrix document against the provider registry.
- `generate_models.py` — fetches the model catalogs and regenerates the built-in model list
  and the wiki model pages.

## tests/

Offline regression tests and native acceptance scripts (not run by CI by default).

- `test-android-config.py` — exercises `../dev/start-mobile-android.sh` host/arch selection with fake tools.
- `test-check-docs.py` — offline regression tests for `../docs/check-docs.py`.
- `test-generate-models.py` — offline regression tests for the catalog generator.
- `test-install.ps1` — mocks the download boundary of `../release/install.ps1`; never installs.
- `test-install.py` — offline tests for `../release/install.sh` version/asset resolution.
- `test-linux-sandbox-real-machine.sh` — Linux bubblewrap sandbox acceptance on a real machine.
- `test-mobile-ios-project.cjs` — verifies the generated Xcode project/entitlement wiring after prebuild.
- `test-mobile-ios-share.py` — runs the real Swift share inbox tests.
- `test-mobile-share-io.py` — Android share staging IO tests without an SDK or emulator.
- `test-profile-isolated.py` — subprocess tests for `../measure/profile-isolated.py`.
- `test-windows-installer-preflight.ps1` — NSIS installer lifecycle acceptance using the fixtures below.
- `test-windows-sandbox.ps1` — Windows native sandbox acceptance (invoked manually, not by CI).
- `test-windows-sandbox-lifecycle.ps1` — sandbox lifecycle acceptance actions
  (Snapshot / ExpectBundled / SeedCleanupFixture / ExpectStopped).
- `test_s2_compaction.py` — local S2-compaction smoke against a built `future` binary.
- `validate-ios-share-profiles.py` — fails before archiving when the host/extension
  provisioning profiles lack shared entitlements.
- `test_ios_share_profiles.py` — offline unit tests for `validate-ios-share-profiles.py`.
- `windows-installer-fixture.cs` / `windows-installer-preflight.nsi` — fixtures consumed by
  `test-windows-installer-preflight.ps1`.

## ci/

CI helper scripts (invoked from the workflow files).

- `check-headless-linux.sh` — verifies the release binary is statically linked, then
  starts it in a clean, networkless Linux image with an old glibc (the portable-bundle baseline).
- `install-apt-packages.sh` — installs CI-only Ubuntu packages with a fast failover from
  the Azure runner mirror to Ubuntu's public archive.

## screenshots/

Headless-Chrome screenshot/video capture tooling for documentation
(see `docs/guide/screenshots.md`).

## skill_reco/

Skill-recommendation benchmark harness (bench scripts, datasets and a local UI).

## compaction_experiment/

Closed-book/open-book compaction experiments (see `docs/internals/compaction/`).
