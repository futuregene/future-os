# Pi TUI vs Xihu TUI — 完整差异对比

## 1. 架构与设计模式

| 方面 | Pi TUI | Xihu TUI |
|------|--------|----------|
| 组件模式 | 完整 Component 接口 (`render`, `handleInput`, `invalidate`, `wantsKeyRelease`) | 无统一组件接口，每个组件自行定义 API |
| 容器模式 | `Container extends Component`，支持 `addChild`/`removeChild`/`clear`，递归 render | 无容器概念，App 直接持有所有组件引用 |
| TUI 类 | `TUI extends Container`，继承所有容器能力 | `App` 是独立类，不继承任何基类 |
| 焦点管理 | `Focusable` 接口 + `CURSOR_MARKER`，IME 光标定位 | 无焦点系统，editor 始终接收输入 |
| InputListener | 链式 InputListener 管道，支持拦截/修改输入 | 无此机制，直接分发到 handleKey |
| Overlay 系统 | 完整的 overlay 栈，支持 anchor/百分比定位、margin、visible 回调、nonCapturing、focus 排序 | 简单的 `Overlay` 联合类型，手动居中定位 |
| OverlayHandle | 返回 handle 对象，支持 hide/setHidden/focus/unfocus/isFocused | 无此概念 |

## 2. 渲染引擎 (doRender)

| 方面 | Pi TUI | Xihu TUI |
|------|--------|----------|
| 渲染调度 | `process.nextTick()` + `setTimeout` 双阶段 | 仅 `setTimeout` |
| force render | 将 previousWidth/Height 设为 -1 触发 full redraw | 清空 previousLines，设为 0 |
| 视口追踪 | `previousViewportTop`、`maxLinesRendered`、`cursorRow`、`hardwareCursorRow` 完整追踪 | 仅有 `hardwareCursorRow` |
| clearOnShrink | 可配置（`PI_CLEAR_ON_SHRINK` 环境变量），内容缩小时清除空白行 | 无此机制 |
| 内容增长处理 | 追加模式：检测 `appendedLines`，从 `previousLines.length` 开始渲染 | 无追加模式，始终全量比较 |
| 行宽验证 | 检测 `visibleWidth(line) > width`，写 crash log 并抛出错误 | 无验证 |
| 调试日志 | `PI_DEBUG_REDRAW` 环境变量输出重绘原因到 `~/.pi/agent/pi-debug.log` | 无 |
| 调试 dump | `PI_TUI_DEBUG` 环境变量 dump 完整渲染状态到 `/tmp/tui/` | 无 |
| overlay 合成 | `compositeOverlays()` 将 overlay 合成到 lines 中，在 diff 比较之前 | 直接在 doRender 中写入 next 数组 |
| line resets | `applyLineResets()` 在每行末尾添加 `\x1b[0m\x1b]8;;\x07` | 无统一 reset，各组件自行处理 |
| Kitty 图片 | 完整的图片 ID 追踪、删除、变更检测 | 不支持 |
| 删除行处理 | 计算 extraLines，逐行清除并移动光标回位 | 无专门的删除行处理 |

## 3. 终端管理 (Terminal)

| 方面 | Pi TUI (ProcessTerminal) | Xihu TUI (NodeTerminal) |
|------|--------------------------|------------------------|
| 生命周期 | `start(onInput, onResize)` / `stop()` | 无 start/stop，App 自行管理 |
| stdin drain | `drainInput()` 退出前排空 stdin，防止按键泄漏到 shell | 无 |
| 原始模式 | 保存 `wasRaw` 状态，stop 时恢复 | 直接设置，不恢复 |
| Kitty 键盘协议 | 查询 `\x1b[?u`，检测响应，启用 `\x1b[>7u`（flag 1+2+4） | 不支持 |
| modifyOtherKeys | Kitty 超时后 fallback 到 `\x1b[>4;2m` | 不支持 |
| bracketed paste | 启用 `\x1b[?2004h`，stop 时关闭 | 不支持 |
| StdinBuffer | 内置 StdinBuffer，10ms 超时，拆分批量输入为单个序列 | 无，char-by-char 循环处理 |
| paste 事件 | StdinBuffer 检测 paste，重新包装 `\x1b[200~...\x1b[201~` 后 emit | 无 |
| SIGWINCH | 启动时发送 `SIGWINCH` 刷新终端尺寸 | 无 |
| Windows VT Input | 通过 koffi 调用 kernel32 启用 `ENABLE_VIRTUAL_TERMINAL_INPUT` | 无 |
| 终端标题 | `setTitle()` 通过 OSC 0 设置窗口标题 | 无 |
| 进度指示 | `setProgress()` 通过 OSC 9;4 显示不确定进度条 | 无 |
| 写入日志 | `PI_TUI_WRITE_LOG` 环境变量，记录所有 stdout 输出 | 无 |
| columns/rows | getter 属性 | 方法 `getWidth()`/`getHeight()` |
| clearScreen | `\x1b[2J\x1b[H` (清除屏幕并移到 home) | `\x1b[2J\x1b[H` (相同) |
| clearLine | `\x1b[K` (无参数 2) | `\x1b[2K` |

## 4. 输入处理与按键 (Keys)

| 方面 | Pi TUI | Xihu TUI |
|------|--------|----------|
| 按键匹配 | `keys.ts` (1400行)，支持 Kitty CSI-u、modifyOtherKeys、legacy 三种协议 | `parseEscSeq()` (20行) + `charToKeyEvent()` (10行) |
| 类型安全 KeyId | `KeyId` 联合类型 + `Key` helper 对象，编译时 autocomplete | 无类型，运行时字符串匹配 |
| 按键解析 | `parseKey()` 返回标准 keyId 字符串 | `parseEscSeq()` 返回自定义 KeyEvent |
| 修饰键 | 支持 ctrl/shift/alt/super 全部组合（单/双/三修饰键） | 仅 ctrl、shift |
| Kitty 协议 | 完整解析：CSI u、CSI 箭头键、CSI 功能键、Home/End 变体 | 不支持 |
| modifyOtherKeys | 解析 `\x1b[27;mod;code~` 格式 | 不支持 |
| 功能键 | F1-F12 全部支持 | 不支持 |
| 按键释放检测 | `isKeyRelease()` / `isKeyRepeat()` 检测 Kitty flag 2 事件 | 不支持 |
| Kitty printable 解码 | `decodeKittyPrintable()` 从 CSI-u 序列提取可打印字符 | 不支持 |
| 非拉丁键盘 | 通过 baseLayoutKey fallback 支持非拉丁键盘布局 | 不支持 |
| 重映射键盘防护 | 检测已知 Latin 字母和符号，避免 Dvorak/Colemak 误匹配 | 不支持 |
| KeybindingsManager | `keybindings.ts` (245行)，可配置绑定、冲突检测、用户覆盖 | 无，全部硬编码在 `handleKey()` |
| StdinBuffer | 批量输入拆分为单个序列，处理部分 escape 序列 | 无 |
| escape 超时 | 无（由 StdinBuffer 处理） | 50ms timeout 检测 standalone Escape |

## 5. 文本处理 (Utils)

| 方面 | Pi TUI (utils.ts, 1141行) | Xihu TUI |
|------|---------------------------|----------|
| visibleWidth | Intl.Segmenter + east-asian-width + emoji 检测 + 宽度缓存 | 仅 `stripAnsi().length` |
| wrapTextWithAnsi | 完整词换行，AnsiCodeTracker 跨行保留样式 | 无，由 marked 库处理 markdown |
| AnsiCodeTracker | 追踪 bold/dim/italic/underline/blink/inverse/hidden/strikethrough + fg/bg 颜色 + OSC 8 超链接 | 无 |
| applyBackgroundToLine | 先 pad 再 bg，保证全宽背景 | 无（各组件自行处理） |
| truncateToWidth | 支持 ANSI、emoji、CJK，可选省略号和 padding | 无 |
| sliceByColumn / sliceWithWidth | 按可见列提取子串，支持 strict 模式 | 无 |
| extractSegments | 单次遍历提取 before/after 段落，用于 overlay 合成 | 无 |
| extractAnsiCode | 处理 CSI、OSC、APC 序列 | 无 |
| normalizeTerminalOutput | 泰文/老挝文 AM 元音兼容性分解 | 无 |
| 制表符处理 | 替换为 3 空格 | 未处理 |
| graphemeWidth | 零宽字符检测、emoji 预过滤、RGI_Emoji regex、区域指示符、east-asian-width | 无 |

## 6. Editor 编辑器

| 方面 | Pi TUI (editor.ts, 2293行) | Xihu TUI (editor.ts, 226行) |
|------|----------------------------|----------------------------|
| 行数 | 多行编辑，可视化行换行 | 单行 |
| 布局 | wordWrapLine + visualLineMap，TextChunk 接口 | 无 |
| 撤销 | UndoStack\<EditorState\>，结构化克隆 | 无 |
| Kill Ring | Emacs 风格 kill/yank/yank-pop，accumulate 模式 | 无 |
| 字符跳转 | forwardCharJump / backwardCharJump | 无 |
| 粘贴标记 | >10行或>1000字符时插入 paste markers，submit 时展开 | 无 |
| 自动补全 | AutocompleteProvider 集成，debounce、AbortController、请求队列 | 无（App 中简单实现） |
| 斜杠命令补全 | 上下文检测、命令名补全、参数补全 | 仅命令名补全 |
| 文件路径补全 | Tab 触发，支持 fd 模糊搜索 | 不支持 |
| 快捷键 | Ctrl+A/E/K/U/W、Alt+D/Backspace、Home/End、PageUp/Down | 仅 left/right/home/end/backspace/delete/ctrl+A/E/U/W |
| Shift+Enter | 换行 | 不支持 |
| 光标渲染 | 反向视频 + CURSOR_MARKER（用于 IME 定位） | 反向视频 |
| 滚动指示 | "↑ N more" / "↓ N more" 边框提示 | 无 |
| 历史导航 | Up/Down 在空编辑器时导航历史 | Up/Down 在非空时也导航历史 |
| 边框 | 可配置边框颜色 | 无边框 |
| paddingX | 可配置水平内边距 | 无 |
| bracketed paste | 通过 StdinBuffer 重新包装 | 不支持 |
| segmentWithMarkers | paste marker 感知的 grapheme 分割 | 无 |

## 7. Markdown 渲染

| 方面 | Pi TUI (markdown.ts, 853行) | Xihu TUI (markdown.ts, 267行) |
|------|------------------------------|------------------------------|
| 架构 | 实现 Component 接口 | 独立 MarkdownRenderer 类 |
| 文本样式 | DefaultTextStyle 接口（color, bgColor, bold, italic, strikethrough, underline） | 仅使用 fg/bold/italic/dim 函数 |
| 删除线 | StrictStrikethroughTokenizer，严格的 `~~text~~` 匹配 | 不支持 |
| 超链接 | OSC 8 超链接渲染（可点击链接） | 仅颜色高亮，不可点击 |
| 引用块 | 嵌套 block-level token 渲染，border 前缀 | 简单的 `│ ` 前缀 |
| 列表 | 嵌套深度跟踪，有序/无序支持 | 简单列表渲染 |
| 代码块 | highlightCode 回调支持 | 无回调 |
| 表格 | 动态列宽计算、单元格换行、行分隔符 | 静态列宽计算 |
| 渲染缓存 | cachedText/cachedWidth/cachedLines | 无缓存 |
| 背景色 | 通过 applyBackgroundToLine 在 padding 阶段应用 | 内联 bg() |
| 内边距 | paddingX/paddingY 支持 | 无 |
| 内容换行 | 通过 wrapTextWithAnsi 词换行 | 简单按空格分割 |
| setText | 支持动态更新文本 | 无 |
| 图片行 | isImageLine 检测，透传图片序列 | 无 |
| 样式前缀 | stylePrefix 追踪，在嵌套 inline token 的 RESET 后重新应用样式 | 无 |

## 8. 组件对比

### Pi TUI 独有组件

| 组件 | 描述 |
|------|------|
| **Box** | 容器组件，支持 paddingX/paddingY、背景色、渲染缓存 |
| **Text** | 多行文本，词换行 + 背景色 + padding |
| **TruncatedText** | 单行截断文本，省略号支持 |
| **Spacer** | 空白行分隔 |
| **Loader** | 旋转动画加载器（braille 字符），可配置帧和间隔 |
| **CancellableLoader** | 继承 Loader，按 Escape 可取消 |
| **Image** | Kitty/iTerm2 图片渲染，回退到文本占位符 |
| **Input** | 单行文本输入（503行），支持 UndoStack、KillRing、bracketed paste、Kitty printable 解码、光标渲染 |
| **SettingsList** | 设置列表，支持子菜单、模糊搜索、值循环 |

### Xihu TUI 独有组件

| 组件 | 描述 |
|------|------|
| **ChatArea** | 聊天视图，消息渲染、thinking 显示、tool call 状态 |
| **Footer** | 状态栏，显示 pwd/model/thinking/token 统计/上下文使用率 |
| **AutocompletePopup** | 简单自动补全弹窗（硬编码斜杠命令列表） |

### 共同组件的差异

| 组件 | Pi TUI | Xihu TUI |
|------|--------|----------|
| **SelectList** | 实现 Component + handleInput，KeybindingsManager，两列布局，滚动指示，setSelectedIndex/setFilter，onSelectionChange 回调 | 独立类，字符串匹配按键，简单渲染 |
| **Editor** | 见上文 Editor 章节 | 见上文 Editor 章节 |
| **Markdown** | 见上文 Markdown 章节 | 见上文 Markdown 章节 |

## 9. 样式/主题系统

| 方面 | Pi TUI | Xihu TUI |
|------|--------|----------|
| 主题方式 | chalk 风格的函数式主题（(text) => string） | 256 色数字常量 + fg/bg 辅助函数 |
| 颜色函数 | 不追加 RESET，允许组合 | 每个函数末尾追加 RESET |
| Theme 接口 | 无统一 Theme 类型，各组件的 theme 独立定义 | 有统一的 Theme 接口，包含 20+ 字段 |
| 颜色定义 | 无集中颜色常量 | `C` 常量对象，集中管理 |

## 10. RPC/通信

| 方面 | Pi TUI | Xihu TUI |
|------|--------|----------|
| 通信方式 | 进程内（与 Go backend 编译在一起） | HTTP + SSE（与独立 xihu server 通信） |
| RPC 协议 | 无（直接调用） | JSON-RPC over HTTP + SSE 事件流 |
| 会话管理 | 由 Go backend 管理 | RpcClient 封装所有 RPC 方法 |

## 11. 依赖

| 依赖 | Pi TUI | Xihu TUI |
|------|--------|----------|
| marked | ^15.0.12 | ^18.0.3 |
| chalk | ✓ | ✗ |
| get-east-asian-width | ✓ | ✗ |
| mime-types | ✓ | ✗ |
| koffi (optional) | ✓ (Windows VT input) | ✗ |
| @xterm/headless (dev) | ✓ (测试) | ✗ |
| TypeScript 编译器 | tsgo | tsc |
| Node engines | >=20.0.0 | 无要求 |

## 12. 其他差异

| 方面 | Pi TUI | Xihu TUI |
|------|--------|----------|
| 退出时光标定位 | stop() 中移动到内容末尾 + `\r\n` | stop() 中同样处理 |
| 光标隐藏 | 启动时 `hideCursor()`，仅在 showHardwareCursor 时显示 | 启动时 `hideCursor()` |
| 鼠标支持 | Mouse tracking 支持（通过 StdinBuffer 处理） | 有 MOUSE_TRACK_ON/OFF 常量但未启用 |
| 终端尺寸查询 | 查询 cell size `\x1b[16t`（用于图片渲染） | 不支持 |
| Termux 适配 | `isTermuxSession()` 检测，Termux 中高度变化不做 full redraw | ✅ 已实现 (2026-05-14) |
| 错误处理 | 行宽溢出抛出 Error 并写 crash log | ✅ 已实现 (2026-05-14)，写 `~/.xihu/crash.log` |
| debug 日志 | `PI_DEBUG_REDRAW=1` 写 `~/.pi/agent/pi-debug.log` | ✅ 已实现 (2026-05-14)，写 `~/.xihu/debug.log` |
| onDebug 回调 | Shift+Ctrl+D → onDebug | ✅ 已实现 (2026-05-14) |
| fullRedrawCount | 追踪 full redraw 次数 | ✅ 已实现 (2026-05-14)，`getFullRedrawCount()` |
| hardwareCursor getter/setter | `getShowHardwareCursor()`/`setShowHardwareCursor()` | ✅ 已实现 (2026-05-14) |
| clearOnShrink getter/setter | `getClearOnShrink()`/`setClearOnShrink()` | ✅ 已实现 (2026-05-14) |
| matchesKey() | 符号化 key matching 函数 | ✅ 已实现 (2026-05-14)，~180 行完整实现 |
| extractSegments strictAfter | strictAfter + width tracking | ✅ 已实现 (2026-05-14) |
| normalizeTerminalOutput | NFD 兼容性分解 (Thai/Lao AM) | ✅ 已实现 (2026-05-14) |
| KeybindingManager 符号 ID | 符号化 keybinding ID + 用户覆盖 | ✅ 已实现 (2026-05-14)，`add()` 返回 Symbol ID，`applyOverrides()` 支持用户覆盖 |
| KeyId 模板字面量类型 | 编译时类型安全的联合类型 | ✅ 已实现 (2026-05-14)，`BaseKey | Mod+BaseKey | Mod+Mod+BaseKey | Mod+Mod+Mod+BaseKey` |
| Windows VT Input | koffi kernel32 调用 | Unix only，不适用 |
| Autocomplete @attachment | @ 触发 fd 模糊文件搜索 | ✅ 已实现 (2026-05-14)，AttachmentProvider 通过 fd 模糊搜索文件 |
| Editor jump mode | f{char} 字符跳转 | ✅ 已实现 (2026-05-14)，ctrl+f/ctrl+shift+f 触发 jump 模式 |
| PI_TUI_WRITE_LOG | 记录所有 stdout 输出到文件 | ✅ 已实现 (2026-05-14)，write() 中检查环境变量，写入 ~/.xihu/write.log |
| PI_TUI_DEBUG dump | dump 渲染状态到 /tmp/tui/ | ✅ 已实现 (2026-05-14)，fullRender 时 dump 状态到 /tmp/tui/render-{ts}.log |
| tsgo 构建 | 使用 tsgo 替代 tsc | ✅ 已实现 (2026-05-14) |
