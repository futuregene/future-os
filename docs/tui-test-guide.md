# TUI 测试指南

> 本指南供大模型在 tmux 中交互式测试 FutureOS TUI。每次改完 TUI / agent 代码后，按本文档逐步执行验证。

## 准备

1. 确认 agent 已启动（`pgrep future-agent`，否则 `make run-agent &`）
2. 在 tmux 中启动 TUI，终端尺寸 120×40：

```bash
SESSION="tui-test"
tmux kill-session -t "$SESSION" 2>/dev/null; sleep 0.5
tmux new-session -d -s "$SESSION" -x 120 -y 40 \
  "cd /Users/geilige/future-os/tui && bun run src/index.ts 2>&1"
sleep 3
```

3. 操作 TUI 的基本方式：

| 操作 | 命令 |
|------|------|
| 发送按键 | `tmux send-keys -t tui-test "<keys>"` |
| 发送 Enter | 在 keys 后加 `Enter`，如 `send-keys "/help" Enter` |
| 清除输入 | `send-keys C-u` |
| 关闭弹窗 | `send-keys Escape` |
| 截取画面 | `tmux capture-pane -t tui-test -p -e` |
| 截取尾部 | `tmux capture-pane -t tui-test -p -e -S -<行数>` |

4. 每步之间 `sleep <秒>` 等待渲染或 agent 响应。

---

## 自动测试步骤

按顺序执行以下 14 项测试。每项包含：目的、发送的按键、等多久、检查什么。

### 1. `/help` —— 帮助弹窗

```
目的: 验证 overlay 正常显示和关闭
操作: 输入 /help → Enter → 等 0.5s → 截屏检查 → Escape 关闭
检查: 屏幕出现 "Terminal UI Help" 或帮助内容表格
```

### 2. `/status` —— 初始会话状态

```
目的: 验证状态字段正确（初始 Queries=0）
操作: C-u 清输入 → 输入 /status → Enter → 等 0.5s → 截屏
检查:
  - 显示 "Queries: 0"
  - 显示当前 model 名（含 deepseek 字样）
  - 显示 CWD、Thinking、Context 等字段
```

### 3. 简单提问 —— Agent 基础响应

```
目的: 验证 Agent 能正常回复
操作: C-u 清输入 → 输入 "What is 2+2? Answer with just the number." → Enter
      等 8s → 截屏
检查: 屏幕出现 "4"（Agent 的回答）
```

### 4. `/status` —— 提问后 Queries 递增

```
目的: 验证 Queries 计数正确
操作: C-u → /status → Enter → 等 0.5s → 截屏
检查: 显示 "Queries: 1"
Escape 关闭（如有弹窗）
```

### 5. Tool call (bash) —— 工具调用展示

```
目的: 验证工具调用展示格式（`$ cmd` / `read path` 等）
操作: C-u → 输入 "Run ls in the current directory" → Enter → 等 12s → 截屏尾部 30 行
检查:
  - 出现 "$ ls" 格式的 bash 工具调用
  - 出现 ls 的输出内容（如 src/ 目录）
  - 工具调用后 Agent 有回复文字
```

### 6. `/status` —— 两次提问后 Queries=2

```
目的: 验证多次提问后计数
操作: C-u → /status → Enter → 等 0.5s → 截屏
检查: 显示 "Queries: 2"
Escape 关闭
```

### 7. `/new` —— 创建新会话

```
目的: 验证新会话创建
操作: C-u → /new → Enter → 等 1s → 截屏
检查: 出现 "New session" 或 session ID
```

### 8. `/sessions` —— 会话列表

```
目的: 验证会话列表展示（name、queryCount、时间三列对齐）
操作: C-u → /sessions → Enter → 等 1.5s → 截屏尾部 40 行
检查:
  - 列表项不含原始 session ID（有 name 或 first_message 作为标签）
  - 每项显示 "NQ" 格式的 query 计数（如 "2Q"）
  - 每项显示 model 名和更新时间
Escape 关闭
```

### 9. `ctrl+p` —— 模型切换

```
目的: 验证模型循环切换
操作: Escape 关闭弹窗 → 等 0.2s → ctrl+p → 等 1s → 截屏底部
检查: Footer 中 model 名发生变化（不再是之前的模型）
```

### 10. 切换到历史 session

```
目的: 验证 session 切换和消息加载
操作: C-u → /sessions → Enter → 等 1.5s
      按 Down 两次选中一个历史 session → Enter
      等 3s → 截屏尾部 20 行
检查: 出现 "Switched to session" 或切换提示
```

### 11. 历史消息渲染

```
目的: 验证历史 session 中工具调用格式正确（非 call_id）
操作: 在切换后的 session 中截全屏
检查（逐一确认）:
  - bash 工具展示格式为 "$ 命令"（非 "bash" 或 call_xxx）
  - read 工具展示格式为 "read 文件路径"
  - write 工具展示格式为 "write 文件路径"
  - edit 工具展示格式为 "edit 文件路径"
  - 工具调用行不包含输出内容（只有 header）
  - 如果有 thinking，以斜体灰色显示
```

### 12. Session 文件字段

```
目的: 验证 session JSONL 文件包含 name、tool_args、thinking 字段
操作: 不操作 TUI，直接检查磁盘文件
      LATEST=$(ls -t ~/.future/agent/sessions/*.jsonl | head -1)
      检查:
        grep -c '"name":"' $LATEST → 应有 tool 条目带 name
        grep -c 'tool_args' $LATEST → 应有条目带 tool_args
        grep -c '"thinking":"' $LATEST → 应有 assistant 条目带 thinking
  三项检查都有非零计数即为通过
```

### 13. `/compact` —— 上下文压缩

```
目的: 验证压缩命令正常执行
操作: C-u → /compact → Enter → 等 1s → 截屏
检查: 出现 compact 相关提示（如 "Context compacted" 或无报错）
```

### 14. `/cwd` —— 切换工作目录

```
目的: 验证工作目录切换
操作: C-u → /cwd /tmp → Enter → 等 0.5s
      C-u → /status → Enter → 等 0.5s → 截屏
检查: CWD 字段显示 "/tmp"
```

---

## 手动专项测试

以下场景脚本难以精确判断，需要大模型自行观察截屏内容并判断。

### A. Overlay 弹窗无残影

1. `/sessions` 打开列表 → 上下滚动到底再滚回头 → **确认第一行的选中高亮/背景色无异常**
2. 选中一个 session 切换 → 再次 `/sessions` → **确认弹窗内容完整刷新，无上次的残留文字或高亮**

### B. 自动补全

1. 输入 `/s` → 等 0.3s → 截屏 → **确认出现补全列表，含 /sessions、/status、/scoped-models**
2. Tab 接受补全 → 等 0.2s → **确认输入框变为完整命令**

### C. 长时间 streaming 不崩溃

1. 发送一个会触发多步工具调用的 prompt（如 "read this file, write a summary, then run tests"）
2. 观察整个过程中 TUI 无卡死，spinner 正常旋转，Footer 实时更新 token 计数

### D. 窄终端

1. 关闭当前 tmux 重新开 80×24 的 session
2. `/status` → **确认各字段不截断到不可读**
3. `/sessions` → **确认列表不溢出**

---

## 测试完成后的清理

```bash
tmux kill-session -t tui-test
```

---

## 常见失败原因速查

| 现象 | 可能原因 | 排查方向 |
|------|---------|---------|
| TUI 连不上 agent | Agent 未启动 | `pgrep future-agent` |
| Queries 不递增 | `/status` 走了 fallback 创建新 session | 确认 agent 未重启 |
| 工具调用显示 call_xxx | session 是旧代码保存的 | 新建 session 再测 |
| thinking 不显示 | session 是旧代码保存的 | 同上 |
| Overlay 重叠/残影 | 差分渲染缓存未清 | Ctrl+L 强制重绘 |
| Line exceeds terminal width | 某行超出终端宽度 | 检查 `padToWidth` 截断逻辑 |
