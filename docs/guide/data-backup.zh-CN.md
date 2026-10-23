# 数据备份与恢复

[English](data-backup.md)

`future data` 是本地 FutureOS home 的离线、协同备份与恢复入口。它复制两个
SQLite 数据库，以及承载会话、附件、评审仓库与 Agent 凭据的托管文件树，用
SHA-256 校验每一个复制出的字节，并能把这一整套放回原来的路径。它不是在线备份：
当 Desktop 后端或 Agent 占用着 home 时它会拒绝运行，也从不替你停掉任何进程。

下文的 `<home>` 指 FutureOS home —— 通常是 `~/.future`（Windows 为
`%USERPROFILE%\.future\`），或 `FUTURE_HOME` 指定的绝对目录。命令作用于当前进程
解析出的那个 home，所以执行命令的环境必须与你停掉进程的环境一致。没有 `--home`
参数。[目录布局](directory-layout.zh-CN.md) 说明了 home 下其余内容的归属。

## 命令

| 命令 | 参数 | 作用 |
| --- | --- | --- |
| `future data backup <new-directory>` | 恰好一个路径 | 向一个尚不存在的目录生成并校验备份。 |
| `future data verify <backup-directory>` | 恰好一个路径 | 重新校验已有备份。只读；不加锁，也不需要 Agent。 |
| `future data restore <backup-directory> --yes` | 路径加字面量 `--yes` | 把备份恢复到它被创建时的 home。 |
| `future data resume` | 无 | 完成被中断的恢复切换。 |
| `future data rollback` | 无 | 恢复被中断的恢复操作之前的文件。 |
| `future data --help`、`-h` | 无 | 打印同样的用法说明。 |

没有其他参数：任何其他组合都会以 `Invalid data command.` 加用法说明失败，进程退出码
为 1。`--yes` 不是跳过确认的开关，而是必须给出的第二个参数；缺少它同样算无效命令。

## 备份包含什么

备份是一个封闭集合，而不是 home 的镜像。只有下列根被收集；不在其中的内容不会被备份。

| `<home>` 下的路径 | 内容 |
| --- | --- |
| `agent/agent.db` | Agent 存储：会话、运行、事件、技能清单（SQLite 快照）。 |
| `app/app.db` | Desktop 存储：workspace、线程、运行、审批（SQLite 快照）。 |
| `app/images` | 每线程的图片树（`thumb/`、`origin/`）。 |
| `app/review` | 每 workspace 的影子 git 评审仓库。 |
| `workspaces/chat` | 每线程的聊天工作目录。 |
| `agent/sessions`、`agent/run-events` | 保留的旧 JSONL 迁移来源。 |
| `agent/session-backups` | `future session reset` 写入的重置前历史快照。 |
| `agent/settings.json`、`agent/models.json`、`agent/auth.json` | Agent 配置，包含 provider 凭据。 |

两个数据库都以独立快照的形式复制，而不是“文件加 WAL”：先对在线数据库开
`BEGIN IMMEDIATE` 事务阻塞其他写入者，再以只读方式打开并用 SQLite 的快照接口导出，
因此两个快照与托管文件清单不会跨越一次提交。数据库的 `-wal`/`-shm` 伴生文件从不
复制；如果它们出现在备份内的数据库旁边，校验会直接失败。

托管文件逐字节复制，以仅属主可读写（`0600`）写入后再算一次哈希；复制期间发生变化的
文件会让备份失败。符号链接和特殊文件被拒绝，而不是被跟随。备份根目录的
`manifest.json` 记录格式、id、源 home、目录清单以及每个文件的
`{path, bytes, sha256}`。

**备份含凭据。** `agent/auth.json` 就在里面，而且归档是普通文件、不加密。请存放在
加密介质上并保持私密。

## 不包含什么

home 下的其余一切，例如：

- `channels/config.json` 与各通道状态、TUI 设置、按键绑定与日志、定时任务账本
  `tasks/tasks.db`；
- `remote_pairings.json`、`remote_pairing.json`、`remote_peers.json`、
  `app/remote-previews/`、`app/relay-root-certs.pem`；
- 已安装技能（`agent/skills/`）、浏览器状态（`agent/browser/`）、Agent 日志、
  `run/`；
- `workspaces/chat` 之外的用户工作目录 —— 用户选定的项目目录从不复制。

这些属于配置或可重建状态。恢复确实需要时请单独复制。

## 开始前：停掉所有写入者

命令会去取进程自己持有的同一对锁，而且不会等待：

- `<home>/app/desktop.lock` —— Desktop 后端在整个生命周期持有时，GUI 和前台运行的
  `future host` 都一样。
- `<home>/agent/agent-instance.lock` —— Agent 持有。Desktop 或 `future host` 运行的是
  内置 Agent 时，停掉该进程即可同时释放两把锁；独立运行的 `future agent`，或 TUI
  自行拉起的 Agent，需要单独停掉。

所以要：退出 Desktop，停掉终端里的 `future host`/`future agent`，关闭自行持有 Agent
的客户端。锁被占用时会如实报告 —— `Desktop is running; quit it before data
maintenance` 或 `Agent is running; stop it before data maintenance`。

同一处检查也区分“被进程占用”和“文件系统根本不支持锁”（某些网络共享与 FUSE 挂载）。
后一种会说明这一点并要求你把 home 移到本地磁盘；这时退出进程没有用。

`future data verify` 是例外：它只读备份目录，不碰 home 和它的锁，因此可以在一切都在
运行时使用。

## 制作备份

```sh
future data backup /absolute/path/outside/the/home/backup-2026-10-23
```

- 目标目录**必须不存在**。备份从不覆盖，重试需要新目录；其父目录会按需创建。
- 目标必须**在 home 之外**。检查比较规范化后的路径，指回 home 的符号链接同样会被拦下。
- 备份先写进目标旁边的 `.future-backup-*` 暂存目录，在那里校验，再用一次重命名发布
  —— 失败不会在目标位置留下半个备份。
- 存在未完成的恢复时，新备份会被拒绝；请先处理它（见下文）。
- 成功时输出 `Verified backup: <destination>`。

每次都是全量复制，没有增量模式。目标空间要按托管文件树加两个数据库来准备。

## 校验备份

```sh
future data verify /absolute/path/outside/the/home/backup-2026-10-23
```

校验会读 `manifest.json`，以下任一不满足即失败：

- 格式标记与版本是当前支持的；
- 清单里的每个文件都存在且大小、SHA-256 与记录一致，列出的目录也都存在；
- 备份里没有清单未列出的文件或目录 —— 遗漏一条记录不能把存在的历史变成“不存在”；
- 两个数据库旁边没有 `-wal`/`-shm`/`-journal` 伴生文件；
- 两个数据库都通过 `PRAGMA quick_check`，Desktop 数据库头校验通过，Agent 数据库的
  `application_id` 与 schema 版本受支持。

成功时输出 `Verified backup <id>: <N> files`。请保持备份目录原样：往里增删文件，或用
工具打开其中的数据库（可能生成伴生文件），都会让它失效，之后的 verify 与 restore 都会
拒绝它。

## 恢复到原 home

```sh
future data restore /absolute/path/outside/the/home/backup-2026-10-23 --yes
```

先像 `backup` 那样停掉写入者。然后命令会：

1. 校验备份，并要求清单记录的 home 与当前 home 的规范化路径相同 —— 恢复不能搬家，
   因为已保存的工作目录路径与 workspace 身份要保持原意；
2. 把整套内容复制到 `<home>/.restore-work-<id>/incoming/`，在那里再校验一遍，并轮换
   两个数据库里的恢复代次标记（消息、运行、会话 id 不变）；
3. 写入日志与围栏文件 `<home>/.restore-incomplete.json`；
4. 逐个重命名切换托管根：现有内容移到 `<home>/.restore-work-<id>/previous/`，恢复内容
   移到原位。原有的 WAL/SHM 随其数据库一起让位，以免覆盖刚恢复的数据库；
5. 校验最终文件集，然后把围栏文件重命名进工作目录，解除围栏。

只有清单枚举的托管根会被移动。实例锁、技能目录、通道/TUI 配置以及 home 内其他文件
保持不动，所以用旧备份恢复不会把 home 的其余部分一并回滚。

成功时输出 `Restore completed. Previous files are retained for rollback inspection.`。
恢复前的文件保留在 `<home>/.restore-work-<id>/previous/`，没有任何东西会自动删除该
目录。恢复后请检查它，确认不再需要旧副本时自行删除。

因为恢复会替换两个数据库，备份之后写入的内容都会消失。如果当前状态可能还有价值，
先做一份新备份再恢复。

## 中断后的恢复：resume 与 rollback

一次恢复包含许多次重命名，可能在任意时刻被杀掉：断电、终端崩溃、合上笔记本。它按这种
情况设计：

- 围栏 `<home>/.restore-incomplete.json` 在**第一个**原文件移动之前就已写入，因此
  切换了一半的 home 不会被误认为健康状态。
- `future data resume` 从停止处继续切换，在移动下一个原文件之前会重新校验剩余的恢复内容。
- `future data rollback` 撤销这次恢复：已装上的恢复内容退回 `incoming/`，保留的原文件
  回到 home 中的原位，并且只有每个原文件的摘要仍然一致时回滚才会完成。
- 方向记录在日志里，所以回滚开始之后再执行 `resume` 会继续回滚，而不是重新应用备份。

两个命令都需要同一对锁 —— 先停掉写入者 —— 并且都可以重复执行：每个重命名边界都可续做，
重复运行是继续而不是从头开始。没有待完成恢复时，`resume` 或 `rollback` 会因为读不到
日志而失败。成功时输出 `Recovery operation completed; startup is available again.`

不要为了让卡住的恢复“过去”而删除工作目录或修改围栏。原文件缺失或被改动会按路径报告，
并刻意保留围栏；删除围栏只会掩盖一个不一致的 home。

## 启动围栏

只要 home 里存在 `.restore-incomplete.json`，所有会打开数据的组件都会拒绝启动，并给出
修复办法（`future data resume` 或 `future data rollback`）：

- Desktop 后端（GUI 或无窗口模式）及其 store 连接；
- Agent 启动时，以及它打开数据库时，包括技能注册表；
- 原本会自行拉起本地 Agent 的客户端。

围栏不是某条命令里的检查、可以换一个入口绕开：它就是 home 自身的状态。成功的
`resume`/`rollback` 通过把该文件重命名进恢复工作目录来解除它，于是这次操作被留下记录，
而不是被抹掉。

## 故障排查

| 提示 | 含义 | 处理 |
| --- | --- | --- |
| `Desktop is running; quit it before data maintenance` | 有东西持有 `app/desktop.lock`：GUI，或另一个终端/用户会话里的 `future host`。 | 退出它。看不到窗口时，找到前台 host 进程并用 Ctrl+C 停掉。 |
| `Agent is running; stop it before data maintenance` | 有东西持有 `agent/agent-instance.lock`。 | 停掉 Agent。`agent/agent-instance.json` 记录了发布该锁的进程 pid、主机、可执行文件与 home。 |
| `this file system will not lock <path> …` | home 位于不支持可用文件锁的共享或挂载上。 | 把 home 移到本地磁盘；在那里无法确认没有写入者在运行。 |
| `backup destination already exists` | 备份从不覆盖。 | 换一个新目标。 |
| `place complete backups outside the FutureOS home` | 目标解析后落在 home 之内。 | 移到 home 之外。 |
| `restore must target the original FutureOS home; managed working-directory paths are retained` | 当前 home 与清单中的路径不同。 | 在备份来源的机器与绝对路径上恢复；迁址的 home 需要手工迁移，而不是这条命令。 |
| `A FutureOS data restore is incomplete. Run 'future data resume' or 'future data rollback' …` | 围栏存在。 | 完成或回滚这次恢复；在此之前其他组件都不会启动。 |
| `managed file changed during backup` | 复制文件时仍有写入者在活动。 | 停掉所有写入者后重新备份。 |
| `backup inventory disagrees with its manifest`、`backup file verification failed`、`unexpected SQLite sidecar` | 备份目录在写好后被改动，或复制不完整。 | 完整重拷，或重新备份；不要就地“修复”。 |
| `restore preimage changed or is missing: <path>; startup remains fenced` | 该根保留的恢复前副本被改动或删除。 | 若你另有副本就放回原处再 `rollback`；否则工作目录是旧状态的唯一证据。 |

## 相关文档

- [目录布局](directory-layout.zh-CN.md) —— home 下的每个路径。
- [离线协同备份与恢复](../architecture/sqlite-migration.zh-CN.md#离线协同备份与恢复)
  —— 存储契约视角，含 `app/backups/` 与 `agent/backups/` 下的迁移快照。
- [Desktop Agent 生命周期与恢复](desktop-agent.zh-CN.md) —— 谁持有 home 及其锁。
