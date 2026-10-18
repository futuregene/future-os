# FutureOS GUI color scheme

> ([中文](COLOR.zh-CN.md)) All GUI colors go through the **semantic tokens**
> defined in [`desktop/src/styles/globals.css`](../../../desktop/src/styles/globals.css)
> (`@theme` holds the light palette, a `:root.dark` block overrides the same
> variables), never raw Tailwind named colors (`blue-300` / `green-50` …). When
> picking a color for a new or changed component, choose from the semantic
> tokens below — by *what the color expresses*, not by "I want a blue".

## Light and dark

The app ships both palettes. Every token exists in both, so a component writes
`bg-surface` / `text-ink-soft` once and never branches on the theme. Switching
is a `.dark` class on `<html>`, applied by `lib/theme.ts`; the choice (Light /
Dark / Follow system) lives in Settings → General and is persisted in
`localStorage`. The values are CSS variables, which is why dark mode needs no
`dark:` variants — the palette flips underneath the same utility (`bg-surface`
resolves to `var(--color-surface)`).

Every table below lists `light / dark`. A token with one value is identical in
both themes.

## Principles

- **Semantics first**: pick tokens by purpose (accent? status? neutral surface?),
  not by hue.
- **Status badges always use the `<Badge tone>` component**, never a hand-written
  `status → color` className mapping.
- **Solid fills are a separate token from text colors.** `accent` brightens in
  the dark palette so it stays readable as text on a dark surface; a solid
  button keeps the darker `accent-fill` so its white label stays legible in both
  themes. The same split exists for `danger` / `danger-fill`.
- **Exception — category colors**: where colors **distinguish sibling categories**
  (not status), the raw palette may be kept. The semantic tokens are few and
  cannot express N sibling categories. The current (and only) exception is
  `errorTypeMeta` in `features/runs/runErrorMeta.ts`: the six run error
  subtypes (`stream_disconnected`, `command_failed`, `model_failed`,
  `abort_requested`, `timeout`, `unknown`) each keep a raw `text-*` color,
  additionally disambiguated by an icon. Run-event rows and everything else use
  the semantic tokens.

## Token list

### Neutral / surfaces

| token            | light     | dark      | usage                                |
| ---------------- | --------- | --------- | ------------------------------------ |
| `canvas`         | `#f6f7f9` | `#0d1116` | bottom-most canvas background        |
| `surface`        | `#ffffff` | `#161b22` | card / panel / popup surface         |
| `surface-panel`  | `#f8faff` | `#11161d` | right context panel base             |
| `surface-subtle` | `#f1f4f8` | `#1d232c` | secondary surface (hover, segmented) |
| `code-surface`   | `#f6f8fa` | `#0f141a` | code block background                |
| `line`           | `#d9dee7` | `#2b333d` | standard border                      |
| `line-soft`      | `#e8edf4` | `#222a34` | soft border / divider                |
| `ink`            | `#172033` | `#e6ebf2` | primary text                         |
| `ink-soft`       | `#5d687a` | `#a3adbb` | secondary text                       |
| `ink-muted`      | `#8a94a6` | `#78828f` | muted text / placeholder / icon      |
| `ink-strong`     | `#0f172a` | `#f4f7fb` | emphasized headings                  |

### Accent / interaction

| token                   | light                  | dark                   | usage |
| ----------------------- | ---------------------- | ---------------------- | ----- |
| `accent`                | `#2563eb`              | `#60a5fa`              | text, icons, active borders, dots |
| `accent-soft`           | `#e8f0ff`              | `#16243c`              | accent light background |
| `accent-fill`           | `#2563eb`              | `#2563eb`              | solid accent surface (primary button) |
| `accent-fill-hover`     | `#1d4ed8`              | `#1d4ed8`              | solid accent hover |
| `accent-fill-disabled`  | `#bfdbfe`              | `#1e3a5f`              | solid accent disabled |
| `accent-pulse`          | blue 28%               | blue 30%               | Skills navigation hint pulse glow |
| `focus`                 | `#93c5fd`              | `#3b82f6`              | focus ring |

### Search highlight

| token            | light / dark | usage |
| ---------------- | ------------ | ----- |
| `search-match`   | `#fde047`    | ordinary matches in current-session search |
| `search-current` | `#fb923c`    | currently located search match |
| `search-ink`     | `#0f172a`    | text on a highlight (stays dark in both themes — the marks are bright) |

### Status (triple: text / light bg / border)

Each status has three variants — `X` (text), `X-soft` (bg), `X-line` (border) —
combined for badges and callouts.

| status    | `X` text            | `X-soft` bg         | `X-line` border     | semantics |
| --------- | ------------------- | ------------------- | ------------------- | --------- |
| `success` | `#15803d` / `#4ade80` | `#f0fdf4` / `#10251a` | `#bbf7d0` / `#1e4a33` | success / completed / applied / connected |
| `danger`  | `#dc2626` / `#f87171` | `#fef2f2` / `#2a1517` | `#fecaca` / `#5b2528` | failure / danger / disconnected / discarded |
| `warning` | `#b45309` / `#fbbf24` | `#fffbeb` / `#2a2110` | `#fde68a` / `#5a4419` | warning / pending / waiting for approval |
| `info`    | `#1d4ed8` / `#60a5fa` | `#eff6ff` / `#14233b` | `#bfdbfe` / `#2a4270` | info / checking |

`danger-fill` (`#dc2626` / `#b91c1c`) is the solid destructive-button surface —
white text on it, so it stays dark in the dark palette.

> Status badges use `<Badge tone="success|danger|warning|info|accent|neutral">`
> directly; the component bakes in the triple.

### Activity / generating

| token        | light     | dark      | usage |
| ------------ | --------- | --------- | ----- |
| `generating` | `#f59e0b` | `#fbbf24` | streaming-in-progress indicator (amber `animate-ping` dot) |

### Scrollbar (classic webkit thumb)

| token             | light     | dark      | usage |
| ----------------- | --------- | --------- | ----- |
| `scrollbar`       | `#c8ced9` | `#38414d` | scrollbar thumb normal (`styles/globals.css`) |
| `scrollbar-hover` | `#aeb7c6` | `#4a5563` | scrollbar thumb hover |

### Diff (GitHub style)

| token              | light     | dark      | usage |
| ------------------ | --------- | --------- | ----- |
| `diff-add`         | `#e6ffec` | `#12261a` | added-line background |
| `diff-add-line`    | `#aadfb8` | `#2f6b45` | added-line left border |
| `diff-remove`      | `#ffebe9` | `#2b1519` | removed-line background |
| `diff-remove-line` | `#ffc9c9` | `#7a3438` | removed-line left border |

### Shadows

| token                     | usage |
| ------------------------- | ----- |
| `shadow-panel`            | panel / card lift |
| `shadow-dialog`           | dialog / popup |
| `shadow-sidebar-divider`  | soft inner shadow on the left-rail divider |
| `shadow-sidebar-floating` | left-rail floating preview shadow |

Shadow values are `--shadow-*` custom properties; the dark palette deepens them
(a light-theme shadow barely reads on a dark surface). They are wired in
`desktop/tailwind.config.js`, which therefore keeps only `boxShadow`.

### Mask / overlay

| token     | light               | dark                | usage |
| --------- | ------------------- | ------------------- | ----- |
| `overlay` | `rgba(0, 0, 0, 0.6)` | `rgba(0, 0, 0, 0.7)` | fullscreen modal mask (`bg-overlay`) |

> All modals (dialogs, settings, file preview, …) reuse this layer via the
> shared `components/ui/Overlay` (`bg-overlay` + backdrop blur) — **do not**
> write a mask color anywhere else; to adjust the shade change the token, to
> adjust the blur change `Overlay`.

### Non-CSS palettes

Two places cannot use CSS variables and resolve the theme in JavaScript:

- **Terminal** (`features/terminal/theme.ts`): `TERMINAL_THEME` (light) and
  `TERMINAL_DARK_THEME` (dark), selected by `terminalTheme(resolvedTheme)`.
  `theme.test.ts` pins a minimum WCAG contrast for every ANSI color in both.
- **Code highlighting** (`features/markdown/useCodeHighlighter.ts`): Shiki
  `github-light` / `github-dark`, chosen from `useResolvedTheme()`.

When adding a color to one of them, add it to both variants.

## Color-picking quick reference

- Text → `ink` / `ink-soft` / `ink-muted` / `ink-strong`
- Background → `canvas` / `surface` / `surface-panel` / `surface-subtle` / `code-surface`
- Border → `line` / `line-soft`
- Primary action / active → `accent` for text, border and icons; `accent-fill`
  (+ `accent-fill-hover` / `accent-fill-disabled`) for solid buttons; focus ring → `focus`
- Status (success / failure / warning / info) → `<Badge tone>`, or manually
  `text-X` + `bg-X-soft` + `border-X-line`
- Destructive solid button → `danger-fill`
- Diff → `diff-add*` / `diff-remove*`
- Popup mask → `overlay` (use the shared `Overlay` component, `bg-overlay`)

## Anti-patterns

- ❌ Writing raw Tailwind colors like `bg-blue-50` / `text-green-700` /
  `focus:ring-blue-100`
- ❌ Hand-writing `function xxxStatusClass(): string` returning status color
  classes — use `<Badge tone>` instead
- ❌ Writing a mask color inside a component (`bg-black/…` / `bg-*/…`) — use
  the `overlay` token via the shared `Overlay`
- ❌ Hard-coding a light-only value (a white panel, a fixed dark text color) in
  a component — it will break in the other theme
- ✅ The only exception: **category colors** for run error subtypes use the raw
  palette to distinguish sibling kinds; see "Principles" above

## Source

Token values are defined in
[`desktop/src/styles/globals.css`](../../../desktop/src/styles/globals.css):
`@theme` for the light palette, `:root.dark` for the dark overrides, `:root` /
`:root.dark` `--shadow-*` for shadows. **Change colors there only**; never scatter
raw colors across components.
