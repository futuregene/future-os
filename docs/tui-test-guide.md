# TUI Test Guide

> 在 tmux 中自动化测试 TUI 功能的流程。每次改完 TUI 代码后执行。

## 前置条件

```bash
# Agent 必须运行
make run-agent  # 或提前启动

# TUI 依赖安装
make install-tui
```

## 自动化测试脚本

```bash
#!/bin/bash
# 保存为 /tmp/tui-tmux-test.sh 然后 bash 执行

set -e
SESSION="tui-test"
TUI_DIR="$PWD/tui"

tmux kill-session -t "$SESSION" 2>/dev/null || true
sleep 0.5

# 120x40 终端
tmux new-session -d -s "$SESSION" -x 120 -y 40 \
    "cd $TUI_DIR && bun run src/index.ts 2>&1"
sleep 3

send() { tmux send-keys -t "$SESSION" "$@"; }
capture() { tmux capture-pane -t "$SESSION" -p "$@"; }
wait_sec() { sleep "$1"; }
clear_input() { send C-u; wait_sec 0.2; }

# ── 1. /help ────────────────────────────────────────────────────────────────
echo "1. /help"
clear_input; send "/help"; send Enter; wait_sec 0.5
capture | grep -q "Terminal UI Help" && echo "  PASS" || echo "  FAIL"
send Escape; wait_sec 0.2

# ── 2. /status (初始状态) ───────────────────────────────────────────────────
echo "2. /status"
clear_input; send "/status"; send Enter; wait_sec 0.5
capture | grep -q "Queries: 0" && echo "  PASS" || echo "  FAIL"
clear_input

# ── 3. 简单提问 ─────────────────────────────────────────────────────────────
echo "3. Simple question"
send "What is 2+2? Answer with just the number."; send Enter; wait_sec 8
capture | grep -q "4" && echo "  PASS" || echo "  FAIL"
wait_sec 3

# ── 4. 查询后 /status ───────────────────────────────────────────────────────
echo "4. /status after query"
clear_input; send "/status"; send Enter; wait_sec 0.5
capture | grep "Queries: 1" && echo "  PASS" || echo "  WARN: queries not 1"
clear_input

# ── 5. 工具调用 (bash) ─────────────────────────────────────────────────────
echo "5. Tool call (bash)"
clear_input; send "Run ls in current directory"; send Enter; wait_sec 12
capture | grep -E '\$ ls|bash' && echo "  PASS" || echo "  WARN"
capture | grep "src" && echo "  PASS (output)" || echo "  WARN: no output"
wait_sec 3

# ── 6. /status (Queries=2) ─────────────────────────────────────────────────
echo "6. /status after tool"
clear_input; send "/status"; send Enter; wait_sec 0.5
capture | grep "Queries: 2" && echo "  PASS" || echo "  WARN"
send Escape; wait_sec 0.1

# ── 7. /new ─────────────────────────────────────────────────────────────────
echo "7. /new"
clear_input; send "/new"; send Enter; wait_sec 1
capture | grep -iE "session|New" && echo "  PASS" || echo "  WARN"
clear_input

# ── 8. /sessions 列表 ──────────────────────────────────────────────────────
echo "8. /sessions"
clear_input; send "/sessions"; send Enter; wait_sec 1.5
capture -S -40 | grep "Q" && echo "  PASS" || echo "  WARN: no Q counts"
send Escape; wait_sec 0.2

# ── 9. ctrl+p 切换模型 ─────────────────────────────────────────────────────
echo "9. Model cycle (ctrl+p)"
send Escape; wait_sec 0.2; send "ctrl+p"; wait_sec 1
echo "  INFO: check footer for model change"
clear_input

# ── 10. 切换到历史 session ──────────────────────────────────────────────────
echo "10. Switch to historical session"
clear_input; send "/sessions"; send Enter; wait_sec 1.5
send Down Down Enter; wait_sec 3
capture -S -20 | grep -i "switch\|session" && echo "  PASS" || echo "  WARN"
clear_input

# ── 11. 历史消息渲染检查 ───────────────────────────────────────────────────
echo "11. Historical messages"
capture | grep -E '\$ ls|read |write |edit ' && echo "  PASS: tool calls" || echo "  INFO: no tools"
echo "  Done"

# ── 12. Session 文件字段检查 ────────────────────────────────────────────────
echo "12. Session file fields"
LATEST=$(ls -t ~/.future/agent/sessions/*.jsonl | head -1)
grep -q '"name":"bash"\|"name":"read"' "$LATEST" && echo "  PASS: name" || echo "  INFO: no name"
grep -q "tool_args" "$LATEST" && echo "  PASS: tool_args" || echo "  INFO: no tool_args"
grep -q "thinking" "$LATEST" && echo "  PASS: thinking" || echo "  INFO: no thinking"

# ── 13. /compact ────────────────────────────────────────────────────────────
echo "13. /compact"
clear_input; send "/compact"; send Enter; wait_sec 1
echo "  PASS (command sent)"
clear_input

# ── 14. /cwd ────────────────────────────────────────────────────────────────
echo "14. /cwd"
clear_input; send "/cwd /tmp"; send Enter; wait_sec 0.5
send "/status"; send Enter; wait_sec 0.5
capture | grep "/tmp" && echo "  PASS" || echo "  WARN"
clear_input

echo ""
echo "=== Done. tmux attach -t $SESSION ==="
```

## 运行

```bash
make run-agent &        # 启动 agent
sleep 2                 # 等 agent 就绪
bash /tmp/tui-tmux-test.sh
```

## 手动测试项目

测试脚本无法覆盖的场景，需要手动验证：

### Overlay 渲染
1. 输入 `/sessions` → 确认弹窗正常
2. 选择 session → 确认切换正常
3. 再次 `/sessions` → **检查 overlay 无重叠/错乱**
4. 上下滚动到列表末尾再滚回头 → **检查无残留行**

### 模型切换
1. `ctrl+p` 切几次 → footer 更新
2. `/model deepseek-v4-pro` → 确认切换

### 工具调发展示
1. 发送需要工具的 prompt（如 "read this file"）
2. 观察工具调用格式：`$ cmd` / `read /path` / `write /path` / `edit /path`
3. 完成后 `/sessions` 切回该 session → 历史工具调用仍正确显示

### Thinking 显示
1. 确保 thinking 等级非 off
2. 提问 → 确认 thinking block 以斜体显示
3. 切历史 session → 确认历史 thinking 也有

### Session 文件
```bash
# 检查最新 session 文件包含所有新字段
LATEST=$(ls -t ~/.future/agent/sessions/*.jsonl | head -1)
echo "--- name ---"; grep -c '"name":"' "$LATEST"
echo "--- tool_args ---"; grep -c "tool_args" "$LATEST"
echo "--- thinking ---"; grep -c '"thinking":"' "$LATEST"
```

## 常见问题排查

| 现象 | 原因 | 解决 |
|------|------|------|
| TUI 连不上 agent | Agent 未启动 | `make run-agent` |
| 答非所问 | 切到了错误 session | `/new` 创建新 session |
| Overlay 重叠 | 差分渲染缓存 | `requestRender(true)` 或 Ctrl+L |
| 工具调用显示 call_id | Session 保存缺少 name 字段 | 确认是最新 agent 代码 && 新 session |
| thinking 不显示 | Session 保存缺少 thinking 字段 | 同上 |
