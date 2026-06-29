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

```
目的: 确认 TUI 能连接 agent 并显示 UI
操作: 启动后等 3s，截全屏
检查:
  - 顶部显示 "future-tui vX.X.X"
  - 底部 footer 显示 cwd、model、thinking、token/cost
  - 出现 "[skills] ..." 技能列表
  - 无 "Failed to connect" 或 connection error
```

### 1.2 Footer 实时更新

```
目的: 确认 streaming 时 footer 的 token/cost 实时变化
操作: 提问一个简单问题，在 Agent 回答过程中截屏 2-3 次
检查:
  - streaming 过程中 ↑in 和 ↓out 数字持续增长
  - 回答结束后 ¥cost 更新为非零值
  - spinner 在 streaming 时旋转，结束后停止
```

---

## 二、所有 Slash 命令

### 2.1 `/help` —— 帮助弹窗

```
命令: /help
检查:
  - overlay 弹出，标题含 "Help"
  - 列出所有快捷键（ctrl+c/p/r/t, tab, ↑↓, enter, escape）
  - 列出所有 / 命令
  - Escape 可关闭
```

### 2.2 `/status` —— 会话状态

```
命令: /status
检查:
  - Model 名称正确
  - Provider 正确
  - Context window 显示（如 131K）
  - Max output tokens 显示
  - Session ID 显示
  - CWD 显示当前工作目录
  - Thinking level 显示（off/minimal/low/medium/high/xhigh）
  - Permission 显示（all/workspace/none）
  - Queries 数为整数（新 session 为 0）
  - Auto compaction 显示 on/off
  - Context tokens/percent 显示
  - Tokens in/out 显示
  - Cost 显示（¥ 符号）
```

### 2.3 `/model <id>` —— 切换模型

```
命令: /model deepseek/deepseek-chat
检查:
  - 回复显示 "Model: deepseek/deepseek-chat"
  - Footer model 字段立即更新
  - 再发 /status 确认 model 已切换
```

### 2.4 `/sessions` —— 会话列表

```
命令: /sessions
检查:
  - overlay 弹出，标题含 "Sessions"
  - 每项展示 name（或 first_message 前 60 字）而非 session ID
  - 每项展示 "NQ" query 计数
  - 每项展示 model 名
  - 每项展示更新时间
  - name 列和 metadata 列对齐
  - ↑↓ 可滚动，选中高亮连续背景
  - 滚到底再滚回头无残影
  - Enter 选中切换，Escape 取消
```

### 2.5 `/sessions` —— 树形视图

```
命令: /tree
检查:
  - overlay 弹出，标题含 "Session Tree"
  - 子 session 有缩进和 ├─ / └─ 连接线
  - 父 session 的祖先线用 │ 而非空格
  - 当前 session 用 ▶ 标记
  - 切换后 /tree 再打开，▶ 移到新 session
```

### 2.6 `/new` —— 创建新会话

```
命令: /new
检查:
  - 回复显示 "New session:" + session ID
  - 如果之前有正在运行的 prompt，先 abort 再创建
  - /status 确认新 session 的 Queries=0
```

### 2.7 `/stop` —— 停止当前生成

```
命令: /stop（在 Agent streaming 时发送）
准备: 先发送一个会让 Agent 思考较久的问题
操作: 等 2s → /stop
检查:
  - Agent 停止输出
  - 回复 "Stopped." 或类似提示
  - 不会返回 /status 的内容
```

### 2.8 `/clone <sessionId>` —— 克隆会话

```
命令: /clone 20260xxx-xxxxxxxx
检查:
  - 回复确认克隆成功或显示新 session ID
  - /sessions 中能看到克隆出的 session
```

### 2.9 `/fork` —— Fork 会话

```
命令: /fork
准备: 先用 /sessions 找到想要 fork 的 session
操作: 切换到目标 session → /fork → 按提示选择从哪条消息 fork
检查:
  - 回复确认 fork 成功
```

### 2.10 `/name <name>` —— 设置会话名

```
命令: /name my-test-session
检查:
  - /status 中 Session name 更新为 "my-test-session"
  - /sessions 列表中该 session 显示新名称（而非 first_message）
```

### 2.11 `/cwd <path>` —— 切换工作目录

```
命令: /cwd /tmp
检查:
  - 回复 "CWD: /tmp"
  - /status 中 CWD 变为 /tmp
  - Footer cwd 立即更新
```

### 2.12 `/compact` —— 压缩上下文

```
命令: /compact
检查:
  - 无报错
  - 回复提示压缩完成或无需压缩
```

### 2.13 `/reload` —— 重载技能和上下文

```
命令: /reload
检查:
  - 无报错
  - 回复列出 reloaded 的技能
```

### 2.14 `/scoped-models` —— 模型范围配置

```
命令: /scoped-models
检查:
  - overlay 弹出，列出所有可用模型
  - 每项显示 ✓（启用）或 ✗（禁用）
  - Space 切换启用/禁用状态
  - Footer 显示 "x/y enabled"
  - Enter 保存，回复确认 "Model scope saved (x/y enabled)"
  - Escape 取消，状态回滚
  - 过滤输入可筛选模型（输入模型名关键字）
```

### 2.15 `/settings` → settings overlay

```
命令: ctrl+r 之后选 settings
操作: ctrl+r → 在 sessions 列表中选 settings
或者: /settings（如果该命令存在）
检查:
  - overlay 弹出
  - 包含 "Reload"、"Model"、"Thinking"、"Sessions" 选项
```

### 2.16 `/approve` / `/reject` —— 审批工作流

```
准备: 需要先触发一个需要审批的 tool call
      （如 write 到 workspace 外的路径）
命令: /approve <requestId> 或 /reject <requestId>
检查:
  - 审批通过后工具继续执行
  - 拒绝后工具返回拒绝提示
```

---

## 三、键盘快捷键

### 3.1 `ctrl+c` —— 中断 / 退出

```
操作: ctrl+c
检查:
  - 如果 Agent 在 streaming：中断当前输出（不退出）
  - 如果 Agent 空闲：退出 TUI
```

### 3.2 `ctrl+p` —— 循环切换模型

```
操作: ctrl+p
检查:
  - Footer model 名称变化
  - 每次按下切换到下一个可用模型
```

### 3.3 `ctrl+t` / `Shift+Tab` —— 循环切换 Thinking 等级

```
操作: ctrl+t
检查:
  - Footer thinking 字段变化
  - 顺序: off → minimal → low → medium → high → xhigh → off
```

### 3.4 `ctrl+r` —— 打开 Sessions 列表

```
操作: ctrl+r
检查: 同 /sessions
```

### 3.5 `PageUp` / `PageDown` —— 整页滚动

```
操作: PageUp → PageDown → PageUp
检查:
  - 聊天区域整页上下滚动
  - 不会跳到 overlay 上去
```

### 3.6 `ctrl+↑` / `ctrl+↓` —— 行滚动

```
操作: ctrl+↑ 按 3 次 → ctrl+↓ 按 3 次
检查: 聊天区域以 3 行为单位滚动
```

### 3.7 `Tab` —— 自动补全

```
操作: 输入 /s → 等 0.3s → Tab
检查:
  - 出现补全列表（/sessions, /status, /scoped-models）
  - Tab 接受第一个匹配
  - ↑↓ 可在候选中导航
  - 继续输入可进一步筛选
  - Escape 关闭补全
```

---

## 四、Editor（输入框）

### 4.1 光标移动

```
操作（逐一测试）:
  - left / right 左右移动
  - ctrl+b / ctrl+f 左右移动
  - ctrl+left / ctrl+right 按词移动
  - alt+b / alt+f 按词移动
  - home / end 行首/行尾
检查: 光标移动到预期位置
```

### 4.2 删除与 Kill/Yank

```
操作:
  1. 输入 "hello world test"
  2. ctrl+w 删除 "test"
  3. ctrl+a → ctrl+k 删除到行尾
  4. ctrl+y 粘贴 → 应出现刚删除的内容
检查: Kill ring 正常工作
```

### 4.3 多行输入

```
操作:
  1. 输入 "line1"
  2. 按 Enter (不提交，编辑器应支持换行)
  3. 输入 "line2"
检查:
  - 如果编辑器支持多行，输入区域高度增加
  - 两行都可见
```

### 4.4 Visual 模式 (`ctrl+v`)

```
操作:
  1. 输入 "hello world"
  2. ctrl+v 进入 visual 模式
  3. left 选中 "world"
  4. ctrl+y 复制选中区域
检查: 选中区域有视觉反馈
```

### 4.5 Undo (`ctrl+-`)

```
操作:
  1. 输入 "hello"
  2. ctrl+w 删除
  3. ctrl+- 撤销
检查: 恢复 "hello"
```

---

## 五、Chat 渲染

### 5.1 User 消息

```
目的: User bubble 展示
操作: 发送任意消息，截屏
检查: User 消息有彩色背景（Box 样式）
```

### 5.2 Assistant 消息（Markdown）

```
目的: Markdown 正确渲染
操作: 发送 "Write a short code block in python"
检查:
  - 代码块有语法高亮
  - 粗体/斜体/链接正确渲染
  - 列表正确显示
```

### 5.3 Thinking block

```
目的: Thinking 以斜体灰色显示
操作: 确保 thinking ≠ off → 发送需要思考的问题
检查:
  - 在回答内容出现前，有斜体灰色 thinking 文字
  - thinking 和正文之间有分隔
  - 完成后 /sessions 切到该 session → 历史 thinking 也显示
```

### 5.4 Tool 调用展示

```
目的: 验证每个工具调用的显示格式
操作: 发送分别触发 bash / read / write / edit 的 prompt
检查:
  - bash: 显示 "$ 命令" 格式
  - read: 显示 "read 文件路径[:行范围]"
  - write: 显示 "write 文件路径"
  - edit: 显示 "edit 文件路径"
  - 工具行只显示 header，不显示 output
  - 工具 output 出现在后续 assistant 消息中
```

### 5.5 Tool 调用错误状态

```
目的: 工具调用失败时有错误提示
操作: 发送 "Run a command that does not exist in bash"
检查: 错误工具调用有视觉区分（红色背景或 error 标记）
```

### 5.6 长输出截断

```
目的: 长输出不会撑爆终端
操作: 发送 "cat a large file" 或触发返回大量文本的工具
检查:
  - 没有 line exceeds terminal width 崩溃
  - 长文本正确换行
```

---

## 六、Session 持久化

### 6.1 新字段保存

```
目的: 验证 name / tool_args / thinking 保存到 JSONL
操作: 发送一条会触发 tool call 的 prompt，等完成后
检查: 最新 session 文件中:
  grep -c '"name":"' → tool 条目含 name
  grep -c 'tool_args' → 含 tool_args
  grep -c '"thinking":"' → assistant 条目含 thinking
```

### 6.2 切换到历史 session

```
目的: 切换到旧 session 后展示正确
操作:
  1. 提问 → 等完成 → /new → 再提问
  2. /sessions → 选中第一个 session → Enter
检查:
  - 消息正确加载（包括 user/assistant/tool）
  - tool call 显示格式正确（非 call_xxx）
  - thinking 正确显示
  - toolStatus 显示为 complete（非 running spinner）
```

### 6.3 Session name 默认值

```
目的: 未命名的 session 使用 first_message 作为显示名
操作: 新建 session → 提问 → /sessions
检查: 列表中该 session 显示的是首条消息的前若干字，不是 ID
```

### 6.4 Query count 显示

```
目的: /status Queries 和 /sessions Q 计数正确
操作:
  1. 新 session → /status → Queries=0
  2. 提问 1 次 → /status → Queries=1
  3. 再提问 1 次 → /status → Queries=2
  4. /sessions → 该 session 显示 "2Q"
检查: 计数等于 user message 数量（不包含 tool/assistant 内部消息）
```

---

## 七、Overlay 渲染稳定性

### 7.1 反复打开关闭无残影

```
目的: 连续多次打开同一 overlay 无视觉错乱
操作:
  1. /sessions → Escape
  2. /sessions → Escape
  3. /sessions → Escape
  4. /help → Escape
  5. /sessions → Escape
检查: 每次弹出/关闭后屏幕干净，无残留文字或高亮
```

### 7.2 滚到底再滚回头

```
目的: SelectList 滚动边界处理正确
操作: /sessions → 按 Down 直到最后一项 → 再按 Down（wrap to top）
检查:
  - 回到第一项时高亮正确
  - 中间无空白行残留
  - 滚动指示器（↑ more / ↓ more）显示/消失正常
```

### 7.3 Overlay 上打字

```
目的: 过滤输入正常
操作: /scoped-models → 输入 "deep"
检查:
  - 列表只显示含 "deep" 的模型
  - 选中索引重置为 0
  - 删除过滤文字后恢复完整列表
```

---

## 八、窄终端（80×24）

```
准备: 关闭 120×40 session，重新开 80×24
操作:
  1. /status → 各字段不截断
  2. /sessions → 列表项可见 metadata
  3. 提问 → 长文本正确换行
  4. /help → overlay 不超出屏幕
检查: 所有输出在 80 列内正确排版，无水平溢出
```

---

## 九、Stability

### 9.1 连续多轮对话不崩溃

```
操作: 连续发送 5 条不同的 prompt
检查: TUI 不会卡死或崩溃，每次都能正常回复
```

### 9.2 快速切换 session 不崩溃

```
操作: /sessions → 选 session1 → 等加载 → /sessions → 选 session2 → ...
      重复 3 次
检查: 每次切换都正常加载消息，无 crash
```

---

---

## 十、Steer / FollowUp / Interrupt

### 10.1 Steer（中断并替换当前生成）

```
目的: 验证 steer 能中断当前 prompt 并注入新指令
操作:
  1. 发送一个长 prompt（如 "write a poem with 20 stanzas"）
  2. 等 2s 让 Agent 开始 streaming
  3. 输入 "/steer write only 3 stanzas instead" → Enter
  4. 等 5s → 截屏
检查:
  - Agent 立即停止原输出
  - 按照 steer 的新指令重新生成（诗变短了）
  - messageCount / Queries 增加的是 steer 而非新 user message
```

### 10.2 FollowUp（排队追加）

```
目的: 验证 followUp 在当前生成完成后追加
操作:
  1. 发送 "What is 2+2?"
  2. 在 streaming 过程中输入 "/followUp also tell me what 3+3 is" → Enter
  3. 等 Agent 回答完 → 截屏
检查:
  - Agent 先回答 2+2=4，然后自动回答 3+3=6
  - 两次回答在同一个 session 中
```

### 10.3 Interrupt（ctrl+c 中断 streaming）

```
目的: 验证 ctrl+c 在 streaming 时中断但不退出
操作:
  1. 发送一个会让 Agent 运行较久的 prompt
  2. 等 2s → ctrl+c
  3. 等 1s → 截屏
检查:
  - TUI 不退去
  - 显示 "(aborted)" 提示
  - Agent 停止输出
  - 可以继续发送新 prompt
```

### 10.4 Interrupt（ctrl+c 退出空闲 TUI）

```
目的: 验证 ctrl+c 在空闲时退出 TUI
操作: 等 Agent 空闲 → ctrl+c
检查: TUI 退出（tmux session 结束）
注意: 测试后需重新启动 tmux session
```

---

## 十一、Auto Retry / Auto Compaction

### 11.1 Auto Retry

```
目的: 验证 context-length 错误时自动重试
操作:
  1. 确认 auto_retry 开启（/status 检查）
  2. 发送会导致 context overflow 的长对话（多轮提问填满 context）
  3. 观察是否自动触发 compaction/retry
检查: 遇 context-length 错误时不直接报错，而是自动 compact 后重试
```

### 11.2 Auto Compaction 触发

```
目的: 验证 context 达到 90% 时自动压缩
操作:
  1. 确认 auto_compaction 开启（/status 检查）
  2. 连续多轮提问直到 context 超过 90%
  3. 观察 /status 中 context_tokens 的变化
检查: context 接近上限时自动压缩，压缩后 context_tokens 明显减少
```

### 11.3 手动 Compaction

```
目的: 验证 /compact 命令效果
操作:
  1. 多轮提问后 /status 记录 context_tokens
  2. /compact → 等 3s
  3. /status 对比 context_tokens
检查: compact 后 context_tokens 应减少，旧消息被 summarize
```

---

## 十二、Export / Import / Delete

### 12.1 Export HTML

```
目的: 验证 session 导出为 HTML
操作: /export → 等 2s → 检查输出路径
检查:
  - 提示导出成功或显示文件路径
  - 文件存在且内容非空
```

### 12.2 Import Session

```
目的: 验证从文件导入 session
操作: /import <path> → 等 2s
检查:
  - 提示导入成功
  - /sessions 列表中可以看到导入的 session
```

### 12.3 Delete Session

```
目的: 验证删除 session
操作: /sessions → 选中一个 session → /delete <sessionId>
检查:
  - 提示删除成功
  - /sessions 列表中该 session 已消失
```

---

## 十三、Image Input (Kitty Protocol)

### 13.1 图片显示

```
目的: 验证 Kitty 图片协议在支持时正常工作
操作: 如果终端支持 Kitty（如 iTerm2/WezTerm/kitty）:
  1. 通过 file:// 或 base64 发送一张图片
  2. 确认终端能渲染图片
  3. 检查: TUI 中图片不破坏文本布局
```

---

## 十四、Session Fork / Clone 链路

### 14.1 Fork Lineage

```
目的: 验证 fork 的 session 保留 parent_session_id
操作:
  1. 在 session A 中提问
  2. /fork → 选择 fork 节点 → 创建 session B
  3. /tree → 检查 session B 显示为 session A 的子节点
检查:
  - /tree 中能看到 A → B 的父子结构
  - session B 中只包含 fork 点之前的消息
  - session B 可以独立继续对话
```

### 14.2 Clone Session

```
目的: 验证 clone 创建完整独立副本
操作:
  1. 在 session A 中提问多轮
  2. 切换到 session A → /clone → 创建 session C
  3. /sessions → 切换到 session C
检查:
  - session C 包含 A 的全部消息
  - 在 C 中继续提问不影响 A
```

---

## 十五、Permission Level

### 15.1 Workspace Permission

```
目的: 验证 workspace 权限限制工具执行范围
操作:
  1. 设置 permission_level = "workspace"
  2. 尝试 read /etc/passwd
检查: 工具被拒绝执行，提示超出 workspace
```

### 15.2 None Permission

```
目的: 验证 none 权限只允许只读
操作:
  1. 设置 permission_level = "none"
  2. 尝试 write 一个文件
检查: write 被拒绝
```

---

## 十六、Thinking Level 效果验证

### 16.1 不同 Thinking Level 的可见差异

```
目的: 验证 thinking level 改变影响输出
操作:
  1. ctrl+t 切换到 "off" → 提问 "explain recursion"
  2. ctrl+t 切换到 "xhigh" → 提问 "explain recursion"
  3. 对比两次回答
检查:
  - off: 无 thinking block，回答相对简短
  - xhigh: 有斜体灰色 thinking block，回答可能更详细
  - /status 中 Thinking 字段正确反映当前等级
```

---

## 十七、gRPC 重连

### 17.1 Agent 断连后恢复

```
目的: 验证 TUI 在 agent 重启后自动重连
操作:
  1. TUI 运行中 → kill agent 进程
  2. 观察 TUI 状态
  3. 重启 agent
  4. 等 5s → 尝试提问
检查:
  - Agent 断连时 TUI 显示 "(not connected)" 或类似提示
  - Agent 恢复后 TUI 自动重连
  - 重连后 /status 正常
```

---

---

## 十八、异常检测与恢复

### 18.1 Agent 进程突然终止

```
目的: 验证 TUI 在 agent 崩溃后能优雅降级并自动重连
操作:
  1. TUI 正常运行中 → 在另一个终端 `kill -9 $(pgrep future-agent)`
  2. 等 3s → 截屏
  3. 重启 agent (`make run-agent &`)
  4. 等 5s → 截屏 → 尝试提问
检查:
  - Agent 被 kill 后 TUI 不崩溃
  - 终端/stream 报错时 TUI 捕获异常不退出
  - Footer 或提示区域显示连接断开状态
  - Agent 重启后 TUI 自动重连（无需手动重启 TUI）
  - 重连后 /status 正常，可以继续提问
```

### 18.2 Agent 连接拒绝

```
目的: 验证 TUI 在 agent 未启动时的行为
操作: 先停掉 agent → 启动 TUI
检查:
  - TUI 启动不崩溃
  - 显示 "(not connected)" 或连接失败提示
  - 不会持续刷屏报错
  - Footer 显示异常状态
```

### 18.3 Context Length Overflow

```
目的: 验证超过 context window 时 TUI 的错误展示
操作:
  1. 连续多轮对话填满 context
  2. 发送一条新 prompt → 等 agent 处理
  3. 截屏观察
检查:
  - 如果是 auto_retry: TUI 显示 "retrying..." 提示
  - auto_compaction 自动压缩后重试成功
  - 如果最终失败: TUI 显示清晰的错误消息（如 "context length exceeded"）
  - TUI 不会卡死或退出
  - 可以 /compact 后重试
```

### 18.4 工具调用超时

```
目的: 验证长时间运行的工具（如 sleep 60）的取消行为
操作:
  1. 发送 "sleep 60" → 等 3s → ctrl+c
检查:
  - 工具被中断，显示 "[Tool execution cancelled]"
  - 不会死等 60 秒
```

### 18.5 工具返回超大结果

```
目的: 验证超大输出（>100K chars）不会被截断到不可读
操作: 发送 "cat a very large file" (如 200K+ 的日志文件)
检查:
  - Agent 的 tool result 被 cap 到 100K
  - TUI 不会因输出过大而崩溃
  - 不会触发 "line exceeds terminal width" 异常
```

### 18.6 审批超时

```
目的: 验证审批请求在超时后自动取消
操作:
  1. 触发需要审批的工具调用
  2. 不回复审批，等 30s+
检查:
  - 审批请求自动过期（不会永久等）
  - 工具调用以 "cancelled" 状态结束
  - TUI 清理审批请求后不再显示
```

### 18.7 快速连续操作

```
目的: 验证高频操作不导致竞态
操作:
  1. 快速连续按 ctrl+p 5 次
  2. 快速连续 /sessions → Escape → /sessions → Escape 5 次
  3. 快速连续 /status → /sessions → /help 各一次
检查:
  - TUI 不崩溃，不卡死
  - overlay 不重叠
  - 最后显示的是最后操作的 overlay
```

### 18.8 gRPC 流中断

```
目的: 验证 streaming 过程中 gRPC 连接中断的恢复
操作:
  1. 发送 prompt → 在 streaming 过程中 kill agent
  2. 等 3s → 截屏
检查:
  - TUI 不死循环不崩溃
  - streaming 状态被重置（isStreaming → false）
  - 显示 stream error 提示
  - 连接恢复后可以发送新 prompt
```

### 18.9 输入非法命令

```
目的: 验证非法输入不崩溃
操作:
  1. 输入空消息 → Enter（不应发送）
  2. 输入 "/" → Enter
  3. 输入 "/unknown_command" → Enter
  4. 输入超长文本（>10K chars）
检查:
  - TUI 不崩溃
  - 未知命令给出提示或静默处理
  - 超长文本正常截断或发送
```

### 18.10 Session 文件损坏

```
目的: 验证 session 文件损坏后的容错
操作:
  1. 找到最新 session 文件 → 删除文件头部几行
  2. /sessions → 尝试切换到损坏的 session
检查:
  - TUI 不崩溃
  - 显示 "Failed to load" 或跳过损坏 session
  - 其他正常 session 仍可切换
```

### 18.11 Terminal 尺寸突变

```
目的: 验证终端 resize 后的渲染正确性
操作:
  1. 在 120×40 tmux 中运行 TUI
  2. tmux resize-window -x 80 -y 24 → 等 0.5s → 截屏
  3. tmux resize-window -x 120 -y 40 → 等 0.5s → 截屏
检查:
  - resize 后布局自适应（footer/chat/input 重新分配空间）
  - 不出现 "line exceeds terminal width" 异常
  - /help overlay 在新尺寸下居中
```

### 18.12 并发 Prompt + Abort

```
目的: 验证 prompt 和 abort 的竞态
操作:
  1. 发送 prompt
  2. 立即 /stop
  3. 再发送一条新 prompt
检查:
  - 旧 prompt 被正确 abort
  - 新 prompt 正常执行
  - 不会出现 "agent is busy" 错误
```

### 18.13 切换 Session 时 Agent 正忙

```
目的: 验证切换 session 不阻塞
操作:
  1. 发送一条 prompt（让 agent 进入 streaming）
  2. streaming 过程中 /sessions → 选择一个 session → Enter
检查:
  - 切换正常（不需要等 streaming 结束）
  - 新 session 消息正确加载
  - 原 session 的 streaming 继续（如果 agent 支持后台运行）
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
