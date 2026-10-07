# Tasks（任务）架构设计

> 任务 = 一个可复用的工作单元（prompt + 工作目录 + 模型/思考等级 + 全权限），由**触发器**决定何时运行；每次运行产生一条普通会话，并留下运行台账，可被反省改进 prompt。

与 `future loop` 的边界：loop = 长期目标 + 证据 + 门禁 + 多 agent 编排；tasks = **固定动作 + 触发器**，触发即跑。两者不共享表、不互相替代。

---

## 0. 定位

- **位置**：新增 crate `orchestration/tasks`（crate 名 `future-tasks`），与 `orchestration/loop` 同层。
- **宿主**：desktop（GUI）、headless desktop、TUI 三个 host 共用同一份执行器；mobile 不 host，走 desktop 远程桥。
- **分层**：确定性内核（`next_due` / join / claim / 截断）全是纯函数，与 host 无关；执行器通过 `Executor` trait 接 agent；反省（LLM）只能提**提案**，永不直接改状态。

---

## 1. 借鉴 loop 的设计（该抄什么 / 不该抄什么）

| Loop 的做法 | Tasks 的对应 |
|---|---|
| 内核确定性、agent 只做判断（floors/signals/bounds） | claim/`next_due`/join/`truncate` 纯函数；反省只提议 |
| "先提交、后尽力刷新"（`sync_compat`） | 任务的投影刷新（会话徽标/通知）失败不回滚已提交的运行 |
| turn envelope：schema 头 + 上游预算 + 头尾截断 + 索引永不丢 + 全文指针 | 任务信封 `<task ... schema="task-v1">` + `future task output <run-id>` 指针 |
| 独立 task class 防 frontier 误认领 | `task_runs.kind`（`main`/`manual`/`chain`/`reflection`）分开 |
| 独立状态根（`<cwd>/.future/loop/`） | 独立状态根 `<home>/.future/tasks/`（独立 SQLite，不进 app.db） |

**不抄**：fencing token、多 agent 竞争、claim-stealing、事件溯源 JSONL（用 SQLite ACID）、monitor cadence（loop 已有）、quota/should-run（简化为 tick 的 claim 判定）。

---

## 2. 数据模型

状态根：`<home>/.future/tasks/`（`<home>` = 每个 host 自己的 FutureOS home）。实例级互斥：`<home>/.future/tasks/.lock`（单 host tick 持锁）。

SQLite：`tasks.db`，WAL + `busy_timeout=5000`。

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- schema_version

CREATE TABLE tasks (
  id               TEXT PRIMARY KEY,          -- tsk_...
  name             TEXT NOT NULL,
  enabled          INTEGER NOT NULL DEFAULT 1,
  prompt           TEXT NOT NULL,
  prompt_version   INTEGER NOT NULL DEFAULT 1,
  cwd              TEXT NOT NULL,
  model_id         TEXT,                      -- provider/model，空=默认
  thinking_level   TEXT,                      -- off|minimal|low|medium|high|xhigh
  session_policy   TEXT NOT NULL DEFAULT 'new',  -- new | existing
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

桌面投影：`app.db` 的 `threads` 表加可选列 `task_id TEXT`（无 FK）。会话行显示小徽标 + 任务名；任务/运行台账不回 app.db。

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

## 5. 执行链路（tick → claim → 执行）

```
tick（30s，墙上时钟）
 └─ 捞出 enabled=1 且 (pending_request_at IS NOT NULL OR (schedule 且 next_due_at<=now))
      └─ claim 四步（朴素，非"事务性"）：
           1. 读 due
           2. INSERT task_runs(status='running', kind/origin/actor/due_at)
           3. 推进 next_due_at / 清 pending_request_at
           4. spawn 执行
      └─ execute：
           1. session_policy 分支：new→新建会话 / existing→打开 tasks.thread_id（懒建）
           2. existing 模式：等会话空闲 → compact → 等终态（超时 2 分钟）
           3. 合成信封（schema="task-v1" + 上游摘要预算 1200 + 索引保留 + 全文指针）
           4. provision + run_prompt（permission="all"，sandbox tier="off"）
           5. 收尾：task_runs ← status/finished_at/result_summary
                    + 标记上游边（命中 on）
                    + 反省档位 → 追加 reflection run（kind=reflection）
           6. notify（host 决定：GUI 发事件 / headless 只写台账）
```

**崩溃残留**：启动对账把 `status='running'` 且进程已死的 run 标 `failed("interrupted")`；链式等待在与上游收尾同一个 SQLite 事务里，进程死在中间不丢触发。

**重叠**：任务在跑时新触发记 `skipped`（不排队）。

## 6. 会话策略

| `session_policy` | 语义 |
|---|---|
| `new`（默认） | 每次触发新开会话，标题 `任务名 · 2026-10-07 09:00` |
| `existing` | 复用同一条会话（首次懒建 `tasks.thread_id`），每次运行前**自动压缩**（明确前置阶段，超时 2 分钟；失败即本次 run 失败） |

压缩语义：手动 RPC `compact` 异步 worker（`operationId` + `compaction_unchanged`/`compaction_failed`）；与 run 互斥（run 在跑时 `session_busy`）。

## 7. 反省（prompt 优化建议）

- **范围**：只看本次 run 窗口——本次 prompt、result_summary、run 状态、本会话内 run 结束后用户追加消息数。
- **输出**：末段 JSON `{verdict, prompt, reason, confidence}`。
- **`ask`（默认）**：进 `revisions(status=proposed)` 等采纳；**`auto`**：过护栏才自动生效（置信度≥0.7、主 run 成功、只改 prompt、24h 内自动生效≤1 次、不等于最近被拒版本、不重复同天提案）。
- **防震荡**：轻量检查（不喂历史）——新 prompt ≠ 最近被拒版本；同天不重复同内容提案。
- **护栏参数**：代码常量，不进 DB。

## 8. Executor trait

```rust
pub trait Executor {
    fn ensure_agent(&self) -> Result<AgentHandle>;
    fn create_or_open_session(&self, t: &Task, due: i64) -> Result<Session>;
    fn pre_compact_if_reused(&self, s: &Session) -> Result<()>;
    fn run_prompt(&self, s: &Session, t: &Task, envelope: &str) -> Result<RunOutcome>;
    fn notify(&self, event: &TaskEvent);
}
```

host 实现：
- **GUI desktop**：sidecar supervisor + `create_thread`/`provision_agent_session_with_policy` + `emit_remote_activity`/`emit_threads_updated`。
- **headless desktop**：同实现，`notify` 只写台账。
- **TUI**：agent gRPC 起独立会话（不创建 desktop thread）；`notify` 更新 TUI 状态。
- **mobile**：不 host，走 desktop 远程桥（能力 `tasks_v1`）。

## 9. CLI（`future task`）

```
future task list|show|add|edit|enable|disable|remove
future task run <id|name> [--prompt-file DRAFT] [--wait] [--timeout 15m] [--json]
future task runs <id> [--limit 20] [--json]
future task output <run-id> [--format markdown|json] [--tail N] [--full]
future task feedback <run-id> good|bad [--note "..."]
future task prompt log|apply|revert <id>
future task deps <id> [--json]
```

- `run` 默认只排队；`--wait` 阻塞到终态并打印 status/thread_id/result_summary。
- `output` 走 agent gRPC `get_session_entries`（agent 不在时回落到 `task_runs.result_summary`，并明确提示全文需要 agent）。
- 帮助里明确与 `future loop todo` 区分。

## 10. 技能 `future-task`

`skills/builtin/future-task/SKILL.md`（skills 子模块独立 PR），`allowed-tools: Bash(future:*)`。

- prompt 约束：自包含 / 无交互 / 单次有界 / 副作用最小 / 输出契约 / 可验收 / 稳定可归因 / 禁止自我扩散权限。
- 迭代工作流：`add → run --wait → feedback → edit --prompt-file → run --prompt-file → 满意落库 → 观察 ask 提案 → 视情况升 auto`。
- 版本对齐：SKILL.md 写"最低 `future` 版本 x.y.z"。

## 11. UI

- **desktop**：左侧导航「任务」（`ActivityRail` 新增 `tasks` 分区）；列表/编辑/运行台账/prompt 版本 diff/依赖视图（all/any + "等待 2/3 个上游"）；单行紧凑，低频操作收进行内菜单。
- **mobile**：Settings 栈内「任务」页（能力 `tasks_v1` 判定）；列表只带截断摘要（wire 预算）；详情/编辑/运行历史/版本。

## 12. 明确不做

每年触发器；cron 表达式与命名时区；多对一之外的高级编排（fan-out 是普通 todo 的事）；OS 常驻服务；shell/loop/skill 类动作；系统通知；每任务工具白名单（全权限是唯一策略）。

---

## 13. 关键不变量（不改）

- desktop 是唯一执行者（headless desktop 是同实现少通知）；CLI/mobile 只排队/读写状态。
- 全权限是 v1 唯一策略（`permission_level="all"` + `sandbox tier="off"`）。
- 五种触发 + 短月顺延月末。
- 手动 run 不消费依赖边；链式触发才消费。
- 反省默认开启（`ask`），只看本次 run 窗口。
