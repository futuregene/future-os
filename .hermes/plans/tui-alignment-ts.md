# xihu TUI → TS pi-mono TUI 对齐分析

**生成时间**: 2026-05-08
**参考实现**: `/Users/geilige/pi-mono/packages/coding-agent/src/modes/interactive/`

---

## 一、Footer（关键差异）

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **Context%** | ✅ 显示 `73.2%/128k (auto)`，>90%红 >70%黄 | ❌ 已移除（注释写 "ctx:-- confusing"） | 恢复 context% 显示 |
| **auto-compact 标志** | ✅ 若开启 auto-compact，显示 `(auto)` | ❌ 无 | 添加 |
| **model/thinking 颜色** | `dim` 灰色（无强调） | 蓝色 `#61afef` + Bold | 改为 dim 灰色 |
| **provider 前缀** | 仅当 `providerCount > 1` 时显示 `(openai)` | 始终尝试显示 | 条件化 |
| **stats 格式化** | ↑12.3k ↓5.1k R2.4k W100 $0.042 | 相同 ✓ | - |
| **Line1 格式** | `~cwd (branch) • sessionName` | 相同 ✓ | - |
| **扩展行** | 字母排序，空格分隔，dim 色 | 相同 ✓ | - |

**⚠️ 注意**: TS 的 Stats 和 rightSide 用**两个独立的 dim 包裹**（因为 context% 可能含 ANSI reset），xihu footer 只用一个 dim。

---

## 二、启动横幅（Welcome Message）

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **Expand/Collapse** | `ExpandableText` 组件，折叠时一行提示，展开时全快捷键 | 始终显示 version + keybinding + skills | 实现折叠/展开 |
| **折叠内容** | `Pi v1.x · interrupt键 interrupt · / commands · expand键 more` | N/A | 添加 |
| **展开内容** | 完整快捷键表 + skills + extensions | N/A | 添加 |
| **Quiet mode** | 若 `quietStartup`，header 为空白 | 无 | 可后续添加 |

---

## 三、中断处理（Enter / Escape）

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **Enter 行为** | **steer 注入**（不中断，注入当前回合） | Abort + 重新开始 | 改为 steer 模式 |
| **Alt+Enter** | followUp（排队等回合结束） | 无 | 添加 followUp |
| **Escape 键** | 流式中 → abort streaming | 无 abort 键 | 添加 Escape abort |
| **Ctrl+C** | 第一按清空编辑器，500ms内第二按退出 | Ctrl+C 直接退出（条件：Empty && !streaming） | 对齐 |
| **Backslash+Enter** | 若无 Shift+Enter，`\Enter` 插入换行 | 无 | 添加 workaround |

---

## 四、Thinking 显示

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **切换方式** | 全局 `hideThinkingBlock` 二进制：全隐藏或全显示 | 每条 thinking 单独展开/折叠 | 改为全局 toggle |
| **隐藏态** | 斜体 `thinkingText` 色显示 "Thinking..." 标签 | N/A（每条折叠态显示不同内容） | 改为统一 "Thinking..." |
| **可见态** | thinking 内容作为 Markdown 渲染，斜体 + thinkingText 色 | 可见 ✓ | - |
| **Shift+Tab 切换** | `cycleThinking` 循环 off→low→medium→high→xhigh | 相同 ✓ | - |

---

## 五、工具调用显示

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **状态颜色** | 背景色：pendingBg / errorBg / successBg | 无背景色 | 添加背景色状态 |
| **默认展开** | ❌ 默认折叠（expanded = false） | ✅ 显示参数 | 改为默认折叠 |
| **bash 耗时** | "Took X.Xs" / "Elapsed X.Xs" | 无 | 添加耗时显示 |
| **工具标签** | 加粗工具名 + toolTitle 色 | 工具名 ✓ | - |
| **折叠预览** | bash: 最后 20 行；write/read: file_path | file_path ✓ | - |

---

## 六、Help Overlay

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **渲染方式** | 内联 Markdown 追加到 chatContainer | **Overlay 弹窗** | 改为内联渲染 |
| **内容** | Navigation / Editing / Other 三表格 + Extensions | KeyMap 分组（Global/Editor/Chat/Tools） | 对齐为三表格 |
| **键位显示** | 动态从 KeybindingsManager 读取 | 静态常量 | 可后续动态化 |
| **触发** | `/hotkeys` 命令 + `ctrl+o` | `ctrl+o` + `/hotkeys` | 相同 ✓ |

---

## 七、Enter 换行

| 方面 | TS pi-mono | xihu (当前) | 修复 |
|------|-----------|------------|------|
| **提交键** | Enter | Enter | ✓ |
| **换行键** | Shift+Enter（默认），Ctrl+J 也视作换行 | Ctrl+J | 添加 Shift+Enter |
| **Backslash workaround** | `\Enter` → 插入换行 | 无 | 添加 |

---

## 修改优先级

**P0（必须）**:
1. Footer: 恢复 context%，model/thinking 改 dim 灰色，provider 条件化
2. Interrupt: Enter → steer（不 abort），Escape → abort

**P1（重要）**:
3. Welcome: ExpandableText 折叠/展开
4. Thinking: 全局 toggle（hideThinkingBlock）
5. Tool: 状态背景色 + 默认折叠 + 耗时

**P2（改进）**:
6. Help: 内联 Markdown 渲染
7. Alt+Enter followUp
8. Backslash+Enter workaround

---

## 涉及文件

- `internal/tui/components/footer.go` — context%, dim color, provider logic
- `internal/tui/app.go` — interrupt steer vs abort, Escape handler, welcome expand/collapse, help inline
- `internal/tui/components/chat_viewport.go` — thinking global toggle, tool status colors + defaults
- `internal/tui/components/editor.go` — Shift+Enter newline, backslash workaround
