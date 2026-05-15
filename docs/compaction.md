# 上下文压缩（Context Compaction）

## 概述

上下文压缩用于在对话 token 数超过阈值时，自动或手动裁剪历史消息，防止超出模型的上下文窗口。压缩策略与 Go 版 `internal/compaction/` 1:1 兼容。

## 触发方式

### 手动触发

用户在 TUI 中输入 `/compact` 命令，或调用 gRPC `ExecuteCommand(type: "compact")`。

### 自动触发（代码已实现但未启用）

自动压缩的判断逻辑在 `compaction::should_compact()`：

```rust
context_tokens > context_window - reserve_tokens
```

- `context_window`：模型上下文窗口大小（默认 200000）
- `reserve_tokens`：预留空间，默认 16384

即当累计 token 数超过 `context_window - 16384`（约 183,616）时触发。

**当前状态**：`Loop::with_transform_context()` 已定义，但 ServerSession 未调用它。因此自动压缩在 agent loop 中实际未生效。只有手动 `/compact` 可用。

## 压缩策略

### 第一步：Token 估算

使用字符数估算 token 数（字符 ÷ 4 ≈ tokens）：

- **用户消息**：content 中所有 text 块的字符数之和
- **assistant 消息**：text 字符数 + tool_calls 中 function name 长度 + function arguments（JSON 字符串）长度
- **系统消息**：同上

```rust
// agent/src/compaction/mod.rs:52
pub fn estimate_tokens(msg: &Message) -> i32 {
    let chars = count_content_chars(&msg.content);
    match msg.role.as_str() {
        "assistant" => {
            let mut c = chars;
            if let Some(ref tcs) = msg.tool_calls {
                for tc in tcs {
                    c += tc.function.name.len() as i32;
                    if let serde_json::Value::String(ref s) = tc.function.arguments {
                        c += s.len() as i32;
                    }
                }
            }
            c
        }
        _ => chars,
    }
}
```

### 第二步：寻找安全切点

只在安全位置切割，避免在工具调用中间截断：

- **安全角色**：`user`、`assistant`（无 tool_calls 时）、`system`
- **不安全**：assistant 消息携带 tool_calls（正在调用工具）、tool 结果消息

```rust
// agent/src/compaction/mod.rs:95
pub fn find_valid_cut_points(messages: &[Message]) -> Vec<usize> {
    // 遍历消息，收集所有 user / assistant(无 tool_calls) / system 的索引
}
```

### 第三步：选择切点

从消息列表尾部向前遍历，累加 token 数，当达到 `keep_recent_tokens` 阈值时，找到该位置之后最近的合法切点：

```rust
// agent/src/compaction/mod.rs:110
pub fn find_cut_point(messages: &[Message], keep_recent_tokens: i32) -> usize {
    // 从后向前累加 token 数
    // 达到 keep_recent_tokens 时，选该位置后的第一个合法切点
    // 保证至少保留 keep_recent_tokens 的最近上下文
}
```

### 第四步：生成摘要

不调用 LLM 生成摘要，仅提取文件操作记录：

```rust
// agent/src/compaction/mod.rs:127
pub fn extract_file_operations(messages: &[Message]) -> (Vec<String>, Vec<String>) {
    // 扫描所有 assistant 消息的 tool_calls
    // 提取 read/read_file → read_files
    // 提取 write/write_file/edit/patch → modified_files
}
```

摘要格式：
```
[Context compaction: Previous conversation summarized. Files read: a.txt, b.txt. Modified: c.rs.]
```

### 第五步：构建新消息列表

```rust
// agent/src/compaction/mod.rs:209
let mut result = vec![Message {
    role: "user",
    content: Some(serde_json::json!([{
        "type": "text",
        "text": format!("[Context compaction: {}]", summary),
    }])),
    ..Default::default()
}];
result.extend(messages[cut..].to_vec());  // 保留切点之后的消息
```

## 参数配置

### 用户触发（manual compact）参数

| 参数 | 值 | 说明 |
|------|-----|------|
| `reserve_tokens` | 160000 | 预留 token 空间 |
| `keep_recent_tokens` | 80000 | 最近保留的 token 数 |
| `context_window` | 0（使用默认 200000） | 上下文窗口大小 |

定义在 `agent/src/rpc/mod.rs:640-644`。

### 自动压缩参数（settings.json）

```json
{
  "compaction": {
    "enabled": true,
    "reserveTokens": 16384,
    "keepRecentTokens": 20000
  }
}
```

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `enabled` | true | 是否启用自动压缩 |
| `reserveTokens` | 16384 | 触发阈值 = context_window - reserve_tokens |
| `keepRecentTokens` | 20000 | 压缩后保留最近多少 token |

### 引擎默认值

`EngineConfig::with_defaults()` 中：
- `compaction_reserve_tokens`: 16384
- `compaction_keep_recent_tokens`: 20000

## 压缩前后对比

```
压缩前 messages (7条，~50000 tokens):
  [user] 问题1
  [assistant] 回答1 (带 tool_calls)
  [tool] read a.txt
  [assistant] 继续回答
  [user] 问题2
  [assistant] 回答2
  [user] 问题3              ← keep_recent_tokens 覆盖区域

压缩后 (3条，~20000 tokens):
  [user] [Context compaction: ... Files read: a.txt. Modified: .]
  [assistant] 回答2
  [user] 问题3
```

消息数从 7 条减少到 3 条（切点在第 5 条消息），切点前的内容被一条摘要消息替代。

## 关键文件

| 文件 | 作用 |
|------|------|
| `agent/src/compaction/mod.rs` | 核心压缩逻辑 |
| `agent/src/agent/mod.rs:175-205` | agent loop 中应用 transform_context |
| `agent/src/rpc/mod.rs:631-663` | compact RPC 处理 |
| `agent/src/config/mod.rs:21-37` | CompactionSettings 定义与默认值 |
| `agent/src/engine/mod.rs:33-34` | EngineConfig 中的 compaction 参数 |
| `agent/src/events/mod.rs:213-223` | compaction_start/end 事件 |

## 待完善

1. **摘要不含 AI 生成内容**：当前摘要仅列出文件名，不包含对话内容的 AI 摘要。Go 版 pi 也使用类似策略。
