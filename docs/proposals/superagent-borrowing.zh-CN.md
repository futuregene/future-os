**提案，未立项**

# 借鉴 `deepseek-ai/dsh-superagent-prototype`：一页提案

读过的东西：<https://github.com/deepseek-ai/dsh-superagent-prototype>（MIT，约 8.4k 行 TypeScript + 约 2k 行编号设计文档 + 约 90 个测试文件）。它是一个**单仓库**的常驻 agent：听 GitHub/飞书，判断一条消息是否授权了工作，在自己的 worker 会话与目录里干活，回到原对话答复。

本页只借**设计形状与测试形状**，不借它的运行时前提，也不复制它的代码文本；每条都给出双方的 `文件:行`。现在不实现任何一条——四选一，或全否。

## 0. 为什么只借一点点

它稀缺的只有三样：授权模型、注意力模型、带契约测试的 prompt 工程。三样里只有一样能在不动产品决策的前提下落地。其余我们不是没有，而是覆盖面更窄——第 3 节指出差别在哪。

它的运行时前提也不成立，第 2 节把它们列为本仓的反面清单。

## 1. 借什么

### B1. 给"模型能读到的一切"加一个契约测试（建议先做这一条）

- **对方**：`tests/model-surface.spec.ts:40`（按角色给字符预算：orchestrator 59k / worker 40.5k）、`:50`（退役词表）、`:53`（禁止句表）、`:377`（文案里写的词数上限必须等于代码里的常量）、`:389-396`（预算断言）。它读组装后的 prompt、工具定义、skills 与提示文本，只要出现不存在的工具名、schema 选项、环境变量或术语就红。
- **我们**：prompt 在 `agent/src/prompt/mod.rs:19` 组装，来自约 1,400 行散文、4 个内置工具（`agent/src/tools/mod.rs:482`）、从用户目录发现的 skills（`agent/src/skills/mod.rs:31,69`）和三份编译期资产（`agent/src/prompt/communication.md:1-12` 等）。
- **差在哪**：没有任何东西拿这段文本去核对现实。散文里点名了具体命令和数字——`agent/src/prompt/mod.rs:501` 写 `future session abort`、`:520` 写 `max 600s`、`:1176` 靠手写断言守住 "at most three in flight"——所以 CLI 改了子命令名，prompt 会继续描述旧的，直到模型撞上。
- **代价**：`future-agent` 新增 1 个测试文件，250–350 行断言；无 schema；**不改已有行为**。唯一风险是第一天就红，所以起始放一份显式的已知漂移清单，只准减不准加。
- **验收**：`cargo test -p future-agent --test model_surface` 在当前树上全绿；并证伪一次——把 prompt 里某个命令改成不存在的子命令，该测试必须失败。可观测行为不变，证据就是重新引入漂移时测试变红。
- **回退**：删测试文件与清单，运行时零影响。

### B2. 给自主工作加一个 `observe` 档，并记录"凭什么授权"

- **对方**：`src/policy.ts:84`（`mode: 'off' | 'observe' | 'auto'`）、`:191`（确定性门控：谁可以发起工作，强度 `explicit`/`inferred`）、`:213`（工作恢复前重校验授权）、`docs/002-authorization.md:91-109`（observe 只记录"本来会接"而不接；授权记录策略版本；策略变更**通知**在跑工作而不是取消它）。
- **我们**：`channels/src/policy.rs:75-120` 只决定一条消息**能不能被回答**（dm/group 白名单、@ 门），不携带"这段工作凭什么被授权""依据哪版规则"。定时任务明写 full-permission：`docs/architecture/tasks.md:3`、`orchestration/tasks/src/lib.rs:1`。goal 有 gate，但 gate 记录的是决定，不是授权（`docs/architecture/goal-control-plane.md:55`）。全仓的 `--dry-run` 只作为破坏性命令的预览（`cli/src/commands/session_prune.rs:45`）和 goal 的 backfill 重放工具（`orchestration/goal/src/console.rs:608`）存在，没有一个"只记录不执行"的工作档位。
- **差在哪**：白名单会话里的任何人都能让 agent 直接干活，且没有办法先看"它本来会做什么"再放行。
- **代价**：三个 crate（`future-channel`、`future-tasks`、`future-goal`）加持久化字段，约 600–900 行。**会改已有行为，且要改 schema**（任务运行记录、goal 事件）。必须拆三步落地：先只记录 → 再执行 → 再恢复前重校验。
- **验收**：(1) 打开 observe 跑一次定时任务，任务账本里产生 1 条 `would_run` 记录且工具调用为 **0**；(2) 把某人从白名单移除后，其此前授权的长任务在下一次恢复时被拒——新测试 `mandate_revalidation`；(3) 改规则后在跑工作收到通知而不被取消，在事件流上断言。
- **回退**：档位默认 `off`，新字段可空；回退是删读取路径，不是删列。

### B3. 分段覆盖 prompt，并记住它基于哪一版内置文本

- **对方**：`src/app/prompts.ts:29-30`（每个覆盖存 `base`，即它据以改写的内置文本的 key）、`:186-192`（`defaultChanged` 报出"内置默认已经动过"的覆盖）、`:202`（保存必须报 base 版本，过期被拒）。
- **我们**：`agent/src/prompt/mod.rs:23-24`（`custom_prompt` 整体替换身份段）、`:106-107`（`append_prompt` 追加在后）。没有按段覆盖，也没有任何办法告诉用户"你改过的这段，默认值已经变了"。
- **差在哪**：用户一旦定制，之后每一次内置改进要么静默覆盖他的，要么静默失效。
- **代价**：`future-agent` 配置 schema + prompt 组装，约 300–450 行。**会改已有行为**：`custom_prompt`/`append_prompt` 与新 overlay 的优先级必须定义，迁移后输出要与今天逐字节一致。
- **验收**：prompt 模块单测——没改过的段跟随新默认，改过的段保留并报 `default_changed`；再用 golden 快照证明旧 `custom_prompt` 配置组装结果与今天的 `build_prompt` 完全相同。
- **回退**：没有 overlay 字段即今天的行为，删除该字段不需要动组装顺序。

### B4. 一套稳定的"为什么结束 / 为什么在等"代码词表

- **对方**：`src/core/reasons.ts:5-32`——理由是一个 `{code, values, triggerId}`；agent 读英文模板，面板按 `reason.<code>` 用自己的语言措辞；source 与 gate 可以自带码。
- **我们**：`desktop/src/features/vibe/attention.ts:23,36` 用字符串联合表示对话状态，`orchestration/goal/src/work_items/attention.rs:33-40` 用 `status: String` 表示目标状态，措辞则分别住在 `tui/src/workbench/i18n.rs`（1,811 行）和 `desktop/src/i18n/locales`。
- **差在哪**："为什么停了、在等谁"没有机器可读形式，于是三端把同一件事写成三种说法，既不能比较也不能过滤。
- **代价**：`future-goal`/`future-tasks` 的码表加三端映射，约 150 行；不动 schema（码写进已有字符串字段）；不改行为（旧文案保留，只加码）。
- **验收**：`reason_codes_are_stable`（码集合的序列化快照）加各端 locale 的 key 存在性测试；行为不变由现有快照测试证明。
- **回退**：在客户端来读之前无人读新字段；删码表即回到今天的字符串。

## 2. 不借什么

| 不借 | 对方证据 | 照搬到我们会坏在哪 |
| --- | --- | --- |
| 单机、单进程、单个 SQLite 文件 | `src/app/store.ts:54`（打开文件并取目录锁） | 我们同时跑 host、远端对端和桌面；"一个进程独占一个文件"会与 remote-host 和桌面并存冲突 |
| 无审批执行 | `docs/003-execution.md:27,80`——worker 以操作员 OS 用户身份跑命令，且包装"不是安全边界" | sandbox 与审批路径是我们的产品差异点；照搬等于把审批降级成 prompt 纪律 |
| 一个部署只服务一个仓库 | `docs/007-architecture.md:221`、`src/deployment.ts:51`（`SETUP_FIELDS`） | 我们的 agent 跨项目、跨工作区；把"单仓"写进部署是配置上的倒退 |
| router + worker 双会话 | `docs/003-execution.md:27-31`、`docs/007-architecture.md:509`（5 秒 shell/job 等待上限） | prompt 预算与延迟翻倍；我们已有 sub-session，能借的只是几条礼貌规则，那些单独引用了 |
| "注意力派生"这个**模式本身** | `src/core/stage.ts:9` | 我们已经有两份更贴合的：`desktop/src/features/vibe/attention.ts:36`（三态、首个命中即止）与 `orchestration/goal/src/work_items/attention.rs:103`（目标 gate）。缺的是 tasks/mobile 的覆盖面，不是这个想法——见第 3 节 |
| 持久化 effect/outbox 队列 | `src/app/effects.ts:5`（`Later`：推迟但不计一次失败） | 队列+退避+永久失败分类已在 `channels/src/outbox.rs:122` 与 `channels/src/delivery.rs:65`；第二份实现会让投递语义分叉 |
| 永不回收记录 | `docs/007-architecture.md:484`（"Nothing deletes old records yet"） | 我们为 `run_events` 增长付过账（`docs/guide/run-journal.md`） |
| 把部署策略当架构 | 200 行 auto-merge 上限、100 词聊天硬截断、`docs/003-execution.md:27` 的作业上限 | 那是一个团队的规则，不是 agent 系统的不变量 |

它的沟通 prompt 是唯一值得引用的散文资产（一句话转述、注明出处、改写而非照抄）：只在"人必须行动"或"人能用上结果"时说话；PR 存在不是交付里程碑；发出前逐句自问"删掉它，读的人还能理解结果或采取动作吗"；CI 与 bot 永远不能授权工作；意图不清楚时保持安静、让后续消息自己确立，而不是拿闲聊去问人确认。

## 3. 暂缓（小件，下次动到这些文件时顺手）

| 项 | 对方的点 | 我们的落点 | 验收 |
| --- | --- | --- | --- |
| 内存仓储镜像，用于域测试 | `src/core/memory.ts:10`，50 行，排序与嵌套回滚都与 SQLite 仓储一致 | goal/域测试仍需要数据库文件，而我们的 memory 记着多条由此产生的 flaky | 同一组域测试在内存实现上通过，含排序与回滚 |
| 健康条目单写者 | `src/infra/health.ts:18`（第二次认领直接抛错） | 没有东西断言某个健康键归谁 | 同名认领两次必须 panic，由测试断言 |
| 架构文档即被测清单 | `tests/docs.spec.ts:58`（`src` 下每个模块都必须出现在架构表里） | `docs/README.md` 索引了文档，但新模块可以不登记就落地 | 新增模块不登记即让文档门失败 |

## 4. 代价汇总（改了已有行为的单列）

| 条目 | crate | 量级（估算） | schema | 改已有行为 |
| --- | --- | --- | --- | --- |
| B1 契约测试 | `future-agent` | 250–350 行测试 | 否 | 否 |
| B2 observe + 授权记录 | `future-channel`/`future-tasks`/`future-goal` | 600–900 行 | **是** | **是**（分三步） |
| B3 prompt 分段覆盖 | `future-agent` | 300–450 行 | 配置 schema | **是**（优先级 + 迁移） |
| B4 理由码词表 | `future-goal`/`future-tasks`/客户端 | 约 150 行 | 否 | 否 |

## 5. 如果只能做一件

**B1。** 它是唯一一条"改善在跑的运行时却不改变它的行为"的：它要抓的漂移已经真实存在（一段手写散文点名 CLI 子命令和数字，没有任何东西核对），回退的爆炸半径是一个文件，而且它会让后三条更容易评审——同一个测试文件正好成为 B2–B4 声明"模型现在被允许读到什么"的地方。

B2 的产品价值最大，但不是我能启动的：它要先回答"谁有权让 FutureOS 干活"，那是产品决定。

## 6. 需要你定的

1. **B2 的前置**：FutureOS 里谁可以发起工作——只有 owner、白名单会话，还是一张规则表？不答这个，B2 只能画草图。
2. **B3 的前置**："用户可改 agent 行为"是不是产品方向？若"skills 加 AGENTS.md 就够了"，B3 应当被否掉而不是排期。
3. **B4 的前置**：要不要 TUI/桌面/手机共用一套词表？今天的重复有些是刻意的（手机端只占一行），所以这一条很可能答案是"不做"。

原始材料：<https://github.com/deepseek-ai/dsh-superagent-prototype> — `docs/001-domain.md` … `docs/007-architecture.md`、`src/core/`、`src/policy.ts`、`tests/model-surface.spec.ts`。MIT 许可；本文转述设计并标注行号，未复制其代码。
