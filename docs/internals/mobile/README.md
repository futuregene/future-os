# FutureOS Mobile

> ([中文](README.zh-CN.md)) FutureOS Mobile is the native mobile terminal for the
> desktop Remote feature. Android and iOS are both supported; the business
> layer, theming, i18n, and credential storage stay cross-platform. Running the
> APK on a HarmonyOS phone via Zhuoyitong is not a native HarmonyOS client; the
> system-picker and cross-app file-operation boundaries are in the
> [HarmonyOS compatibility audit](harmonyos-compatibility.md).

The install-and-pairing user flow is in the
[phone remote guide](../../wiki/en/Remote.md); this page is for source
development and distribution maintenance.

## Current capabilities

- Pair by scanning the desktop's one-time QR code, or paste the pairing code
  manually.
- One phone can save multiple Desktop pairings and switch between them; each
  Desktop still allows only one paired phone.
- Connects to remote NATS using a device-independent NKey, short-lived JWTs,
  and refresh tokens.
- Shows the desktop's online status and session list; creates or continues
  desktop conversations.
- Streams replies, thinking, and tool execution; supports approval, stop,
  model, thinking level, and rename.
- Sessions support pin, rename, single delete, and multi-select batch delete;
  workspaces can be deleted wholesale (including their sessions; files inside
  the desktop's workspace directory are not deleted), and workspace
  collapse state survives restarts.
- Paginated history loading, deduplication by `(runId, idx)`, and real-time
  event backfill via `get_events_since` after reconnection.
- Pick images from the system photo library or shoot with the system camera;
  add file attachments through the system file picker; download session
  attachments and preview images, Markdown, text, and JSON in the app — other
  supported types go to the system app to open, save, or share. The preview
  page can also share the original file or open it in another app; a missing
  reader does not block saving and sharing.
- The folder button atop a session shows the session's directory on the
  desktop; enter subdirectories, go back up, refresh the list, or toggle hidden
  files; tapping a file reuses the phone preview/system-open flow.
- Android supports **sharing** text/images/files from other apps; iOS adds a
  native Share Extension that, after saving, opens FutureOS to import
  text/links/images/files. You can create a normal/workspace session or pick an
  existing one; content appends to the target session's draft, preserving
  existing text and attachments — nothing uploads or auto-sends before
  confirmation. iOS save/external-open also integrate the native document
  picker/Open In respectively; the legacy native bundle keeps the share-sheet
  fallback. The extension needs an Apple App Group and a separate signing
  profile, and must be explicitly enabled with `FUTURE_IOS_SHARE_EXTENSION=1`;
  the existing release workflow is unchanged and does not package the extension
  by default. See [iOS capabilities and signing config](ios-platform-parity.md).
- seed, JWT, and refresh token are stored separately in the Android
  Keystore-backed SecureStore; plaintext never enters AsyncStorage, logs, or QR
  codes.

## UI language

- Default "follow system": the UI uses Chinese when the system's preferred
  language is Chinese (including simplified and traditional regions), English
  otherwise.
- "Settings → Language" in the session list offers "Follow system / 中文 /
  English"; it can also be chosen at the bottom of the pairing page before
  pairing.
- A manual choice takes effect immediately and is saved locally; restarts,
  desktop switches, or going offline do not change it.
- Choosing "follow system" immediately re-reads the system language; changing
  the language in system settings syncs automatically when returning to the
  app.

## Chat hints and update reminders

- The input area keeps two rows: one for text input, one shared by the model,
  thinking mode, `/` skills, attachments, and the send/stop buttons. Below
  360pt or with large font sizes, model and thinking merge into a "model
  settings" entry; action buttons keep a 44pt touch target.
- Typing `/` at the start of a message or after whitespace, or tapping the `/`
  left of attachments, expands the skill candidates above the input. Keywords
  filter by skill name, description, and available Chinese metadata; selecting
  inserts only `/技能名 ` without auto-sending. Closing the candidates keeps
  the draft; URLs and paths with separators do not trigger completion. While
  the candidates are open, the current Agent's installed-skill catalog is
  refreshed (remote read-only `list_skills`, handshake capability `skills_v1`);
  older desktops show an upgrade hint, with manual full-command input still
  working.
- The chat top shows "Workspace · Name" or "Non-workspace session", including
  for newly created sessions before any message is sent.
- System notification permission is requested after pairing connects.
  Notifications fire when the current desktop's task completes or fails —
  including the currently open session; manual stop does not send a completion
  notification. System notifications contain no message body; when
  unauthorized, the foreground uses in-app hints. Real-time events and session
  snapshots jointly detect the end state, avoiding missed or duplicated
  snapshot reminders for short tasks.
- These are **local notifications, not offline push**. iOS builds declare no
  APNs push capability and Android builds declare no FCM receive permission;
  ordinary backgrounding keeps at most a 5-second fast-return grace period,
  photo/camera/file pickers at most 60 seconds; the remote connection is
  released afterwards. The OS may also pause JavaScript earlier. A running
  task that was observed only reminds after returning to the foreground and
  syncing to the completed state. Real-time notifications are not guaranteed
  after the system kills the app, and restarting does not pop all historical
  completion records. Notification permission and Do-Not-Disturb are managed
  by the phone OS.
- Updates are checked on launch, return to foreground, and during sustained
  use (successful checks 6 hours apart, retried after network failure). The
  same version prompts only once per launch; confirming continues with the
  Android package / iOS App Store update flow. Local dev packages only open the
  download link; TestFlight updates remain managed by TestFlight.
- The update dialog shows current and available versions on separate lines,
  keeps the build number or commit hash, and marks test, nightly, dev, or local
  builds. Dev/local builds do not claim a nightly build as a newer version;
  buttons distinguish browser download, Android download-and-install, and Go to
  App Store.

A new native dependency `expo-notifications` was added; existing dev clients
need a rebuild and reinstall — refreshing JS alone is not enough.

Reliable reminders after long background, lock screen, or process termination
still need platform-side push delivery, device token registration, and
APNs/Android push configuration; that chain does not exist today. Notification
permission granted, or holding WSS for a short time, must not be read as having
offline push.

### UI and connection regression checks

- Delete session, batch delete, delete workspace, and unpair desktop uniformly
  use `DialogSurface` and common buttons; session rename uses the same form on
  iOS/Android, and download progress reuses the same dialog surface.
- Android prompt-dialog bodies disable native text selection to avoid OEM
  selection-control gray backgrounds; iOS keeps text selection. Both ends can
  still scroll to read the full prompt. Real devices must cover download/update
  dialogs, long text, large font sizes, and landscape.
- Files the user actively opens or downloads under 1 MiB no longer prompt a
  cellular-data confirmation; at 1 MiB, cellular and unknown network types
  prompt separately, with detection failure treated as unknown. Wi-Fi/Ethernet
  and cache hits do not prompt. Attachments, file links, original-file
  save/share, and Markdown images share the same rule with no automatic
  prefetch.
- Attachment sources and file open/save/share menus reuse `ActionMenu`; after
  choosing a source, wait for the menu to close before invoking the system
  camera, photo, or file picker. The Android gallery uses `PickVisualMedia`
  (system-chosen compatibility fallback), no longer preferring `ACTION_PICK`
  into arbitrary third-party galleries. System permission dialogs and system
  pickers are still drawn by the OS.
- Returning to the list from a session keeps the list instance and scroll
  position; the list's reverse enter animation is not replayed. In the
  workspace and conversation lists, independent sessions keep compact spacing
  without widening when other sessions appear or child sessions expand; only
  the parent session shows an expand button, and child sessions keep hierarchy
  indentation plus expand-button placeholder space.
- The input area stays two rows (text + toolbar); narrow screens merge model
  settings without shrinking action-button touch targets.
- Skill regressions: typed and button-inserted `/`, Chinese search, mid-body
  replacement, close/back key, load-failure retry, late skill results when
  switching desktops, and narrow/large-font layouts after keyboard popup.
  Automation covers interactions and boundaries; native cursor and soft
  keyboard behavior on iOS/Android still needs real-device verification.
- The iOS `inactive → active` temporary overlay does not trigger a full
  restore; a real background return still checks the channel. Network path
  changes first probe the existing connection. When the authenticated heartbeat
  disappears, restore the encrypted channel — the relay service's PONG alone
  must not be read as the Desktop being healthy. When a candidate connection
  fails, the operational state is restored only after the old channel passes an
  encrypted two-way RPC verification.

Real-device re-tests: enter/return repeatedly after scrolling the list; expand
child sessions in the workspace and conversation lists, checking unchanged
spacing for independent sessions in the same and other groups, covering search
and multi-select states; open/cancel all three attachment sources; quickly
background; switch Wi-Fi/cellular; wait for automatic recovery after Desktop
restart. Specifically check "new session received but badge still yellow": the
badge and actions must recover together after the normal encrypted command
channel is restored; one-way event reception with unanswerable commands must
not be misreported as green. Automation cannot replace real-device picker and
lock-screen tests.

### Latest-content sync and streaming presentation

The paging-latency baseline, optimization results, and real-device phased logs
for entering streaming sessions are in
[sync performance measurements](streaming-sync-performance.md). The
"semantic snapshot + incremental" protocol for cold start/cache invalidation,
compatibility boundaries, and real-data A/B results are in
[snapshot recovery optimization](streaming-sync-snapshot-optimization.md).
History still loads only the latest three user exchanges, with earlier content
loaded on scroll-up; reopening with a full cache still backfills only the
increments.

- Reopening a session shows "Syncing latest content…" even with cached
  messages, until status, history, and event backfill complete; failure retry
  or waiting for reconnection keeps the corresponding hint. This state is
  separate from "model generating".
- The hint floats above the timeline, is not inserted as a list row, and does
  not change scroll position when appearing/disappearing.
- Streaming text reveals only new suffixes in batches, about a 192ms window per
  reveal, new Markdown blocks fading in over ~180ms; cached history shown the
  first time is not retyped, and already-shown blocks do not replay animations
  per token. Copy and file operations still use the raw data. When the system
  reduce-motion setting is on, no progressive reveal or fade-in.
- The original reading anchor is kept: viewing old messages does not force
  scrolling to the bottom. Complex tables, image loading, and foldable-size
  changes still need real-device checks — automated logic tests are not
  animation-smoothness tests.

## Environment

- Node.js 24 or later (consistent with the repo-root `.nvmrc` and CI).
- Android Studio, JDK 17, and Android SDK / Build Tools 36.
- An Android device with developer mode and USB debugging enabled, or an
  Android emulator.

The project version comes from the repo-root `scripts/version.mjs`, the same
version source of truth as the desktop. The pairing control-plane address is
decided by the `claim_url` in the desktop QR code, not by the APK's dev/release
variant. One APK can pair desktops at `https://future-os.cn` and
`https://test.future-os.cn`; renewal and revocation addresses are saved with
each desktop's credentials, so environments are not mixed when switching
desktops. Existing pairings are unaffected; the app update channel is still
decided by the version.

QR codes allow only these two trusted HTTPS domains and the exact pairing claim
path; any third-party domain, non-HTTPS address, non-default port, URL user
info, or extra query/fragment is rejected (PA003). The QR code only selects a
trusted service; arbitrary address access is not allowed.

NATS always connects over `wss://` (TLS): neither test nor production
environments issue plaintext `ws://` addresses, and the client refuses pairing
when receiving a non-`wss://` address. Android no longer allows cleartext
traffic.

## Manual Android run

```bash
cd mobile
npm install
npm run android:device
```

Or from the repo root:

```bash
make run-mobile-android
```

Expo generates a gitignored local `mobile/android/`, then compiles and installs
the debug APK.

After first launch:

1. Sign in on the FutureOS desktop, open Remote, and click "Pair and start".
2. Grant the phone camera permission and scan the desktop QR code.
3. After pairing, pick a desktop session or create a new conversation.

The QR code is valid for 5 minutes and usable only once. After unpairing a
device you must scan again.

### Browsing session files

Enter an existing session and tap the folder button at the top to open
"Session files". The panel shows the session's directory on the desktop (not a
phone-local directory); tap a folder to enter, navigate with "Back up" or
"Session root". Manual refresh and a hidden-file toggle are supported, with
workspace sessions showing hidden files by default. The Android system back
key/edge gesture goes up one directory level at a time and closes the file
panel only at the root. The top back button always returns straight to the
chat; closing a file preview keeps the current directory.

This feature needs both phone and desktop updated and the desktop online; a new
session without a first message has no file entry. Images, Markdown, text, and
JSON preview in-app; other supported types use the system open/save/share flow
with the existing 10 MiB download cap and file-type whitelist. Unsupported file
types show a hint. Directory browsing is limited to the current session root
and its subdirectories; protected FutureOS credential files and symlinks
pointing outside the root are not listed. Tapping a file again re-verifies the
content identity to avoid showing a stale cache.

### Adding and switching multiple desktops

Tap the desktop badge at the top of the session list (or "Paired desktops" in
Settings), choose "Scan to add desktop", and scan another Desktop's QR code.
Existing pairings are not overwritten; afterwards tap a desktop in the list to
switch — no re-scanning needed. The last selected Desktop is restored on app
launch; only the selected Desktop is operated at a time, without aggregating
other desktops' live sessions.

Each desktop in the list has a pencil button to rename it; the name is saved
locally only and never synced to the Desktop or platform; clearing the name
shows the Desktop's id again. The list is centered on wide screens with left
and right margins kept.

Unpairing removes only the selected Desktop and does not affect other pairings.
The Desktop-side one-phone limit is unchanged: switching phones still requires
regenerating a QR pairing, and the old phone's binding is revoked per the
existing server rules. Legacy single-Desktop credentials migrate to the new
list automatically, without re-pairing.

Credentials are stored per Desktop in SecureStore, with a single index
committing the full credential set and current selection. Switching clears the
session directory and timeline; drafts are per-Desktop, and pending
send/resume confirmations are isolated per pairing. Legacy pending operations
migrate only to their original pairing and are never sent to a new Desktop;
legacy non-isolated ordinary drafts do not auto-migrate.

## Quality control

```bash
npm run typecheck
npm run lint
npm test
npm run check
```

The repo root provides matching entry points:

```bash
make lint-mobile
make test-mobile
make check-mobile
```

## iOS development

`app.config.ts` configures the bundle identifier (`cn.futureos.mobile`),
minimum system version (iOS 16.4), camera permission, and SecureStore/Keychain;
the React Native business layer does not depend on Android-only APIs — Android
and iOS share one pairing, session, and chat implementation.

Install Xcode and the matching iOS simulator runtime before the first iOS
development. `mobile/ios/` is generated by `expo prebuild`; it is a local build
artifact, gitignored, not committed.

### Environment

- macOS + Xcode (with iOS SDK and simulator runtime).
- A simulator or device on iOS 16.4 or later.

### Manual iOS run (simulator)

```bash
cd mobile
npm install
npm run ios
```

Or from the repo root:

```bash
make run-mobile-ios
```

Or use the one-shot launch script (creates/starts a simulator, installs deps,
prebuilds, and runs automatically):

```bash
scripts/start-mobile-ios.sh          # dev mode (Metro + debug build)
scripts/start-mobile-ios.sh release  # release mode (standalone, no Metro)
```

### Manual iOS run (device)

Connect the iPhone to the Mac via USB, select a development team in Xcode,
then:

```bash
cd mobile
npm run ios:device
```

A free Apple ID suffices for on-device debugging; App Store submission needs a
paid Apple Developer account.

### GitHub Action TestFlight distribution

The main distribution path is the reusable workflow
`.github/workflows/build-ios-testflight.yml`, coordinated by `build.yml` /
`release.yml`, which builds the signed `.ipa` and uploads it to TestFlight:

1. Configure in GitHub repo Settings → Secrets:
   - `IOS_DIST_CERT_P12_BASE64` / `IOS_DIST_CERT_P12_PWD` — iOS Distribution
     certificate.
   - `IOS_PROVISIONING_PROFILE_BASE64` — App Store provisioning profile for the
     main app. The Share Extension is not in the existing release by default;
     enabling it needs a separate profile and follow-up signing workflow
     configuration — see [signing config](ios-platform-parity.md); the workflow
     is not modified in this pass.
   - `APPLE_API_KEY` / `APPLE_API_KEY_ID` / `APPLE_API_ISSUER` — App Store
     Connect API Key.
2. Triggered through the build/release coordination workflow; the iOS
   sub-workflow is not triggered manually on its own.
3. The coordination workflow passes a numeric-only marketing version and an
   increasing build number.
4. After upload, manage testers in App Store Connect → TestFlight; external
   testing may need Beta review.
5. Testers install FutureOS via TestFlight invitation.

> Local test packages come from `scripts/version.mjs` and may carry a dev
> suffix; TestFlight versions must be numeric-only, with the build number
> strictly increasing within one marketing version.

### iOS platform notes

- Bundle identifier is uniformly `cn.futureos.mobile`, shared by Android and
  iOS.
- NATS always uses `wss://`, with no plaintext/cleartext exceptions configured;
  iOS relies on the system ATS default to allow TLS WebSocket, and Android
  keeps cleartext forbidden.
