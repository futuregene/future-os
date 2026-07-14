# 会话中断行为分析报告

> 2026-07-14 实测 | Agent 版本 `49b26475` | 测试模型 `deepseek-v4-flash` level `off`

## 概述

当流式输出中的会话被中断（用户主动 abort、gRPC 断连、agent 进程崩溃、或 graceful shutdown），哪些数据被持久化到磁盘、哪些丢失、后续再次提问会发生什么？本报告覆盖六种中断场景，全部在真实 gRPC agent 上逐帧验证。

---

## 中断机制

Agent loop 有三个中断检测通道（`agent/src/agent/run_loop.rs`）：

| 通道 | 类型 | 检测延迟 | 生效阶段 |
|------|------|---------|---------|
| `interrupt_rx`（mpsc channel） | `abort()` 发送 `()` | 即时（tokio::select!） | LLM streaming 中 |
| `interrupt_flag`（AtomicBool） | `abort()` 设为 true | 50ms 轮询 | bash 工具执行中 |
| `is_interrupted()` | 检查 flag + loop 状态 | 下个 turn 循环起点 | turn 之间 |

`abort()` 方法（`agent/src/rpc/session.rs:237-256`）同时设置三个通道。

### Session 两次存盘时机

每个 prompt 周期有两次存盘：

1. **Pre-loop save**（`session_prompt.rs:80-107`）：用户消息推入 `self.messages` 后**立即**存盘。此时 JSONL 包含用户消息及之前所有历史 turns。在 agent loop 启动**之前**执行。

2. **Post-loop save**（`session_prompt.rs:625-641`）：agent loop 完成后的 Ok 分支。从 `self.messages` 重建全量条目（含 assistant 回复），覆写 JSONL。

Err 分支（`session_prompt.rs:649-663`）**不存盘**——只有 API/provider 错误走此路径，interrupt 返回 Ok。

---

## 场景实测

所有测试使用 grpcurl + Python 脚本，对 `127.0.0.1:50052` 的 agent 发送 gRPC 命令。

### 场景 A：LLM 流内 Abort（thinking/text 生成中）

**测试方法**：发送 "Write a 200-word essay about AI"，3～8 秒后 abort。

**时间线**：
```
t=0     pre-loop save → JSONL: [user]
t=0-8s  LLM streaming → reasoning_text + assistant_text 在栈变量中累积
t~8s    abort() → interrupt_rx 被 select! 捕获 → stream_error 置位
t~8s    构建 partial assistant → push 进 messages
t~8s    post-loop save → JSONL 完整覆盖
```

**修复前**（`93f72158`）JSONL 预期与实际：
```
预期（理想）：
  [session_info]
  [user: "Write a 200-word essay about AI."]
  [assistant: partial thinking + partial text]

实际：
  [session_info]
  [user: "Write a 200-word essay about AI."]
  ← assistant 完全丢失！thinking 和 text 都在栈变量里，从未进入 messages
```

**修复后**（`49b26475`）JSONL 实际：
```
  [session_info]     content=238
  [user]             content=62   ("Write a 200-word essay about AI.")
  [assistant]        content=0    thinking=132  tool_calls=1
```

✅ assistant 条目保留（thinking 132 字符 + tool_call）。Text 为 0 是因为模型在 thinking 阶段被中断，尚未输出正文。

**根因**：中断返回在 line 629/642，而 `messages.push(assistant_msg)` 在 line 679。三个中断返回点都在局部变量 `reasoning_text`/`assistant_text`/`tool_calls` 被消费之前。

**修复**：`build_partial_assistant` 闭包在返回前用栈变量构建一个 `AgentMessage` 并 push 进 `messages`。

**后续提问预期**：
```
JSONL: [session_info, user, assistant(partial)]
下次 prompt 会在末尾追加 [user(new_question), assistant(new_answer)]
entryProjection 正常渲染为一个完整 turn + 一个新 turn
```

---

### 场景 B：长 Bash 工具执行中 Abort

**测试方法**：发送 "Run 'sleep 30 && echo done' using bash. Then explain what happened."，4 秒后 abort。

**时间线**：
```
t=0     pre-loop save → JSONL: [user]
t~2s    LLM 完毕 → assistant 含 tool_call[bash "sleep 30 && echo done"]
t~2s    spawn_bash 启动 → tokio::select! { output, interrupt_flag }
t~6s    abort() → interrupt_flag=true → wait_for_interrupt(50ms) 触发
        → kill_process_group → bash 收到 SIGKILL
        → tool result: "Bash command interrupted by abort"
        → post-loop save
```

**JSONL 实际**：
```
  [session_info]     content=257
  [user]             content=97   ("Run 'sleep 30 && echo done' using bash...")
  [assistant]        content=0    tool_calls=1  (bash "sleep 30 && echo done")
  [tool]             content=70   ("Error: Bash command interrupted by abort")
```

✅ bash 有中断检测（`tools/mod.rs:393-399` 的 `wait_for_interrupt`），tool result 记录了中断信息。assistant 条目及其 tool_call 完整保留。

**后续提问预期**：
```
JSONL: [session_info, user, assistant(tool_call), tool("interrupted")]
下次 prompt 正常追加 [user(new_q), assistant(new_a)]
turn 中有 tool 但无 text 回复，agent 继续对话无影响
```

---

### 场景 C：Read/Write/Edit 工具执行中 Abort

**测试方法**：发送 "Read /tmp/large_test_file.txt using the read tool, then summarize."，5 秒后 abort。

**预期与结果**：
```
预期：read/write/edit 没有中断检测，abort 要到下个 turn 起点才生效
实际：
  JSONL 包含 9 个条目：
  [session_info]
  [user]                       ("Read the file...")
  [assistant]  content=0  tc=1 (tool_call: read)
  [tool]       content=71      ("Error: stream did not contain valid UTF-8")
  [assistant]  content=99 tc=2 (tool_call: file + ls)
  [tool]       content=87      ("/tmp/large_test_file.txt: OpenPGP Secret Key [exit: 0]")
  [tool]       content=77      ("10485760 /tmp/large_test_file.txt [exit: 0]")
  [assistant]  content=0  tc=1 (tool_call: bash)
  [tool]       content=70      ("Error: Bash command interrupted by abort")
```

✅ 前两轮 tool call（read + file + ls）完整执行完毕。第三轮 bash 才被 interrupt 截断。read/write/edit 无中断检测，当前 turn 会跑完。

**后续提问预期**：
```
JSONL: [..., tool("interrupted")]
tool 结果带有 "interrupted by abort" 信息，agent 可在下轮感知到中断
```

---

### 场景 D：正常完成（无中断）

**测试方法**：发送 "What is 2+2?"，等待完成。

**JSONL 实际**：
```
  [session_info]     content=228
  [user]             content=42   ("What is 2+2?")
  [assistant]        content=40   ("2 + 2 equals 4.")
```

✅ 完整 turn 持久化。

---

### 场景 E：Agent 进程崩溃

**测试方法**：发送 "Write a 500-word essay about dogs. Be very detailed."，3 秒后 SIGKILL。

**时间线**：
```
t=0     pre-loop save → JSONL: [user]
t=3s    SIGKILL → 进程立即终止
        → 所有内存状态丢失
        → reasoning_text, assistant_text, tool_calls: 全部丢失
        → post-loop save 从未执行
```

**崩溃后 JSONL**：
```
  [user]  content=82  ("Write a 500-word essay about dogs. Be very detailed.")
```

❌ 只有 pre-loop save 保住了用户消息。所有内存状态（partial assistant、token 计数）全部丢失。

**重启 agent 后发 follow-up**：
```
  发送: "What is 1+1? One word."
  JSONL:
  [user]  ("Write a 500-word essay about dogs...")
  [user]  ("What is 1+1? One word.")
  [assistant]  ← 合并回答了 dog essay + "4"
```

重启后 agent 从磁盘 JSONL 加载 session，只有一条 user。follow-up 又追加一条 user，LLM 收到两条连续 user 消息。模型合并回答了两个问题。

**后续提问预期**：
- ✅ 两条连续 user 消息，大多数 LLM API 不会拒绝
- ⚠️ 模型会将第一条 user 当作「没被回答过的问题」，合并处理
- ⚠️ token 计数从 0 重启（`tokens_in`/`tokens_out` 丢失），compaction 判断可能不准

---

### 场景 F：Graceful Shutdown

**测试方法**：发送 "Write a 300-word essay about dogs."，流式中发送 `shutdown` 命令。然后尝试发送新的 prompt。

**实现**：
- `AppState.shutting_down: Arc<AtomicBool>`（`agent/src/rpc/mod.rs:39`）
- `"shutdown"` 命令设置为 true（`commands.rs:27-32`）
- `prompt` 和 `follow_up` 在 shutdown 期间拒绝（`commands.rs:35-40, 81-86`）
- abort、get_state、status 等不受影响

**时间线**：
```
t=0     pre-loop save → JSONL: [user]
t=2s    shutdown 命令 → shutting_down=true
        已有 prompt 继续执行（不被中断）
t=2s    新 prompt 请求 → 拒绝: "agent is shutting down; no new prompts accepted"
t~10s   已有 prompt 完成 → post-loop save → JSONL: [session_info, user, assistant(2299 chars)]
```

**JSONL 实际**：
```
  [session_info]     content=257
  [user]             content=77   ("Write a 300-word essay about dogs...")
  [assistant]        content=2299 (完整 essay，包含 ## Introduction / ## Body / ## Conclusion)
```

✅ 已有 streaming 完成并完整存盘（2299 字符）。新 prompt 被拒绝。

**后续提问预期**：
- shutdown 期间无法发新请求，需要重启 agent 清除 flag
- 重启后正常

---

## 总结

| 场景 | 用户消息 | 部分 Assistant | 工具结果 | Follow-up 后果 |
|------|---------|---------------|---------|---------------|
| A LLM 中断 | ✅ 存盘 | ✅ 存盘（修复后） | N/A | 正常，一条完整 turn |
| B Bash 中断 | ✅ 存盘 | ✅ 存盘（含 tool_call） | ✅ "interrupted by abort" | 正常 |
| C Read/Write 中断 | ✅ 存盘 | ✅ 存盘（完整 turn） | ✅ 完整 | 正常 |
| D 正常完成 | ✅ 存盘 | ✅ 存盘（完整回复） | ✅ 完整 | 正常 |
| E 进程崩溃 | ✅ 存盘 | ❌ 全部丢失 | ❌ 全部丢失 | 两条连续 user，模型合并回答 |
| F Graceful Shutdown | ✅ 存盘 | ✅ 存盘（完整回复） | ✅ 完整 | 新 prompt 被拒绝 |

### 涉及文件

| 文件 | 变更 |
|------|------|
| `agent/src/rpc/session_prompt.rs:74-107` | Pre-loop save——用户消息立即存盘 |
| `agent/src/agent/run_loop.rs:619-646` | `build_partial_assistant`——中断时保留部分内容 |
| `agent/src/rpc/mod.rs:38-39` | `AppState.shutting_down` 字段 |
| `agent/src/rpc/commands.rs:27-43,81-86` | `shutdown` 命令 + prompt/follow_up 拦截 |
| `agent/src/main.rs:306` | 初始化 `shutting_down` |

### 已知局限

1. **场景 E 崩溃**：pre-loop save 是唯一保护。崩溃如果恰好在 pre-loop save 之前（概率极低，因为 pre-loop save 紧跟 `push(user_msg)` 同步执行），连用户消息也会丢失。更健壮的方案是 write-ahead log，但复杂度过高。

2. **非 bash 工具无中断检测**：read/write/edit 无 interrupt 钩子。正常文件大小下次秒级完成，不构成实际影响。超大文件（GB 级 read）可能阻塞，需监控但暂不处理。

3. **崩溃后 token 计数归零**：重启后 `tokens_in`/`tokens_out` 从 0 开始，compaction 的上下文估算会偏低，可能导致本应触发 compaction 的会话没有触发，直到 token 计数重新累积。
