# Mobile issue audit — 2026-09-16

Audited against `origin/main` through `fac02c09` (including #663–#666).
The supplied report has six items; it repeats number 4 and jumps from 5 to 11.
A screenshot alone does not establish the installed app's version.

| Report item | Finding and action | Regression evidence |
| --- | --- | --- |
| 2: Cannot view/download Word and Excel | Current code already routes `.doc`, `.docx`, `.xls`, `.xlsx` to independent open/save/share actions. Opening requires an installed reader; saving/sharing does not. Added coverage for all four formats and Windows paths with Chinese filenames. Office content is not rendered inside the chat. | `useFileDownload.test.ts`, `fileTypes.test.ts`; existing native file-action tests |
| 3: Show/hide button appears ineffective | Current code toggles dotfiles correctly. The video's final directory shows ordinary `.py` and `.xlsx` files; toggling hidden files should not remove these. Added both toggle directions and a directory containing only ordinary generated files. No production change was needed. | `SessionFilesPanel.test.ts`; backend `remote_host::session_files` tests include `.hidden` and protected-path checks |
| 4a: Desktop-hidden models still appear on phone | Confirmed missing filtering. The remote catalogue now honors the desktop's provider-qualified hidden IDs. Settings writes notify connected phones after commit. An explicit `allModelsHidden` response clears stale models without mistaking an intentionally empty catalogue for Agent warm-up. | Backend `model_catalog_respects_desktop_visibility`; mobile catalogue, global-event and selector tests |
| 4b: Desktop model change leaves old phone label until reopening | Confirmed the phone only applied model/thinking settings on explicit conversation open. Live settings events now update the selected conversation without writing commands back. Reconciliation restores changes missed while disconnected. Stale reads cannot overwrite a newer event or another session. | Conversation-controller, timeline-controller and provider-wiring tests |
| 5: Desktop formulas display correctly, mobile differs | Confirmed raw-TeX fallback. Added offline MathJax 4 + the self-contained TeX SVG font, rendered with the existing native SVG component. Inline and display math are supported; display math scrolls horizontally. Basic and AMS notation includes roots, fractions, powers and matrices. Unsupported, incomplete or oversized formulas retain readable source. No WebView, CDN or remote rendering. | Real SVG-layout and Markdown-component tests; Android/iOS Hermes exports; visual inspection of actual generated SVG in a browser |
| 11: Cannot continue typing while replying on Android | Already fixed by #663. Composer remains editable while streaming, retains the next draft through updates/completion, and does not send it automatically. | Existing `ComposerDock.test.ts` streaming-draft and stop-request tests |

## Validation and limits

- Mobile: TypeScript, ESLint and the complete Jest suite.
- Desktop backend: Rust 1.97.0 formatting, all-target Clippy with warnings denied, and complete tests for both GUI and headless configurations.
- Android and iOS production JS/Hermes bundles exported successfully, including MathJax's native environment and Metro font-alias compatibility.
- Actual SVG outputs for roots, fractions, powers and a matrix were rendered and visually inspected without missing glyphs or clipping. This is **not** a native-device screenshot test.
- No original test phone, installed APK/IPA version or Office reader configuration was supplied. Physical Android/iOS open/save handoff and inline-math baseline/layout still need device acceptance; unit mocks and bundle success do not establish those OS behaviors.

## Device acceptance checklist

1. Open generated Word/Excel files from session files and chat links. Save/share them with no reader installed; open them with a compatible reader installed. Verify cancellation and Chinese filenames.
2. Use a directory with both `.hidden` and normal files. Toggle twice; only dotfiles should change visibility.
3. Hide one desktop model, then all, then restore one while the phone is connected. Confirm the selector follows the desktop. Repeat after reconnecting.
4. Change the active conversation's model/thinking level from desktop. Confirm the phone updates without reopening; changes in another conversation must not affect it.
5. Check inline roots and display fractions/matrices on Android/iOS, including larger accessibility font sizes and narrow screens. Unsupported TeX must remain visible as source.
6. During a streamed reply, type a second draft. Confirm it survives completion and is not sent until explicitly submitted.
