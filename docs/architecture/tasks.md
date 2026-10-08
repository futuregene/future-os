# Tasks（任务）架构设计

> 任务 = 一个可复用的工作单元（prompt + 工作目录 + 模型/思考等级 + 全权限），由**触发器**决定何时运行；每次运行产生一条普通会话，并留下运行台账，可被反省改进 prompt。

与 `future loop` 的边界：loop = 长期目标 + 证据 + 门禁 + 多 agent 编排；tasks = **固定动作 + 触发器**，触发即跑。两者不共享表、不互相替代。

---

## 0. 定位

- **位置**：新增 crate `orchestration/tasks`（crate 名 `future-tasks`），与 `orchestration/loop` 同层。
- **宿主**：desktop（GUI）与 headless desktop 已接，共用同一份 tick 循环；TUI 未接；mobile 不 host，走 desktop 远程桥。
- **分层**：确定性内核（`next_due` / join / claim / 截断）全是纯函数，与 host 无关；执行器只有一份，host 差异收敛成 `Notifier` 闭包（见 §8）。

> **本版落地范围**：内核 + store + `future task` CLI（list/show/add/run/runs）+ desktop tick 与面板 + headless + mobile 桥与页面 + `future-task` 技能。
> **设计保留、尚未实现**：反省闭环（§7）、CLI 的 edit/enable/remove/feedback/prompt/deps/output（§9）、TUI host、`threads.task_id` 徽标（§2）、实例级 `.lock`（当前靠单 host + `has_running_run` 的 overlap 判定，足以避免重复执行）。

---

## 1. 借鉴 loop 的设计（该抄什么 / 不该抄什么）

| Loop 的做法 | Tasks 的对应 |
|---|---|
| 内核确定性、agent 只做判断（floors/signals/bounds） | claim/`next_due`/join/`truncate` 纯函数（已实现）；反省只提议（待接线） |
| "先提交、后尽力刷新"（`sync_compat`） | 通知失败不回滚已提交的运行（`notify` 不参与结果判定） |
| turn envelope：schema 头 + 上游预算 + 头尾截断 + 索引永不丢 | 任务信封 `future_tasks_run_envelope_v1`（schema 头 + 身份/run/trigger/settings 块 + 上游摘要预算/索引保留 + Instruction + 完成契约）；全文指针命令 `future task output <run-id>` |
| 独立 task class 防 frontier 误认领 | `task_runs.kind`（`main`/`manual`/`chain`/`reflection`）分开 |
| 独立状态根（`<cwd>/.future/loop/`） | 独立状态根 `<home>/.future/tasks/`（独立 SQLite，不进 app.db） |

**不抄**：fencing token、多 agent 竞争、claim-stealing、事件溯源 JSONL（用 SQLite ACID）、monitor cadence（loop 已有）、quota/should-run（简化为 tick 的 claim 判定）。

---

## 2. 数据模型

状态根：`<home>/.future/tasks/`（`<home>` = 每个 host 自己的 FutureOS home；`FUTURE_HOME` 覆盖整根，与 agent 一致）。

> 设计中的 `<home>/.future/tasks/.lock` 实例互斥**未实现**：当前靠单一 host 执行 + `has_running_run` 的 overlap 判定避免重复跑同一条任务。两个 host 指向同一个 home 同时 tick 是已知未覆盖场景。

SQLite：`tasks.db`，WAL + `busy_timeout=5000`。

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- schema_version

CREATE TABLE tasks (
  id               TEXT PRIMARY KEY,          -- tsk_...
  name             TEXT NOT NULL,
  enabled          INTEGER NOT NULL DEFAULT 1,
  prompt           TEXT NOT NULL,
  prompt_version   INTEGER NOT NULL DEFAULT 1,
  cwd              TEXT NOT NULL,             -- 工作目录；conversation_mode=chat 且未指定时为空串（用会话自带的临时工作区）
  model_id         TEXT,                      -- provider/model；表单要求必选，CLI 可留空=用应用默认
  thinking_level   TEXT,                      -- off|minimal|low|medium|high|xhigh；表单要求必选
  session_policy   TEXT NOT NULL DEFAULT 'new',  -- new | existing
  conversation_mode TEXT NOT NULL DEFAULT 'workspace',  -- workspace | chat（新任务的表单默认 chat）
  thread_id        TEXT,                      -- session_policy=existing 时的宿主会话绑定（懒建）
  trigger_kind     TEXT NOT NULL,             -- manual | schedule
  trigger_json     TEXT NOT NULL,
  dep_join         TEXT NOT NULL DEFAULT 'all',  -- all | any
  next_due_at      INTEGER,                   -- 仅 schedule；NULL=不再触发
  last_run_at      INTEGER,
  pending_request_at INTEGER,                 -- 待执行的显式/链式触发（tick 消费）
  pending_origin   TEXT,                      -- ui | cli | chain | schedule
  pending_actor    TEXT,                      -- agent:<sid> / user / cli / task:<id>
  reflection       TEXT NOT NULL DEFAULT 'ask',  -- off | ask | auto（默认开启）
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER
);
CREATE UNIQUE INDEX tasks_active_name ON tasks(name) WHERE deleted_at IS NULL;

CREATE TABLE task_deps (                      -- 下游任务依赖的上游任务
  task_id          TEXT NOT NULL REFERENCES tasks(id),
  upstream_task_id TEXT NOT NULL REFERENCES tasks(id),
  on               TEXT NOT NULL,             -- success | failure | completed
  PRIMARY KEY (task_id, upstream_task_id),
  FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
  FOREIGN KEY (upstream_task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE TABLE task_dep_state (                 -- 凑齐进度（join=all 用）
  task_id          TEXT NOT NULL,
  upstream_task_id TEXT NOT NULL,
  satisfied_run_id TEXT,                      -- NULL = 这条边还没到
  satisfied_at     INTEGER,
  PRIMARY KEY (task_id, upstream_task_id),
  FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE TABLE task_runs (
  id             TEXT PRIMARY KEY,            -- trn_...
  task_id        TEXT NOT NULL REFERENCES tasks(id),
  kind           TEXT NOT NULL,               -- main | manual | chain | reflection
  origin         TEXT NOT NULL,               -- schedule | ui | cli | chain | reflection
  actor          TEXT,                        -- agent:<sid> / user / cli / task:<id>
  due_at         INTEGER,                     -- 仅 kind=main
  status         TEXT NOT NULL,               -- running|completed|failed|skipped
  thread_id      TEXT, session_id TEXT, run_id TEXT,
  prompt_version INTEGER,
  result_summary TEXT,                        -- 截断到 2000 字符（head…tail）
  feedback       TEXT, feedback_note TEXT,    -- good | bad + 备注
  started_at INTEGER, finished_at INTEGER, error_message TEXT,
  UNIQUE(task_id, due_at)                     -- NULL 互不冲突：manual/chain/reflection 不受约束
);

CREATE TABLE task_prompt_revisions (
  id         TEXT PRIMARY KEY,
  task_id    TEXT NOT NULL REFERENCES tasks(id),
  version    INTEGER NOT NULL,
  prompt     TEXT NOT NULL,
  source     TEXT NOT NULL,                   -- user | reflection | rollback
  status     TEXT NOT NULL,                   -- active | superseded
  reason     TEXT, confidence REAL, source_run_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(task_id, version)
);
```

桌面投影：任务**不进 `app.db`**。任务会话就是普通会话，靠标题「任务名 · 时间」在会话列表里辨认；任务 ↔ 会话的映射由 `task_runs.thread_id` 承担（可从任务侧反查，也可从 Runs 面板打开）。

> 待办（本版刻意未做）：在 `app.db` 的 `threads` 上加可选列 `task_id`，让侧栏直接给任务会话打徽标。它需要一条 GUI 版本化迁移（`desktop/CLAUDE.md` 规则 7）与 `ER.md` 同步，而标题已经能满足"会话出现在列表里"这一需求，所以留到有真实需要时再加。

---

## 3. 触发器

```jsonc
{ "kind": "manual" }

{ "kind": "schedule", "mode": "once",     "date": "2026-12-24", "time": "09:00" }
{ "kind": "schedule", "mode": "interval", "every_minutes": 120, "anchor": 1760000000000 }
{ "kind": "schedule", "mode": "daily",    "time": "09:00" }
{ "kind": "schedule", "mode": "weekly",   "days": ["mon","wed","fri"], "time": "10:00" }
{ "kind": "schedule", "mode": "monthly",  "day": 31, "time": "09:00" }   // 无 31 日的月份 → 当月最后一天
```

规则（纯函数 `next_due(trigger, after) -> Option<i64>`，带单测）：

- **interval**：锚定网格——到期点 = `anchor + k·every`，取最小未来网格点；不漂移靠网格，不靠补跑。
- **daily/weekly/monthly**：日历网格（本机本地时区墙钟），错过的日历点补跑一次。
- **短月**：monthly `day > 当月天数` 时顺延到当月最后一天；UI/CLI 常驻可见此规则。
- **once**：跑完 `enabled=0`。

## 4. 依赖（deps）与 join

`dep_join ∈ all | any`，每条边 `on ∈ success | failure | completed`。

- **边的满足**：上游 run 到终态 → 命中该边 `on` → 记 `task_dep_state.satisfied_run_id = run_id`（每边只记最近一次）。`skipped`/`reflection`/`test` 不记。
- **claim 判定**（tick 内）：
  - `all`：所有边都有 `satisfied_run_id` → 触发并**清空全部边**。
  - `any`：至少一条边有标记 → 触发并**清空当时已满足的边**。
- **手动 run 永不消费边标记**；只有链式触发（`kind=chain`）消费。
- **任务运行中**：凑齐也不消费，挂到下个 tick——链式触发是"挂起等待"，不像定时触发那样"跳过"。
- **环检测**：claim 时沿 deps 回溯，命中自身即标 `failed("cycle")`（深度上限 32）；写入时仅早提示。
- **触发器只有一种选择**（三个客户端）：`手动 / 依赖触发 / 一次性 / 每隔 N 分钟 / 每天 / 每周 / 每月`。选「依赖触发」就是 `trigger_kind=manual` + 有边；下面的依赖编辑器随选择出现，边为空时它提示先添加上游。存储上 `trigger_kind` 与依赖边是**正交的**（CLI 可以只给 `--daily --depends-on A`），所以一个既有定时又有上游的任务仍然把边显示出来，并附一句说明（否则边既看不见也删不掉）；在编辑器里换回定时**不会**顺手删边（草稿保留整组依赖，保存时照写）。列表/摘要的触发标签据此区分：仅依赖触发且无自身定时的任务显示「依赖触发」而不是「手动」。
- **谁来写边**：三个地方共用同一套语义与同一条拒绝路径（先算上这条边的完整图，`would_cycle` 就拒）——`future task add/edit --depends-on A[:success|failure|completed]`（edit 是**整组替换**）、桌面编辑器的「依赖触发」块、手机编辑器（能力 `task_deps_v1`）。前端都把依赖当成“整组期望值”提交：写入按当前库里的边做调和（保留不变的不重写、改条件改一条、删边删一条、新边补一条），而不是把 UI 事件顺序当成写入顺序。手机端点与桌面面板共用同一份 `tasks.db`，所以三个入口的权限/校验一致。

## 5. 执行链路（tick → claim → 执行）

```
tick（30s，墙上时钟；`Run now` 写入 pending_request_at 后会把循环立刻唤醒一次）
 └─ 捞出 enabled=1 且 (pending_request_at IS NOT NULL OR (schedule 且 next_due_at<=now))
      └─ claim 四步（朴素，非"事务性"）：
           1. 读 due
           2. INSERT task_runs(status='running', kind/origin/actor/due_at)
           3. 推进 next_due_at / 清 pending_request_at
           4. spawn 执行
      └─ execute：
           1. session_policy 分支：new→新建会话 / existing→打开 tasks.thread_id（懒建）
           2. existing 模式：等会话空闲 → compact → 等终态（超时 2 分钟）
           3. 合成信封（`future_tasks_run_envelope_v1`：身份/run/trigger/settings 块 + 上游摘要预算 1200 + 索引保留 + 全文指针 → `Instruction:` → 完成契约）
           4. provision + run_prompt（permission="all"，sandbox tier="off"）
           5. 收尾：task_runs ← status/finished_at/result_summary
                    + 标记上游边（命中 on）
                    + 反省档位 → 追加 reflection run（kind=reflection）
           6. notify（host 决定：GUI 发事件 / headless 只写台账）
```

**崩溃残留**：启动对账把 `status='running'` 且进程已死的 run 标 `failed("interrupted")`；链式等待在与上游收尾同一个 SQLite 事务里，进程死在中间不丢触发。

**重叠**：任务在跑时新触发记 `skipped`（不排队）。

### 5.1 运行信封（`future_tasks_run_envelope_v1`）

每次运行的首条 prompt = 信封 + 任务自己的 prompt + 完成契约，格式与 loop 的 turn envelope 同构（纯文本块，非 XML 标签）：

```
── future_tasks_run_envelope_v1 ──
task: 每周进展周报 | id: tsk_... | prompt-version: 3
run: kind=main | due=2026-10-07T09:00:00+08:00
trigger: {"mode":"weekly","days":["mon","fri"],"time":"10:00"}
run settings: cwd=/Users/... | conversation=workspace | session=new (fresh conversation per run) | model=... | thinking=high

Upstream results: (summaries only …)          ← 仅有上游依赖时
- upstream tsk_a "upstream" [run trn_b status=completed finished=...]: …
Full output of one source: future task output <run-id> …

Instruction:
<任务 prompt 原文>

Completion contract:
- 本次运行无人应答（unattended），要自行选择并记录，不要以提问结尾。
- 结尾写一份简短报告（它就是本次 run 的 result_summary）：做了什么、产物与路径、与之前运行的新增、仍不确定的事。
```

要点：**身份与运行上下文的唯一可信来源**（"什么时候跑的""跑在哪个模型/目录"）；chat 任务没有目录时**不打印空 `cwd=`**；`session=existing` 附带“同一会话、本次运行前已压缩”语义；上游块保留索引与预算规则（见 §1 表格）。

## 6. 会话策略

| `session_policy` | 语义 |
|---|---|
| `new`（默认） | 每次触发新开会话，标题 `任务名 · 2026-10-07 09:00` |
| `existing` | 复用同一条会话（首次懒建 `tasks.thread_id`），每次运行**直接接在同一会话后**——不压缩（见下） |

会话类型（`conversation_mode`）决定归档位置与工作目录：`workspace` 会话归档在工作目录下（必须有 `cwd`，服务端空值直接报错）；`chat` 会话出现在「对话」里、用会话自带的临时工作区，**不需要目录**（表单默认 chat，两个客户端一致；CLI 仍可给 `--cwd`，给了就沿用）。

**为什么不再预压缩**：`existing` 的意义就是承接前几轮的结果，而压缩既花一遍全历史，又把这次运行回来要用的上下文丢掉。需要短上下文时用户自己在会话里压缩（手动 `compact` RPC 仍在，与 run 互斥）。

## 7. 反省（prompt 优化建议）—— 已接线

每次运行结束后（`reflection != off`，且该运行本身不是反省），host 会**另起一次 agent turn 做反省**：新开一条 chat 会话（标题 `任务名 · suggestion · 时间`，创建后立即归档，侧栏不显示；run 台账里有 thread_id，面板的「打开会话」可直接读到推理过程）。

- **输入（只看本次 run 窗口）**：本次 prompt、`result_summary`、run 状态与错误、以及用户对该 run 的 `feedback` 判定与备注。不喂历史，避免“基于长历史的大重写”无人能审。
- **输出**：末段 JSON `{verdict: keep|improve, prompt, reason, confidence}`；解析容忍代码围栏与前后解释（取**最后一个**完整对象，字符串里的花括号不影响）。
- **`ask`（默认）**：写入 `task_prompt_revisions`，`status=proposed`、`source=reflection`、`version=PROPOSAL_VERSION(0)`（建议不是版本，不进版本序列）、带 `confidence` 与 `source_run_id`。UI/CLI 列出并可一键采纳。
- **`auto`**：过护栏才直接生效，否则降级为建议——run 必须 `completed`、置信度 ≥0.7、24h 内没有已生效的建议（`REFLECTION_AUTO_INTERVAL_MS`）。
- **防震荡（两种档位都适用）**：建议文本与当前 prompt 相同 / 与“被当前版本替代的那一版”相同 / 24h 内重复过同内容建议 → 不记录。护栏全是 `kernel.rs` 里的纯函数（`decide_proposal`），参数是代码常量。
- **采纳**：`prompt apply` / 面板的「采用建议」走 `prompt_change_revisions`（被替换那版记 `superseded`，新生效那版 `source=reflection`），并把建议行标为 `applied`。
- **失败隔离**：反省失败（agent 不可达、回复不可解析、超时）只记在自己那条 `kind=reflection` 台账上；被反省的 run 不受影响。反省进行中不阻塞任务自己的下一次 claim（`has_running_run` 只看非 reflection 的运行）。

`future task feedback` 的判定现在**有读取者**了：反省 prompt 会带上「用户对这次运行的判定」。

## 8. Host 抽象（实际实现：`Notifier`，不是 Executor trait）

执行器只有一份，住在 desktop 的 `tasks.rs`；host 差异只有「跑完之后怎么通知」，因此抽象是一个闭包而不是 trait：

```rust
pub type Notifier = Arc<dyn Fn(Option<&str /* thread_id */>) + Send + Sync>;

pub fn start<R: tauri::Runtime>(app: AppHandle<R>)  // GUI：emit_remote_activity + emit_threads_updated
pub fn start_headless()                            // 无 GUI：通知降级为空，台账就是信号
async fn run_loop(notify: Notifier)                // 启动对账 + 每 30s tick
```

理由：`Executor` trait 需要 5 个方法，但它们全部只被这一份实现调用，且都是对 `agent_bridge` 既有函数的薄包装——加一层 trait 只增加间接性。tick 循环本身与 GUI 无关（`start_headless` 与 GUI 走同一个 `run_loop`），所以"headless 也支持"是同一份代码少一个通知出口，而不是第二套实现。

host 现状：
- **GUI desktop**：已接（`lib.rs` setup 里 `tasks::start`）。
- **headless desktop**：已接（`headless/mod.rs` 里 `tasks::start_headless`）。
- **mobile**：不 host；经 remote 桥管理（能力 `tasks_v1`）。
- **TUI**：**未接**。crate 与 CLI 已经可用，TUI 面板是后续增量。

## 9. CLI（`future task`）

```
future task list [--all] [--json]
future task show <id|name> [--json] [--prompt]
future task add --name N --prompt P|--prompt-file F --cwd D
                [--model M] [--thinking L] [--session new|existing]
                [--conversation workspace|chat]
                [--reflection off|ask|auto] [--disabled] [--json]
                [--depends-on A[:success|failure|completed]]… [--join-any]
                (--manual | --at | --every | --daily | --weekly | --monthly)
future task edit <id|name> [any add flag] [--enable|--disable]
future task enable|disable <id|name>
future task remove <id|name> [--yes]
future task run <id|name> [--wait] [--timeout 15m] [--json]
future task runs <id|name> [--limit N] [--json]
future task output <run-id> [--tail N] [--json]
future task feedback <run-id> good|bad [--note "…"]
future task upstream|deps <id|name> [--json]
future task prompt log|apply|revert <id|name> [revision-id]
```

- CLI 只是同一份 `tasks.db` 的客户端：写立即落盘，desktop 下个 tick 生效；**CLI 从不自己执行 run**。
- `run` 默认只排队并如实说明；`--wait` 轮询台账到终态，`--json` 给出 `status` / `runId` / `threadId` / `resultSummary`。
- `output <run-id>` 是信封里“全文指针”的实现：读该 run 会话的**最后一条 assistant 文本**，先打印 run 身份（任务/状态/prompt 版本/会话）再打印正文；复用同一会话时，如果该会话之后又被别的 run 用过，会额外提示那个 run 的 id（否则文字容易被归错到错的那轮）。`--tail N` 只看结尾。
- **prompt 版本与建议**：`prompt log` 把待定建议单独列在 `Suggestions (not applied)` 下（带 `confidence`、理由与可直接执行的 `prompt apply` 命令）；已采纳的建议标 `applied` 但不编号（建议没有版本号，采纳时才产生下一版）。`prompt apply` 接受版本 id 或建议 id。
- `runs` 与 `prompt log` 对**已删除任务**仍可读（按 id 或名字）：`remove` 是软删，台账与版本历史正是审计要的东西；`list` 里则不再出现。
- `enable` 会为 schedule 重算下次时间（暂停跨过时间点的任务否则永不触发）；`remove` 软删并清掉指向它的依赖边。
- 帮助里明确与 `future loop todo` 区分。

**实现与文档必须一致**：`cli/src/commands/task.rs` 有一个测试逐行解析 `help::TASK_HELP`，
要求它与分发器列出**同一批**子命令（双向集合相等）。这是为已经真实发生过的一类缺陷设的闸门：
命令面被描述给用户/技能/其它 agent，而分发器没有对应分支，文档里的调用直接报 `Unknown argument`。

## 10. 技能 `future-task`

`skills/builtin/future-task/SKILL.md`（skills 子模块，独立 PR）。内容以"prompt 怎么写"为主：无人值守下的 8 条约束、触发器选择（含短月顺延必须告知用户）、`run → 判读 → feedback → 改 → 再跑` 的迭代闭环、反省建议的采纳/回退（§7a）、以及其它 agent 用 `future task run --wait --json` 触发的约定（有外部副作用的先取得用户同意）。

**端到端验证**（真实 agent + 真实模型，不进 CI）：`desktop/src-tauri/src/tasks.rs` 里两个 `#[ignore]` 测试——`a_queued_run_against_a_real_agent_*` 自检跑一段真 run 并断言总结与建议行，`serve_pending_runs_until_idle` 是个一次性 host（与 headless 同一套 `tick`，不要登录/远程桥），用来配合 CLI 手测：

```bash
# 一个一次性实例：agent 与 host 共用同一个 HOME（否则 chat 工作区与 agent 的托管根不一致）
HOME=/tmp/try future agent --home /tmp/try/.future --grpc-addr 127.0.0.1:5099 &
FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 future task add … && future task run NAME
HOME=/tmp/try FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 \
  cargo test --lib tasks::tests::serve_pending_runs_until_idle -- --ignored --nocapture
```

- 技能里引用的命令必须限定在 §9 已实现的那批——首版技能描述了一批不存在的子命令（`edit`/`feedback`/`prompt log`/`upstream`/`--depends-on`），
  已按实际实现修正。
- **技能与 CLI 的版本对齐没有自动化守卫**：skills 是独立仓库且父仓库的 CI 不检出该 submodule，
  所以"技能写了、CLI 没有"只能靠人核对（或本文件的 §9 与技能一起改）。父仓库里那条帮助/分发一致性测试覆盖不了跨仓库这一层。
- 技能里明确写出反射尚未接线，避免向用户承诺不会发生的提案。

**已实现的配套命令（与技能同步）**：`edit` / `enable` / `disable` / `remove` / `feedback` / `upstream` / `prompt log|apply|revert`。

## 11. UI

- **desktop（已实现）**：左侧导航「任务」（`ActivityRail` 展开/收起两种形态都有；位置在「技能」之后、「手机遥控」之前，与需要长期维护的条目同类）；面板含列表、编辑器（prompt/cwd/模型/思考等级/会话策略/反省档位/5 种触发 + 短月提示）、运行台账、提示词版本与应用。任务页是自己的两栏视图（列表 + 详情），所以**不显示右侧上下文面板**（那描述的是当前会话，不是任务）。
- **mobile（已实现）**：Settings 栈内「任务」页，由 `tasks_v1` 能力门控；列表不带 prompt 正文（wire 预算），详情单独取；含运行记录、依赖状态、版本应用。依赖的**编辑**另由 `task_deps_v1` 门控（`set_task_dep`/`remove_task_dep` 两条命令 + 能力位）：老桌面端只显示只读依赖列表并提示升级，新桌面端在编辑器里有与桌面同形的「依赖触发」块（逐边条件 + 新增/移除 + 多上游 join）。
- **依赖编辑器（两端）**：它是触发器选项的一部分（选「依赖触发」才出现，已有上游时也保留），草稿持有整组依赖、保存时调和；候选列表是其它仍存活的任务（新建时无自身 id，所以是全部任务）；成环由后端拒绝（两头看到同一条错误文案），因此前端无需拉取全图；新增边默认「成功之后」。

两个客户端共用同一套表单规则：新任务默认**对话会话**（因此不要求目录；切到工作区会话才要），模型与思考等级**必须显式选择**（没有“默认”选项，未选时保存被拒并说明缺哪项），模型列表只给用户在「模型」页启用的那批（任务已绑定但被停用的模型仍保留可选，避免编辑其它字段时静默改写）。

**即时性**：`Run now`（桌面按钮、手机点击）写 `pending_request_at` 后调用 `tasks::wake()` 唤醒 tick 循环，运行在毫秒级开始，而不是等下一个 30s 节拍（tick 仍是唯一的 claim 者）；运行开始/结束时 host 既有 `threads-updated` 事件，任务面板据此重读列表与已打开的详情（否则会一直停在“已排队”）。

**建议的可见性**：待定建议在**任务列表行**上是强调色计数标签（`pendingProposals`，桌面与手机同一字段），在详情里是一条带「建议」标记 + 置信度 + 「采用建议」按钮的记录（不是 `v0`；已采纳的标「已采用」）。采纳后：新生效那版 `source=reflection`、被替换那版记 `superseded`、建议行标 `applied`；`store.insert_revision` 会把同一个任务里更早的 `active` 行降为 `superseded`，任何时刻只有一个版本在生效。


## 12. 明确不做

每年触发器；cron 表达式与命名时区；多对一之外的高级编排（fan-out 是普通 todo 的事）；OS 常驻服务；shell/loop/skill 类动作；系统通知；每任务工具白名单（全权限是唯一策略）。

---

## 13. 关键不变量（不改）

- desktop 是唯一执行者（headless desktop 是同实现少通知）；CLI/mobile 只排队/读写状态。
- 全权限是 v1 唯一策略（`permission_level="all"` + `sandbox tier="off"`）。
- 五种触发 + 短月顺延月末。
- 手动 run 不消费依赖边；链式触发才消费。
- 反省默认开启（`ask`），只看本次 run 窗口。
