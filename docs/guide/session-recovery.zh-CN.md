# 重启后会话被围栏了怎么办

Agent 重启（或被 kill）时如果有工具调用正在执行，那次调用的结果没能写进 journal，Agent
就不知道它到底跑没跑：文件可能已经写了，命令可能已经改了东西。于是恢复逻辑把**整个会话**
围起来——所有会产生副作用的工具（`shell`、`write`、`edit`……）一律拒绝，只有 `read` 能过，
每次尝试都返回同一句话：

```
[RECOVERY_UNCERTAIN] The previous operation's effect could not be verified:
shell (call_00_…). This is a session-wide fence, not a transient error: every
tool except `read` is refused for the rest of this session, and retrying one of
these calls returns this same message. Do not retry them. …
```

重试没有用，换个命令也没用：围栏是会话级的，直到有人显式放行。

## 放行

```bash
future session unblock <session-id>                       # 放行
future session unblock <session-id> --why "我核对过了"     # 附上记录在案的理由
future session unblock <session-id> --json                # 机器可读
```

这个命令是幂等的：对已放行（或从未被围栏）的会话再跑一次，会输出 `not fenced: nothing to
release`，并且不再追加任何记录。

放行的只是被围栏的那几次调用，而且必须是真正被围栏的那几次——记录里写的是精确的
`(run_id, call_id)`，所以一次放行不可能悄悄覆盖它没提到的调用。

## 放行做了什么、没做什么

- **会**让会话恢复可用：从下一次 run 开始，`shell`、`write`、`edit` 照常工作。
- **会**让那次被中断的调用仍然接受一次质询：它下一次被调用时返回
  `[RECOVERY_ABORTED_OPERATION]`，要求模型先用只读方式核对结果，只在核对显示"确实没发生"
  时才重新执行。这条警告每个 run 只出现一次，不是永久——与用户主动中止的操作同一档。
- **不会**重放那次被中断的调用，也不执行任何东西。
- **不会**把这次操作标记为"已验证"。"已验证"是关于世界的事实（对能回读的完整 `write`、
  对可证明不会产生副作用的 shell 命令，`RecoverySafety` 自己就能证明），人的决定不是那个
  事实。把人的决定说成已验证，正是围栏要防的那种谎。

## 为什么不自动放行

围栏防的是一件具体的事：未知副作用被重放，或被默认为"已经没问题"。那次 shell 没人盯着跑完，
Agent 不能自己决定可以继续——它可能删了一个目录、写了半个文件、推了一个 commit。模型说
"看起来没事"也不算证据。所以放行是人的决定，记进会话自己的历史里，而且刻意很窄：一条记录、
精确的调用列表、谁放的、什么时候、可选理由。

如果你什么都没核对就放行，那是你接受了这个风险；Agent 仍然会提醒模型在重新执行前先核对。

## 记录了什么

追加一条 `custom` 类型的 entry，`name` 为 `recovery_acknowledged`：

```json
{
  "type": "custom",
  "name": "recovery_acknowledged",
  "content": {
    "calls": [{"run_id": "run-…", "call_id": "call_00_…", "tool": "shell"}],
    "acknowledged_at": "2026-10-25T22:31:07+08:00",
    "by": "cli",
    "reason": "我核对过构建目录"
  }
}
```

会话历史里的其它内容一个字节都没动：被中断的调用和它的 run 终态标记原样保留，所以"到底
有没有发生"这个问题在放行之后仍然可以回答。

这条记录只存在于 journal 里。会话视图（`future session transcript`、历史分页、HTML 导出）
和模型的上下文都会跳过自己不认识的 entry 类型，所以它不会混进模型读到的东西，也不会显示成
一轮对话。要把它读回来、并把它放行的 call id 与那次运行记下的 `[RECOVERY_UNCERTAIN]` 对着看：

```bash
sqlite3 "file:$HOME/.future/agent/agent.db?mode=ro" \
  "SELECT timestamp_ms, json_extract(metadata_json,'$.name'), content_json
     FROM entries
    WHERE session_id = '<session-id>' AND entry_type = 'custom'
    ORDER BY position;"
```

`mode=ro` 保证只读：这个存储只有 Agent 一个写者。

## 放行之前先核对

Agent 没法证明当时发生了什么，但通常可以便宜地查出来：

- `future session transcript <id> --grep <文本>` — 看后续有没有已经记录结果的同类调用。
- 看被中断的 `write`／`edit` 的目标文件：里面是不是已经有那段内容了？
- 对 shell 命令，看它的可观察结果（`git status`、构建目录、输出文件）。

核对完再放行，接下来的事交给模型；它本来就会被要求先只读核对。

## 其它客户端

放行动作就是一个 RPC 命令 `acknowledge_recovery`（`session.id` 会话，`message` 可选理由，
`created_by` 放行者），不经过 `future` 命令行的客户端可以直接调用。响应里列出被放行的调用：

```json
{"released": [{"run_id": "run-…", "call_id": "call_00_…", "tool": "shell"}],
 "already_released": false, "replayed": false}
```
