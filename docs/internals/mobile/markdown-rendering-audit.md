# Mobile Markdown rendering audit

> ([中文](markdown-rendering-audit.zh-CN.md)) Audit date: 2026-09-14. Covers
> assistant messages (including streaming fragments) and Markdown attachment
> previews; not an Android/iOS real-device acceptance report.

## Rendering chain and boundaries

- `TimelineCard` assistant text fragments and fragmentless assistant messages →
  `MarkdownText`.
- `PreviewModal`'s Markdown files → the same `MarkdownText`, with the
  `file-preview` mode enabled.
- `@future-os/markdown` uses Remark + GFM + math-syntax plugins, converting to
  shared nodes; mobile renders with React Native `Text` / `View` / `Image` —
  **not a browser HTML renderer**.
- User messages are intentionally plain text (only file references and external
  links parsed); thinking processes and tool outputs are also not full Markdown
  rendering entry points. Do not misreport these product boundaries as
  assistant-Markdown failures.

## Confirmed and fixed issues

| Item | Cause / fix |
| --- | --- |
| H1–H6 hierarchy lost | The shared parser flattened H4–H6 into H3; mobile then merged H1/H2. All six levels keep their semantics; mobile distinguishes font sizes and marks accessibility headings, and desktop uses the correct heading tags in sync. |
| Incorrect ordered-list numbers | The start number never reached the shared node; mobile always started at 1. `start` is kept (including 0 and nested lists), both ends consume the field. The mobile marker no longer forces a 22-point fixed width, avoiding squeezing 3+ digit numbers. |
| Reference-style links wrong | Only top-level definitions were collected, and later definitions overwrote earlier ones. Changed to traverse nested definitions in source order and use the first definition. |
| Table newlines ineffective | `<br>` was treated as ordinary HTML text. Only attribute-less `<br>` / `<br/>` / `<br />` (case-insensitive) convert to newlines; other HTML still displays safely as literal text. |
| Footnotes cannot match their body | References kept `[^id]`, but footnote bodies became unmarked blockquotes. The body gets the same marker back; this is still a readable degradation, not full footnote interaction. |
| Wide tables squeezed into narrow columns | Originally all columns used `flexBasis: 0`, splitting the screen evenly. The whole table gets horizontal scrolling, uniform column widths with a minimum, updating with container width and system font size; missing cells are still padded. |
| Code layout and iOS fonts | Originally long lines wrapped, and generic `monospace` is not a valid iOS font name. Added horizontal scrolling, kept newlines/indentation/selection-copy, shows the code language; iOS uses Menlo. |
| Fixed image boxes, images embedded in Text | Originally fixed 240×160 — narrow containers could overflow, wide/tall images had obvious padding. Remote images move out of selectable Text, sized by container width and the post-load original aspect ratio; keeps formatting around the image, external-link taps, and load-failure text fallback. |

## Still unimplemented / intentional limits

| Item | Current behavior |
| --- | --- |
| Math | `$…$`, `$$…$$`, `\(…\)`, `\[…\]` parse, but mobile only shows the TeX source — no real typesetting of fractions/matrices/sup-subscripts. Desktop's KaTeX DOM cannot be used directly in native Text. No formula engine added this round. |
| Diagrams like Mermaid | Shown as code blocks, no diagrams generated. |
| Code syntax highlighting | Language labels and monospace code work, but no token coloring or a dedicated code copy button; native text selection and message copy still work. |
| Footnotes, TOC anchors | Footnotes have matching markers but no superscript/jump/back; `#anchor` links stay non-clickable. |
| Arbitrary HTML | Beyond the inline newline above, HTML (like `<details>`, `<sup>`, `<img>`) shows source and does not execute. Arbitrary HTML/scripts must not be enabled just for display. |
| Local images | The follow-up image optimization supports tap-to-load into the body, showing directly when a verified cache exists; images in document previews resolve against the original document directory. Uncached images are not silently downloaded; the open-file entry remains. Phone-local URI documents without a desktop base directory do not guess paths; unsupported formats still go through the original-file entry. |
| Remote image formats and network | Only HTTP(S) allowed; concrete formats depend on the platform Image decoder; SVG, network failures, or platform HTTP policy may trigger text fallback. No SVG decoding or relaxed network security policy added this round. |
| File-link text | The follow-up image optimization keeps the original inline nodes — images, bold, and other formatting inside links are no longer lost. |

**Math typesetting** remains an obvious display gap. Images received the
follow-up optimization below, but unit tests or browser verification cannot
substitute for native acceptance.

## Automated checks

Run in an isolated worktree:

```text
cd mobile
npm run typecheck
npm run lint
npm test

cd ../desktop
npm run typecheck
npx eslint "src/features/markdown/**/*.{ts,tsx}"
npx vitest run src/features/markdown
```

New regressions cover: six heading levels, 0/non-1 start numbers and nested
lists, nested and duplicate definitions, table newlines and the HTML safety
boundary, footnote markers, wide-table column alignment and container-width
updates, code fidelity and platform fonts, image structure/aspect/retry after
URL change, and safe taps on images inside links.

Additionally, char-by-char appends over heading, nested formatting, task list,
quote, table, reference-link, image, math, code, and footnote samples compare
incremental results against full-parse results per prefix, checking streaming
completion and text replacement.

First-round results (PR #609): mobile 82 test files, 1060 tests all pass;
desktop full frontend regression 110 test files, 968 tests all pass; both ends'
TypeScript and ESLint pass. 19 new tests total (18 mobile, 1 desktop).

Jest's simulated layout events only verify component structure and size
computations — they **cannot prove Yoga native layout, gestures, or pixel
effects are correct**.

## Follow-up image optimization (2026-09-14)

Done in the isolated `claude/markdown-media` worktree; local main untouched.

- Shared parser: recursively collects local image references inside links;
  preserves images/rich text inside local file-link labels; image alt is no
  longer lost during label-text extraction.
- Desktop: remote-image failure states are isolated per URL and recoverable
  after a URL change; non-link images can retry; tall images can
  expand/collapse; preview and workspace path changes isolate stale resource
  states. Image links keep their original tap target without nested
  retry/expand buttons; linked images still show as thumbnails. Resolved
  images outside the workspace can be explicitly tap-loaded but are never
  auto-read from disk.
- Mobile: local images tap-load into the body; only verified image caches are
  auto-displayed; size limits and cellular confirmation are kept, no automatic
  model-path downloads. Session/desktop changes cancel old requests and prevent
  old results entering the new session; repeat taps on the same image do not
  download concurrently.
- Mobile document preview: image paths resolve against the original desktop
  document path, not the cache URI downloaded to the phone. Windows drive
  letters, backslashes, relative directories, and `..` all have tests; real
  canonicalize/authorization remains with the desktop backend.
- Original security boundaries kept: images outside the desktop workspace are
  not auto-read; dangerous protocols still rejected; images mobile cannot
  natively decode keep the open-original-file entry — no forced inline
  SVG/HTML execution.

Browser verification uses the real `MarkdownContent`, image components, and
global CSS, stubbing only Tauri IPC; samples are locally provided 640×1600
SVGs, no real user files. 280px and 640px containers each load 3 images
(remote, external-link-wrapped local, local-link-wrapped local), collapsed
height 320px for all; expanded, the first image is ~698px / 1598px; no image
horizontal overflow observed, `a button` count is 0, console error-free. The
test page and server were cleaned up after verification.

Full regression: mobile 84 test files / 1085 tests, desktop 111 test files /
977 tests pass; both ends' TypeScript / ESLint pass.

Tauri app-side and Android/iOS real-device acceptance not yet executed; browser
stubbing cannot prove real asset authorization, mobile network, or native Image
decoding correctness. Math, Mermaid, highlighting, and footnote jumps are not
part of this round's image changes; the limits above still apply.

## Pending real-device acceptance checklist

No adb found in this environment's PATH, SDK env vars, or the default user SDK
directory; no emulator/device was run, and browser results are not equated with
native verification.

On Android and iOS, check with both chat replies and Markdown attachment
previews:

1. Narrow screens around 320/375/430 points, portrait/landscape switching,
   system large fonts: ordinary paragraphs do not overflow; a 4–8 column table
   scrolls horizontally with consistent rows/columns, vertical message
   scrolling still works.
2. Code blocks with 200-character long lines, blank lines, tabs, two-space
   indentation: long lines wrap to the phone width instead of scrolling
   sideways, no line is clipped at the block's bottom edge, content past the
   collapsed line limit is reachable through the expand control, newlines and
   indentation are preserved, long-press copies.
3. H1–H6, italic inside bold, strikethrough, inline code, escapes, hard breaks,
   nested quotes, multi-paragraph bodies/code blocks inside lists: formatting
   and order correct.
4. `0.`, `9.`, `99.`, `100.` start lists and task lists: numbers neither reset
   nor truncated; screen readers identify headings and read-only task state.
5. Wide/tall/square images, images in tables or lists, images wrapped in bold
   or links: image stays in its container; fallback text on failure; a new URL
   reloads; external links open.
6. Streaming tables appearing incrementally, unclosed code fences, links, and
   formulas: no crash, and completion matches one-shot display; no obvious
   scroll-back in long messages.
7. HTTP(S)/mailto links, file links with spaces/Chinese/Windows drive letters:
   handled per existing routing; javascript/data etc. are not handed to the
   system to execute.
8. For math, Mermaid, HTML, TOC anchors, local images, and footnotes, use the
   boundary list above for acceptance — avoid misjudging full support as
   present.
