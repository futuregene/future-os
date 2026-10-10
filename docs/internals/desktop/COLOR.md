# FutureOS GUI color scheme

> ([中文](COLOR.zh-CN.md)) All GUI colors go through the **semantic tokens** defined
> in [`tailwind.config.js`](../../../desktop/tailwind.config.js), never raw Tailwind
> named colors (`blue-300` / `green-50` …). When picking a color for a new or
> changed component, choose from the semantic tokens below — by *what the color
> expresses*, not by "I want a blue".

## Principles

- **Semantics first**: pick tokens by purpose (accent? status? neutral surface?),
  not by hue.
- **Status badges always use the `<Badge tone>` component**, never a hand-written
  `status → color` className mapping.
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

| token            | hex       | usage                                |
| ---------------- | --------- | ------------------------------------ |
| `canvas`         | `#f6f7f9` | bottom-most canvas background        |
| `surface`        | `#ffffff` | card / panel / popup surface         |
| `surface-panel`  | `#f8faff` | right context panel base (near-white, faint theme blue) |
| `surface-subtle` | `#f1f4f8` | secondary surface (hover base, segmented background) |
| `code-surface`   | `#f6f8fa` | code block background                |
| `line`           | `#d9dee7` | standard border                      |
| `line-soft`      | `#e8edf4` | soft border / divider                |
| `ink`            | `#172033` | primary text                         |
| `ink-soft`       | `#5d687a` | secondary text                       |
| `ink-muted`      | `#8a94a6` | muted text / placeholder / icon      |
| `ink-strong`     | `#0f172a` | emphasized headings                  |

### Accent / interaction

| token             | hex       | usage                          |
| ----------------- | --------- | ------------------------------ |
| `accent`          | `#2563eb` | primary accent (primary button, active state, links) |
| `accent-soft`     | `#e8f0ff` | accent light background        |
| `accent-hover`    | `#1d4ed8` | accent hover                   |
| `accent-disabled` | `#bfdbfe` | accent disabled                |
| `accent-pulse`    | blue 28%  | Skills navigation hint pulse glow |
| `focus`           | `#93c5fd` | focus ring                     |

### Search highlight

| token            | hex       | usage                        |
| ---------------- | --------- | --------------------------- |
| `search-match`   | `#fde047` | ordinary matches in current-session search |
| `search-current` | `#fb923c` | currently located search match |

### Status (triple: text / light bg / border)

Each status has three variants — `X` (text), `X-soft` (light bg), `X-line`
(border) — combined for badges and callouts.

| status    | `X` text  | `X-soft` bg | `X-line` border | semantics                          |
| --------- | --------- | ----------- | --------------- | ---------------------------------- |
| `success` | `#15803d` | `#f0fdf4`   | `#bbf7d0`       | success / completed / applied / connected |
| `danger`  | `#dc2626` | `#fef2f2`   | `#fecaca`       | failure / danger / disconnected / discarded |
| `warning` | `#b45309` | `#fffbeb`   | `#fde68a`       | warning / pending / waiting for approval |
| `info`    | `#1d4ed8` | `#eff6ff`   | `#bfdbfe`       | info / checking                   |

> Status badges use `<Badge tone="success|danger|warning|info|accent|neutral">`
> directly; the component bakes in the triple.

### Activity / generating

| token        | hex       | usage                                                                          |
| ------------ | --------- | ------------------------------------------------------------------------------ |
| `generating` | `#f59e0b` | streaming-in-progress indicator (amber `animate-ping` dot; the solid dot and the translucent glow both use this token) |

### Scrollbar (classic webkit thumb)

| token             | hex       | usage                                                                   |
| ----------------- | --------- | ------------------------------------------------------------------------ |
| `scrollbar`       | `#c8ced9` | scrollbar thumb normal (`styles/globals.css` references it via `theme(colors.scrollbar)`) |
| `scrollbar-hover` | `#aeb7c6` | scrollbar thumb hover                                                   |

### Diff (GitHub style)

| token              | hex       | usage              |
| ------------------ | --------- | ------------------ |
| `diff-add`         | `#e6ffec` | added-line background |
| `diff-add-line`    | `#aadfb8` | added-line left border |
| `diff-remove`      | `#ffebe9` | removed-line background |
| `diff-remove-line` | `#ffc9c9` | removed-line left border |

### Shadows

| token                     | usage                          |
| ------------------------- | ------------------------------ |
| `shadow-panel`            | panel / card lift              |
| `shadow-dialog`           | dialog / popup                 |
| `shadow-sidebar-divider`  | soft inner shadow on the left-rail divider |
| `shadow-sidebar-floating` | left-rail floating preview shadow |

### Mask / overlay

| token     | value                | usage                                                            |
| --------- | -------------------- | --------------------------------------------------------------- |
| `overlay` | `rgba(0, 0, 0, 0.6)` | fullscreen modal background mask (pure black 60%, alpha baked into the token, use `bg-overlay` directly) |

> The mask color is semantically different from the nav blue-black `ink-strong`
> (`#0f172a`), so it gets its own token. All modals (dialogs, settings, file
> preview, …) reuse this layer via the shared `components/ui/Overlay`
> (`bg-overlay` + `backdrop-blur-[1px]`) — **do not** write a mask color
> anywhere else; to adjust the shade change the token, to adjust the blur change
> `Overlay`.

## Color-picking quick reference

- Text → `ink` / `ink-soft` / `ink-muted` / `ink-strong`
- Background → `canvas` (bottom-most) / `surface` (cards and popups) /
  `surface-panel` (right context panel) / `surface-subtle` (secondary) /
  `code-surface` (code blocks)
- Border → `line` / `line-soft`
- Primary action / active → `accent` (+ `accent-hover` / `accent-disabled`);
  focus ring → `focus`
- Status (success / failure / warning / info) → `<Badge tone>`, or manually
  `text-X` + `bg-X-soft` + `border-X-line`
- Diff → `diff-add*` / `diff-remove*`
- Popup mask → `overlay` (use the shared `Overlay` component, `bg-overlay`),
  never a hand-written mask color

## Anti-patterns

- ❌ Writing raw Tailwind colors like `bg-blue-50` / `text-green-700` /
  `focus:ring-blue-100`
- ❌ Hand-writing `function xxxStatusClass(): string` returning status color
  classes — use `<Badge tone>` instead
- ❌ Writing a mask color inside a component (`bg-black/…` / `bg-*/…`) — use
  the `overlay` token via the shared `Overlay`
- ✅ The only exception: **category colors** for run error subtypes use the raw
  palette to distinguish sibling kinds; see "Principles" above

## Source

All tokens are defined in
[`tailwind.config.js`](../../../desktop/tailwind.config.js). **Change colors
there only**; never scatter raw colors across components.
