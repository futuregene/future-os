# FutureOS GUI color scheme

> ([中文](COLOR.zh-CN.md)) All GUI colors go through the **semantic tokens**
> defined in [`desktop/src/styles/globals.css`](../../../desktop/src/styles/globals.css)
> (`@theme` holds the default light palette, a `:root.dark` block overrides the
> same variables, and each color theme adds a generated `[data-palette="…"]`
> block that does the same), never raw Tailwind named colors
> (`blue-300` / `green-50` …).
> When picking a color for a new or changed component, choose from the semantic
> tokens below — by *what the color expresses*, not by "I want a blue".

## Appearance and color theme (two axes)

The look has **two independent choices**, both in Settings → General, both
persisted in `localStorage` before the first paint:

| axis | values | applied as | picks |
| --- | --- | --- | --- |
| **Appearance** | Light / Dark / Follow system | `.dark` on `<html>` | which rendition is painted |
| **Color theme** | 14 palettes — FutureOS (default), Ayu, Catppuccin, Dracula, Everforest, Gruvbox, Kanagawa, Nightfox, Nord, One Dark, Rosé Pine, Solarized, Tokyo Night, Vesper | `data-palette` on `<html>` | the hues |

They are separate because they answer different questions: "how bright" and
"what does it feel like". Every color theme ships **both** renditions, so a
chosen theme still follows the system — Rosé Pine in a light window is Dawn, in
a dark one Moon. A combined list of six light + six dark entries would make the
user re-pick their look whenever the OS flipped.

The values are CSS variables, so a component writes `bg-surface` /
`text-ink-soft` once and never branches on either axis: `bg-surface` resolves to
`var(--color-surface)`, and whichever theme is active has set that variable.
That is also why neither axis needs a `dark:` variant or a per-theme variant.

Every table below lists `light / dark` for the **default** theme (FutureOS). A
token with one value is identical in both renditions.

## Palettes

Each palette is **data**: `src/theme/palettes.json` holds one object per theme
with its token values for both renditions, its terminal palette and its Shiki
theme. Everything that needs to know a theme reads that file —

- `desktop/scripts/gen-theme-palettes.mjs` renders the `[data-palette="…"]` blocks in
  `styles/globals.css`. They are **generated**, between the
  `/* <theme-palettes> */` markers: edit the JSON, then
  `node desktop/scripts/gen-theme-palettes.mjs`.
- `src/theme/palettes.ts` is the typed accessor (ids, terminal palettes, Shiki
  themes) used by the harness-free consumers: the embedded terminal and code
  highlighting.
- `lib/theme.ts` owns the *choice* (which id, persisted) and applies it as a
  `data-palette` attribute on `<html>`.

Nothing else repeats a color, and `styles/globals.test.ts` fails with the
regeneration command when the stylesheet and the JSON disagree, so a forgotten
run is caught rather than shipped. This is the same shape opencode uses for its
built-in themes (one data object per theme, semantic roles with a light and a
dark value): with fourteen palettes, hand-written CSS in three places would
drift the first time somebody edited one of them.

Each generated rule still has two selector forms:

```css
[data-palette="nord"] { … }                      /* light rendition */
.dark [data-palette="nord"],
:root.dark[data-palette="nord"] { … }            /* dark rendition */
```

- `[data-palette="nord"]` matches the root **and** any element carrying the
  attribute, which is how the Settings picker paints one preview card per theme
  (see `features/settings/ThemePalettePicker.tsx`) while the app stays on its
  active theme.
- The dark block needs both selectors: the root carries `.dark` and
  `data-palette` on the same element, a preview carries only `data-palette`
  inside a `.dark` ancestor. Neither may lose to `:root.dark`, which both forms
  avoid at equal-or-higher specificity — and every palette block comes **after**
  `:root.dark` in the file, which is what decides it for the light form.
  `styles/globals.test.ts` pins that order, every palette's token set, and that
  the `futureos` block still mirrors `@theme` / `:root.dark`.

Deliberately **not** per-palette:

- `search-*` — a search hit is a bright mark with dark text in every theme, so
  the shared values stay.
- `--shadow-*` — depth belongs to the rendition, not the hue.

### The shipped themes

`futureos` (the default) then alphabetical: `ayu`, `catppuccin`, `dracula`,
`everforest`, `gruvbox`, `kanagawa`, `nightfox`, `nord`, `one-dark`,
`rose-pine`, `solarized`, `tokyo-night`, `vesper`.

Each is derived from its upstream project's own colors (Nord's Polar Night /
Snow Storm / Frost, Solarized's base03 / base3, Gruvbox's bg0 / bg1, Rosé Pine
Dawn / Moon, Catppuccin Latte / Mocha, Tokyo Night Night / Day, Dracula /
Alucard, One Dark / One Light, Everforest Medium, Kanagawa Wave / Lotus,
Nightfox / Dayfox, Ayu Mirage / Light, Vesper). Where an upstream palette is
deliberately low-contrast (Everforest and Kanagawa most of all), its ink ramp is
nudged until it clears the floors below — the hue stays, the legibility does
not.

**Syntax highlighting** comes from the editor theme the upstream project ships,
so a code block belongs to the window around it rather than being a tinted
GitHub theme. Where the project ships only one rendition, the palette names the
nearest available editor theme for the other half:

| palette | light | dark |
| --- | --- | --- |
| `futureos` | `github-light` | `github-dark` |
| `ayu` | `ayu-light` | `ayu-dark` |
| `catppuccin` | `catppuccin-latte` | `catppuccin-mocha` |
| `dracula` | `rose-pine-dawn` (closest) | `dracula` |
| `everforest` | `everforest-light` | `everforest-dark` |
| `gruvbox` | `gruvbox-light-medium` | `gruvbox-dark-medium` |
| `kanagawa` | `kanagawa-lotus` | `kanagawa-wave` |
| `nightfox` | `one-light` (closest) | `nord` (closest) |
| `nord` | `nord` | `nord` |
| `one-dark` | `one-light` | `one-dark-pro` |
| `rose-pine` | `rose-pine-dawn` | `rose-pine-moon` |
| `solarized` | `solarized-light` | `solarized-dark` |
| `tokyo-night` | `one-light` (closest) | `tokyo-night` |
| `vesper` | `one-light` (closest) | `vesper` |

### Adding a theme

1. Add one object to `src/theme/palettes.json` (the token names are the
   `--color-*` names from `@theme`, minus `search-*`; both renditions; a
   terminal palette; a Shiki light/dark pair).
2. Add its id to `THEME_PALETTES` in `lib/theme.ts` (picker order — default
   first, then alphabetical) and its name/description to
   `i18n/locales/{en,zh}/settings.json` under `theme.palettes`.
3. `node desktop/scripts/gen-theme-palettes.mjs`.

`src/theme/palettes.test.ts` then checks the things a hand-written palette gets
wrong: every token present in both renditions, the contrast floors, the ANSI
colours on the terminal's background, and that the Shiki ids actually exist.

### Contrast floors

Checked when a color theme is added, and pinned for the terminal palettes by
`features/terminal/theme.test.ts`:

| pair | floor |
| --- | --- |
| `ink` on `canvas` / `surface` | 7:1 |
| `ink-soft` on `surface` | 4.5:1 |
| `ink-muted` on `surface` | 3:1 (muted text is deliberately low) |
| `accent`, `success`, `danger`, `warning`, `info` on `surface` | 4.5:1 |
| white on `accent-fill` / `danger-fill` | 4.5:1 |
| `ink` on `surface-subtle` / `code-surface` | 6:1 |
| `search-ink` on `search-match` | 4.5:1 |

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

Two places cannot use CSS variables and resolve the theme in JavaScript, so each
needs a palette × rendition entry — they are the only place a new color theme
can be forgotten silently:

- **Terminal** (`features/terminal/theme.ts`): the xterm palette stored beside
  the theme's tokens in `palettes.json`, read through `paletteTerminal(id,
  rendition)` — selected by `terminalTheme(palette, theme)`, or
  `currentTerminalTheme()` when xterm is constructed, which cannot read a hook
  value. `theme.test.ts` walks every palette.
- **Code highlighting** (`features/markdown/useCodeHighlighter.ts`): the Shiki
  light/dark pair stored in the same object, read through `paletteShiki(id,
  rendition)` and bundled up front. The mapping table under "The shipped
  themes" above lists which editor theme each palette reuses.

Both are covered by `src/theme/palettes.test.ts` (ANSI contrast on the
terminal's own background, and that every Shiki id actually exists), so neither
can be forgotten when a theme is added.

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

Token values live in two places, and which one depends on the kind of color:

- **The default palette and shadows** —
  [`desktop/src/styles/globals.css`](../../../desktop/src/styles/globals.css):
  `@theme` for the light rendition, `:root.dark` for the dark one, `:root` /
  `:root.dark` `--shadow-*` for shadows.
- **Every other color theme** —
  [`desktop/src/theme/palettes.json`](../../../desktop/src/theme/palettes.json),
  rendered into the stylesheet by `desktop/scripts/gen-theme-palettes.mjs`. Do not edit
  the generated blocks by hand; edit the JSON and regenerate.

Never scatter raw colors across components.
