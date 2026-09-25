# iOS platform capabilities and Share Extension

> ([中文](ios-platform-parity.zh-CN.md))

## Added this round

- **Other App → FutureOS**: the native `ShareViewController` supports text, web
  links, images, and files. The user picks FutureOS in the system share sheet,
  adds a note and saves; opening FutureOS afterwards, after pairing, creates a
  normal/workspace session or selects an existing one, content appends to the
  target draft, and nothing uploads before send confirmation. The extension
  does not touch remote credentials, does not go online, and does not use
  responder-chain / `UIApplication` hacks to force-open the main app.
- **Save to Files**: `UIDocumentPickerViewController(forExporting:asCopy:)`, the
  user chooses the location; saving no longer borrows the share sheet.
- **Open with another app**: `UIDocumentInteractionController`'s Open In menu,
  with iPad anchors; a missing reader only affects opening, not save and share.
- **Share out** keeps using `UIActivityViewController`. When the legacy iOS
  native bundle lacks the new file module, open and save keep the original
  share-sheet fallback; updating JS cannot conjure the Share Extension — a
  rebuild is required.
- Removed the `future-native-ui` Android module, npm dependency, and invalid
  mocks that no longer have production callers.

## Inbox boundaries

`future-share-intent/ios/ShareInbox.swift` compiles into both the extension and
the main app's Pod. The extension only writes independent temporary batches,
publishing atomically after all copies complete; the main app only consumes
committed batches.

- Single file ≤10 MiB, total files ≤20 MiB, at most 10 attachments, text
  ≤256 KiB; finer limits like image counts reuse the main app's attachment
  validation. Over-limit files keep no half-copies; the remaining usable
  content is kept with a hint.
- Files are copied chunk by chunk; whole original images are not decoded into
  extension memory; display names are kept, physical paths use random names.
- File URLs are copied within the provider callback's validity period;
  security-scoped grants are released once used.
- At most 10 pending/in-flight batches in the App Group, a cross-process lock
  protects capacity and consumption; pending content expires in 7 days,
  interrupted staging batches are reclaimed after 1 day, and caches the main
  app took but never used are reclaimed after 7 days.
- One batch is read per foregrounding; multiple shares queue; returning to the
  foreground continues reading. Switching desktops does not auto-send to
  another desktop. File bodies and credentials never enter share links or logs.
- The manifest parser validates paths, file sizes, and symlinks; a corrupt
  batch does not block later batches, and a cache-write failure does not
  consume the input.

## Apple configuration and signing

The plugin `plugins/withIosShareExtension.js`, when
`FUTURE_IOS_SHARE_EXTENSION=1` is set, generates and embeds the extension during
Expo prebuild, avoiding committing `mobile/ios/`. The extension version syncs
with the main app, supports iPhone / iPad, minimum iOS 16.4. The extension is
off by default to keep the existing host-only signing flow unchanged.

Configure under the same Apple Developer team:

| Item | Identifier |
| --- | --- |
| Main App ID | `cn.futureos.mobile` |
| Share Extension ID | `cn.futureos.mobile.share` |
| App Group shared by both App IDs | `group.cn.futureos.mobile.share` |

1. Register the extension App ID and the App Group; enable and associate this
   App Group for both App IDs.
2. Regenerate the main app's profile and create a separate profile for the
   extension.
3. Locally run `FUTURE_IOS_SHARE_EXTENSION=1 npx expo prebuild --platform ios`
   in `mobile/`, select the same team for both targets in Xcode, and pick the
   matching development/distribution profiles.
4. For manual signing, also set `FUTURE_IOS_MANUAL_SIGNING=1` during prebuild;
   the plugin references `$(FUTURE_APP_PROFILE)` and `$(FUTURE_SHARE_PROFILE)`
   for the two targets respectively. When invoking xcodebuild, pass these two
   custom settings — do not pass one global `PROVISIONING_PROFILE_SPECIFIER`
   that overrides every target. The IPA export's `provisioningProfiles`
   dictionary must contain profiles for both bundle IDs.
5. You can first decode both profiles with `security cms -D -i <profile>`, then
   run `python3 scripts/tests/validate-ios-share-profiles.py <host.plist>
   <share.plist>` to check App IDs, team, validity, distribution type, and App
   Group.

**The CI/release workflow is not modified in this pass.** The existing workflow
has no opt-in set, keeps producing builds without the extension/App Group, and
file save/open still work. Adding a secret alone does not enable the
extension; distributing it later needs separate signing workflow
configuration. This is Apple-backend configuration that merging code alone
cannot complete — "code supports it" must not be presented as "live on
TestFlight".

## Verification

- `npm run check` (in `mobile/`): types, lint, Jest; covering standalone file
  operations, legacy-bundle fallback, presenting UIKit only after the download
  dialog really closes, and cancel/failure paths.
- `python3 scripts/tests/test-mobile-ios-share.py`: the real Swift inbox's atomic
  consumption, failure retry, byte limits, Unicode, corrupt manifests, path
  escape, directory/symlink rejection, and queue capacity.
- After explicitly enabling the extension and prebuilding, run
  `node scripts/tests/test-mobile-ios-project.cjs`: verifies the generated project's
  sources, embedding relationships, entitlements, repeated runs, version sync,
  and per-target signing settings.
- On macOS with Xcode/iOS SDK, after installing Pods in `mobile/ios/`:
  `xcodebuild -workspace FutureOS.xcworkspace -scheme FutureOS -configuration
  Debug -sdk iphonesimulator -destination 'generic/platform=iOS Simulator'
  CODE_SIGNING_ALLOWED=NO build`. This compiles the App, both native modules,
  and the enabled extension without Apple keys. This machine only has Command
  Line Tools, so the full UIKit/App compilation was not executed; existing CI
  does not include this native compile check.

Real-device acceptance still needed: Safari links, selected text, single/multi
photos, PDF/Chinese filenames from the Files app; save then pair when unpaired;
consecutive shares, cancels, over-limit, same-name files, cold
start/foreground-background; iCloud file downloads; iPhone/iPad save-location
selection, cancel, with/without external reader, system share, and remote
connection recovery.

## Kept system differences

iOS updates go through App Store / TestFlight, not APK downloads. Android
Activity-recreation recovery, notification channels, and the system back key
are not iOS feature gaps. Task notifications are local-only on both ends;
long-background/offline push is unimplemented on both ends.
