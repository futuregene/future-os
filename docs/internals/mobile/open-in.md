# Open files with FutureOS

> ([中文](open-in.zh-CN.md)) This is the system **Open In / open with another app**
> entry, not the existing share entry. Native configuration changes require
> re-prebuild and package install; updating JS alone does not add the app to
> the system list.

## Behavior and boundaries

- Android: registers `ACTION_VIEW`, receiving `content://` URIs with read
  grants from the system file provider. Supports PDF, Word (doc/docx),
  PowerPoint (ppt/pptx), Excel (xls/xlsx), text (including Markdown/CSV),
  images (including JPEG/PNG), and files sent as octet-stream. Does not take
  over HTTP/HTTPS, pairing links, or arbitrary `file://` private paths.
- iOS: declares document UTIs; the main app's Expo AppDelegate subscriber
  receives file URLs. Uses an independent local inbox, so no Share Extension,
  App Group, or new signing entitlements are needed. After obtaining
  security-scoped access, reads and copies are coordinated on a serial
  background queue; source files are not edited in place. Temporary copies
  UIKit places in this app's `Documents/Inbox` are cleaned up; external
  originals are not deleted.
- After receiving, the conversation-choice menu is reused: new normal
  conversation, workspace conversation, or an existing session; content
  appends to the attachment draft. While unpaired it is not consumed, and
  **nothing uploads or triggers a model request before the user confirms
  sending**.
- Cold start reads the pending inbox; hot start / already-foreground triggers
  reads through native events, with AppState foreground reads as a supplement.
- The attachment size/count/type validation is reused. The iOS inbox: single
  file 10 MiB, single batch 20 MiB, at most 10 pending batches, 7-day expiry;
  Android reuses `ShareFileCopier` limits and cache cleanup. Over-limit or read
  failures show a hint.
- Whether the app appears in the system list also depends on the MIME/UTI the
  sender provides, read-only grants, and whether the system menu is used. This
  is not a native Office layout editor; files are brought into a FutureOS
  conversation for the assistant to process.

## Automated verification

Run `npm run check` in `mobile/`: includes assertions on the actual Expo plugin
generated config, native inbox events, read races, drafts not auto-sending, and
the existing share regressions.

Run `python3 scripts/tests/test-mobile-ios-share.py` at the repo root: executes the
real inbox copy code with macOS Swift, covering Chinese filenames, file
contents, one-time consumption, over-limit, corrupt input, and path safety.

Android native (needs JDK 17, Android SDK; first run downloads the Robolectric
test runtime):

```sh
cd mobile
npx expo prebuild --platform android --no-install
cd android
./gradlew :future-share-intent:testDebugUnitTest
```

iOS builds the main app with a full Xcode and verifies UIKit callbacks on a
device; Foundation tests, Expo config checks, and Swift syntax checks cannot
substitute for a full iOS compile or real-device tests.

## Package acceptance

Test from the Android file manager, the iOS Files app, and common source apps
respectively:

1. Choose "open with another app" for doc/docx, pdf, ppt/pptx, md, jpeg/png and
   confirm FutureOS is in the list.
2. Receive in three app states — not started, backgrounded, open; Chinese and
   space-containing names preserved, attachments appear exactly once.
3. Pick new-session and existing-session separately, existing drafts kept; on
   cancel nothing uploads, and after sending the desktop reads the matching
   content.
4. Check hints when unpaired-then-pair, over-limit, grant expired, or a cloud
   file is unreadable; the original file is unchanged.
5. The original share, gallery share, multi-file share, and pairing links still
   work.
