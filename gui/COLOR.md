# FutureOS GUI 配色方案

GUI 的颜色全部走 [`tailwind.config.js`](tailwind.config.js) 里定义的**语义 token**,不直接用 Tailwind 原生具名色(`blue-300` / `green-50` …)。新写或改组件选色时,从下面的语义 token 里挑——按「这个色表达什么」选,而不是「我想要个蓝色」。

## 原则

- **语义优先**:按用途选 token(强调?状态?中性表面?),不按色相。
- **状态徽章统一用 `<Badge tone>` 组件**,不手写 `status → color` 的 className 映射。
- **例外——分类色**:用颜色**区分并列种类**(而非状态)的地方,可保留原生色板。语义 token 只有有限几个、不足以表达 N 种并列分类。当前例外:`eventCategoryClass`(事件类别 6 种)、`formatErrorType`(错误子类型 5 种,另有 icon 辅助区分)。

## Token 清单

### 中性 / 表面
| token | hex | 用途 |
|---|---|---|
| `canvas` | `#f6f7f9` | 最底层画布背景 |
| `surface` | `#ffffff` | 卡片 / 面板 / 弹层表面 |
| `surface-subtle` | `#f1f4f8` | 次级表面(代码块、hover 底、分段背景) |
| `line` | `#d9dee7` | 标准边框 |
| `line-soft` | `#e8edf4` | 弱边框 / 分隔线 |
| `ink` | `#172033` | 主文字 |
| `ink-soft` | `#5d687a` | 次要文字 |
| `ink-muted` | `#8a94a6` | 弱化文字 / 占位 / 图标 |
| `ink-strong` | `#0f172a` | 强调标题 |

### 强调 / 交互
| token | hex | 用途 |
|---|---|---|
| `accent` | `#2563eb` | 主强调(主按钮、激活态、链接) |
| `accent-soft` | `#e8f0ff` | 强调浅底 |
| `accent-hover` | `#1d4ed8` | 强调 hover |
| `accent-disabled` | `#bfdbfe` | 强调禁用 |
| `focus` | `#93c5fd` | focus ring |

### 状态(三件套:文字 / 浅底 / 边框)
每个状态有 `X`(文字)、`X-soft`(浅底)、`X-line`(边框)三个变体,组合用于徽章和提示框。
| 状态 | `X` text | `X-soft` bg | `X-line` border | 语义 |
|---|---|---|---|---|
| `success` | `#15803d` | `#f0fdf4` | `#bbf7d0` | 成功 / 已完成 / applied / 已连接 |
| `danger` | `#dc2626` | `#fef2f2` | `#fecaca` | 失败 / 危险 / 已断开 / discarded |
| `warning` | `#b45309` | `#fffbeb` | `#fde68a` | 警告 / 待处理 / pending / 等待审批 |
| `info` | `#1d4ed8` | `#eff6ff` | `#bfdbfe` | 信息 / 检查中 |

> 状态徽章直接用 `<Badge tone="success|danger|warning|info|accent|neutral">`,组件已把三件套烘进去。

### Diff(GitHub 风格)
| token | hex | 用途 |
|---|---|---|
| `diff-add` | `#e6ffec` | 新增行底色 |
| `diff-add-line` | `#aadfb8` | 新增行左边框 |
| `diff-remove` | `#ffebe9` | 删除行底色 |
| `diff-remove-line` | `#ffc9c9` | 删除行左边框 |

### 阴影
| token | 用途 |
|---|---|
| `shadow-panel` | 面板 / 卡片浮起 |
| `shadow-dialog` | 对话框 / 弹层 |

## 选色速查

- 文字 → `ink` / `ink-soft` / `ink-muted` / `ink-strong`
- 背景 → `canvas`(最底) / `surface`(卡片) / `surface-subtle`(次级)
- 边框 → `line` / `line-soft`
- 主操作 / 激活 → `accent`(+ `accent-hover` / `accent-disabled`);focus ring → `focus`
- 状态(成功 / 失败 / 警告 / 信息)→ `<Badge tone>`,或手动 `text-X` + `bg-X-soft` + `border-X-line`
- diff → `diff-add*` / `diff-remove*`

## 反模式

- ❌ 直接写 `bg-blue-50` / `text-green-700` / `focus:ring-blue-100` 等 Tailwind 原生色
- ❌ 手写 `function xxxStatusClass(): string` 返回状态色类 —— 改用 `<Badge tone>`
- ✅ 唯一例外:**分类色**(事件类别 / 错误子类型)用原生色板区分并列种类,见上方「原则」

## 来源

所有 token 定义在 [`tailwind.config.js`](tailwind.config.js)。**改色统一改那里**,不在组件里散写原生色。
