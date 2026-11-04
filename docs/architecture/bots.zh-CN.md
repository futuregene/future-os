# 机器人与频道（Bots and Channels）设计

[English](bots.md)

状态：设计稿。**实现尚未开始**：本文只用于评审与排期，落地由单独任务启动。Updated: 2026-11-04。

本文只描述设计：不含代码改动，不改任何实际配置，不创建任何 bot，也不触碰 `~/.future/` 下的运行数据。文中"现状即可支撑"一律指仓库里**已有**的机制，逐条给出代码位置与命令行；需要改动的地方单独成节，不与现状混写。

设计要解决的问题：在桌面端把"bot"与"频道"做成一等对象，让用户按角色而不是按任务组织工作，并保证机器人一侧的会话与用户自己的会话**完全隔离**。

---

## 1. Grok Bot 的事实

只写有原文出处的事实。产品文档在 `docs.x.ai`，账号与计费由 Cursor 承载；另有一份对 xAI 团队 72 小时直播的第三方整理（Field Notes），用来补官方文档没写的失败面，**它不是官方文档**，凡是引用都标明。

| 事实 | 原文 |
|---|---|
| Bot 是"有名字、有职责、有自己会话、上下文随时间累积"的持久同事，不是一个聊天线程 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots) |
| 所有 bot 共用**同一台云电脑**：文件对每个 bot 可见，浏览器 cookie 与命令行凭据共享；每个 bot 只拿到自己的一块"屏幕"，那是并行工作面、**不是安全边界** | [Use the computer and apps](https://docs.x.ai/grok-bot/computer-and-apps) |
| 电脑上有一个共享工作区 `/workspace` | 同上，"Work with files" |
| 记忆是**每 bot 独立**、长期、由 bot 自己决定何时保存；少数"关于你"的事实（名字、时区）全账号共享 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots)，"What a Bot remembers" |
| 群聊里 bot **可以使用你在私聊里对它说过的话**；每个 bot 保留自己的记忆，不与他人共享；群里所有人都能看到它说的话 | 同上，"Across chats, Bots, and calls" |
| 群聊 2–6 个 bot；`@Bot` 定向、`@everyone` 全员；正常说话时"由参与的 bots 自己决定谁该回应" | [Message and collaborate](https://docs.x.ai/grok-bot/chat-and-collaboration)，"Start a group chat" / "Direct a message" |
| bot 之间是**异步 handoff**：接收方被唤醒、处理、稍后回复；移交出现在各自私聊而不是群里 | 同上，"Let Bots hand work off" |
| 审批卡、密钥请求、邮件/Slack 草稿**不进群聊**；bot 对群的消息只能是纯文本 | 同上，"Start a group chat" |
| 有一个正式的"primary Bot"：主要联系人，**主动**汇报进展并与其他 bot 协调；没有时产品弹一张 "Introducing your primary Bot" 卡片，一键创建或从现有 bot 指定；可随时替换；共享 bot 不能当总管 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots)，"Your primary Bot" |
| 现有 bot 可以**建议或直接创建**一个专注的新 bot（当某项工作需要长期负责人时）；官方同时提醒"想保持名册小就先问一句再建" | 同上，"Create a Bot" |
| Skill 是**全账号一个库**，每个 bot 都看得到，没有按 bot 挂载 | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations)；另见 [`grok-bot-cli` 的 `gbot skills`](https://github.com/ScriptedAlchemy/grok-bot-cli) |
| Routine 归属于**某一个 bot**，按时间或事件触发，在云端跑 | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations)，"Create a routine" |
| 失败面一：群里每个 bot 都会回每条消息，表现为 "eager、爱说话、互相抢话" | [Field Notes · ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md)（第三方整理） |
| 失败面二：routine 频率是最贵的漏点（每 15 分钟一次 = 每天上百条） | 同上，§34（第三方整理） |
| 失败面三：**bot 工厂会过拟合**——"一个给新 bot 写描述的 bot，倾向于按它刚创建的那个场景过拟合" | [Field Notes · PRODUCT](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)（第三方整理） |
| 实践者的"chief of staff"模式：所有其他 bot 都通过它来建，它自己给新 bot 做 onboarding，并持有"谁在做什么" | [Field Notes · ORCHESTRATION](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/agents/ORCHESTRATION.md)（第三方整理） |
| 复制 bot 会带走档案、技能、routine、头像，**不带**会话历史、已学记忆、附件 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots)，"Share a Bot" |

从这些事实里抽出三条对设计有决定作用的：上下文是**分层**的（bot 私有 × 共享 × 场景）；唤醒在它的产品里是**模型判断**，而不是内核判断；隔离的边界在**记忆与会话**，不在文件系统。

---

## 2. 相对于 Grok Bot 的三处取舍

| # | 取舍 | Grok Bot | 本设计 | 代价 |
|---|---|---|---|---|
| D1 | bot 有自己的 workspace | 没有。全账号共享一台电脑与一个 `/workspace` | 每 bot 一个 workspace，是它的身份与记忆载体 | 跨 bot 交接必须显式；文件不自动可见 |
| D2 | 唤醒由内核判定 | 由参与的 bot 自己决定是否回应，实测会抢话 | 三档发言策略；`eager` 用**廉价判定**代替"每个成员一次完整 run" | 需要判定器，且判定不一定准（§6.3） |
| D3 | 记忆是文件 | 服务端、无界面、只能聊天里问"你记得什么" | `FUTURE.md` + `.future/memory/*.md`，可 grep、可 diff、可手改 | 文件不会自己总结，漏记需要人来补 |

**不抄的一件**：共享一台电脑。它用隔离换便利，而 FutureOS 的 workspace 本身就是权限与上下文单位；抄它等于把唯一的结构性优势扔掉。

---

## 3. 概念与数据模型

### 3.1 概念

| 概念 | 定义 |
|---|---|
| **Bot** | 一个 workspace（`AGENTS.md` 写职责、`FUTURE.md` 是记忆索引、`.future/memory/*.md` 是细节）+ 模型与思考级别 + 名字头像 + **一条私聊会话** |
| **Channel** | 成员（≥ 2 个 bot，可加人）+ **频道 workspace**（共享板）+ 转录账本 + 宪章 + 发言策略 |
| **Binding** | `(bot × channel)` 的会话：这个 bot 在这个频道里的上下文容器，懒创建 |
| **Turn** | 路由内核派发的一次发言：一次真实 run，结束时把最终文本写入转录 |
| **总管 Bot** | 一个 `is_primary` 的 bot，职责是路由、维护名册、替用户创建新 bot。它不是特权 agent，只是职责不同的 bot |
| **会话总管** | 用户自己的普通会话里那个 agent。它必须看不见、也碰不到机器人与频道的会话（§8） |

Bot 恰好等于一个 workspace 是刻意的：`cwd` = bot workspace 时，现有提示词组装器会自动注入 `AGENTS.md`（职责）与 `FUTURE.md`（记忆索引），见 §10.1。

### 3.2 表与库

定义、成员关系与转录账本放在**独立的库** `~/.future/bots/bots.db`，与 `tasks.db`、`fusion.db` 同级、同构：`store_dir`／`store_path`／`open`／`open_existing_read_only`、`PRAGMA busy_timeout`、`meta.schema_version`，模板见 `orchestration/tasks/src/store.rs`。

为什么不并进 `app.db`：`app.db` 承载启动、三个列表与 run 状态，是全应用读得最频繁的库，而转录是高频追加；表若放进 `app.db`，内核 crate 就得依赖 `host::store`，把控制面和展示层焊死；`app.db` 已是发布版 schema，加表要走版本化迁移，而新库首次使用时自建、零迁移风险；转录是 append-only，可能需要自己的回收，与 threads 的更新／归档语义不同。

隔离不需要同库：判断点是枚举器自己的表上的 `threads.scope`（§8.2），不是与 `bots` 的 JOIN。

```sql
bots(
  id TEXT PRIMARY KEY, name TEXT NOT NULL, job TEXT NOT NULL,
  avatar_shape TEXT, avatar_color TEXT,
  workspace_id TEXT NOT NULL,                   -- 软引用 app.db workspaces.id
  model_id TEXT, thinking_level TEXT, permission_level TEXT,
  dm_thread_id TEXT,                            -- 软引用 app.db threads.id
  is_primary INTEGER NOT NULL DEFAULT 0,        -- 每个 home 至多一个
  created_by_bot_id TEXT REFERENCES bots(id),   -- 只作审计，不授予权限
  pinned INTEGER NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);
CREATE UNIQUE INDEX bots_one_primary ON bots(is_primary) WHERE is_primary = 1;

channels(
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  charter TEXT NOT NULL DEFAULT '',
  workspace_id TEXT NOT NULL,                    -- 软引用 app.db workspaces.id
  summary TEXT NOT NULL DEFAULT '',
  speak_policy TEXT NOT NULL DEFAULT 'router',   -- mention_only | router | eager
  router_bot_id TEXT REFERENCES bots(id),        -- NULL = 用总管
  allow_private_context INTEGER NOT NULL DEFAULT 0,
  max_turns INTEGER NOT NULL DEFAULT 8,
  max_depth INTEGER NOT NULL DEFAULT 2,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER, deleted_at INTEGER
);

channel_members(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  role_note TEXT NOT NULL DEFAULT '', joined_at INTEGER NOT NULL,
  PRIMARY KEY (channel_id, bot_id));

channel_bindings(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  session_id TEXT, persona_hash TEXT,
  PRIMARY KEY (channel_id, bot_id));

channel_turns(
  id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, seq INTEGER NOT NULL,
  author_kind TEXT NOT NULL,                 -- user | bot | system
  bot_id TEXT, text TEXT NOT NULL,
  run_id TEXT, session_id TEXT,
  status TEXT NOT NULL,                      -- queued | running | done | failed | cancelled
  mentioned_json TEXT NOT NULL DEFAULT '[]',
  wake_reason TEXT,                          -- 谁或哪条规则把它叫醒的
  error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER);
CREATE UNIQUE INDEX channel_turns_seq ON channel_turns(channel_id, seq);
```

对既有表的唯一改动（在 `app.db`）：

```sql
-- host/src/store/migrations.rs 的版本化迁移；schema.rs 同步给新库
ALTER TABLE threads ADD COLUMN scope TEXT NOT NULL DEFAULT 'user';  -- user | bot | channel
```

`scope` 是隔离机制的唯一真相列（§8），不是派生字段：`bots.dm_thread_id` 与 `channel_bindings` 里出现过的线程，`scope` 必须与之一致，并由一致性检查兜底（§3.3）。

复用而非新建：`workspaces`（bot 与频道各占一行 `kind='user'`，出现在侧栏是可接受的，也让人能直接打开目录）、`threads`（bot 私聊就是一条普通 thread）、会话本身仍归 agent 的库（`created_by='bot'` 只是多一个标记）。

### 3.3 跨库一致性

独立库的代价是**没有跨库事务**。这里只有三类跨库操作，逐条定死：

| 操作 | 顺序 | 崩在中间的表现与修复 |
|---|---|---|
| 建 bot | ① 写 workspaces 行 → ② 建 thread（`scope='bot'`）→ ③ 写 bots 行 | ①②之后崩：孤儿 workspace／thread，被启动扫到的清理器按 `scope='bot'` 且无 bots 行判定回收 |
| 建频道 | ① 写 workspaces 行 → ② 写 channels 行 | 孤儿 workspace 同上回收 |
| 删 bot／频道 | ① stop 执行器并清队列 → ② 删 bots.db 行（软删）→ ③ 删 thread 与 workspace | ②之后崩：库里没有 bot、thread 还在——它已被 `scope='bot'` 排除在用户列表外，启动扫描补删 |

三条规则：

1. **单写者**：`bots.db` 的写者是 host（GUI 与 headless 共用一份服务），与 tasks 的假设相同；其他进程只读或走服务。
2. **引用悬空只降级、不报错**：`dm_thread_id` 指向的 thread 不存在时，bot 显示为"会话已删除，可重建"，不阻塞列表、不抛错。与 tasks 的 `session_deleted` 是同一套做法。
3. **启动扫描幂等**：修复器只做两件事——回收无主 workspace／thread，补删已删 bot 的 thread。它不推断用户意图，也不自动重建会话。

目录布局：

```
~/.future/bots/
  bots.db                    # 定义、成员、binding、转录账本
  workspaces/<slug>/         # 各 bot 的默认工作区（普通 user workspace，指向这里）
```

状态目录与工作区目录分开，且**不落在** `~/.future/workspaces/chat/`——那里是 per-thread 工作区，会被回收器清理。bot 也可以指向用户已有的 workspace，此时不新建目录。

### 3.4 CLI 面

与其他控制面一致：读写同一份 `bots.db`，不新增后端。

```
future bot list|show|new|rename|set|hide|delete
future channel list|show|new|add|remove|post|policy
```

`future bot new` 是"一句话建 bot"的唯一入口：人和 bot 走同一条路。

---

## 4. 上下文装配

### 4.1 三层

| 层 | 内容 | 生命周期 | 注入方式 |
|---|---|---|---|
| **L1 bot 私有** | 职责（`AGENTS.md`）、记忆索引（`FUTURE.md`，按 30 条 / 100 行 / 12 KB 上限 lint）、细节记忆（按需 `read`）、私聊会话历史 | 永久 | `cwd` 自动注入前两项；私聊历史在 bot 自己的会话里 |
| **L2 频道共享** | 宪章、成员花名册（名字 + 一句话职责）、全体机器人花名册、频道 workspace 路径、频道摘要、发言策略、说话规则 | 频道存活期；内容 hash 变化才刷新 | 会话级 `append_system_prompt` |
| **L3 本回合信封** | 谁叫醒你、那条消息原文、转录窗口、本回合要求 | 单回合 | 用户消息正文前缀 |

### 4.2 装配形状

系统提示词（每个 `(bot × channel)` binding 一份）：

```
<现有身份段 / 工具段 / 审批段 / 通讯段 —— 不变>

# Project Context            ← cwd 的 AGENTS.md（bot 的职责）
# Workspace Memory           ← cwd 的 FUTURE.md（bot 的记忆索引）
# Available Skills           ← 全局技能库（全账号共享）

<append_system_prompt —— 频道层>

# 频道：Launch
宪章：<charter>
成员（按发言策略决定谁接话）：
- @Writer（写发布稿）
- @Reviewer（只列阻塞问题）
- @你（Research）
本机其他机器人（不在本频道，仅供你判断该找谁）：
- @Support（客户问题）
频道工作区：<channel-workspace>（与所有人共享）
频道摘要：<summary>

# 说话规则
- 你的回合以一条频道消息结束；直接给结论，不要复述别人说过的话
- 要别人参与就 @ 他，并说清要他做什么；不 @ 等于没叫
- 频道工作区可以自由读写；你自己的 workspace 是私有的，不要把私密内容写进频道
- 你要别人手里的文件，给出绝对路径并 @ 他，不要假设对方读得到
- 不要列举、打开或引用本机用户的会话；你只看得见本频道与你的名册
```

用户消息（每回合）：

```
[频道最近 N 条]
Research · 10:02 · <正文，单条截断 800 字>
Writer   · 10:04 · <正文>
[现在] @Research 说：<原文>
[你被 @ 了] 回答上面这条；不需要别人参与就不要 @ 任何人。
```

### 4.3 放置规则（有代码依据）

1. **append 段在 project context 与 workspace memory 之后**，因此它可以覆盖 bot 自己的 `AGENTS.md`。频道层必须**只做加法**：不复述、不改写职责。依据：`agent/src/prompt/mod.rs::build_prompt` 的段序，以及 `agent/src/rpc/session_prompt.rs::build_system_prompt` 把 append 段读在 context 与 memory 之后。
2. **稳定前缀优先**：L1、L2 放系统提示词，L3 放用户消息；L2 只在 `AGENTS.md`／`FUTURE.md`／宪章／成员变化时刷新（内容 hash）。本仓实测：替换系统提示词、去掉工具定义或多加一行，已预热前缀上的命中从 93.7% 掉到 **0%**，整段对话重新按全量计费（[压缩设计](../internals/compaction/compaction.zh-CN.md)，缓存与前缀一节）。
3. **身份不重复注入**：L1 由 `cwd` 提供，L2 不再抄一遍，否则同一份职责出现两次，模型会按更靠后的那条行事。

### 4.4 预算与截断

| 项 | 默认 | 说明 |
|---|---|---|
| 转录窗口 | 3k token，按模型上下文自适应 | 从最旧的整条消息开始裁，不裁半条 |
| 单条消息 | 800 字，head…tail | 与 `task_runs.result_text` 同一形状 |
| 滚动摘要 | `channels.summary`，≤ 1200 字 | 代表被裁掉的消息，由一次独立摘要 run 写入 |
| 摘要触发 | 被裁消息累计 > 2000 字或 > 8 条 | 避免每回合都摘要 |
| 私聊上下文 | 默认**不进**频道 | 频道开关 `allow_private_context`，默认 0 |

### 4.5 融合规则

- **记忆只能自己写**：bot 只写自己 workspace 的 `FUTURE.md` 与 `.future/memory/`。跨 bot 传递只能通过"说出来的话"或频道工作区的文件，与 Grok Bot 的"除非一个 bot 给另一个发消息，否则什么都不共享"一致（[PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)）。
- **频道摘要不是记忆**：`channels.summary` 是频道的事实，只注入在该频道发言的回合，不写进任何 bot 的 `FUTURE.md`。
- **过程不进转录**：一个回合贡献最终文本 + 一行活动摘要，完整工具过程留在 binding 会话里可展开。代价是看不到别人正在做什么，用一行状态补。
- **失败进转录**：`failed`／`cancelled` 写一条 system 消息，说明谁失败、为什么、怎么重试。Grok Bot 的群聊没有这个位置，现场表现是"看起来没人理你"。

---

## 5. 发言策略

`channels.speak_policy` 三档，差别只在"判断"与"干活"是否分开：

| 策略 | 每条用户消息的模型调用 | 手感 | 成本量级 |
|---|---|---|---|
| `mention_only` | K 次真实 run（K = 被 @ 的数量） | 像分机号 | 1× |
| `router` | 1 次轻判定 + K 次真实 run | 接近"它自己接了" | ≈1× |
| `eager` | N 次轻判定（并行）+ K 次真实 run | 与"让它们自己决定"一致 | ≈1×，N ≤ 6 |

关键差别在量级：真实 run 带身份段、工具定义、技能清单、记忆索引，是数十 KB 级；判定请求只带"一句职责 + 最近 3 条 + 这条消息"，几百 token，且不带工具。因此 `eager` 把每个成员都问一遍，总代价仍远小于"每个成员跑一次完整 run"——这正是 Grok Bot 的群聊成本来源（[ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md)）。具体倍率要实测（§15）。

**默认是 `router`。** 新建频道默认把总管计入成员（§9.2），由它判定；用户拒绝创建总管时退到 `mention_only`，不会在没有判定器的情况下假装 `router`。`eager` 默认关闭，作为频道级选项保留。

判定器的契约：

- 输入只有：本机花名册（名字 + 一句话职责）、本频道成员、最近 3 条消息、这条新消息、自己的职责一句话。
- 输出只有：`(bot_id, 一句理由)` 列表，或"无人接话"。
- 不带工具、不读记忆、不读工作区，因此会误判。两条缝：判定可以回 `maybe`，只有在预算尚有余量时才落地为真实 run；被误唤醒的 bot 有权在自己的回复里说"这条不该我"，并计入预算。
- 判定失败（超时、模型报错）→ 退回 `mention_only`，并写一条 system 消息，绝不静默不回。
- `wake_reason` 记进 `channel_turns`，让"为什么它接了"可解释。

---

## 6. 路由内核

放在 `orchestration/bots`，与 goal／tasks／fusion 同构：确定性内核 + 纯函数 + SQLite 持久化，执行器在宿主。

```rust
mentions(text: &str, roster: &[BotRef]) -> Vec<BotId>
enqueue(turn: &Turn, mentioned: Vec<BotId>) -> Vec<QueuedTurn>
judge(roster, recent, message) -> Vec<(BotId, String)>   // 仅 router / eager
should_stop(queue, budget, seen) -> Option<StopReason>
```

`mentions` 的解析规则：

1. 一个提及是 `@` + 与花名册**某个名字完全相等**的最长匹配，名字后必须是空白、行尾或标点。
2. `@` 前必须是空白或行首（`mail@host`、`src/@scope` 不算）。
3. 代码围栏与 `>` 引用行内不解析。
4. 名字大小写不敏感匹配，输出用规范名。
5. 排除作者自己；重复提及只算一次，保留首次顺序。
6. 不匹配任何名字的 `@xxx` 是普通文本：不报错、不唤醒。

终止与预算：

| 规则 | 默认 | 理由 |
|---|---|---|
| 每条用户消息的 bot 回合上限 | 8 | 直接对应"每个 bot 都回每条消息"这个成本坑 |
| `@` 链深上限 | 2 | 通常"一个 bot 叫两个 bot 一次"就够 |
| 同一 bot 不连续发言 | 硬规则 | 防自我循环与两个 bot 互相 @ 烧预算 |
| 回合内不 @ 任何人 | 结束链 | 沉默即终止 |
| `@` 非成员 | 不唤醒，记一条 system 提示 | 不做隐式拉人 |
| `@all` | 允许，计入上限 | 保留能力，让成本可见 |

"谁被 @ 了"是可以精确判定的字符串问题，因此交给代码；"这条消息该不该由我接"是语义判断，因此交给判定器，且判定器的产物必须可解释、可回退。

---

## 7. 执行器

`host/src/bots/executor.rs`，与 `host/src/tasks.rs` 的 tick 循环同构（GUI 与 headless 共享同一份服务，Tauri 侧只是薄适配，参见 `desktop/src-tauri/src/commands/tasks.rs`）。

- **同频道串行**，保证转录顺序；**跨频道最多 2 路并行**，与 fusion 的并发上限一致。
- **一个回合 = 一次真实 run**：按 binding 会话发 `future run --session <sid> --model … --thinking … "<信封>"`。
- **回合结束**：最终文本写入 `channel_turns`（`status='done'`），解析其 `@` 时按 `speak_policy` 决定是否再跑判定，然后派发；无人 @ 且判定也说没人接，链结束。
- **超时**：单回合 600 秒量级；超时先 abort，再写 system 消息。
- **审批**：bot 的动作可能停在审批上。频道里只显示"等待批准"一行，审批卡回到 bot 私聊与全局待办（与 Grok Bot 的"审批不进群"一致）。因此**人不在时频道会停下来**——这是设计意图，必须让人看得见（§11.2）。
- **取消**：删除频道或关闭 bot 时先 stop 再 delete，遵循 goal 平面"delete 先 stop"的教训。
- **刷新**：任何写操作成功后发 `bots-updated`，与 `tasks-updated` 同一做法——否则"手机端口建了一个 bot、桌面上看不见"会复现任务面板当年的现场报告（见 [tasks.md](tasks.zh-CN.md) §11）。

---

## 8. 会话隔离（硬要求）

要求有三条，且都是"必须"：

1. "最近"与"工作区"列表**不得**包含机器人创建的会话；
2. 机器人页面**不得**包含"最近"与"工作区"的会话；
3. 用户会话里的 agent（会话总管）**不得**触碰机器人与频道的会话。

### 8.1 两层，各用各的手段

会话在系统里有两条独立的枚举路径，而它们分别位于两个不同的库里：

| 层 | 枚举器 | 位置 | 手段 |
|---|---|---|---|
| **线程层** | 桌面的工作区／最近／归档三视图、远程合并行、手机端目录 | `app.db` 的 `threads` | `threads.scope` 过滤（§8.2） |
| **会话层** | `future session list`、`#` 会话提及、会话搜索 | agent 自己的库 | **bot 跑在独立 agent 实例里**，用户侧实例里根本没有这些会话（§8.3） |

线程层必经过滤：界面列表读的就是 `threads`（`host/src/store/threads.rs::list_threads` 今天只过滤 `status != 'deleted'`，全量返回）。会话层则不靠过滤，而是靠**不在同一个库里**——这是要求 3 唯一结构性的解法。

### 8.2 线程层

- `threads.scope`（`user` | `bot` | `channel`）是唯一真相列；`list_threads` 及其同类查询加 `scope = 'user'`，机器人页走 `scope <> 'user'`。一处同时满足要求 1 与要求 2。
- `#` 会话提及在机器人与频道表面**整段禁用**（要求 2 的落点；bot 之间用 `@`，语义不冲突）。频道里的 `@` 菜单只含机器人，不含用户的会话。
- `scope` 不是派生字段：`bots.dm_thread_id` 与 `channel_bindings` 里出现过的线程必须带对应 `scope`，由 §3.3 的一致性扫描兜底。

### 8.3 会话层：bot 的独立 agent 实例

bot 的会话跑在**第二个 agent 实例**里，状态根为 `<home>/bots/agent-home/`。这不是新机制：`FUTURE_HOME` 的文档注释写的就是这件事——"替换整个状态根，这正是第二条完全隔离的 Agent 实例的跑法：它有自己的单例锁、数据库、会话、日志与本地 IPC 端点"（`agent/src/utils/mod.rs::future_home`）。

```
~/.future/bots/
  bots.db                       # 定义、成员、绑定、转录账本（§3.2）
  agent-home/                   # FUTURE_HOME：bot 的 agent 实例
    agent/agent.db              # bot 的会话与 run 事件
    agent/sessions/             # bot 的会话文件
    agent/settings.json         # bot 实例自己的设置
    run/agent.sock              # bot 实例自己的 IPC 端点
  workspaces/<slug>/            # 各 bot 的默认工作区
```

由此得到三件事：用户侧 agent 的会话表里没有 bot 会话（不可枚举，而非被过滤）；频道转录与 bot 会话的增长不拖慢用户的 `agent.db`（本仓有过 run journal 撑到 4.4 GiB 的现场）；两个实例可以独立重启，一个崩溃循环不拖另一个。

**配置共享**（本方案唯一需要改 agent 的地方）：`models.json`、`auth.json`、技能目录都在 `<home>/agent/` 下，`FUTURE_HOME` 一换就变成两份——加 provider、登出、装技能都要做两次。v1 的做法是加一个**配置目录覆盖**（指向用户那份 `agent/` 配置目录），只影响读取、不动数据。不用符号链接（Windows 需要特权或开发者模式），也不用复制（两份会漂移）。技能上：`~/.future/agent/skills` 跟着 home 走，而 `~/.agents/skills` 本来就在真实用户 home 下、不受影响——用同一个覆盖把它们指回同一路径，bot 与用户看到同一套技能。此后的目标形态是把 `models.json`、`auth.json`、`skills/` 提到 `~/.future/config/`，两边天然共享、覆盖字段作废；那是一次改动已发布布局的任务（迁移、备份清单、沙箱硬拒路径、文档），与机器人功能分开排（§12 第 2 条）。`settings.json` 不进共享：每实例一份（§12 第 3 条）。

### 8.4 实例的形状

| 方面 | 做法 |
|---|---|
| 进程与监督 | `desktop/src-tauri/src/agent_supervisor.rs` 现有的看门狗与崩溃恢复链参数化一个 home，跑两条；headless 宿主同理 |
| 端点 | bot 实例用自己的 `run/agent.sock`；执行器连它，界面从不直接连（只认"机器人"这一个概念） |
| 备份 | `future data` 的枚举里加 `bots/bots.db` 与 bot 的 `agent-home`（现有清单是 `agent/agent.db`、`app/app.db`、`fusion/fusion.db`、`tasks/tasks.db`，见 [data-backup.md](../guide/data-backup.zh-CN.md)） |
| 诊断 | `future run／session／logs／doctor` 多一个"哪个实例"维度；今天用 `FUTURE_HOME=… future session list` 已可做到，由 bot 服务内部封装，不暴露给界面 |
| 远程与手机 | 仍走宿主代理（`bots_v1`），手机不需要知道有两个 agent |

### 8.5 必须逐条验收的入口

漏一处就会出现"我明明没建这个会话"。以下都要在验收时点一遍，且都要有测试：

1. 「工作区」视图及其树（含归档区）；
2. 「最近」视图（含分页与最近活跃排序）；
3. `#` 会话提及菜单；
4. 会话搜索；
5. 未读点与 run 状态轮询；
6. 审批与待办计数——bot 停在审批上的回合不得把红点挂到用户会话上；
7. 远程合并行（含另一台桌面推来的会话目录）；
8. 手机端会话目录与「最近」；
9. 用户侧 `future session list`（要求 3 的可观测面：不是"过滤掉了"，而是"本来就没有"）。

### 8.6 边界与残留

**它不是安全边界。** bot 与用户是同一个操作系统用户、同一台机器，而 bot 默认完全访问（§10）：一条 `sqlite3 ~/.future/agent/agent.db` 照样能读用户的库。独立实例买到的是**结构性不可枚举与事故隔离**，不是"读不到"。要后者只有两条路：给 bot 加沙箱策略（与全访问冲突），或换操作系统用户（超出范围）。这句话必须被当成设计陈述，而不是安全承诺。

**认证式拒绝不在本设计内。** 若要连"知道 session id 也读不到"，需要 agent 对 `created_by='bot'` 的会话拒绝 `transcript`／`run`／`delete`，直到调用者被认证为宿主执行器；而那依赖一条可信 CLI 通道，本仓记录为**尚未设计**——`agent/src/sandbox/rules.rs` 的层 0 注释写着 "proper fix is a dedicated credential channel（agent injects a short-lived scoped token via env …）" 并标注未排期。它排序在独立实例之后。

---

## 9. 总管 Bot 与建 bot 的门槛

### 9.1 总管是一个职责不同的 bot，不是特权 agent

它建 bot 的能力就是它本来就能跑的 CLI（`future bot new`、`future channel new`）。没有第二条权限路径，也没有它专属的后门。

| 项 | 设计 |
|---|---|
| 唯一性 | `bots.is_primary`，部分唯一索引保证每个 home 至多一个 |
| 创建 | 首次进入机器人视图时给一张卡："设置你的总管"，一键创建（默认工作区 `<home>/bots/workspaces/chief/`、默认模型、默认职责模板），或从现有 bot 指定一个。对齐 [Grok Bot 的 primary Bot](https://docs.x.ai/grok-bot/bots) |
| 职责 | 路由（`channels.router_bot_id` 为空时由它判定）、维护名册、替用户建 bot 与频道、汇报进展。**不是**通用助手 |
| 记什么 | 名册、谁在做什么、上次派活的结果（写在它自己的 `FUTURE.md`） |
| 不该做什么 | 不列举、不读取、不引用用户的会话；不代为回答用户会话里的问题 |

### 9.2 建 bot 的门槛

| 动作 | 削掉什么 |
|---|---|
| 一键建总管 | 目录、模型、职责模板全给默认值 |
| 一句话建成员 | 在总管私聊里说"再建一个负责发布稿的"，总管按**固定骨架**写 `AGENTS.md`（职责／边界／输入来源／验收／何时找谁），不是自由发挥——这正是对治"bot 工厂过拟合"的做法（[PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)） |
| 建频道自带默认成员 | 新建频道默认把总管算进去，"至少两个 bot"由"总管 + 1"满足 |
| 从会话长出来 | 一段会话在承担长期职责时，给一条建议："要把这件事固化成 bot 吗？" |

一条护栏：**名册只增不减是这套系统的典型病**。名册页显示"谁是总管 / 谁最近 7 天没被用过"，建议归档而不是删除——隐藏代替删除是 Grok Bot 明确写下的做法（[Create and manage Bots](https://docs.x.ai/grok-bot/bots)：想留着就隐藏，别删）。总管一次建多个之前先问一句，同样来自它的建议。

### 9.3 自然语言建 bot 的路径

总管在自己的私聊里调用 `future bot new --name … --job …`，写库成功即建好，桌面通过 `bots-updated` 刷新。因为 bot 会话默认完全访问（§10），这条命令不会被沙箱拦住。

### 9.4 一到三句话：创建的三条路径

目标手感：用户不需要选目录、不需要选模型、不需要写职责模板。三条路只在“职责由谁产生”上不同。

| 路径 | 用户做什么 | 背后发生什么 | 代价 |
|---|---|---|---|
| **A 首屏一键** | 点"设置你的总管" | 建目录、按骨架写 `AGENTS.md`、写行、开会话 | 只解决第一个，也是唯一需要点的那个 |
| **B 输入框一句话** | 在机器人页输入 `新机器人：负责发布稿，接受 Writer 定的调子，只写不审` | 名字从职责取（冲突加序号）→ 建目录 → 按骨架写 `AGENTS.md` → 写行 → 打开聊天 | 职责来自用户原话，系统不做推断 |
| **C 先建后长** | 先建一个占位 bot，**第一句话就是职责** | 第一个回合结束时，执行器按骨架把这句话压成 `AGENTS.md`；无职责前标灰 | 多一次对话；成败取决于骨架质量 |

```
你：你负责把每周的发布稿写成短文，风格照上次那篇
它：（把这句话写进自己的 AGENTS.md，复述三条边界，问一个必要的问题）
```

**确认规则**（§12 第 8 条）：

- 用户**说出了职责**（"建一个负责发布稿的"）→ 直接建，不问。
- 总管**自己推断**出该有个新 bot（"这活儿值得一个长期负责人"）→ 先问一句再建。

这样"名册只增不减"的护栏还在，但不在主路径上挡人。

**三条护栏**：

1. **固定骨架只填槽位**，不让模型自由写段落——这正是 Grok Bot 自己踩过的坑："bot 工厂给新 bot 写描述，倾向于按它刚创建的那个场景过拟合"（[PRODUCT](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)）。职责文件是可读可改的普通文件，这是相对它的结构性优势。
2. **占位 bot 无职责时标灰**，不参与频道默认成员，也不自动开 routine。
3. **默认工作区** `<home>/bots/workspaces/<slug>/`；要指向现有项目时用 `future bot set <id> --workspace <path>`，不重建。

---

## 10. 权限与沙箱

### 10.1 默认：完全访问

bot 会话创建时 `permission_level='all'`，且**不传 `sandbox_tier`**。按 `agent/src/rpc/session_prompt.rs`，无策略时走 `ResolvedSandbox::disabled`，`evaluate()` 第一行即 `return Allow`，规则层整套不评估。

由此得到三个明确结论：

1. **双根问题消失**：bot workspace 与频道 workspace 都能直接读写，不需要 `extra_roots`，也不需要生成规则文件。
2. **层 0 也不生效**：`models.json`、`approval_rule.json` 的硬拒同样不评估，bot 可以改自己的模型配置与规则文件。这是"完全访问"的字面含义，写在这里避免误读。
3. **`future bot new` / `future channel new` 不需要审批**，因此"一句话建 bot"是零摩擦的。

### 10.2 由此产生的不对称

文件层面没有隔离，会话层面要求完全隔离（§8）。这两件事不能互相解释：隔离的对象是**会话与上下文**，不是文件。一个 bot 能读整台机器上的任何文件，但它看不到、也进不去用户的任何会话——后者由 bot 的独立 agent 实例（§8.3）给出，不由文件权限给出。§8.6 明说：独立实例不是安全边界。

### 10.3 选择收紧时的手段（非默认）

| 手段 | 做法 | 代价 |
|---|---|---|
| 只开层 0 | 给 agent 加一个"仅层 0"的 tier：不评估 workspace／user 规则，但保留 `models.json`、`approval_rule.json`、凭据文件的硬拒 | 需要改 agent（新增 tier 枚举 + 三端验证） |
| 双根白名单 | `SandboxPolicy.extra_roots` 或 per-bot `approval_rule.json` | 前者改 proto + agent；后者是生成物，路径失效会静默退回审批 |

两者都不在 v1 范围内。

---

## 11. 失败面

### 11.1 两个 bot 看不见对方的 workspace，会不会死锁？

不会死锁，但会静默卡住，两者要分清。

- 结构上没有死锁：唤醒只由 `@` 或判定器触发，且由内核决定；一个 bot 的回合**不能阻塞等待**另一个 bot 的回合，它只能产出文本。没有等待边就没有死锁。
- 会卡住的有三种，都必须可见：
  1. **A 把结果写在自己的 workspace 然后 `@B` 说"看结果"。** B 读那个路径要么失败、要么按 §10 读到（全访问下读得到，但这也意味着"私有"只是约定）。要么 B 回一句"我需要这个文件，请贴出来或放到频道工作区"，要么在没有证据的情况下作答。
  2. **两个 bot 互相 @ 到预算耗尽。** 终止是确定性的：链深到 2 或回合到 8 时内核停止，写一条 system 消息说明原因，最后两条发言留在转录里。人看到的是"它们聊了 4 条然后停了"。
  3. **回合结束既没 `@` 也没回答问题。** 最难区分的一种。缓解：把"你的回合以一条频道消息结束"写进 L2，并在 `done` 却无文本时补一条 system 消息。

### 11.2 一个回合停在审批上

- 症状：该回合停在 `ask`，频道显示"等待批准"，run 不结束；人不在时一直停到超时被 abort。
- 缓解：说话规则要求写前先判断目标位置；频道里等待批准的回合可点击，直接跳到私聊的审批卡；单回合超时后 abort 并写 system 消息，一个 bot 不能拖垮整个频道；**频道内不允许批量批准**。

### 11.3 `@` 语义与既有文件引用的兼容性

现状：在用户会话的编辑器里，`@` 是工作区文件提及（`desktop/src/features/agent/MentionEditor.tsx`，文件 pill 序列化为 `[name](./path)`），`#` 是会话提及。**在此基础上，机器人与频道表面只有一件事不同：`@` 只叫人，不叫文件。**

| 既有行为 | 改动后 | 结论 |
|---|---|---|
| 已存草稿与消息里的文件 pill | 序列化为 markdown 链接，不经过提及解析 | 不受影响 |
| `#` 会话提及 | 用户会话里不变；机器人与频道里整段禁用 | 符合隔离要求（§8.2） |
| 用户会话里的 `@` | 不变：仍提工作区文件 | 两套界面各管各的 |
| 机器人与频道编辑器里的 `@` | **只列机器人**：本频道成员 → 其他机器人 | 菜单里没有文件组（§12 第 6 条） |
| 机器人表面要给文件 | 拖入，或直接写路径 | 文件是工具层的事，不是叫人 |
| 手打 `@./src/main.rs` | 不匹配花名册，按普通文本处理 | 不报错、不唤醒 |
| bot 自己写的 `@Writer`（纯文本） | 内核可解析 | CLI、手机、bot 生成的消息都是纯文本 |
| 机器与文件同名（有个 bot 叫 `docs`） | 花名册匹配优先 | 建 bot 时给命名冲突提示 |

没有数据迁移：既有数据里不存在"`@名字` 表示机器人"的语义，且用户会话里的 `@` 行为未被改变。

---

## 12. 已定的选择

下列每一项都已定，写在这里是为了让后来的人知道**为什么**，以及在什么条件下应该重新考虑（而不是追述它曾经如何）。

| # | 选择 | 理由 | 何时推翻 |
|---|---|---|---|
| 1 | **权限默认全开**：bot 会话不传 `sandbox_tier`，`permission_level='all'`（§10.1） | 执行者就是这台机器的主人，改坏配置的代价是重装一次；而一条必须长期维护的策略层是持续成本 | 把 bot 开放给主人以外的人使用时，换成"仅层 0"（不许改模型配置与密钥） |
| 2 | **配置先"指过去"，后提库**：v1 加一个配置目录覆盖，让 bot 实例读用户那份 `models.json`、`auth.json`、app 作用域技能；把它们整体提到 `~/.future/config/` 是独立的后续任务 | 覆盖是一处改动，行为与提库一致；而提库改动已发布的数据布局，要同时动迁移、备份清单、沙箱硬拒路径与文档 | 第二个实例稳定运行后，按独立任务提库；届时两边自然共享，覆盖字段作废 |
| 3 | **`settings.json` 每实例一份** | 里面有默认模型、审批档位、压缩阈值；bot 想用便宜模型、你想用强的，而且 bot 改它不该改到你 | 若发现两边设置长期完全一致且维护成本成为负担，再评估共享其中的只读字段 |
| 4 | **默认发言策略 `router`**（有总管）；无总管退 `mention_only`；`eager` 默认关（§5） | 一次几百 token 的判定换掉了"每回合都要人做路由"这个最大摩擦，成本几乎不变 | 判定器的误判率实测（§15 第 6 条）高到用户宁可变频点名时 |
| 5 | **`eager` 最多 6 个成员，判官用便宜模型**（§5） | 群聊的价值是几个视角而不是人多；判官只需判断职责相关，不需强推理 | 实测发现便宜判官误判明显高于强模型，且用户频繁开 `eager` 时 |
| 6 | **机器人与频道里 `@` 只列机器人**（§11.3） | 在频道里 `@` 的动作就是叫人；让人先想"这个 `@` 现在指什么"比少一个菜单组贵 | 用户在机器人表面频繁需要引用文件、而拖拽/路径不足以覆盖时，再加回文件组 |
| 7 | **频道里默认不能引用私聊上下文**，频道级开关（§4.4） | 这个泄漏是静默的：群里其他人只看到一句话，不会知道它来自私聊 | 用户抱怨每次都要重新交代背景、且能接受在转录里标注引用来源时 |
| 8 | **总管建 bot：说出职责就直接建，只有它自己推断时先问一句**（§9.4） | "名册只增不减"是这类系统的通病；但确认卡在用户已经想清楚的主路径上，就只剩下摩擦 | 推断类创建频发且用户觉得问得多余时，退成"只对一次建多个确认" |
| 9 | **允许"先建后长"**：无职责的占位 bot 可以存在，第一轮对话结束时按骨架生成 `AGENTS.md`（§9.4） | 这是"setup is a message"的关键一步：用户不必在建之前把职责写完整 | 实测发现占位 bot 大量堆叠且从不补职责时，改为强制在首次使用前补完 |

九条里只有第 2 条带工作量（一次配置覆盖），其余都是已实现机制的组合或参数选择。

## 13. 跨机 bot：不做，以及缺什么

"A 机器上的 bot 进入 B 机器上的频道"。v1 只做单机（宿主所在机器），不是因为复杂，而是下面五件事一件都没有，缺任意一件就只能做成半成品：

1. **可寻址的 bot 句柄**：`future run --session <id>` 只在本机 agent 上解析会话；remote 协议是配对桌面之间的通道，没有"另一台机器上的某个 bot 是可寻址对象"这个概念。
2. **跨机回合协议**：谁持有转录顺序、谁判预算、A 的回合如何写进 B 的转录并去重（需要 `turn_id` + 幂等键 + 时钟裁决）。现有 remote 协议只有 peer presence 与请求/应答。
3. **跨机成员授权**：把 B 的工作区路径交给 A 的 bot，等于让另一台机器写本机文件；沙箱规则与审批都是本机的，没有"本机用户已批准另一台机器的 agent"这一层。
4. **成本归属**：一次回合花两台机器主人的额度，归属未定义。
5. **断线语义**：A 的 host 掉线时那个成员是"暂时不可用"还是"回合失败"，两者的转录与预算后果不同，未定义。

结论：单机版本里"不在本机的 bot"给一条明确的不可用提示，而不是假装它能参与。

---

## 14. 分阶段与验收

| 阶段 | 交付 | 可观测的验收 |
|---|---|---|
| **P1 Bot** | `bots.db` + 库结构、**bot 的独立 agent 实例与配置覆盖**、私聊 thread、导航入口、机器人列表、一键建总管、输入框一句话创建、`future bot` CLI | 建一个 bot、对话三轮、重启后名字与记忆都在；它的私聊不出现在「最近」与「工作区」；**用户侧 `future session list` 里根本没有它**；bot 能用用户已配置的模型与已安装技能；**用一句“新机器人：负责发布稿”建出一个无需任何弹窗、命名与模型选择、且能直接接活的 bot**；`future data` 备份包含 `bots/bots.db` 与 bot 的 `agent-home` |
| **P2 Channel** | `channels`／`members`／`bindings`／`turns`、`threads.scope`、路由内核、三档策略、执行器、频道视图、启动扫描修复器 | 两个 bot + 我：`@` 一个只有一个回；bot `@` 另一个，链深到 2 自动停；一条消息 9 回合被预算截断并说明原因；`router` 档下无人 @ 也能选出该接话的人，且理由可见；§8.5 的九处入口逐个通过；手动删一个 bot 的 thread 后，bots.db 里的 bot 降级为"会话已删除、可重建"而不是报错 |
| **P3 记忆与日常** | 记忆写回可观测、频道摘要、routine 绑 bot、从会话长出来 | 让 bot 记住一条偏好，隔天在新频道里生效；给 bot 建一条每日 routine 并看到运行记录；一段会话固化成 bot 后名册可见 |
| **P4 可选收紧** | 配置提升到 `~/.future/config/`（独立任务）、仅层 0 的 tier（仅对外开放 bot 时）、跨频道 handoff、复制与分享 | 两个实例读同一份配置，覆盖字段删除；bot 读不到 `models.json`；A 把结果交给 B 且 B 的答复回到指定位置；复制出的 bot 不带记忆 |

**不做**（明确写下，避免反复确认）：语音通话、浏览器／桌面操控、从 X 触发、把频道接入 IM 桥、跨机 bot。

---

## 15. 应当由代码验证、而不是由本文承诺的点

每条都是本设计的**假设**，纸上无法定论，需要一次实测或一个测试钉住。

1. **前缀缓存是否稳定**：L2 不变时相邻两回合的 `cache_read` 同量级；改动 `FUTURE.md` 后应看到一次全额重算。判据用本仓已有口径（`cache_read / inputTokens`）。
2. **无策略沙箱下层 0 是否真的不生效**：在 bot 会话里读一次 `models.json`，确认可读——这决定 §10.1 第 2 条是否成立。
3. **九处入口的双向隔离**：逐个入口验，且要验"不出现"而不是"出现了"。缺一个就是半边塌。
4. **独立实例真的跑起来了**：第二个 agent 拿到自己的锁、`agent.db` 与 socket；用户侧实例的会话表里没有它；两个实例可独立重启，一个崩溃不影响另一个。
5. **配置覆盖生效**：bot 实例能读到用户新增的 provider 与刚安装的 app 作用域技能，且不需要第二次配置；用户登出后 bot 也拿不到旧凭据。
6. **判定器的误判率**：在 `router` 与 `eager` 下各跑 20 条真实消息，统计"该叫的没叫"与"不该叫的叫了"，据此决定 `eager` 是否值得默认开放。
7. **频道串行 + 预算截断**：桩 agent 跑 9 回合链，验顺序、终止点与那条 system 消息。
8. **摘要后的可回答性**：把依赖第 3 条消息的提问放到第 40 条之后，验证摘要 + 窗口仍能答对，而不是"看起来答了"。
9. **同一 bot 被两个频道同时 @**：两个 binding 会话互不干扰；排队语义（`append` 默认）是否需要 `--steer`，实测确认不互相打断。
10. **停审批时不拖垮频道**：一个回合停在审批上，同频道其他回合继续，超时后能 abort 并写出 system 消息。
11. **总管建 bot 的零摩擦**：`future bot new` 在 bot 的独立实例里不弹审批，桌面通过 `bots-updated` 在 1 秒内看到新 bot。
12. **手机端能力位**：老 host 没有 `bots_v1` 时，客户端显示"这台电脑不支持"而不是空列表。
13. **跨库修复真的幂等**：在"建 bot 三写"的每一步之间人为杀掉进程，验证启动扫描能把孤儿清理掉、并且重复跑不产生新改动。
14. **生成出来的职责是否过拟合**：连续用一句话建 5 个不同职责的 bot，对比它们的 `AGENTS.md`——若退化成同一段套话，说明骨架的槽位不够硬，需要收紧模板而不是靠用户逐句修。
15. **备份覆盖新库与新 home**：`future data` 备份出来能恢复 `bots/bots.db`、bot 的 `agent-home` 与频道转录；故意漏掉时应当失败而不是静默跳过。

---

## 附录：与既有设计的关系

| 既有能力 | 关系 |
|---|---|
| Scheduled tasks（[tasks.zh-CN.md](tasks.zh-CN.md)） | routine 直接复用；bot 只给 task 一个归属。注意 `future task` 按设计是**全权限**运行，与 bot 的红线一致，不会绕过 |
| Fusion（[fusion.zh-CN.md](fusion.zh-CN.md)） | 同一次 run 内的多模型讨论，不是多 bot 对话；共享"确定性内核 + 宿主执行器"的形状，但 fusion 只读且单回合 |
| Goal control plane（[goal-control-plane.zh-CN.md](goal-control-plane.zh-CN.md)） | 长期目标与人工闸门；频道是多主体在同一会话里协作，不引入目标状态 |
| IM 渠道（`channels/` crate） | 只共享"频道"这个词：那是把外部 IM 接进 agent，这里是 agent 之间与人与 agent 之间的会话 |
| 工作区与会话 | bot 私聊是一条普通 thread，但 `scope='bot'`，与用户会话双向隔离（§8） |
