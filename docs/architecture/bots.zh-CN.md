# 机器人与机器人组：v1 最小可行切片

[English](bots.md)

状态：设计稿。**实现尚未开始**：本文只用于评审与排期，落地由单独任务启动。Updated: 2026-11-04。

本文只描述设计，不含代码改动，不改任何实际配置，不创建任何 bot，也不触碰 `~/.future/` 下的运行数据。

对象叫**机器人组**（下文简称"组"，代码与 CLI 用 `group`），以区别仓库里已有的 `channels/`（飞书、钉钉 IM 桥）。

---

## 1. 为什么先做最小切片

要先验证的不是架构，而是三件产品事实：

1. 人会不会真的按**角色**（"发布稿""审稿"）组织工作，而不是继续按任务开会话；
2. 一个 bot `@` 另一个 bot 的交接有没有实际价值，还是只是看起来热闹；
3. 每次交接带来的额外 run 与 token，用户是否愿意付。

所以 v1 只做：**一个 bot + 一个组（两个 bot 与人）+ 确定性路由 + 组内串行执行 + 可见转录**。独立 agent 实例、独立数据库、配置提库、判定器、金额计量、模型生成职责骨架、三条创建路径，都不进入 v1（§14）。

验收不是"这些模块都建起来了"，而是 §13 的一周真实使用记录。若最小组合没有人用，后面的候选机制都没有存在理由。

---

## 2. v1 的产品形态

### 2.1 用户看到什么

- **机器人页**替代普通会话列表，只显示 bot 与机器人组；普通「最近」「工作区」列表不显示它们。
- **一个 bot 有名字、一句话职责、自己的私聊与记忆**。私聊用于长期相处、明确告诉它"记住这个"。
- **一个组有至少两个 bot、一个共享工作区和一条转录**。转录记录用户消息、bot 回复、失败与系统提示。
- **组内 `@` 只叫机器人**，不叫文件；文件通过拖入或写路径提供。
- **一句话创建**：在机器人页输入 `新机器人：负责发布稿`，即可创建，没有目录、模型或职责表单。

### 2.2 发言体验

| 场景 | v1 行为 |
|---|---|
| 用户 `@Writer 写一版` | 只唤醒 Writer |
| 用户没 `@`，直接说"帮我看看这版" | 由**组负责人**（默认总管）接住；它自己回答，或 `@` 更合适的 bot |
| bot 回复里 `@Reviewer` | 唤醒 Reviewer，接力深度加一 |
| bot 回复没有 `@` | 这条接力结束 |
| 消息 `@` 了非成员 | 不唤醒；转录写一条系统提示 |
| 组切换为"仅点名" | 没 `@` 的用户消息不触发 bot，只留一条系统提示 |

这样，用户不必每句都先想"该叫谁"，同时**bot 之间仍然只有 `@` 才接力**，不会全员抢话。v1 不做"每个成员都判断一次要不要说"，也不做独立的路由判定器。

### 2.3 组会话不复制

组转录是唯一事实来源。每个回合都从账本现取所需消息，**不**为每个 `bot × 组` 保存一份重复会话。bot 的长期上下文只保留在它自己的私聊里。

---

## 3. 概念与数据模型

### 3.1 概念

| 概念 | 定义 |
|---|---|
| **Bot** | 一份角色档案：名字、一句话职责、模型、默认工作区、权限、私聊 thread。默认工作区放 `AGENTS.md`、`FUTURE.md` 与 `.future/memory/` |
| **组** | 至少两个 bot、一名负责人、一个共享工作区、一组发言规则与一条转录 |
| **负责人** | `owner_bot_id` 指定的 bot；未点名的用户消息由它处理。新建组默认用总管 |
| **回合** | 一次真实 run。组回合使用**不落盘的一次性会话**，结束后把最终文本、唤醒原因、run id 与 token 用量写入账本 |

`AGENTS.md` 由用户的原话形成，不由模型扩写。`FUTURE.md` 只在用户或 bot 明确执行"记住这个"时更新。

### 3.2 表都放在现有 `app.db`

v1 不新增数据库。定义、成员、账本与 `workspaces`、`threads` 同库，因此隔离过滤和外键都在一处完成；已发布库需要版本化迁移（`host/src/store/migrations.rs`），新库结构同步到 `host/src/store/schema.rs`。

```sql
bots(
  id TEXT PRIMARY KEY, name TEXT NOT NULL, job TEXT NOT NULL,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  model_id TEXT, thinking_level TEXT,
  permission_level TEXT NOT NULL DEFAULT 'all',  -- 私聊默认完全访问
  dm_thread_id TEXT REFERENCES threads(id),
  is_primary INTEGER NOT NULL DEFAULT 0,         -- 每个 home 至多一个总管
  hidden INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);
CREATE UNIQUE INDEX bots_one_primary ON bots(is_primary) WHERE is_primary = 1;

bot_groups(
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id),
  owner_bot_id TEXT NOT NULL REFERENCES bots(id),
  speak_policy TEXT NOT NULL DEFAULT 'owner',     -- owner | mention_only
  permission_level TEXT NOT NULL DEFAULT 'workspace',
  max_turns INTEGER NOT NULL DEFAULT 8,
  max_depth INTEGER NOT NULL DEFAULT 2,
  transcript_window_tokens INTEGER NOT NULL DEFAULT 3000,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  archived_at INTEGER, deleted_at INTEGER
);

group_members(
  group_id TEXT NOT NULL REFERENCES bot_groups(id),
  bot_id TEXT NOT NULL REFERENCES bots(id),
  role_note TEXT NOT NULL DEFAULT '', joined_at INTEGER NOT NULL,
  PRIMARY KEY (group_id, bot_id)
);

group_turns(
  id TEXT PRIMARY KEY, group_id TEXT NOT NULL REFERENCES bot_groups(id),
  seq INTEGER NOT NULL,
  author_kind TEXT NOT NULL,                 -- user | bot | system
  bot_id TEXT, text TEXT NOT NULL,
  run_id TEXT,                                -- 一次性 run；没有长期 binding session
  status TEXT NOT NULL,                      -- queued | running | done | failed | cancelled | waiting_approval
  mentioned_json TEXT NOT NULL DEFAULT '[]',
  wake_reason TEXT,                          -- user_mention | owner | bot_mention | manual
  input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER,
  error_message TEXT, created_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE UNIQUE INDEX group_turns_seq ON group_turns(group_id, seq);
```

对既有表只加一列：

```sql
ALTER TABLE threads ADD COLUMN scope TEXT NOT NULL DEFAULT 'user';  -- user | bot | group
```

bot 私聊是普通 thread，`scope='bot'`；组的运行不产生 thread。

### 3.3 两个存储系统之间的一致性

v1 只有 `app.db` 与共享的 `agent.db`，没有第三份库。写顺序按"可恢复"设计：

| 操作 | 顺序 | 中途失败后的修复 |
|---|---|---|
| 建 bot | ① 在共享 agent 建私聊会话（`created_by='bot'`）→ ② 在**一个 `app.db` 事务**里写 workspace、thread（`scope='bot'`）、bots 行 | 只有①完成：启动扫描发现无 bots 行对应的 `created_by='bot'` 会话，删除它 |
| 建组 | 一个 `app.db` 事务写 workspace、`bot_groups`、`group_members` | 事务整体回滚，无半成品 |
| 删 bot／组 | ① 停止相关回合并清队列 → ② 软删 `app.db` 行 → ③ 删 agent 私聊会话 | ②后失败：启动扫描补删无主的 bot 会话与 thread；③幂等重试 |
| 组回合 | 一次性 run → 写 `group_turns` | run 没落盘，不存在会话孤儿；只有账本行需要状态收尾 |

规则：**悬空引用只降级、不报错**；启动扫描幂等，只做回收与补删，不推断用户意图、不自动重建会话。

默认工作区位于 `~/.future/bots/workspaces/<slug>/`，不落在会被回收的 `~/.future/workspaces/chat/`。用户也可以给 bot 指定已有 workspace。

---

## 4. 上下文装配

### 4.1 bot 私聊

私聊是共享 agent 里的普通持久会话，`cwd` 指向 bot 的默认工作区。现有组装器每次 run 自动读取：

- `AGENTS.md`：角色职责；
- `FUTURE.md`：记忆索引（仍受 30 条 / 100 行 / 12 KB 上限约束）；
- `.future/memory/*.md`：按需读取的细节。

代码依据：`agent/src/rpc/session_prompt.rs::build_system_prompt` 与 `agent/src/prompt/project_context.rs::load_project_context`。模型、凭据、技能都在同一个 agent 实例里，天然共享，v1 不需要任何配置覆盖。

### 4.2 组回合：一次性装配，不依赖会话历史

每回合新建一次性会话，`cwd` 仍是 bot 的工作区，系统提示词由三层组成：

```
<现有身份、工具与记忆层 —— cwd 自动注入>

<append_system_prompt：组层>
# 组：Launch
负责人：@Chief
成员：@Writer（写发布稿）、@Reviewer（只列阻塞问题）、@你（Research）
组工作区：<绝对路径>（共享；交付物放这里）
# 规则
- 你的回合以一条组消息结束；不复述别人说过的话
- 要别人参与就 @ 他并说清需要什么；不 @ 等于没叫
- 不要列举、打开或引用用户的会话
- 私密内容不写进组工作区
```

用户消息只放本回合需要的内容：

```
[最近 N 条组消息，合计不超过 3000 token，整条裁剪]
[现在] @Research 说：<原文>
[你被 @ 了] 回答这条；不需要别人参与就不要 @ 任何人。
```

- 组层只在职责、成员、组工作区或规则变化时刷新，转录窗口始终在用户消息里，避免每回合改写系统提示词。
- 每回合可带最近约 20 条、最多 3000 token 的账本内容；超出从最旧的整条开始裁。v1 不做滚动摘要。
- 私聊上下文默认不进组；组工作区文件与转录是 bot 之间共享上下文的两条通道。
- 记忆只在明确执行"记住这个"时写回，组回合不自动沉淀记忆。

### 4.3 一次性运行的现有入口

手测或原型可直接用现有 CLI：

```bash
future run --no-session \
  --cwd <bot-workspace> \
  --model <bot-model> --thinking <level> \
  --append-system-prompt "<组层>" \
  --message-file <turn-envelope.md>
```

`--no-session` 表示不持久化会话；`--cwd` 与 `--append-system-prompt` 已存在（见 `future run --help`）。生产路径由 host 执行器发起同样的 run，并附带 §8 的权限参数。

---

## 5. 发言与路由

路由放在确定性内核里，不靠模型猜谁被点名。

### 5.1 唤醒规则

按优先级执行：

1. 用户消息里有成员 `@`：按提及顺序派发，只唤醒被点名的 bot。
2. 用户消息没有 `@` 且策略为 `owner`：只派发给 `owner_bot_id`。负责人可以自己回答，也可以在回复里 `@` 专家。
3. 用户消息没有 `@` 且策略为 `mention_only`：不启动 run，写一条系统提示说明当前是仅点名模式。
4. bot 回复里有成员 `@`：唤醒对应 bot，链深加一；没有 `@` 则结束。
5. `@` 非成员：不唤醒，写系统提示。
6. 负责人被删除或不在组里：组进入"需要指定负责人"状态，不擅自换一个人接话。

### 5.2 `@` 的解析

1. `@` + 花名册完整名字的最长匹配；名字后必须是空白、行尾或标点。
2. `@` 前必须是空白或行首，`mail@host`、路径不算。
3. 代码围栏与 `>` 引用行不解析。
4. 大小写不敏感匹配，输出规范名；排除作者自己；重复只算一次。
5. 不匹配任何名字的 `@xxx` 是普通文本，不报错、不唤醒。

### 5.3 停止条件

| 规则 | 默认 |
|---|---|
| 每条用户消息的 bot 回合上限 | 8 |
| bot `@` 接力深度 | 2 |
| 同一 bot 不连续发言 | 硬规则 |
| 回合没有 `@` 任何人 | 结束接力 |
| 超时、失败、被取消 | 写 system 消息并按规则结束或继续 |

`wake_reason` 必须记录是用户点名、负责人接话还是 bot 接力，让"为什么它接了"可解释。

---

## 6. 执行与转录

执行器在 `host/src/bots/executor.rs`，与 tasks 的 tick 循环同构。

- **同组串行**，保证转录顺序；**跨组最多 2 路并行**。
- 一个回合就是一次 run；最终文本写入 `group_turns`，再解析其中的 `@` 决定下一棒。
- 单回合超时为 600 秒量级；先 abort，再写 system 消息，然后按队列继续或结束。
- **失败必须可见**：失败、取消、空回复都写 system 消息，不能出现"没人理你"的静默状态。
- **审批**：回合停在审批时，组里显示"等待批准"，卡片在 bot 私聊与全局待办里；同组暂停该回合，超时后 abort 并继续，其他组不受影响。不允许在组里批量批准。
- 删除 bot 或组时先停止执行器并清队列，再删数据。

转录只保存最终文本与少量运行元数据；一次性会话的过程历史不作为第二份转录保存。

---

## 7. 会话隔离

三条要求都是硬约束：

1. 「最近」「工作区」不得出现 bot 或组创建的会话；
2. 机器人页不得出现用户的「最近」「工作区」会话；
3. 用户会话里的 agent（会话总管）不得通过列表、搜索或提及进入 bot 的会话。

### 7.1 机制

- `threads.scope` 是线程层唯一真相列：普通列表查询 `scope='user'`，机器人页查询 `scope<>'user'`。`host/src/store/threads.rs::list_threads` 目前只过滤删除状态，所以过滤必须落到查询层，不能只改一个组件。
- bot 私聊会话带 `created_by='bot'` 与 `source_meta.scope='bot'`；agent 侧会话列表默认排除，且不提供"列出 bot 会话"的开关。
- 组回合不落盘，天然不会出现在任何会话列表里。
- `#` 会话提及在机器人与组里整段禁用；`@` 只列 bot。

### 7.2 必须逐条验收的入口

1. 工作区视图与归档区；
2. 最近视图（分页与排序）；
3. `#` 会话提及；
4. 会话搜索；
5. 未读点与 run 状态轮询；
6. 审批与待办计数；
7. 远程合并行；
8. 手机端会话目录与最近；
9. 用户侧 `future session list`。

### 7.3 边界

这是**组织与枚举隔离，不是权限隔离**。共享 agent 下，知道私聊 session id 的人仍可显式读取它；bot 私聊默认完全访问，也意味着 bot 能读机器上的文件。v1 不把"隔离"表述成安全边界。若以后要把 bot 提供给他人使用，再考虑独立实例或认证式拒绝，不在本切片承诺。

---

## 8. 组回合的写权限：现有机制即可

bot 私聊默认完全访问；**组回合默认 `permission_level='workspace'`**。目标不是"只读"，而是：读按现有默认规则处理，写只放行 **bot 工作区与组工作区**，其他写入仍进入审批。

实现只需要客户端已有的两个 run 参数和一份规则文件：

1. host 在 run 请求里传 `permission_level='workspace'`；
2. 同时传桌面已有的 `sandbox_tier`（按平台现有取值），**否则规则层不会被启用**；
3. host 在 `<bot-workspace>/.future/approval_rule.json` 幂等维护一条：

```json
{
  "rules": [
    { "path": "<group-workspace-absolute-path>", "access": "both", "action": "allow" }
  ]
}
```

依据：

- `agent/src/rpc/session_prompt.rs` 的 `ScheduledSettingsSnapshot` 同时含 `permission_level` 与 `sandbox_tier`，沙箱按 `session_cwd` 解析；
- `agent/src/sandbox/rules.rs` 的回退规则是"写在工作区或临时目录内放行，否则 `ask`"，工作区规则文件是独立一层；
- `agent/src/sandbox/seatbelt.rs::build_profile` 除基础可写根外，还会把规则里的 allow 译成 `(allow file-write* …)`，并有测试 `session_allow_rules_emit_allow_verbs` 锁定这一行为。

所以**不需要新增 tier，也不需要在 `SandboxPolicy` 加 typed roots**。CLI 的 `--permission workspace` 只覆盖第 1 项；它没有暴露 `sandbox_tier`，因此不能单凭一条 CLI 命令宣称权限已生效。组回合由 host 直接走 RPC。

这与 bot 私聊的完全访问是两回事：私聊仍可用整台机器；组回合只把交付物写入共享板，其他写入会停下等批准。

---

## 9. 机器人面：创建、记忆、通知与计量

### 9.1 一条创建路径

在机器人页输入：

```text
新机器人：负责发布稿，风格简洁，不自行对外发送
```

系统做四件事：从职责生成可修改的名字与 slug；建默认工作区；把**用户原话**写入 `AGENTS.md`；创建私聊并打开。没有模型生成骨架，没有目录或模型表单，也没有第二、第三条创建路径。

首次使用时可以给一个默认总管；之后新建组默认把它设为负责人。总管只是职责为协调与接话的 bot，没有额外权限。

### 9.2 记忆必须有显式入口

在 bot 私聊或 bot 页提供"**记住这个**"：把当前事实或偏好写入它的 `FUTURE.md` / `.future/memory/`，并立即显示刚写入的条目。

这是设计结论：**不做依赖自动沉淀的"最近学到"列表**。现有记忆纪律刻意少记，没有显式入口时，那种列表长期会是空的。写入可见、可改、可删除，比自动复述更可信。

### 9.3 通知默认安静

只有三种情况提醒：被 `@`、回合失败、等待批准。提供"只看需要我决策的"过滤器；其他活动只进转录，不亮红点。

### 9.4 只显示 token，不显示金额

每回合写一行：`N 次运行 · 输入 X / 输出 Y token · 缓存读取 Z`。没有稳定价格表时不折算金额，避免给出看似精确、实际不可靠的数字。拿不到 usage 就写"未知"，不写成 0。

---

## 10. 代码落点

v1 不新增独立 store，也不新增 agent 实例。

| 位置 | 内容 |
|---|---|
| `orchestration/bots/` | crate `future-bots`：只有纯路由内核与类型；不拥有数据库 |
| `host/src/bots/model.rs` | app.db 模型与视图 |
| `host/src/bots/store.rs` | app.db 读写与迁移配合 |
| `host/src/bots/service.rs` | 创建、编辑、删除、启动扫描；供 Tauri、CLI、远程共用 |
| `host/src/bots/executor.rs` | 组内串行、回合、超时、权限参数、账本 |
| `host/src/bots/notify.rs` | `bots-updated` 与通知 |
| `cli/src/commands/bot.rs`、`group.rs` | `future bot …`、`future group …`，只调 service |
| `desktop/src-tauri/src/commands/bots.rs` | Tauri 薄适配 |
| `desktop/src/features/bots/` | 机器人页、组视图、创建与"记住这个" |
| `desktop/src/i18n/locales/{en,zh}/bots.json` | 文案 |
| `desktop/src/components/layout/ActivityRail.tsx` | 导航与列表切换 |
| `packages/remote-protocol/src/handshake.rs` | `bots_v1` 能力位（移动端接入时） |

依赖方向：`future-bots` 不依赖 host/agent；host 是 `app.db` 既有写路径的拥有者；CLI 不自己拼 SQL。

v1 不改 `packages/rpc/proto/future.proto`：会话、run、`permission_level`、`sandbox_tier` 都已有。只有以后新增权限 tier 或独立实例协议时才需要改。

---

## 11. 边界与不做

### 11.1 跨机 bot 不做

"A 机器的 bot 进 B 机器的组"缺的不是一个开关，而是五项基础能力：

1. 可寻址的跨机 bot 句柄；
2. 跨机回合定序、去重与预算协议（turn id、幂等键、时钟裁决）；
3. "本机用户授权另一台机器的 agent 写本机文件"的成员授权层；
4. 两台机器之间的成本归属；
5. 对端掉线时，成员是"暂不可用"还是"回合失败"的明确语义。

这些都没有现成实现，所以 v1 只做本机。非本机 bot 显示明确的不可用提示。

### 11.2 其他明确不做

语音、浏览器与桌面操控、示范录屏、从 X 触发、接入 IM 桥、共享 bot 给团队、云端长跑。宿主不在跑时，本机 bot 也不会继续工作。

---

## 12. 实施顺序

| 阶段 | 内容 | 可观测验收 |
|---|---|---|
| **M1 Bot** | app.db 迁移、bot 私聊、导航与列表隔离、一句话创建、显式记忆 | 建一个 bot、对话三轮、重启后职责与记忆都在；它的私聊不出现在「最近」「工作区」；`future session list` 不含它 |
| **M2 组** | 组与成员、路由内核、一次性回合、串行执行、组转录、`@` 菜单 | 两个 bot + 人：`@` 一个只回一个；没 `@` 时负责人接住；bot `@` 另一个到深度 2 停止；9 回合被上限截断并说明；失败与超时都可见 |
| **M3 权限与计量** | 组回合 workspace 权限、组工作区 allow、token 行、启动扫描 | 组内可写共享板；写其他位置进入审批；用量与 provider 一致；中途杀进程后扫描幂等修复 |

实现按阶段提交，不一次性落地全部候选机制。

---

## 13. 最小切片的验收与去留判据

由主要设计/使用者在**连续 7 天真实工作**里使用，不用演示任务凑数。记录四项：

1. **组的使用**：至少 5 次真实组会话、覆盖至少 3 个不同任务；记录是否比单会话少搬了上下文。
2. **bot 间接力**：至少 3 次自然发生的 bot `@` 交接，而不是为了测试硬造的 `@`；记录其中几次结果被人直接采用。
3. **角色与路由**：至少 10 个真实任务中，用户认为"知道该找谁"的比例；未点名时负责人接错、需要人改派的比例。若超过 30% 要改派，`owner` 规则不成立，退回仅点名。
4. **成本**：每回合输入、输出、缓存读取 token；与同一任务在单个会话里完成的成本对比。中位数若超过单会话 2 倍，先优化上下文与回合数，不继续加成员。

同时满足以下底线才算"可继续"：九处隔离入口无泄漏；失败与审批都可见；一次真实"记住这个"在隔天的新组里被正确使用。

去留规则：

- 若一周内组使用少于 3 次、且没有自然的 bot 间接力，**停止组功能**，只保留 bot 私聊；
- 若组被使用，但负责人经常接错，**保留 `@` 路由、取消未点名接话**；
- 若成本超过阈值，先砍转录窗口与回合数，不用加内存、加实例来掩盖；
- 只有"角色确实比任务好用、交接确实产生价值、成本可接受"三者同时成立，才进入 §14 的候选机制。

---

## 14. 验证之后再说（不属于 v1）

后续顺序只记顺序，不展开设计；每阶段开工前单独立项，未通过前一阶段的判据不进入下一阶段。

| 阶段 | 一句话 |
|---|---|
| **P1** | Bot App 化：`bot.toml`、接口契约、本地调用与审计 |
| **P2** | 记忆分类与进化提案：事实/偏好/技能、diff、试跑与回滚 |
| **P3** | 模板与 fork 分享：装出来的 bot 不带原实例的记忆与凭据 |
| **P4** | 同账号/团队内的授权调用：经 relay 的鉴权、配额、撤销与审计 |
| **P5** | 跨用户组：relay 上的签名事件日志，成员变更分 epoch |

| 候选 | 什么时候再做 |
|---|---|
| 独立 agent 实例、独立 agent.db、第二套进程监督 | 要把 bot 提供给他人，或共享实例的隔离被真实事故证明不够时 |
| 独立 `bots.db` | app.db 的写入热点或迁移成本被实测证明是问题时 |
| 配置提库到 `~/.future/config/` | 确有第二个实例需要共享配置时；届时连同布局迁移、备份与硬拒路径一起做 |
| `router` 判定器、`eager` 全员轻判定 | §13 证明用户频繁不想点名、且 `owner` 规则不够时；先测误判率与成本 |
| 金额计量与预算闸 | token 用量已证明需要预算控制、且价格来源可靠时 |
| 模型生成职责骨架、先建后长、三条创建路径 | 一句话创建已被真实使用，且手工职责成为明显瓶颈时 |
| 自动"最近学到" | 有稳定的显式记忆产物流之后，再决定要不要自动复盘 |
| 从会话提炼技能、到点自动跑 | 先验证 bot 与组的长期价值；不纳入本切片 |
| 独立持久 binding 会话 | 一次性装配被证明丢失关键上下文时；默认不回到 N×M 复制 |

---

## 15. 已核对与未核对

这一节把"代码事实"与"行为承诺"分开。

### 已核对（读代码）

- `agent/src/rpc/session_prompt.rs`：run 的接受设置含 `permission_level` 与 `sandbox_tier`；沙箱按 `session_cwd` 解析；append 提示词每次 run 重读。
- `agent/src/prompt/mod.rs`、`agent/src/prompt/project_context.rs`：`AGENTS.md` 与 `FUTURE.md` 从 cwd 注入；append 段位于 project context 与 memory 之后。
- `agent/src/sandbox/rules.rs`：规则层顺序、工作区规则文件、写在 cwd 外默认 `ask`。
- `agent/src/sandbox/seatbelt.rs::build_profile`：规则中的 allow 会进入 seatbelt profile；相关测试 `session_allow_rules_emit_allow_verbs` 存在。
- `host/src/store/threads.rs::list_threads`：当前只按删除状态过滤，因此 `scope` 过滤必须进入查询。

### 已核对（运行命令）

- `future run --help`：确认 `--no-session`、`--cwd`、`--append-system-prompt`、`--permission`、`--message-file` 已存在。
- `future session new --help`、`future session set --help`：确认 `--append-system-prompt` 的持久化语义。
- `future skills --help`：确认技能目录与安装机制。
- `make check-docs`：本次文档提交前运行通过。

### 未核对（不能当作已验证事实）

- 尚未用真实 run 验证"规则文件放行组工作区"在 macOS seatbelt 下的端到端行为；
- 尚未验证 Linux、Windows 的规则与 OS 沙箱行为；
- 尚未验证多个组并行、同 bot 被两组同时 `@`、审批挂起与超时竞争；
- 尚未验证跨机行为；
- 尚未验证 token 成本、路由体验、记忆写入频率与任何一周使用指标。

这些项目进入实施后的测试清单，不由本文承诺。

---

## 附录：外部材料

- Grok Bot 文档：[Bots](https://docs.x.ai/grok-bot/bots)、[群聊与协作](https://docs.x.ai/grok-bot/chat-and-collaboration)、[技能与例行任务](https://docs.x.ai/grok-bot/skills-routines-and-automations)、[电脑与文件](https://docs.x.ai/grok-bot/computer-and-apps)。
- 发布讨论（含一个月使用者）：<https://news.ycombinator.com/item?id=49261514>
- 两周实测：<https://www.youtube.com/watch?v=d9c5gloGPMU>
- 额度变化记录与报道：<https://www.youtube.com/watch?v=IULY-h-KwrI>、<https://techcrunch.com/2026/11/02/elon-musk-throttles-grok-bot-limits-due-to-unsustainably-high-costs/>
- bot 舰队的运行方式：<https://granda.org/en/2026/09/06/how-i-run-grok-bot/>
- Field Notes（第三方整理）：[ORCHESTRATION](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/agents/ORCHESTRATION.md)、[ANTIPATTERNS](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/ANTIPATTERNS.md)、[PRODUCT](https://github.com/unicodef1wn/grokbot-field-notes/blob/main/reference/PRODUCT.md)
