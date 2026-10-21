# FutureOS GUI 配色方案

> ([English](COLOR.md)) GUI 的颜色全部走
> [`desktop/src/styles/globals.css`](../../../desktop/src/styles/globals.css)
> 里定义的**语义 token**(`@theme` 是默认浅色，`:root.dark` 覆盖同名变量，每个配色
> 主题再加一段 `[data-palette="…"]` 做同样的事),不直接用 Tailwind 原生具名色
> (`blue-300` / `green-50` …)。新写或改组件选色时,从下面的语义 token 里挑——按
> 「这个色表达什么」选,而不是「我想要个蓝色」。

## 外观与配色主题（两个独立维度）

界面观感由**两个相互独立的选择**决定，都在设置 → 通用，都在首屏绘制前写入
`localStorage`：

| 维度 | 取值 | 落到哪里 | 决定 |
| --- | --- | --- | --- |
| **外观** | 浅色 / 深色 / 跟随系统 | `<html>` 上的 `.dark` | 画哪一套明暗 |
| **配色主题** | FutureOS（默认）· Nord · Solarized · Gruvbox · Rosé Pine · Catppuccin | `<html>` 上的 `data-palette` | 色相 |

分开的原因是它们回答两个不同的问题：「多亮」和「什么气质」。每个配色主题**都自带
深浅两套**，所以选定的主题仍然跟随系统——Rosé Pine 在浅色窗口是 Dawn，在深色窗口是
Moon。若把 6 个浅色 + 6 个深色并成一份列表，系统外观一翻转，用户就得重挑一次。

值都是 CSS 变量，所以组件只写一次 `bg-surface` / `text-ink-soft`，不按任一维度
分支：`bg-surface` 解析成 `var(--color-surface)`，当前生效的主题已经把那个变量设好
了。也正因如此，两个维度都不需要 `dark:` 变体或按主题的变体。

下面的表格列的是**默认主题**（FutureOS）的 `浅色 / 深色`。只有一个值的 token 表示
两套明暗相同。

## 配色主题

每个主题是 `styles/globals.css` 里的一段，重新赋值 `@theme` / `:root.dark` 声明的
**同一组 `--color-*` 变量**。每个主题两种选择器形式：

```css
[data-palette="nord"] { … }                      /* 浅色那一套 */
.dark [data-palette="nord"],
:root.dark[data-palette="nord"] { … }            /* 深色那一套 */
```

- `[data-palette="nord"]` 既匹配根元素，也匹配任何带该属性的元素——设置里的选择器
  就是靠这一条给每个主题画一张预览卡（见
  `features/settings/ThemePalettePicker.tsx`），而应用本身仍停在生效的主题上。
- 深色那段两个选择器都要：根元素同时带 `.dark` 和 `data-palette`，预览卡只在
  `.dark` 祖先下带 `data-palette`。两者都不能输给 `:root.dark`——前者特异性不低，
  后者靠所有主题块都排在 `:root.dark` **之后**。`styles/globals.test.ts` 钉住了这个
  顺序、每个主题的 token 集合，以及 `futureos` 那段是否仍等于 `@theme` /
  `:root.dark`。

刻意**不**按主题改的：

- `search-*`——搜索命中在任何主题里都是亮底配深字，共用的值保持不变。
- `--shadow-*`——阴影深度属于明暗，不属于色相。

每个主题都从上游项目自己的色板取值（Nord 的 Polar Night / Snow Storm / Frost、
Solarized 的 base03 / base3、Gruvbox 的 bg0 / bg1、Rosé Pine 的 Dawn / Moon、
Catppuccin 的 Latte / Mocha），再调整到满足下面的对比度下限。新主题必须定义同一组
token——否则 `styles/globals.test.ts` 会失败；`features/terminal/theme.ts` 缺了它的
终端配色则编译不过。

### 对比度下限

加主题时逐项核对，终端配色另由 `features/terminal/theme.test.ts` 钉住：

| 组合 | 下限 |
| --- | --- |
| `ink` 对 `canvas` / `surface` | 7:1 |
| `ink-soft` 对 `surface` | 4.5:1 |
| `ink-muted` 对 `surface` | 3:1（弱化文字刻意压低） |
| `accent`、`success`、`danger`、`warning`、`info` 对 `surface` | 4.5:1 |
| 白字对 `accent-fill` / `danger-fill` | 4.5:1 |
| `ink` 对 `surface-subtle` / `code-surface` | 6:1 |
| `search-ink` 对 `search-match` | 4.5:1 |

## 原则

- **语义优先**:按用途选 token(强调?状态?中性表面?),不按色相。
- **状态徽章统一用 `<Badge tone>` 组件**,不手写 `status → color` 的 className 映射。
- **实心底色与文字色是两个 token。** `accent` 在深色调色板里会提亮,才能在深色
  表面作为文字可读;实心主按钮用更深的 `accent-fill`,保证上面的白字在两套主题都
  清晰。`danger` / `danger-fill` 同理。
- **例外——分类色**:用颜色**区分并列种类**(而非状态)的地方,可保留原生色板。语义
  token 只有有限几个、不足以表达 N 种并列分类。当前(也是唯一)的例外是
  `features/runs/runErrorMeta.ts` 里的 `errorTypeMeta`:六个运行错误子类型
  (`stream_disconnected`、`command_failed`、`model_failed`、`abort_requested`、
  `timeout`、`unknown`)各自保留原生 `text-*` 色,并另有 icon 辅助区分。运行事件行
  及其它一切使用语义 token。

## Token 清单

### 中性 / 表面

| token            | 浅色      | 深色      | 用途                                 |
| ---------------- | --------- | --------- | ------------------------------------ |
| `canvas`         | `#f6f7f9` | `#0d1116` | 最底层画布背景                       |
| `surface`        | `#ffffff` | `#161b22` | 卡片 / 面板 / 弹层表面               |
| `surface-panel`  | `#f8faff` | `#11161d` | 右侧上下文面板底色                   |
| `surface-subtle` | `#f1f4f8` | `#1d232c` | 次级表面（hover 底、分段背景）       |
| `code-surface`   | `#f6f8fa` | `#0f141a` | 代码块背景                           |
| `line`           | `#d9dee7` | `#2b333d` | 标准边框                             |
| `line-soft`      | `#e8edf4` | `#222a34` | 弱边框 / 分隔线                      |
| `ink`            | `#172033` | `#e6ebf2` | 主文字                               |
| `ink-soft`       | `#5d687a` | `#a3adbb` | 次要文字                             |
| `ink-muted`      | `#8a94a6` | `#78828f` | 弱化文字 / 占位 / 图标               |
| `ink-strong`     | `#0f172a` | `#f4f7fb` | 强调标题                             |

### 强调 / 交互

| token                  | 浅色              | 深色              | 用途 |
| ---------------------- | ----------------- | ----------------- | ---- |
| `accent`               | `#2563eb`         | `#60a5fa`         | 文字、图标、激活态边框、圆点 |
| `accent-soft`          | `#e8f0ff`         | `#16243c`         | 强调浅底 |
| `accent-fill`          | `#2563eb`         | `#2563eb`         | 实心强调底(主按钮) |
| `accent-fill-hover`    | `#1d4ed8`         | `#1d4ed8`         | 实心强调 hover |
| `accent-fill-disabled` | `#bfdbfe`         | `#1e3a5f`         | 实心强调禁用 |
| `accent-pulse`         | 蓝色 28%          | 蓝色 30%          | Skills 导航提示脉冲的光晕 |
| `focus`                | `#93c5fd`         | `#3b82f6`         | focus ring |

### 搜索高亮

| token            | 浅色 / 深色 | 用途                     |
| ---------------- | ----------- | ------------------------ |
| `search-match`   | `#fde047`   | 当前会话搜索的普通匹配项 |
| `search-current` | `#fb923c`   | 当前定位的搜索匹配项     |
| `search-ink`     | `#0f172a`   | 高亮上的文字（两套主题都保持深色——底色是亮的） |

### 状态(三件套:文字 / 底色 / 边框)

每个状态有 `X`(文字)、`X-soft`(底)、`X-line`(边框)三个变体,组合用于徽章和提示框。

| 状态      | `X` text              | `X-soft` bg           | `X-line` border       | 语义                               |
| --------- | --------------------- | --------------------- | --------------------- | ---------------------------------- |
| `success` | `#15803d` / `#4ade80` | `#f0fdf4` / `#10251a` | `#bbf7d0` / `#1e4a33` | 成功 / 已完成 / applied / 已连接   |
| `danger`  | `#dc2626` / `#f87171` | `#fef2f2` / `#2a1517` | `#fecaca` / `#5b2528` | 失败 / 危险 / 已断开 / discarded   |
| `warning` | `#b45309` / `#fbbf24` | `#fffbeb` / `#2a2110` | `#fde68a` / `#5a4419` | 警告 / 待处理 / pending / 等待审批 |
| `info`    | `#1d4ed8` / `#60a5fa` | `#eff6ff` / `#14233b` | `#bfdbfe` / `#2a4270` | 信息 / 检查中                      |

`danger-fill`(`#dc2626` / `#b91c1c`)是实心危险按钮的底色——上面是白字,所以深色
调色板里保持较深的红。

> 状态徽章直接用 `<Badge tone="success|danger|warning|info|accent|neutral">`,组件已把三件套烘进去。

### 活动 / 生成中

| token        | 浅色      | 深色      | 用途 |
| ------------ | --------- | --------- | ---- |
| `generating` | `#f59e0b` | `#fbbf24` | 流式生成中指示灯（琥珀色 `animate-ping` 圆点） |

### 滚动条（经典 webkit 拇指）

| token             | 浅色      | 深色      | 用途 |
| ----------------- | --------- | --------- | ---- |
| `scrollbar`       | `#c8ced9` | `#38414d` | 滚动条拇指常态（`styles/globals.css`） |
| `scrollbar-hover` | `#aeb7c6` | `#4a5563` | 滚动条拇指 hover |

### Diff(GitHub 风格)

| token              | 浅色      | 深色      | 用途         |
| ------------------ | --------- | --------- | ------------ |
| `diff-add`         | `#e6ffec` | `#12261a` | 新增行底色   |
| `diff-add-line`    | `#aadfb8` | `#2f6b45` | 新增行左边框 |
| `diff-remove`      | `#ffebe9` | `#2b1519` | 删除行底色   |
| `diff-remove-line` | `#ffc9c9` | `#7a3438` | 删除行左边框 |

### 阴影

| token                     | 用途                     |
| ------------------------- | ------------------------ |
| `shadow-panel`            | 面板 / 卡片浮起          |
| `shadow-dialog`           | 对话框 / 弹层            |
| `shadow-sidebar-divider`  | 左侧栏分割线内侧柔和阴影 |
| `shadow-sidebar-floating` | 左侧栏浮动预览阴影       |

阴影值是 `--shadow-*` 自定义属性,深色调色板会加深(浅色阴影在深色表面上几乎看不
出来)。工具类在 `desktop/tailwind.config.js` 里接线,因此该文件只保留 `boxShadow`。

### 遮罩层 / Overlay

| token     | 浅色                | 深色                | 用途 |
| --------- | ------------------- | ------------------- | ---- |
| `overlay` | `rgba(0, 0, 0, 0.6)` | `rgba(0, 0, 0, 0.7)` | 全屏模态背景遮罩(直接 `bg-overlay`) |

> 所有模态(对话框、设置、文件预览等)都经共享 `components/ui/Overlay` 复用这一层
> (`bg-overlay` + backdrop blur)——**不要**在别处另写遮罩色;要调整深浅统一改
> token,改模糊改 `Overlay`。

### 非 CSS 的调色板

有两处用不了 CSS 变量、必须在 JS 里解析主题，因此每处都要有「每个主题 × 明暗」的
条目——这也是新主题唯一可能被悄悄漏掉的地方：

- **终端**（`features/terminal/theme.ts`）：每个主题、每套明暗各一份 xterm 配色，
  由 `terminalTheme(palette, theme)` 取；构造 xterm 时改用
  `currentTerminalTheme()`，那里读不到 hook 的值。那张
  `Record<ThemePalette, Record<ResolvedTheme, …>>` 表让漏配成为类型错误，
  `theme.test.ts` 为**每个**主题的每个 ANSI 色钉了最低 WCAG 对比度。
- **代码高亮**（`features/markdown/useCodeHighlighter.ts`）：每个主题、每套明暗各
  一个 Shiki 主题，由 `useThemePalette()` + `useResolvedTheme()` 选，且都在启动时
  一并载入。各主题都用自己上游项目出的编辑器主题（`nord`、
  `solarized-light` / `solarized-dark`、`gruvbox-light-medium` /
  `gruvbox-dark-medium`、`rose-pine-dawn` / `rose-pine-moon`、`catppuccin-latte` /
  `catppuccin-mocha`）；默认主题继续用 `github-light` / `github-dark`，它的色板就是
  从这两套取的。

给这两处加颜色时，每个主题的两套都要加。

## 选色速查

- 文字 → `ink` / `ink-soft` / `ink-muted` / `ink-strong`
- 背景 → `canvas` / `surface` / `surface-panel` / `surface-subtle` / `code-surface`
- 边框 → `line` / `line-soft`
- 主操作 / 激活 → 文字、边框、图标用 `accent`;实心按钮用 `accent-fill`
  (+ `accent-fill-hover` / `accent-fill-disabled`);focus ring → `focus`
- 状态(成功 / 失败 / 警告 / 信息)→ `<Badge tone>`,或手动 `text-X` + `bg-X-soft` + `border-X-line`
- 实心危险按钮 → `danger-fill`
- diff → `diff-add*` / `diff-remove*`
- 弹层遮罩 → `overlay`(用共享 `Overlay` 组件,`bg-overlay`)

## 反模式

- ❌ 直接写 `bg-blue-50` / `text-green-700` / `focus:ring-blue-100` 等 Tailwind 原生色
- ❌ 手写 `function xxxStatusClass(): string` 返回状态色类 —— 改用 `<Badge tone>`
- ❌ 在组件里另写遮罩色(`bg-black/…` / `bg-*/…`)—— 用 `overlay` token,经共享 `Overlay`
- ❌ 在组件里写死只适合浅色的值(纯白面板、固定的深色文字)—— 换主题就会坏
- ✅ 唯一例外:运行错误子类型的**分类色**用原生色板区分并列种类,见上方「原则」

## 来源

token 值定义在
[`desktop/src/styles/globals.css`](../../../desktop/src/styles/globals.css):
`@theme` 是默认浅色调色板,`:root.dark` 是它的深色版本,`[data-palette="…"]`
各段是其它配色主题,`:root` / `:root.dark` 里的 `--shadow-*` 是阴影。
**改色统一改那里**,不在组件里散写原生色。主题 id 本身(`lib/theme.ts` 的
`THEME_PALETTES`)是选择器、终端配色表和 Shiki 表的唯一来源。
