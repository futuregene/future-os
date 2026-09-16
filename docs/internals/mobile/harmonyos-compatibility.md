# HarmonyOS phones: APK compatibility and native capability boundaries

> ([中文](harmonyos-compatibility.zh-CN.md)) Audit date: 2026-09-14. Problem
> environment: Mate XT, HarmonyOS 6.1.0, APK running through Zhuoyitong. This
> page separates source-code checks, official documentation, and pending
> real-device verification items; Android system UIs are not treated as
> HarmonyOS native UIs.

## Conclusion

The current `mobile/` is an Expo / React Native Android+iOS client with no
HarmonyOS native build target. An APK in Zhuoyitong uses Android APIs; the
actual pickers and accessible apps/files are decided by the compatibility
environment and its bridging capabilities. The APK's TypeScript cannot directly
import HarmonyOS `@kit.*`, and changing styles alone cannot claim the HarmonyOS
native camera, gallery, or file picker is being used.

This pass found no verifiable public bridging SDK in the accessible Zhuoyitong
official materials that would let a third-party APK call the HarmonyOS
interfaces in the table below directly. The official homepage scrape was mostly
images, and the open platform returned a login page; this does not prove no
partnership interface exists, only that it could not be verified this time. No
guessed package names, private Intents, or URI schemes were used.

To guarantee these five items are natively provided by HarmonyOS requires a new
HarmonyOS native build target integrating the corresponding Kit, or
implementing and verifying through an officially supported Zhuoyitong bridging
solution first. This APK change is not a HarmonyOS native port.

## Official native interfaces

| Feature | Integration | Documented boundary |
| --- | --- | --- |
| Photo | Camera Kit `cameraPicker.pick` | System provides the capture and confirmation UI; this path needs no camera permission. |
| Gallery | Media Library Kit `photoAccessHelper.PhotoViewPicker.select` | Opens the system gallery, user selects; the interface needs no whole-library permission, and the returned URI is read-only authorization. |
| Phone files | Core File Kit `picker.DocumentViewPicker.select` | Opens the system file picker, needs a UIAbility context; file reads are authorized per URI, no physical-path guessing. |
| Share file | Share Kit `systemShare` | Builds share data from file URI and accurate UTD type, opens the system share panel; the target app must support receiving the type. |
| Open in other app | Ability Kit implicit Want / `startAbility` | Carries URI, MIME type, and read authorization, matching registered handler apps; with a single match it may open directly. |

A system share does not guarantee WeChat appears. The target WeChat version,
file type, and system version must be verified; in particular, "Android SEND
was raised inside Zhuoyitong" must not be treated as "HarmonyOS WeChat received
the file".

Official sources (relevant Huawei text read; Zhuoyitong access limits noted
above):

1. [Capture photos and videos with the system camera](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/camera-picker)
2. [Select media library resources with Picker](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/photoaccesshelper-photoviewpicker)
3. [File picker](https://developer.huawei.com/consumer/cn/doc/harmonyos-references/js-apis-file-picker)
4. [systemShare](https://developer.huawei.com/consumer/cn/doc/harmonyos-references/share-system-share)
5. [Open files with another installed app](https://developer.huawei.com/consumer/cn/doc/harmonyos-faqs/faqs-ability-54)
6. [Zhuoyitong site](https://www.droitong.com/)
7. [Zhuoyitong open platform](https://developer.droiapps.com/)

## Current APK source audit and changes

- Photo: `expo-image-picker`'s `CameraContract` uses the Android camera Intent.
- Gallery: `ImageLibraryContract` uses AndroidX `PickVisualMedia` /
  `PickMultipleVisualMedia` with `legacy: false`; no custom gallery UI, no
  whole-library read permission requested.
- Phone files: `expo-file-system`'s `FilePickerContract` uses
  `ACTION_OPEN_DOCUMENT`.
- `NativeFileActionSheet` explicitly separates "open with another app / save /
  share".
- Android open-external keeps `future-file-handler`'s `ACTION_VIEW`,
  FileProvider, and read-only grants; sharing also sends the real file via
  `future-file-handler`'s `ACTION_SEND`, `EXTRA_STREAM`, `ClipData`, and
  FileProvider — not the cache path as text. Save keeps the Storage Access
  Framework. Share returns after the system accepts the chooser, without
  waiting for the receiving app's result and without keeping a global pending
  promise — otherwise, when the compatibility environment never returns an
  activity result, `expo-sharing` would permanently refuse later shares; the
  return only means the system took over, not that the receiving app sent
  successfully. iOS keeps using `expo-sharing`.
- The reader check is no longer performed when entering the file menu. The VIEW
  capability is checked only when open-external is chosen; a missing PDF reader
  does not block save or share. The preview page also has share and
  open-external entries and fetches the original file, not the preview's
  truncated text.
- iOS save uses `UIDocumentPickerViewController`, open-external uses
  `UIDocumentInteractionController`, share still uses
  `UIActivityViewController`; the legacy native bundle falls back to the share
  sheet and claims no new built-in document reader.
- The file-type whitelist and 10 MiB cap are kept; a cancelled transfer does
  not execute a late file share.

## Real-device acceptance (not executed)

1. Record the Zhuoyitong version; check the camera, media, and file permissions
   granted to Zhuoyitong on the HarmonyOS side, and the APK's internal
   permissions.
2. Open, complete, and cancel the camera, gallery, and file pickers
   separately; record whether the actual page belongs to the HarmonyOS system
   or the compatibility environment. Being able to select a file only proves
   data access works, not that the UI is HarmonyOS native.
3. With no matching reader for a PDF, the file menu still offers save and
   share; open-external hints that the current environment has no handler app.
4. Share a PDF with a Chinese filename, an image, and text; confirm the target
   app receives the real file with correct name and MIME. Separately check
   whether apps inside Zhuoyitong and native HarmonyOS WeChat appear and can
   read it; record the compatibility limit when absent.
5. After installing an accessible reader, verify "open with another app"; no
   promise of enumerating HarmonyOS apps outside Zhuoyitong.
6. Enter a file directory two levels deep, use the system back gesture to
   return level by level; at the root, back returns to the session. The
   top-left button returns to the session directly. After closing a preview or
   system panel, the original directory is unchanged.
7. Reopen a streaming session with existing content, reconnect on weak network:
   the sync/retry hint shows until sync completes, existing messages are not
   cleared.
8. During continuous increments, batch backfill, and code-block/table updates,
   check batch reveal and new-block fade-in; reading old messages does not
   auto-scroll to the bottom, and with the system "reduce motion" enabled
   content shows immediately. Check Mate XT fold/unfold, large font sizes, and
   low frame rates.

Automated tests cover menu routing, original-file sharing, cancel/limits,
return levels, sync states, progressive reveal, and reduce-motion logic; they
cannot replace real-device tests of cross-app authorization, native UIs, and
animation feel in the compatibility environment.
