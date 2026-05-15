# Pi TUI vs Future TUI 差异审计报告

**日期：** 2026-05-15
**目标：** 对齐 Future TUI 与 Pi TUI 的实现，找出所有不一致之处

---

## 1. 事件处理架构

### 现状

| 方面 | Pi | Future |
|------|-----|--------|
| 事件来源 | `@mariozechner/pi-ai` SDK 流 | gRPC `StreamEvents` |
| 文本增量事件 | `text_delta`，带 contentIndex | `text_chunk`，纯文本 |
| 消息粒度 | 多层（message_start → text_start/delta/end → message_end） | 扁平化（agent_start/end + thinking_start/delta/end） |
| turn 事件 | `turn_start` / `turn_end` | 不存在 |
| usage 事件 | 嵌入在 message_end 中 | 独立 `usage` 事件 |
| 自动滚动 | 隐式，由 Container 渲染驱动 | 每个 handler 显式调用 `scrollToBottom()` |

### 差异

- **Future 不处理 `compaction_start` / `compaction_end` 事件**（见第 15 节）
- **事件流粒度不同**：Pi 在 message 层级有更细粒度的事件拆分，Future 在整个 agent 层级处理
- **tool 事件命名不同**：Pi 用 `toolcall_start/delta/end`，Future 用 `tool_start/delta/end`

**严重性：** DIFFERENT

---

## 2. 组件初始化

### Pi 组件列表
`Input` `Editor` `Markdown` `SelectList` `SettingsList` `Box` `Spacer` `Loader` `CancellableLoader` `Image` `Text` `TruncatedText`

### Future 组件列表
`Editor` `ChatArea` `Footer` `MarkdownRenderer` `AutocompletePopup` `AutocompleteManager` `SelectList` `SettingsList` `Box` `Loader` `CancellableLoader` `Image` `TruncatedText`

### 差异

| 组件 | Pi | Future | 严重性 |
|------|:--:|:------:|:------:|
| `Spacer` | 有 | 无 | MISSING |
| `Text` | 有 | 无 | MISSING |
| `Input`（单行） | 有 | 无（被 Editor 替代） | DIFFERENT |
| `ChatArea` | 无 | 有 | FUTURE ONLY |
| `Footer` | 无 | 有 | FUTURE ONLY |
| `AutocompletePopup` | 无 | 有 | FUTURE ONLY |
| `AutocompleteManager` | 无 | 有（与 pi 自动完成架构不同） | DIFFERENT |

**严重性：** MISSING × 2（Spacer, Text），DIFFERENT × 3

---

## 3. 键盘绑定

### Pi 全局绑定
- `enter` → 确认选择
- `escape / ctrl+c` → 取消选择
- `shift+enter` → 新行
- `tab` → 自动完成

### Future 全局绑定（app.ts:213-220）
```
ctrl+c → handleInterrupt（中断/退出）
ctrl+l → 清空聊天
ctrl+p → 切换模型
ctrl+r → 浏览会话
ctrl+s → 打开设置
ctrl+o → 工具输出展开/折叠
ctrl+t → 切换思考级别
shift+tab → 切换思考级别
```

### 差异

| 按键 | Pi 行为 | Future 行为 | 严重性 |
|------|---------|-------------|:------:|
| `ctrl+c` | 发送到编辑器作为取消选择 | `handleInterrupt()` 中断流或退出 | DIFFERENT |
| `ctrl+d` | 向前删除字符（编辑器） | 未绑定 | MISSING |
| `ctrl+l` | 清屏（终端级别） | 清空聊天记录 | DIFFERENT |
| `ctrl+p` | 切换模型 | 切换模型 | 匹配 |
| `ctrl+o` | 展开/折叠工具输出 | 展开/折叠工具输出 | 匹配 |
| `ctrl+t` | 切换思考级别 | 切换思考级别（+ shift+tab） | 匹配 |
| `ctrl+r` | 无 | 浏览会话 | FUTURE ONLY |
| `ctrl+s` | 无 | 打开设置 | FUTURE ONLY |

### Future 中缺失的 Pi 编辑器绑定
`ctrl+b` `ctrl+f` `ctrl+w` `alt+d` `ctrl+u` `ctrl+y` `alt+y` `ctrl+-` `ctrl+]` `ctrl+alt+]`

**严重性：** MISSING × 1（ctrl+d），DIFFERENT × 5

---

## 4. Footer/快捷栏

### Pi
无专门的 Footer 组件。状态信息分散在组件中。

### Future
独立的 `Footer` 组件（footer.ts），显示：
- 左侧：cwd、模型名称、思考级别
- 右侧：token 统计（↑ ↓）、成本、上下文使用百分比（绿色<70%，黄色 70-90%，红色>90%）
- 自动压缩指示器 `(auto)`

### 差异
Future 的 Footer 功能更完善，但 Pi 没有等效实现。

**严重性：** DIFFERENT（但 Future 更优）

---

## 5. 状态管理

### Pi
通过 `MutableAgentState` 管理，字段：
`isStreaming` `streamingMessage` `pendingToolCalls` `errorMessage` `model` `thinkingLevel` `systemPrompt` `messages` `tools`

### Future
App 内部状态对象（app.ts:112-131）：
`model` `thinking` `streaming` `sessionId` `sessionName` `cwd` `version` `skills[]` `contextFiles[]` `extensions[]` `contextTokens/Window/Percent` `tokensIn/Out` `totalCost` `thinkingHidden` `explicitSession`

### 差异

| 字段 | Pi | Future | 严重性 |
|------|:--:|:------:|:------:|
| streamingMessage | 有 | 无 | MISSING |
| pendingToolCalls | 有（Set） | 无 | MISSING |
| context stats | 无 | 有 | FUTURE ONLY |
| thinkingHidden | 无 | 有 | FUTURE ONLY |

**严重性：** MISSING × 2

---

## 6. 模型切换（Ctrl+P）

### Pi
通过 Agent 内部的 `cycleModel()` 实现。

### Future
app.ts:894-903，调用 `client.cycleModel()`（gRPC），更新本地 `state.model` 和 `state.thinking`。

**严重性：** 匹配（实现机制不同但行为一致）

---

## 7. 思考级别（Ctrl+T）

### Pi
通过 Agent `thinkingLevel` 字段，服务器端循环。

### Future
app.ts:911-917，调用 `client.cycleThinkingLevel()`，更新 `state.thinking`。额外有 `toggleThinking()` 控制 `thinkingHidden`（折叠/展开思考内容），Pi 无此功能。

**严重性：** DIFFERENT（Future 多了 thinkingHidden 切换）

---

## 8. 中断（Ctrl+C）

### Pi
`ctrl+c` 被编辑器拦截作为"取消选择"，不会退出程序。中断通过 Agent `abort()`。

### Future
app.ts:640-654，`ctrl+c` 被 App 直接在原始字节 `\x03` 级别拦截：
- 流式传输中 → 调用 `client.abort()` + 添加 "(aborted)" 消息
- 非流式 → **退出程序 (process.exit(0))**

**严重性：** DIFFERENT（Future 在非流式状态下会退出程序，Pi 不会）

---

## 9. 工具展开（Ctrl+O）

### Pi
通过内部 `toolOutputExpanded` 状态切换，显示截断输出 + "N more lines" 提示。

### Future
app.ts:634-638，ChatArea 实现：
- 预览模式：最多 20 行 + "... N more lines (Ctrl+O to expand)" 页脚
- 展开模式：全部行 + "(Ctrl+O to collapse)" 提示
- 按状态颜色编码背景（pending=蓝，success=绿，error=红）
- 每工具类型格式化显示（bash → `$ command`，read → `read path` 等）

**严重性：** 功能匹配，实现细节不同

---

## 10. Overlay 系统

### Pi
完整 overlay 系统（tui.ts:329-434）：
- `showOverlay(component, options)` + `hideOverlay()`
- `resolveOverlayLayout` 支持锚点、边距、百分比值
- `compositeOverlays` 使用 ANSI 片段感知的组合
- 宽度溢出保护（visibleWidth 检查）
- maxHeight 限制

### Future
简化 overlay 系统（app.ts:929-1080）：
- 相同的基础 API（showOverlay/hideOverlay/restoreFocus）
- `compositeLineAt` 使用 `stripAnsiCodes` 简化处理（Pi 使用更复杂的 segment 提取）
- 自动完成在 overlay 之后手动组合

**严重性：** DIFFERENT（Pi 的 overlay 组合更复杂、更正确）

---

## 11. RPC/连接

### Pi
HTTP/SSE 协议：
- `fetch` + `ReadableStream`
- `@mariozechner/pi-ai` SDK
- Bearer token 认证

### Future
gRPC 协议：
- `@grpc/grpc-js`
- proto: `future.proto`
- `ExecuteCommand`（一元）+ `StreamEvents`（服务端流）
- 不安全凭证
- 断线自动重连（2 秒超时）

**严重性：** DIFFERENT（但这是架构选择，不需要对齐）

---

## 12. 启动/初始化序列

### Pi
1. 创建 `ProcessTerminal`
2. 创建 `TUI(terminal)`
3. 添加组件
4. `tui.start()` → 启用原始模式、Kitty 协议
5. 处理初始 prompt
6. `terminal.stop()` 清理

### Future
1. 创建 `NodeTerminal`
2. 创建 `App`（含 GrpcClient/Editor/ChatArea/Footer 等）
3. `app.start()` → gRPC 连接 + 会话设置
4. `showWelcome()` 显示横幅和快捷方式
5. 处理初始 prompt（`--continue`, `--fork`, `--resume` 等）

**严重性：** DIFFERENT

---

## 13. 主题

### Pi
最小主题（8 个颜色）：
`bg` `fg` `accent` `border` `selectedBg` `selectedFg` `dimFg` `error` `success`

### Future
全面主题（40+ 颜色变量）：
包含 Markdown 颜色、工具渲染颜色、思考颜色、消息背景颜色
辅助函数：`fg()` `bg()` `bold()` `dim()` `italic()` `fgRaw()` `bgRaw()` 等

**严重性：** DIFFERENT（Future 更全面）

---

## 14. 图片（Kitty 协议）

### 两者都实现了：
- `extractKittyImageIds` / `collectKittyImageIds` / `deleteKittyImages`
- `queryCellSize` + `consumeCellSizeResponse`
- `expandLastChangedForKittyImages` + `deleteChangedKittyImages`
- `isImageLine` 宽度溢出保护

**严重性：** 功能匹配 ✓

---

## 15. Compaction 事件处理（🔴 高优先级）

### Pi
Agent 引擎在 compaction 期间发出 `compaction_start` 和 `compaction_end` 事件。TUI 层捕获这些事件显示加载指示器。

### Future
`handleAgentEvent()`（app.ts:351-433）**完全不处理 compaction 事件**。switch 语句中无相关 case，事件到达时走 `default: break` 被丢弃。

Future 仅有 `/compact` 斜杠命令（app.ts:700-716），但这是手动操作，不是流式事件处理。

**严重性：** 🔴 **MISSING** — compaction 期间 UI 无反馈

---

## 16. 其他差异

### 16a. 渲染引擎
基本一致（双阶段调度、16ms 间隔、同步输出、差异渲染）。以下细节不同：
- Future 不增加 `fullRedrawCount`，Pi 会
- 调试日志路径：Future → `~/.future_tui/`，Pi → `~/.pi/agent/`
- `PI_TUI_DEBUG` 输出格式不同

**严重性：** DIFFERENT（微小）

### 16b. 渲染管线
Future 有额外的步骤（Footer 数据准备、自动完成组合、行过滤），Pi 没有。

**严重性：** DIFFERENT

### 16c. 编辑器
- Future：**多行**编辑器，支持视觉模式、重做、历史导航、滚动指示器
- Pi：**单行**编辑器，水平滚动、IME 支持

**严重性：** DIFFERENT（架构选择）

### 16d. 鼠标追踪
Future 启用 SGR 鼠标追踪（滚轮支持），Pi 没有。

**严重性：** FUTURE ONLY

### 16e. Windows VT 输入
Pi 通过 `koffi` 支持 Windows VT 输入，Future 无。

**严重性：** MISSING（仅影响 Windows 用户）

---

## 优先级排序

### 🔴 高优先级（应尽快修复）
1. **Compaction 事件处理** — 完全缺失，compaction 发生时 UI 无反馈
2. **Ctrl+C 行为** — Future 非流式时退出程序，与 Pi 不一致

### 🟡 中优先级
3. **Ctrl+D 删除** — 缺失编辑器功能
4. **Ctrl+L 行为** — Future 清空聊天而非清屏
5. **Overlay 组合** — 简化实现可能导致 ANSI 片段重叠问题

### 🟢 低优先级 / 架构差异（不需要对齐）
6. 事件源协议（gRPC vs HTTP/SSE，已确定的架构选择）
7. 编辑器模式（多行 vs 单行，Future 有意为之）
8. 主题系统（Future 更全面，不需回退）
9. Windows VT 支持（大多数用户用 Unix）
10. 鼠标追踪（Future 独有功能）
