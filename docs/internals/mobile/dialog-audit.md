# Mobile dialog audit

> ([中文](dialog-audit.zh-CN.md))

## In-app dialogs

| Scenario | Implementation and handling |
| --- | --- |
| File download, export, share, open-externally failure | `AppAlert` → `AppDialogHost` → `useAppDialog` → `DialogSurface`; keeps the existing timing of showing after download/preview exit. Android low-level error details are selectable for copying. |
| Session rename failure, file list load failure, Markdown/message link open failure | Same `AppAlert` channel, no longer calling the native `Alert`. |
| Cellular download confirmation | Same channel; confirm/cancel/system-back are all handled after the dialog closes, avoiding the download-progress dialog and confirmation dialog contending. |
| Upgrade confirmation, install failure, foreground-notification fallback, iOS toast substitutes | Same channel, allowing non-React callers to submit; multiple requests queue in order, preventing an error from covering an unhandled confirmation. |
| Delete, unpair, update checks and error hints in desktop and session management | Already use a local `useAppDialog`, keeping page-active control; reusing the same `DialogSurface` and button styles. |
| Settings, new session, rename, manual pairing, download progress | Already use `DialogSurface`; no need to switch to another UI set. |
| File operations, attachments, model/thinking-level pickers, share-import menus | Keep the app-styled bottom sheets; same color/spacing/radius tokens, but not forced into a centered error dialog. |
| Connection status details | Keep the in-app popover anchored to the connection status dot; scrollable content with safe-area limits. Not an error confirmation box. |
| File preview | Keep the full-page/pageSheet reading UI; not a prompt box. |

`DialogSurface` unifies the mask, max width, radius, safe area, keyboard
avoidance, and long-content scrolling. Error details use selectable text so a
long native error does not push the close button out. ESLint forbids business
code importing `Alert` directly from `react-native`, preventing a second style
of error dialog from creeping back in.

## System UI boundaries

The system share sheet, other-app open list, camera/gallery/file pickers,
permission requests, and Android short Toasts are still provided by the system
and must not be faked for style uniformity. How the HarmonyOS compatibility
environment renders these panels still needs real-device verification.

## Verification

Automation covers: button actions executed only after iOS/Android closing
completes, cancelable and non-cancelable, duplicate close events, concurrent
error queueing, a confirmation action producing another error, and long error
details using selectable text with the shared dialog container. Existing
`DialogSurface` tests cover safe areas, scrolling, and keyboard handling; no
claim of completed visual acceptance without real-device screenshots.

Real-device acceptance:

1. Share the same PDF three times in a row; share again after cancelling the
   share sheet; share again after returning from the receiving app.
2. Confirm the receiving app gets the real PDF with the original name and
   content; save and open-externally still work.
3. Trigger file/link errors and check the same fonts, buttons, radius, and mask
   as settings/rename.
4. With large fonts, landscape/fold-open, long errors scroll, the close button
   is reachable, and system back closes.
5. The cellular confirmation cancels on the back key; after upgrade
   confirmation, install failure shows both dialogs in sequence.
