# 机器人与频道（Bots and Channels）设计

[English](bots.md)

状态：设计稿，尚未实现。Updated: 2026-11-04。

这份文档只描述设计：不含代码改动，不改 `sandbox` / `approval_rule` 的任何实际配置，不创建任何 bot，也不碰 `~/.future/` 下的运行配置。文中的"现状即可支撑"指的是仓库里**已有的**机制，逐条给了可核对的代码位置和命令行示例；"需要改什么"的部分单独成节，不与现状混写。

本文回答三件事：

1. 对标的 xAI 产品 *Grok Bot* 到底怎么组织 bot、记忆、群聊与上下文（§1，全部带原文链接）。
2. 在 FutureOS 里，一个 bot 和一个频道分别落成什么，上下文怎么装配（§3–§6）。
3. 哪一部分用现有能力就能做（§7），哪一部分必须改 agent 或沙箱（§8），哪些地方会以什么方式失败（§9），以及**必须由你拍板的五个选择**（§10）。

---

## 1. Grok Bot 的事实（调研）

这一节只写有原文出处的事实。Grok Bot 的产品文档在 `docs.x.ai`，由 Cursor 的账号与计费体系承载；另有一份对 xAI 团队 72 小时直播的第三方整理（Field Notes），用来补充官方文档没写的失败面——它**不是**官方文档，引用时会标注。

| 事实 | 原文 |
|---|---|
| Bot 是"有名字、有职责、有自己会话、上下文随时间累积"的持久同事，不是一个聊天线程 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots) |
| 所有 bot 共用**同一台云电脑**：文件对每个 bot 可见，浏览器 cookie 与命令行凭据共享；每个 bot 只拿到自己的一块"屏幕"，而那是并行工作面、**不是安全边界** | [Use the computer and apps](https://docs.x.ai/grok-bot/computer-and-apps) |
| 电脑上有一个共享工作区 `/workspace`，长期项目文件放这里 | 同上，"Work with files" |
| 记忆是**每 bot 独立**、长期、自己决定何时保存；少数"关于你"的事实（名字、时区）全账号共享 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots)，"What a Bot remembers" |
| 群聊里，bot **可以使用你在私聊里对它说过的话**；每个 bot 保留自己的记忆，不与他人共享；群里所有人都能看到它说的话 | 同上，"Across chats, Bots, and calls" |
| 群聊 2–6 个 bot；`@Bot` 定向、`@everyone` 全员；bot 可以自己往群里发消息并在彼此之间移交工作 | [Message and collaborate](https://docs.x.ai/grok-bot/chat-and-collaboration)，"Start a group chat" / "Direct a message" |
| bot 之间是**异步 handoff**：接收方被唤醒、处理、稍后回复，移交出现在各自私聊而不是群里 | 同上，"Let Bots hand work off" |
| 审批卡、密钥请求、邮件/Slack 草稿**不进群聊**；bot 对群的消息只能是纯文本 | 同上，"Start a group chat" |
| Skill 是**全账号一个库**，每个 bot 都看得到，没有按 bot 挂载 | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations)，"Save a skill"；另见 [`grok-bot-cli` 的 `gbot skills` 说明](https://github.com/ScriptedAlchemy/grok-bot-cli) |
| Routine 归属于**某一个 bot**，按时间或事件触发，在云端跑，笔记本合上也在跑 | [Skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations)，"Create a routine" |
| 已知失败面一：群里的 bot "eager、爱说话、互相抢话"，因为**每个 bot 都会回每一条消息** | [Field Notes · ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md)（第三方整理） |
| 已知失败面二：routine 频率是最贵的漏点（"每 15 分钟一次就是每天上百条"） | 同上，§34（第三方整理） |
| 复制 bot 会带走档案、技能、routine、头像，**不带**会话历史、已学记忆、附件；模板分享保留核心记忆、剥离与工作区相关的数据 | [Create and manage Bots](https://docs.x.ai/grok-bot/bots)，"Share a Bot"；以及 [Field Notes · PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)（第三方整理） |

从这些事实里抽出三条对设计有决定作用的：

- **上下文是分层的**：bot 私有（记忆 + 自己的会话）× 共享（电脑上的文件与登录） × 场景（群里的公开转录）。
- **唤醒是显式的**：默认情况下人 `@` 才定向，但"每个 bot 都会回每条消息"这种默认会把成本放大，官方把这列为直播期间的典型坑。
- **隔离的边界是记忆与会话，不是文件系统**：所有 bot 共享一个 `/workspace`。

---

## 2. 与既有想法的三处分歧

| # | 想法 | Grok Bot 的做法 | 本设计的选择 | 代价 |
|---|---|---|---|---|
| D1 | 每个机器人有自己的 workspace | 没有。全账号共享一台电脑、一个 `/workspace`，隔离只在记忆与会话层 | **按想法做**：bot 的 workspace 就是它的身份与记忆载体 | 跨 bot 交接必须显式（文件与记忆不自动可见），不能像 Grok Bot 那样"文件就在那儿" |
| D2 | 只有被 `@` 的机器人才参与 | 提示词层面的期望，实测会抢话 | **做成路由内核的确定性规则**，不由模型判断 | 需要一套预算与终止条件，否则会静默不响应（见 §9.1） |
| D3 | bot workspace + 频道 workspace 双根 | 单一根目录，没有这个问题 | 保留双根，但要为"cwd 之外可写"付一次权限设计成本 | 见 §8：现状下单根写会落到审批，需要方案 B 或 A |

D1 带来一个额外好处，也是本设计最省事的一点：**bot 的身份与记忆可以复用现有的 cwd 上下文注入**，不需要新的记忆子系统（§4.1、§7.1）。

---

## 3. 概念模型与数据模型

### 3.1 概念

| 概念 | 定义 |
|---|---|
| **Bot** | 一个 workspace（`AGENTS.md` 写职责、`FUTURE.md` 是记忆索引、`.future/memory/*.md` 是细节）+ 一个模型与思考级别 + 名字/头像 + **一条私聊会话**（普通 thread） |
| **Channel** | 一组成员（≥ 2 个 bot，可加人）+ 一个**频道 workspace**（共享板）+ 一份转录账本 + 一个频道宪章 |
| **Binding** | `(bot × channel)` 的会话：这个 bot 在这个频道里的上下文容器，懒创建 |
| **Turn** | 一次由路由内核派发的 bot 发言：一次真实 agent run，结束时把最终文本写入转录 |

Bot 恰好等于一个 workspace，这不是巧合而是取舍：`cwd` = bot workspace 时，现有提示词组装器会自动注入 `AGENTS.md`（职责）与 `FUTURE.md`（记忆索引），见 §7.1。

### 3.2 表

放 `app.db`（桌面 GUI 的 store），**不**另开一个像 `tasks.db` 那样的独立库。理由：bot 与频道必须和 `workspaces`、`threads` 在同一个库里做过滤与 JOIN——"bot 私聊不出现在最近列表"这类过滤跨库做不到。`app.db` 的库结构以 `host/src/store/schema.rs` 为"新库的唯一真相"，已发布库另需在 `host/src/store/migrations.rs` 加版本化迁移（该文件的文档注释写明了这条规则）。

```sql
bots(
  id TEXT PRIMARY KEY, name TEXT NOT NULL, job TEXT NOT NULL,
  avatar_shape TEXT, avatar_color TEXT,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  model_id TEXT, thinking_level TEXT, permission_level TEXT,
  dm_thread_id TEXT REFERENCES threads(id),
  pinned INTEGER NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);

channels(
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  charter TEXT NOT NULL DEFAULT '',          -- 频道宪章：这个频道为了什么
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  summary TEXT NOT NULL DEFAULT '',          -- 滚动摘要（§4.4）
  allow_private_context INTEGER NOT NULL DEFAULT 0,
  max_turns INTEGER NOT NULL DEFAULT 8,      -- 每条用户消息的 bot 回合上限
  max_depth INTEGER NOT NULL DEFAULT 2,      -- @ 链深上限
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER, deleted_at INTEGER
);

channel_members(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  role_note TEXT NOT NULL DEFAULT '', joined_at INTEGER NOT NULL,
  PRIMARY KEY (channel_id, bot_id));

channel_bindings(channel_id TEXT NOT NULL, bot_id TEXT NOT NULL,
  thread_id TEXT, session_id TEXT, persona_hash TEXT,
  PRIMARY KEY (channel_id, bot_id));

channel_turns(
  id TEXT PRIMARY KEY, channel_id TEXT NOT NULL, seq INTEGER NOT NULL,
  author_kind TEXT NOT NULL,                 -- user | bot | system
  bot_id TEXT, text TEXT NOT NULL,
  run_id TEXT, session_id TEXT,
  status TEXT NOT NULL,                      -- queued | running | done | failed | cancelled
  mentioned_json TEXT NOT NULL DEFAULT '[]',
  error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER);
CREATE UNIQUE INDEX channel_turns_seq ON channel_turns(channel_id, seq);
```

复用而非新建：`workspaces`（bot 与频道各占一行 `kind='user'`，出现在侧栏是可接受的，也让人能直接打开目录）、`threads`（bot 私聊就是一条普通 thread，`mode='workspace'`，`workspace_id` = bot 的 workspace）。

**不变式**：`channel_bindings.session_id` 指向的会话带 `created_by='bot'` 与 `source_meta={botId, channelId}`。**所有列举会话的入口**（最近列表、`#` 会话提及、会话搜索、手机端会话目录）都要默认排除它们，否则频道内部会话会漏进用户视野——这是本仓已经见过的一类缺陷（会话列表出现不该出现的条目），必须逐个入口验证，不能只在列表组件里过滤。

### 3.3 CLI 面

与其他控制面一致：能读同一份库，不新增后端。

```
future bot list|show|new|rename|delete        （v1 只读 + 建/改名）
future channel list|show|new|add|remove|post  （post = 以人的身份发一条频道消息）
```

---

## 4. 上下文装配

### 4.1 三层

| 层 | 内容 | 生命周期 | 注入方式 |
|---|---|---|---|
| **L1 bot 私有** | 职责（`AGENTS.md`）、记忆索引（`FUTURE.md`，按 30 条 / 100 行 / 12KB 上限 lint）、细节记忆（按需 `read`）、私聊会话历史 | 永久 | `cwd` 自动注入前两项；私聊历史在 bot 自己的会话里 |
| **L2 频道共享** | 频道宪章、成员花名册（名字 + 一句话职责）、全体机器人花名册、频道 workspace 路径、频道摘要、说话规则 | 频道存活期，文件 hash 变化才刷新 | 会话级 `append_system_prompt` |
| **L3 本回合信封** | 谁 `@` 了你、那条消息原文、转录窗口、本回合要求 | 单回合 | 用户消息正文前缀 |

### 4.2 装配出来的形状

系统提示词（每个 `(bot × channel)` binding 一份）：

```
<现有身份段 / 工具段 / 审批段 / 通讯段 —— 不变>

# Project Context            ← 来自 cwd 的 AGENTS.md（bot 的职责）
# Workspace Memory           ← 来自 cwd 的 FUTURE.md（bot 的记忆索引）
# Available Skills           ← 全局技能库（与 Grok Bot 一致：全账号共享）

<append_system_prompt —— 本设计新增的频道层>

# 频道：Launch
宪章：<channels.charter>
成员（只有被 @ 才发言）：
- @Writer（写发布稿）
- @Reviewer（只列阻塞问题）
- @你（Research）
本账号的其他机器人（不在本频道，仅供你判断该找谁）：
- @Support（客户问题）
频道工作区：<channel-workspace>（与所有人共享）
频道摘要：<channels.summary>

# 说话规则
- 你的回合以一条频道消息结束；直接给结论，不要复述别人说过的话
- 要别人参与就 @ 他，并说清要他做什么；不 @ 等于没叫
- 只能在频道工作区写文件；你自己的 workspace 是私有的，不要把私密内容写进频道
- 你要别人手里的文件，给出绝对路径并 @ 他，不要假设你能读到他的工作区
- 需要人批准的动作，在频道里只说"等待批准"，细节回到私聊
```

用户消息（每回合）：

```
[频道最近 N 条]
Research · 10:02 · <正文，单条截断 800 字>
Writer   · 10:04 · <正文>
[现在] @Research 说：<原文>
[你被 @ 了] 回答上面这条；不需要别人参与就不要 @ 任何人。
```

### 4.3 放置规则（有代码依据，不是风格偏好）

三个必须守住的约束：

1. **append 段在 project context 与 workspace memory 之后**，因此它可以覆盖 bot 自己的 `AGENTS.md`。所以频道段必须**只做加法**：不复述、不改写机器人职责。已核对的组装顺序见 `agent/src/prompt/mod.rs::build_prompt` 的段序注释与 `agent/src/rpc/session_prompt.rs::build_system_prompt`（后者把 append 段读在 project context 与 memory 之后）。
2. **前缀稳定优先**：L1、L2 放系统提示词，L3 放用户消息。L2 只在 `AGENTS.md`/`FUTURE.md`/宪章/成员变化时刷新（按内容 hash），否则每回合都会打穿前缀缓存。本仓已有实测：替换系统提示词、去掉工具定义或多加一行，在已预热前缀上的缓存命中都是 **0%**，而同形状请求是 93.7%，整段对话重新按全量计费（[压缩设计](../internals/compaction/compaction.zh-CN.md)，"缓存与前缀"一段）。
3. **身份与记忆不重复注入**：L1 由 `cwd` 自动注入，L2 不再抄一遍，否则同一份职责在提示词里出现两次，模型会按后者行事（更靠后的那条）。

### 4.4 预算与截断

| 项 | 默认 | 说明 |
|---|---|---|
| 转录窗口 | 3k token，按模型上下文自适应 | 超窗口从最旧的整条消息开始裁，不裁半条 |
| 单条消息 | 800 字，head…tail | 与 `task_runs.result_text` 的做法一致 |
| 滚动摘要 | `channels.summary`，≤ 1200 字 | 窗口裁掉的消息由摘要代表；摘要由内核发起一次独立的摘要 run 写入 |
| 摘要触发 | 裁掉的消息累计 > 2000 字或 > 8 条 | 防止每回合都摘要 |
| 私聊上下文 | 默认**不**进频道 | 频道开关 `allow_private_context`，默认 0（§10 第 5 点） |

### 4.5 融合规则（什么进、什么不进）

- **记忆只能自己写**：bot 只能用工具写自己 workspace 的 `FUTURE.md` 与 `.future/memory/`。跨 bot 的记忆传递只能通过"说出来的话"或"频道工作区里的文件"，与 Grok Bot 的"除非一个 bot 给另一个发消息，否则什么都不共享"一致（[PRODUCT.md](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)）。
- **摘要不是记忆**：`channels.summary` 是频道的事实，只注入到在**这个频道里**发言的回合，不写进任何 bot 的 `FUTURE.md`。
- **过程不进转录**：转录只收回合最终文本 + 一行活动摘要；完整工具过程留在 binding 会话里，点开看。这与 Grok Bot 把"工具活动、电脑操作、生成文件"和消息并列展示不同，取舍是我们不让频道被过程噪音填满，代价是"群里看不到别人在做什么"，用一行状态补。
- **失败也进转录**：`failed`/`cancelled` 的回合写一条 system 消息，说明谁失败了、为什么、可以怎么重试。Grok Bot 的群聊没有这个位置，实测后果是"看起来没人理你"。

---

## 5. 路由内核

放在 `orchestration/bots`，与 goal / tasks / fusion 同构：确定性内核 + 纯函数 + SQLite 持久化，宿主持有执行器。

```rust
// 纯函数，无 IO，可单测
mentions(text: &str, roster: &[BotRef]) -> Vec<BotId>
enqueue(turn: &Turn, mentioned: Vec<BotId>) -> Vec<QueuedTurn>
should_stop(queue: &[QueuedTurn], budget: &Budget, seen: &[TurnRef]) -> Option<StopReason>
```

`mentions` 的解析规则（必须精确到可测）：

1. 一个提及是 `@` + 与花名册**某个名字完全相等**的最长匹配；名字前后必须是空白、行首/行尾或标点。
2. `@` 前必须是空白或行首（避免 `mail@host`、`src/@scope`）。
3. 代码围栏（``` 块）内与引用行（`>`）内不解析。
4. 大小写不敏感匹配**名字**，但输出用规范名。
5. 排除作者自己；重复提及只算一次，保留第一次的顺序。
6. 不匹配任何花名册名字的 `@xxx` 是普通文本，不报错、不唤醒。

终止与预算：

| 规则 | 默认 | 理由 |
|---|---|---|
| 每条用户消息的 bot 回合上限 | 8 | 直接对应 [ANTIPATTERNS §35](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md) 的"每个 bot 都回每条消息" |
| `@` 链深上限 | 2 | 官方结论：通常"一个 bot 叫两个 bot 一次"就够了（同上） |
| 同一 bot 不连续发言 | 硬规则 | 防自我循环，也防两个 bot 互相 @ 到预算耗尽 |
| 回合内不 `@` 任何人 | 结束链 | 沉默即终止 |
| `@` 非成员 | 不唤醒；转录里记一条 system 提示"X 不是本频道成员" | 不做隐式拉人 |
| `@all` | 允许，但计入上限并标红 | 保留能力，让成本可见 |

为什么做成代码而不是提示词：提示词约束"谁该说话"在 Grok Bot 上实测会失效（§1 的失败面一）；而"谁被 `@` 了"是一个可以精确判定的字符串问题，交给模型判断只会让成本与顺序不可预测。

---

## 6. 执行器

`host/src/bots/executor.rs`，与 `host/src/tasks.rs` 的 tick 循环同构（同一份服务被 GUI 与 headless 共享，Tauri 侧只是薄适配，参见 `desktop/src-tauri/src/commands/tasks.rs`）。

- **同频道串行**：保证转录顺序，避免两个 bot 抢话。**跨频道最多 2 路并行**（对齐 fusion 的并发上限）。
- **一个回合 = 一次真实 run**：`future run --session <binding-session> --model <bot.model> --thinking <bot.thinking> "<信封>"`。
- **回合结束**：把 run 的最终文本写进 `channel_turns`（`status='done'`），解析其 `@` 决定后续队列；没有 `@` 就结束这条链。
- **超时**：单回合同 fusion 的 600 秒量级；超时先 abort，再写一条 system 消息。
- **审批**：bot 的动作可能停在审批上。频道里只显示"等待批准"一行，审批卡回到 bot 私聊与全局待办（与 Grok Bot 的"审批不进群"一致）。这意味着**人不在时频道会停下来**——这是设计意图，不是缺陷，但必须让人看得见（见 §9.2）。
- **取消**：用户删频道 / 关 bot 时先 stop 再 delete，与 goal 的"delete 先 stop"教训一致。

---

## 7. 现状即可支撑的部分（不改 agent）

这一节是"现在就能做"的清单，每条给出机制与证据。注意这里的"能做"指**机制存在**，不是一个可用的产品：UI、路由、账本都还没写，那是应用侧的活。

### 7.1 身份与记忆：`cwd` 自动注入

`agent/src/rpc/session_prompt.rs::build_system_prompt` 每次 run 都从 `cwd` **重新读取** `AGENTS.md`（依次回退 `CLAUDE.md`、`GEMINI.md`，见 `agent/src/prompt/project_context.rs::load_project_context`）与 `FUTURE.md`（lint 后注入，见 `agent/src/prompt/mod.rs` 的 Workspace Memory 段）。因此：

- bot 的职责 = 它的 workspace 根目录下的 `AGENTS.md`；
- bot 的记忆 = 同一个目录下的 `FUTURE.md` + `.future/memory/*.md`；
- 记忆写回 = bot 用现有 `write`/`edit` 工具写文件，遵守文本里已印出的记忆纪律（何时记、上限、过期清理）。

**零改动**，且与用户在其他会话里获得的记忆行为完全一致。

### 7.2 频道层：会话级 append 提示词

`future session new --cwd <bot-workspace> --name "Research" --append-system-prompt "<频道层>" --json`；之后用 `future session set <sid> --append-system-prompt "<新的频道层>"` 刷新。该段持久化在会话里、每次 run 重新读取、位置在 project context 与 memory 之后（§4.3）。

### 7.3 回合执行

`future run --session <sid> --model <id> --thinking <level> "<信封正文>"`；长文本用 `--message-file <path>`（避免 PowerShell 的编码问题）；不等答复用 `--no-wait` 并在之后 `future session wait <sid>`。

### 7.4 私聊与周期任务

- bot 私聊 = 一条普通 thread + 一个 session，现有会话视图直接可用。
- "bot 的例行任务" = 现有 `future task`：`future task add --name "每日风险清单" --prompt "<skill 名 + 边界>" --cwd <bot-workspace> --session existing --daily --time 09:00`。`tasks.cwd` 与 `session_policy=existing` 已经覆盖"用 bot 的身份、在 bot 的 workspace、跑在 bot 的会话里"这三个条件；后续若要按 bot 筛选，再加 `tasks.owner_bot_id`。

### 7.5 现状**不**覆盖的（都是应用侧）

| 缺的东西 | 归属 |
|---|---|
| `@` 路由内核与预算 | 新 crate `orchestration/bots` |
| 转录账本与执行器 | `host/src/bots/*` + `app.db` 迁移 |
| 多会话合并渲染（一个频道 = N 个 binding 会话的输出） | 新组件；现有投影器是**单会话**的（`packages/thread-projection/src/projection.ts`） |
| 导航入口与左侧列表切换 | `desktop/src/components/layout/ActivityRail.tsx`（`ActivitySection` 联合类型在文件第 61 行附近） |
| 手机与另一台电脑的访问 | remote 协议新增能力位（现有能力位清单见 `packages/remote-protocol/src/handshake.rs`） |

---

## 8. P4：双根权限要改什么

需求本身：bot 参与讨论时，既要能读写自己的 workspace（记忆、草稿），又要能读写频道 workspace（共享板、交付物）。现状下单根 cwd 之外的写入会落进审批（`agent/src/sandbox/rules.rs` 的层级说明：读默认放行，写在工作区/临时目录内放行，否则 `ask`）。

两条路：

| | 方案 B：生成 per-bot 规则文件 | 方案 A：`SandboxPolicy.extra_roots` |
|---|---|---|
| 改什么 | 不碰 agent。内核在成员关系变化时，往 `<bot-workspace>/.future/approval_rule.json` 写一条 `{path: <channel-workspace>, access: "both", action: "allow"}`（规则文件格式见 `rules.rs::RawRule`） | `packages/rpc/proto/future.proto` 的 `SandboxPolicy` 增加 `repeated Root extra_roots {path, access}`；`agent/src/sandbox` 解析后并入该会话的运行期规则层；`REGENERATE_PROTO=1` 重生成 |
| 作用域 | 以 bot 为单位：A 的规则只写在 A 的 workspace 里，不会让 B 获得 A 的写权限 | 以 `(bot × channel)` 会话为单位：最精确 |
| 代价 | 规则文件是"生成物"，与用户手改会冲突，需要 `managed` 标记与幂等重写；workspace 一旦被复制/分享会带走路径 | 需要 agent + 协议改动，跨 Windows/macOS/Linux 三平台验证 seatbelt 与 windows 规则层 |
| 风险 | 规则文件本身可被 bot 读取，泄漏"我是哪些频道的成员"；若路径失效会静默退回 `ask`，表现为频道突然要求审批 | 新增字段的向后兼容：老 agent 不认该字段时必须**降级为 ask**而不是忽略（否则等于提权） |

**推荐：先 B，A 为正解。** 理由：B 能把 P1–P3 完全挡在 agent 改动之外（这正是本设计"先做薄"的前提），而它的作用域天然是 per-bot、不会串到别的 bot；A 更干净，但它引入的是一条安全相关字段的跨版本降级语义，值得单独一个 PR 做，并与三平台沙箱回归一起验收。

注意一处**现在就可以核对的假设**：`read` 在 cwd 之外默认放行，意味着"B 有 A 的 workspace 路径就能读 A 的文件"。这与"bot 之间不共享记忆"的意图冲突，需要显式决定：要么频道宪章里禁止跨 bot 读取，要么给 bot workspace 加一条 `deny` 规则。本设计抽出的判断是——**读默认放行是现状，不是承诺**，所以这一条必须由代码验证（§13 第 3 条）。

---

## 9. 失败面

评审会问的三处，逐条给表现、诊断与缓解。

### 9.1 两个 bot 互相看不见对方的 workspace，会不会死锁？

**不会死锁，但会静默卡住，这两件事要分清。**

- 结构上不可能死锁：唤醒只由 `@` 触发，且由内核判定；一个 bot 的回合**不能阻塞等待**另一个 bot 的回合，它只能产出文本。没有互相等待的边，就没有死锁。Grok Bot 是同一个结构（异步 handoff）。
- 会卡住的具体形态有三种，都必须可见：
  1. **A 只把结果写在自己的 workspace，然后 `@B` 说"看结果"。** B 的回合里 `read` 那个路径会失败（或按 §8 的 deny 规则被拒），于是 B 要么回一条"我需要这个文件，请贴出来/放到频道工作区"，要么在没证据的情况下作答。**缓解**：L2 的说话规则里写明"要文件就给绝对路径，并确认对方能读"；转录里保留 B 的失败回合，而不是吞掉。
  2. **两个 bot 互相 `@` 到预算耗尽。** 终止是确定性的：链深到 2 或回合到 8 时内核停止，写一条 system 消息说明原因，并把最后两条发言留在转录里。人看到的是"它们聊了 4 条然后停了"，不是"卡住了"。
  3. **A 结束回合时既没 `@` 也没回答问题。** 这是最难区分的一种：看起来没人理你。**缓解**：把"回合契约"写进 L2（"你的回合以一条频道消息结束"），并在每次 `done` 但没有文本时补一条 system 消息。

### 9.2 频道工作区的写权限被拒时，bot 怎么表现？

- **症状**：该回合停在审批（`ask`），频道里出现"Research 等待批准"，run 不结束。人不在时它会一直停着，直到回合超时被 abort。
- **为什么不是失败而是停住**：`ask` 是审批层，不是拒绝层。这与 Grok Bot "审批不进群"的取舍一致，但我们的表现更重：Grok Bot 把草稿卡片放到私聊，我们只能给一行状态。
- **缓解**：① 说话规则要求"只在频道工作区写"；② 频道视图把等待批准的回合做成可点击，直接跳到 bot 私聊的审批卡；③ 单回合超时（600 秒量级）后 abort 并写 system 消息，频道不因一个 bot 停摆而卡死；④ 频道内**不允许**批量批准（审批必须逐个看，否则这条红线形同虚设）。

### 9.3 `@` 语义改动对既有文件引用的兼容性

现状：编辑器的 `@` 是**工作区文件**提及（`desktop/src/features/agent/MentionEditor.tsx`，文件 pill 序列化为 `[name](./path)`），`#` 是会话提及。

频道里把 `@` 改成"机器人优先"，兼容性按下面这张矩阵处理：

| 既有行为 | 改动后 | 结论 |
|---|---|---|
| 已存的草稿与消息里的文件 pill | 序列化产物是 markdown 链接，不经过提及解析 | 不受影响 |
| `#` 会话提及 | 不动 | 不受影响 |
| 频道编辑器里 `@` 打开菜单 | 三段：本频道成员 → 其他机器人 → 文件 | 文件仍在同一菜单，只是排第三 |
| 手打 `@./src/main.rs` 之类的路径 | 不匹配任何花名册名字，按普通文本处理 | 不报错，不唤醒 |
| bot 自己写的 `@Writer`（纯文本） | 能被内核解析 | 这是必须的：CLI、手机、bot 生成的消息都走纯文本 |
| 机器与文件同名（有 bot 叫 `docs`） | 花名册匹配优先，`@docs` 会指向 bot | 需要一条 UI 提示；命名冲突在建 bot 时就警告 |

**没有数据迁移**：既有数据里不存在"`@名字` 表示机器人"的语义，改动只发生在编辑器菜单与内核解析两处。

---

## 10. 五个待拍板的点

这一节可以原样转给决策人。每点给两条路、各自要改什么与代价、以及推荐与理由。

### 10.1 频道工作区的双根写权限：先 B 还是直接 A？

- **路 1（B）**：内核生成 per-bot `approval_rule.json`。代价：规则文件是生成物、需幂等重写与 `managed` 标记，路径失效会静默退回审批。收益：P1–P3 完全不动 agent。
- **路 2（A）**：`SandboxPolicy.extra_roots`。代价：proto + agent + 三平台沙箱回归；老 agent 必须降级为 `ask`。收益：作用域精确到 `(bot × channel)`，不写用户可见的生成文件。
- **推荐：先 B，A 作为 P4 的正解**。理由：B 让前三个阶段没有 agent 风险；A 的价值（精确作用域 + 不污染 workspace）要等到"bot 数量 > 3、频道 > 5"时才体现，那时再付跨平台验证的成本更划算。

### 10.2 频道里 `@` 的语义：机器人优先，还是给文件换触发符？

- **路 1**：`@` 在频道里优先匹配机器人，文件退到同一菜单的第三组（落地见 §9.3）。
- **路 2**：保留 `@` = 文件，机器人改用别的符（例如 `@@`）。
- **推荐：路 1**。理由：人的直觉里 `@` 就是"叫人"，而"叫文件"已经有拖拽与 `#` 之外的路径输入两种方式；路 2 会让"叫机器人"变成需要教一次的动作，而它是这个功能的主路径。代价是同一编辑器在不同上下文里 `@` 行为不同，需要在菜单里明确分组（不是靠文档解释）。

### 10.3 bot 私聊是否出现在普通会话列表？

- **路 1**：不出现，只能从机器人入口进。需要按 §3.2 的不变式，在**每个**会话列举入口过滤。
- **路 2**：出现，靠工作区分组区分。
- **推荐：路 1**。理由：Grok Bot 把 bot 与会话分为两个侧栏分区，因为二者回答的问题不同（"我和谁在协作" vs "我在做什么"）；混在一个列表里，人的最近列表会被 bot 的日常对话冲掉。代价是过滤必须做全，遗漏一处就会出现重复条目——这是可验证的（§13 第 5 条）。

### 10.4 频道能否 `@` 非成员自动拉人？

- **路 1**：不自动；转录里给一条 system 提示 + 一个"邀请"按钮。
- **路 2**：自动加入。
- **推荐：路 1**。理由：自动拉人会把"成员是谁"这个可见状态变成不可见的副作用，而成员数直接决定每回合成本（§5 的预算按成员算）。让加成员成为一次显式点击，代价是多一步操作。

### 10.5 频道里 bot 能否引用自己的私聊上下文？

- **路 1**：默认关闭，频道级开关；Grok Bot 默认允许（[原文](https://docs.x.ai/grok-bot/bots)："In a group chat, your Bot can use what you told it in your private chat"）。
- **路 2**：默认开启，与 Grok Bot 一致。
- **推荐：路 1（默认关闭）**。理由：私聊里常有不应公开的内容，而这种泄漏是**静默**的——群里的其他人只会看到一句话，不会知道它来自私聊。允许它的价值（少说一遍背景）远小于一次泄漏的代价；需要时打开开关，并在转录里标注"本回合引用了私聊"。

---

## 11. 跨机 bot：先不做，缺什么

设想是"A 机器上的 bot 进入 B 机器上的频道"。本设计建议**先只做单机**（host 所在机器），理由不是"复杂"，而是下面五件事现在**一件都没有**，缺任意一件这个功能就只能做成半成品：

1. **可寻址的 bot 句柄**。`future run --session <id>` 只在本机 agent 上解析会话；remote 协议是"配对的两台桌面"之间的通道，没有"另一台机器上的某个 bot 是一个可寻址对象"这一概念。
2. **跨机回合协议**。谁持有转录顺序、谁判定预算、A 机器的回合如何写进 B 的转录并去重（需要 `turn_id` + 幂等键 + 时钟裁决）。现有 remote 协议只有 peer presence 与请求/应答，没有多参与方定序。
3. **跨机成员授权**。把 B 机器的工作区路径交给 A 机器的 bot，等于让另一台机器写本机文件；沙箱规则与审批都是**本机**的，没有"这台机器的用户已批准另一台机器上的 agent"这一层。
4. **成本归属**。一次频道回合会花两台机器主人的额度，账单归属未定义；现在连"这次 run 属于哪个频道"都只在本机的账本里。
5. **断线语义**。A 的 host 掉线时，B 的频道里那个成员是"暂时不可用"还是"回合失败"？两者对转录与预算的影响完全不同，未定义。

结论：单机版本里，"不在本机的 bot"给一条明确的不可用提示（"该机器人的执行者在另一台电脑上，先在那边打开"），而不是假装它能参与。

---

## 12. 分阶段与验收

| 阶段 | 交付 | 可观测的验收 |
|---|---|---|
| **P1 Bot** | `bots` 表 + 私聊 thread + 导航入口 + 机器人列表 + 新建（选现有 workspace 或新建 `<home>/bots/<id>/`） | 建一个 bot、对话三轮、重启后名字与记忆都在；它的私聊不出现在"最近"；`future bot list` 能看到它 |
| **P2 Channel** | `channels`/`members`/`bindings`/`turns` + 路由内核 + 执行器 + 频道视图 | 两个 bot + 我：`@` 一个只有一个回；bot `@` 另一个，链深到 2 自动停；一条消息 9 回合被预算截断并说明原因；关掉频道视图后转录仍在 |
| **P3 记忆与日常** | 记忆写回的可观测性（谁在什么时候写了什么）+ 频道摘要 + task 绑 bot | 让 bot 记住一条偏好，隔天在新频道里仍生效；给 bot 建一条每日 routine 并看到它的运行记录 |
| **P4 隔离与手交** | `extra_roots`（方案 A）、跨频道 handoff、bot↔bot 私频道、复制/分享 | 两个 bot 不能读对方的 manifest；A 把结果交给 B，B 的答复回到指定位置；复制的 bot 不带记忆 |

**不做**（明确写下来，避免评审反复问）：语音通话、浏览器/桌面操控、`@bot` 从 X 触发、把频道接到 IM（Feishu/DingTalk）桥、跨机 bot。

---

## 13. 应当由代码验证、而不是由这份文档承诺的点

以下每一条都是本设计的**假设**，写在这里是因为它们在纸上无法定论，需要一次实测或一个测试来钉住。建议后续以验证会话逐条执行，结论回填本文件。

1. **前缀缓存是否真的稳定**：两次相邻回合（L2 不变）的 `cache_read` 应达到同一量级；改动 `FUTURE.md` 后应看到一次全额重算。判据取自本仓已有的口径（`cache_read / inputTokens`）。
2. **方案 B 的规则文件是否真的生效**：`{access: "both", action: "allow"}` 指向频道工作区后，写入不再弹审批；同时验证它**不会**让同机上另一个 bot 获得同样的写权限。
3. **`read` 跨 cwd 的默认放行在三种 tier 下是否一致**：尤其 macOS seatbelt `tier=sandbox` 时，读另一个 workspace 是否仍放行。这决定 §8 末尾那条"读默认放行是现状不是承诺"。
4. **频道串行 + 预算截断**：用桩 agent 跑一条 9 回合的链，验证顺序、终止点与那条 system 消息。
5. **bot × 频道会话不会泄漏**：逐个入口（最近列表、`#` 提及、会话搜索、手机端会话目录、`future session list`）验证，不是只看列表组件。
6. **摘要后的可回答性**：把依赖第 3 条消息的问题提到第 40 条之后再问，验证摘要 + 窗口仍能答对，而不是"看起来答了"。
7. **同一 bot 被两个频道同时 `@`**：两个 binding 会话互不干扰；run 排队语义是 `append`（默认）还是需要 `--steer`，必须实测确认不会互相打断。
8. **审批停住时不拖垮频道**：一个回合停在审批上，验证同频道其他回合还能继续，超时后能 abort 并写出 system 消息。
9. **手机端能力位**：老 host 没有 `bots_v1` 时，客户端必须显示"这台电脑不支持"而不是空列表。

---

## 附：与既有设计的关系

| 既有能力 | 与本设计的关系 |
|---|---|
| Scheduled tasks（[tasks.zh-CN.md](tasks.zh-CN.md)） | routine 直接复用；bot 只是给 task 一个"归属" |
| Fusion（[fusion.zh-CN.md](fusion.zh-CN.md)） | 同一次 run 内的多模型讨论，**不是**多 bot 对话；两者共享"确定性内核 + 宿主执行器"的形状，但 fusion 是只读的、单回合的 |
| Goal control plane（[goal-control-plane.zh-CN.md](goal-control-plane.zh-CN.md)） | 长期目标与多人闸门；频道是"多主体在同一会话里协作"，不引入目标状态 |
| IM 渠道（`channels/` crate） | 与本文件的"频道"只有名字相同：那是把外部 IM 接进 agent，这里是 agent 之间的会话 |
