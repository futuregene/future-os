# Agent SQLite 迁移与运维

Agent 的唯一运行时存储为 `~/.future/agent/agent.db`；Desktop 继续使用独立的 `~/.future/app/app.db`。数据库模型见 [ER](../desktop/DEV_MD/ER.md#7-agent-sqlite-存储)，用户体验见 [PRODUCT](../desktop/DEV_MD/PRODUCT.md)。Agent 与所有客户端必须同步升级，不支持新旧 RPC 混搭。

## 自动导入

新版 Agent 在实例锁保护下、对外服务前导入旧 `sessions/*.jsonl` 与所属 `run-events/` 文件。每个会话独立事务：成功数据与导入记录一起提交；实质性损坏使整个会话跳过，其他会话继续。只允许可确认的截断尾行保留完整前缀并记录警告；磁盘、SQLite 等全局故障停止启动。

源文件不删除、不更新。导入后只读写 SQLite，没有 JSONL 双写、离线回退或降级。迁移记录独立于会话保留：成功项不重复导入，跳过项不自动重试，已删除项不复活。跳过会话仍可能保留 Desktop 入口，打开时明确失败，不伪造空历史。

## 检查与显式重试

先停止该用户的 Agent 及可能启动它的客户端；始终遵守每 OS 用户一个 Agent，不通过改 socket/端口启动第二个实例。

```sh
future-agent --migrate-sessions
future-agent --retry-session-import 会话ID
```

报告只含会话标识、状态、错误来源/行号/受控类别和警告数，不含对话正文。只有 skipped 项可重试；修复来源后显式执行，不覆盖成功项或已删除项。

隔离验证时将 `sessions/` 与同级 `run-events/` 复制到独立目录，使用：

```sh
future-agent --migrate-sessions --migration-source /绝对路径/隔离副本/sessions
future-agent --retry-session-import 会话ID --migration-source /绝对路径/隔离副本/sessions
```

数据库创建在该 `sessions/` 的父目录。隔离路径不豁免 Agent 实例锁。

## 备份与恢复边界

- 停止 Agent 后备份完整数据库及仍存在的 `-wal` / `-shm`；不要在运行期间仅复制 `agent.db`。
- JSONL 只是导入前快照，不包含后续 SQLite 新对话。重建库不能恢复这些新增数据，不应作为常规排障手段。
- 未发布开发布局不维护升级链；不认识的布局明确拒绝打开，不自动删除或覆盖。正式发布后的 schema 变化必须提供迁移。
- 高频 delta 按 100 ms / 128 条 / 64 KiB 微批提交，语义事件、读取与关闭构成刷盘边界。异常退出可能丢失未提交 delta；100 ms 是调度目标，不是丢失窗口的硬上限。
- `WAL`、`synchronous=FULL` 和事务保护已提交数据；WAL 保留目标不是磁盘占用硬上限，不需要每次启动运行 `VACUUM`。

## 回归要求

自动化夹具完全构造，使用临时目录，不读取个人对话。保留事务回滚、唯一性/冲突、导入幂等/跳过、删除墓碑、fork、运行恢复、工具隔离、分页范围、事件游标和队列故障测试。真机验证涵盖冷打开、首次回复、上翻、工具详情、压缩、异常、重启和重连；单元测试不替代跨平台及 WebView 验收。
