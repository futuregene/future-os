# TUI 测试指南

> 本指南供大模型在 tmux 中交互式测试 FutureOS TUI。每次改完 TUI / agent 代码后，按本文档逐步执行验证。
>
> 终端尺寸：**120 列 × 40 行**。窄终端专项测试用 80×24。

## 准备

```bash
# 1. Agent
pgrep future-agent || (cd /Users/geilige/future-os/agent && cargo run &) && sleep 2

# 2. tmux session
SESSION="tui-test"
tmux kill-session -t "$SESSION" 2>/dev/null; sleep 0.5
tmux new-session -d -s "$SESSION" -x 120 -y 40 \
  "cd /Users/geilige/future-os/tui && bun run src/index.ts 2>&1"
sleep 3
```

### 操作 TUI 的方式

| 操作 | tmux 命令 |
|------|----------|
| 发送文本 | `tmux send-keys -t tui-test "text"` |
| 发送 Enter | `send-keys Enter` |
| 清除输入行 | `send-keys C-u` |
| 关闭弹窗/取消 | `send-keys Escape` |
| 截取全屏（含 ANSI） | `tmux capture-pane -t tui-test -p -e` |
| 截取尾部 N 行 | `capture-pane -p -e -S -N` |
| 等待渲染/响应 | `sleep N` |

---

## 一、启动与基础 UI

### 1.1 TUI 正常启动


### 1.2 Footer 实时更新


---

## 二、所有 Slash 命令

### 2.1 `/help` —— 帮助弹窗


### 2.2 `/status` —— 会话状态


### 2.3 `/model <id>` —— 切换模型


### 2.4 `/sessions` —— 会话列表


### 2.5 `/sessions` —— 树形视图


### 2.6 `/new` —— 创建新会话


### 2.7 `/stop` —— 停止当前生成




### 2.10 `/name <name>` —— 设置会话名


### 2.11 `/cwd <path>` —— 切换工作目录


### 2.12 `/compact` —— 压缩上下文


### 2.13 `/reload` —— 重载技能和上下文


### 2.14 `/scoped-models` —— 模型范围配置




---

## 三、键盘快捷键

### 3.2 `ctrl+p` —— 循环切换模型


### 3.3 `ctrl+t` / `Shift+Tab` —— 循环切换 Thinking 等级


### 3.7 `Tab` —— 自动补全


---

## 四、Chat 渲染

### 5.1 User 消息


### 5.2 Assistant 消息（Markdown）


### 5.3 Thinking block


### 5.4 Tool 调用展示


### 5.5 Tool 调用错误状态



---

## 六、Session 持久化

### 6.1 新字段保存


### 6.2 切换到历史 session


### 6.3 Session name 默认值


### 6.4 Query count 显示


---

## 七、Overlay 渲染稳定性

### 7.1 反复打开关闭无残影


### 7.2 滚到底再滚回头


### 7.3 Overlay 上打字


---

## 八、窄终端（80×24）


---

## 九、Stability

### 9.1 连续多轮对话不崩溃


### 9.2 快速切换 session 不崩溃


---

---

## 十、Steer / FollowUp / Interrupt





---

## 十一、异常检测与恢复

### 11.1 快速连续操作


### 11.2 输入非法命令


### 11.3 Terminal 尺寸突变


### 11.4 并发 Prompt + Abort


---

## 十二、Compaction

### 12.1 手动 Compaction 效果

```
目的: 验证 /compact 减少 context token 数量
操作:
  1. /status 记录 context_tokens
  2. /compact → 等 3s
  3. /status 对比 context_tokens
检查: compact 后 context_tokens 减小或不变（旧消息被 summarize）
```

### 12.2 context overflow 处理

```
目的: 验证 context 接近上限时自动触发 compaction
操作:
  1. 确认 auto_compaction 开启（/status）
  2. 连续多轮提问（每轮带较多上下文）
  3. 观察 /status 中 context_tokens 在接近 90% 时是否触发 compaction
检查: 不出现 crash；若有 auto_retry，错误时自动重试而非直接失败
```

---

## 清理

```bash
tmux kill-session -t tui-test
```

---

## 常见失败速查

| 现象 | 原因 | 排查 |
|------|------|------|
| TUI 连不上 agent | Agent 未启动 | `pgrep future-agent` |
| Line exceeds terminal width | 某行超出终端 | 检查 `padToWidth` / `truncateToWidth` |
| Overlay 重叠/残影 | 差分渲染缓存 | Ctrl+L 或 `requestRender(true)` |
| 工具调用显示 call_xxx | 旧 session 文件 | 新建 session 再测 |
| thinking 不显示 | 旧 session 文件 | 同上 |
| Queries 不递增 | 走了 fallback 新 session | 确认 agent 未重启 |
| 中文乱码 | from_utf8_lossy | 确认最新 agent（字节 buffer 已修） |
