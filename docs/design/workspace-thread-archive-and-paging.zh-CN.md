# 工作区与会话：分页加载与归档设计

> 状态：设计定稿，尚未实现。本文汇总多轮讨论的最终结论，作为后续实施的依据。
> 主要范围：桌面端（`desktop/` + `host/`），兼述远程目录、移动端与 TUI/CLI 的影响。

## 1. 背景与现状

随着用户的会话与工作区增多，当前"全量加载 + 全量刷新"的列表模型成为性能与可用性瓶颈。

| 层 | 现状 | 关键位置 |
|---|---|---|
| 数据模型 | `threads` 表已有 `status('active'/'archived'/'deleted')` + `archived_at`；`workspaces` 表无归档字段（`cleanup_status` 是临时工作区的磁盘清理状态机，与归档无关） | `host/src/store/schema.rs` |
| 存储层 | `list_threads()` 全量返回所有非删除会话（含归档），无分页；`archive_thread`/`restore_thread` 已存在；`list_workspaces()` 全量返回 | `host/src/store/threads.rs`、`workspaces.rs` |
| 命令层 | Tauri 命令 `archive_thread`/`restore_thread` 已注册；无分页版 list 命令 | `desktop/src-tauri/src/commands/threads.rs` |
| 前端加载 | `useThreadStore` 启动时全量加载，每次增删改后 `refreshStore` 全量重载；靠 `thread-activity-updated` 等推送做行级补丁 | `desktop/src/components/layout/hooks/useThreadStore.ts` |
| 前端渲染 | Rail 树视图客户端过滤 `status === "active"`；Recent 视图有客户端窗口（`RECENT_PAGE_SIZE = 30` + "显示更多"）；搜索是纯客户端过滤 | `ActivityRail.tsx`、`recentOrder.ts` |
| 归档 UI | 半成品：行菜单在 `archived` 时显示"恢复"，但活跃行没有"归档"动作，也没有列出已归档会话的界面 | `ActivityRailMenus.tsx` |
| 消息历史 | 已有游标分页先例：`get_session_entries_page(before, limit)` + `historySource` 栅栏 | `host/src/application/threads.rs` |
| 删除 | 已是树级级联（`thread_delete_closure`，删父带子） | `host/src/store/threads.rs` |
| 建树规则 | 父不在列表（含已归档）时子提升为根；pinned 必为根；超三层拍平 | `desktop/src/components/layout/threadTree.ts` |
| Agent 侧 | `list_sessions` RPC 全量枚举，无分页、无归档概念 | `agent/src/rpc/commands/session_lifecycle.rs` |

结论：归档的后端骨架已存在，缺 UI 闭环、workspace 归档与列表分页。分页要改的不只是 SQL——前端"全量加载 + 全量刷新"的模式必须一起改。

## 2. 目标与非目标

**目标**

1. **三类列表全部分页加载**：工作区列表、工作区内的对话、临时对话（聊天区）。启动与刷新只加载首屏所需数据。
2. 完整的归档闭环：归档动作 → 从默认列表消失 → 可浏览、可搜索、可查看详情、可恢复。
3. 工作区可归档（隐藏），归档后不加载其会话。
4. 归档是非破坏性的：历史、Agent session、恢复后继续对话的能力全部保留。

**非目标（本期不做）**

- 自动归档策略（如 N 天不活跃自动归档、run 结束后自动归档）。
- Agent session 层的归档与分页（TUI/CLI 保持全量可见）。
- 列表的完整虚拟化渲染（先用分页控制数据量 + 现有窗口化渲染）。
- 远程合并列表的分页（远程目录本身就是全量快照，保持客户端合并）。

## 3. 归档语义

### 3.1 状态与单位

| 状态 | 语义 | 默认列表 | 历史 / Session | 可逆 |
|---|---|---|---|---|
| active | 正常 | 显示 | 完整 | — |
| archived | 用户整理收起（归档即隐藏，不引入单独的"隐藏"状态） | 隐藏 | 完整保留，恢复后可继续对话 | 一键恢复 |
| deleted | 现有硬删除流程 | 消失 | 删除（停 run、删 JSONL、清目录） | 不可逆 |

**归档单位是 fork 树**（顶层会话 + 它的全部派生会话），与删除的树级级联（`thread_delete_closure`）形状一致。一组会话要么都在、要么都收起来，不存在"父归档、子活跃"等混合状态——此前讨论的跨状态矩阵因此整体消失，无需任何提升显示或血缘标注逻辑。

归档与恢复都按树进行：恢复归档组中的任一条即整组回到默认列表。血缘无损（`parent_session_id` 从不被归档触碰），活跃区的树从当前可见集合实时重建。

### 3.2 统一前置规则：范围内无活跃会话

**任何归档都要求目标范围内没有活跃会话，否则拒绝并点名。**

| 操作 | 前置条件 | 不满足时 |
|---|---|---|
| 归档单组会话（fork 树） | 组内全部不活跃 | 拒绝，点名活跃的成员 |
| 归档工作区 | 组内所有会话组全部不活跃 | 拒绝，点名活跃的会话（列出前几个 + 总数） |
| 批量归档（选择模式） | 每组独立判定 | 不活跃的归档，活跃的跳过并逐组汇报 |

- **活跃判定**取并集：store 的 `runs` 表非终态行（`running` / `queued` / `waiting_approval`）∪ agent 的 streaming 快照（`list_streaming_sessions`，纯内存扫描，覆盖其他客户端在推动会话的情况）。`waiting_approval` 必须算活跃：审批等待中的会话被归档后 badge 消失，run 会无声卡住。
- **stale run 行先收敛**：崩溃残留的非终态 run 行不能挡住归档。活跃性核验先走现有收敛路径（`settle_interrupted_run_from_agent`），收敛后再判定。
- **拒绝文案可操作**："「XXX」正在运行 / 等待审批，结束或停止后再归档"。
- **不提供"停止并归档"按钮**：停止 run 应发生在会话界面（用户能看到它正在做什么），归档操作保持纯粹——只改可见性标记，从不触碰运行。
- **"先归不活跃的"由批量选择覆盖**：想先收掉不活跃组的用户走选择模式的批量归档（跳过活跃项并汇报），工作区归档弹窗保持"拒绝 + 说明"一个出口。

### 3.3 副作用与不变式

- **归档自动取消置顶**（含组内所有成员）；恢复时不还原 pin。归档列表按 `archived_at` 倒序。
- 归档不停止 run、不改 `parent_session_id`、不动 agent session 与历史。
- **工作区归档 = 容器标记 + 查询时 JOIN 隐藏**，不级联改写会话的 `status`。恢复工作区时，此前单独归档过的会话仍保持归档——状态不丢。
- **不变式**：GUI 操作不会产生"活跃会话存在于已归档范围内"。
- **容器自动唤醒**：自动路径（TUI/CLI 按 cwd 落新会话，`reconcile_thread_workspace` / 导入）把新会话挂进已归档工作区时，工作区自动恢复。这是唯一合法的"复活"——新工作落进目录说明用户又在用它了；不做的话会出现"正在运行但被 JOIN 藏掉"的会话。
- **跨客户端活动不复活已归档会话**：其他端在已归档会话里继续跑时，归档视图显示未读/运行指示，但会话保持归档（尊重整理意图）。
- **审批永远可操作**：即使会话已归档，来自其他端的待审批请求仍可见、可决策——安全入口永不隐藏。
- **不做"跑完自动归档"**：不活跃不是单向事件（跑完→被归档→其他端继续→又在跑），且与恢复存在竞态。若未来确有需求，形态是归档弹窗里显式勾选"结束后自动归档"（持久化 pending 标记 + run 完成 hook），而非默认行为。

## 4. 分页加载设计

### 4.1 范围：三类列表都分页

1. **工作区列表**（`kind='user'`；`temporary` 工作区是聊天会话的容器，不作为独立列表项）。
2. **工作区内的对话**（`mode='workspace'`，按 workspace 分组，每组一个分页流）。
3. **临时对话**（`mode='chat'`，聊天区，一个分页流）。

此外 Recent 视图的 earlier 段与归档视图同样走分页（见 4.4）。

### 4.2 keyset 游标，不用 offset

列表会因 `last_message_at` 推送不断重排，offset 分页会重复/漏行；仓库已有游标先例（`get_session_entries_page`）。

- 排序键：`(pinned DESC, activity_ts DESC, id DESC)`，`activity_ts = COALESCE(last_message_at, updated_at, created_at)`——与现有 `list_threads` 排序一致，UI 无感知切换。
- **pinned 行单独一次小查询全量返回**（pin 是手动选的快捷方式，数量天然小），分页只作用于非 pinned 部分，正好匹配现有 UI 的全局 pinned 分区。
- 游标编码 `(activity_ts, id)`，服务端用行值比较 `WHERE pinned = 0 AND (activity_ts, id) < (?, ?)`。游标不透明、无需栅栏（keyset 天然容忍并发重排）。

### 4.3 按根分页 + 页内补后代

按"行"分页有个隐蔽的坑：fork 子的 `activity_ts` 通常比父新，翻页时可能取到子而父还在后面的页里，树就画错了。因此：

1. 游标分页只作用于**根会话**（`parent_session_id` 为空，或父不在当前 scope/状态集合内的行）；
2. 取到一页根之后，用一次小查询把这批根的**全部活跃后代**捞出（按 `parent_session_id` → agent session id 的 IN 映射）。

**一页 = 若干棵完整的树**，与 rail 键盘导航已有的 block 概念（一个 block = 一棵 fork 树）一致。页大小按根计数（默认 50 个根），实际行数略多（各根带上后代）但有界。树级归档保证"可见根的后代必然全部可见"，补后代查询无需再做状态过滤。

### 4.4 各视图的加载行为

- **树视图**：折叠的工作区折叠时**不加载其会话**（从视觉折叠变成真懒加载），展开时取第一页；每个工作区组和聊天区底部有"加载更多"。
- **Recent 视图**：active 段来自 `list_active_threads`（全量，数量小）；earlier 段走 `recent` scope 的全局游标分页，替换现在的客户端 `recentLimit` 窗口，`RECENT_PAGE_SIZE = 30` 保留为页大小。
- **归档视图**：平铺、按 `archived_at DESC`、每页 50 组。
- **工作区列表**：与对话同一套游标机制（`archived_at IS NULL` 默认过滤），pinned 工作区同样单独全量。

### 4.5 事件驱动替代全量刷新（分页的前置条件）

分页后 `refreshStore` 全量重载必须先拆掉，否则翻页状态会被每次 mutation 冲掉。

- 新增 **`store-changed` 推送**：携带行级补丁 `{kind: thread|workspace, op: created|updated|archived|restored|deleted, record}`。所有 store mutation 命令提交后发布（命令已返回记录，补丁与返回值同源），走现有 `crate::events::Event` + Tauri 事件桥（`thread-activity-updated` 是先例）。
- 前端 reducer 按排序键插入/移动/移除行；落在已加载窗口外的变更不影响窗口（翻页时自然取到）。
- mutation 命令的返回值直接 patch + 推送补丁 + 30s 轮询收敛（现有模式保留）。
- 未读数、待审批数（`usePendingApprovalCounts`）保持独立的全量小查询，不随列表分页。

### 4.6 服务端搜索

分页后客户端搜索必然漏结果，改为服务端搜索：

```
search_threads(query) -> { active: [...], archived: [...] }
```

语义与现有一致（标题或工作区名的子串匹配，`LIKE` 转义），结果分区显示——归档会话因此第一次可被找到。点已归档结果直接打开只读详情，不必先切视图。远程合并列表的搜索保持现有客户端过滤（远程目录是全量快照）。

## 5. 数据模型变更

遵循 `host/src/store/schema.rs` 的迁移规则（`SCHEMA` 常量 + 不可变的 `VERSIONED_MIGRATIONS` 条目）：

```sql
-- workspaces 新增列（VERSIONED_MIGRATIONS 新条目 + SCHEMA + ADDED_COLUMNS 同步）
ALTER TABLE workspaces ADD COLUMN archived_at INTEGER;

-- 分页游标索引（表达式索引）
CREATE INDEX idx_threads_page ON threads(
  status, workspace_id, pinned,
  COALESCE(last_message_at, updated_at, created_at), id
);
CREATE INDEX idx_workspaces_page ON workspaces(
  archived_at, pinned, COALESCE(last_opened_at, updated_at), id
);
```

`threads` 无需结构变更（`status`/`archived_at` 已有）。现有 `idx_threads_recent` 与新索引有重叠，迁移时评估保留其一。

`workspaces.last_opened_at` 的**唯一写入点**是 `store::mark_thread_opened`：打开某个会话即把它所属用户工作区记为"最后使用"（`temporary` 聊天容器不算列表项，不写）。没有这个写入点，该键恒等于工作区创建时间，游标就退化成"按创建天数排序"——常用的老目录必然沉到第一页之后。

## 6. 后端接口（host 层）

```rust
// 分页查询
list_threads_page({ scope, cursor, limit }) -> { threads, next_cursor }
// scope: chat | workspace(id) | recent | archived；limit clamp 1..=100，默认 50（根）
list_workspaces_page({ scope: active | archived, cursor, limit }) -> { workspaces, next_cursor }

// 归档（树级；现有 archive_thread 单行语义升级为树级）
archive_thread_tree(thread_id) -> ThreadRecord        // 活跃核验 + 闭包归档
restore_thread_tree(thread_id) -> ThreadRecord
batch_archive_threads(ids) -> { archived_count, skipped: [{id, reason}] }
archive_workspace(workspace_id) / restore_workspace(workspace_id)  // 仅 kind='user'

// 辅助
list_active_threads() -> Vec<ThreadRecord>   // 非终态 run + streaming，供 Recent active 段与 badge
search_threads(query) -> { active, archived }

// 事件
store-changed: { kind: thread|workspace, op: created|updated|archived|restored|deleted, record }
```

归档闭包复用删除闭包的遍历（`thread_delete_closure` 按真实 `parent_session_id` 血缘走，不受 UI 三层拍平影响，已有环保护），过滤 `status != 'deleted'`。

## 7. 前端设计

### 7.1 store 改造（`useThreadStore`）

- bootstrap：并发取 pinned 全量 + chat scope 首页 + workspaces 首页 + `list_active_threads`；其余 scope 按需取。
- 每个 scope 维护 `{rows, nextCursor, loading}`；翻页追加。
- `refreshStore` 退役，改为 mutation 返回值 patch + `store-changed` 补丁 + 轮询收敛。
- active 槽容忍"当前打开的是不在已加载窗口内的归档会话"（`get_thread` 按需取）；重选逻辑把"正在查看归档详情"认作合法状态，不把用户踢回活跃列表。

### 7.2 归档入口：切换器同行的小图标按钮

归档不是切换器的第三段，而是 `RailViewSwitch` **同一行右侧**的独立小图标按钮，视觉低半档：

```
[ 树 | 最近 ]          [🗃]
 └─ 主视图切换器        └─ 归档入口（icon-only）
```

- `Archive` 图标 size-3.5、按钮约 size-6、`text-ink-muted`，hover 才显现底色——平时是安静的灰，`ml-auto` 顶到行尾与切换器拉开间距。
- 不带数字徽标；数量放 tooltip（"已归档 · 42 组"）。
- 归档视图激活时按钮呈 active 态（`bg-surface-subtle text-ink`），切换器两段都不高亮。
- 视图状态仍归 `useRailView`（`view: "tree" | "recent" | "archived"`），持久化；键盘命令加 `rail.view.archived`。

### 7.3 归档视图

两个区，对应两种归档单位：

- **已归档工作区**：组标题行（名称、归档时间、会话数、恢复按钮），可展开浏览组内会话（这些会话自身状态仍是 active，只是被 JOIN 隐藏，同样可打开详情）。
- **已归档会话组**：按 fork 树平铺，`archived_at` 倒序。行显示标题、所属工作区、归档时间、组成员数（"共 3 条"）、归档期间的跨客户端活动指示（未读点 / 运行指示，不复活但要可见）。

行操作：恢复整组、删除整组、重命名。点击行 = 打开只读详情。

### 7.4 只读详情

点击归档行后，中间区像正常会话一样打开，但只读：

| 能力 | 归档态行为 |
|---|---|
| 消息历史 | 完整可读，`get_session_entries_page` 分页原样可用（读取路径不检查 `status`，零改动） |
| 输入框 | 禁用，替换为横幅："此会话已归档 · 恢复后可继续对话 **[恢复]**" |
| 置顶 / 模型切换 / 其他输入型操作 | 禁用 |
| 重命名 | 允许（归档也需要整理，走现有 rename 命令） |
| 审批 | 仍可操作（其他端发起的待审批请求不隐藏） |
| 跨客户端活动 | 显示运行/未读指示，不自动复活 |

横幅上的"恢复"形成闭环：找到 → 查看 → 确认 → 一键恢复 → 输入框解锁。离开归档视图时 active 回到之前的活跃会话，归档详情的打开是瞬时态，不持久化。

### 7.5 首次归档引导

对标现有 `SkillIntroBubble` + `skillAttention` 脉冲的模式：

1. 归档成功 → 被归档的行在列表里**收起淡出**（150–200ms）；
2. 归档图标按钮**脉冲 2–3 下**（约 1.5s），按钮旁弹出指向气泡：

   > **已归档的会话收在这里**
   > 随时可查看详情或恢复。
   > [查看已归档]　[知道了]

3. 气泡 8 秒自动消失或点"知道了"关闭；`archiveGuideSeen` 写入 app settings（同 `skillIntroDismissed` 的 `changeSettings` 路径），只出现这一次；
4. 点"查看已归档"→ 切到归档视图，刚归档的组滚动到可见并短暂高亮。

边界：rail 收起态（icon-only）退化为轻量 toast"已归档，可在归档区查看 [查看]"，同样写 seen；首次归档是工作区或批量归档时用同一套通用文案；之后的归档不再出气泡和脉冲，但**行退出动画保留**（它是每次归档的基础反馈）；`prefers-reduced-motion` 关闭脉冲与滑出动画，气泡照出。

### 7.6 菜单与批量

- 会话行菜单加"归档"：单节点树直接归档；多节点树弹轻量确认"将同时归档整组共 N 条会话"（子节点上触发时文案说明包含顶层会话）。活跃核验在后端做，菜单项不做客户端预禁用；拒绝时返回具体成员名单。
- 工作区头部菜单加"归档工作区"（仅 `kind='user'`）。
- 选择模式工具条加"归档所选"：选中行先归并成树（选中一个成员 = 选中整组，去重），逐组核验，合格的整组归档，不合格的按组汇报跳过原因。

## 8. 其他端影响

| 端 | 一期 | 二期（按需） |
|---|---|---|
| 远程目录（host publisher） | session payload 透传 `archived` 标志（从 app.db 读，无需改 agent） | 移动端默认过滤归档 + 查看入口 |
| 移动端 | 不动 | 同上 |
| TUI / CLI | 不动（`list_sessions` 全量可见，符合高级工具定位） | `list_sessions` 加 `cursor/limit`；TUI `/resume` 分页 |
| Agent | 不动 | 同上，纯性能优化（`summary_rows` 已是索引查询，千级可承受） |

## 9. 分期实施

| 期 | 范围 | 验收 |
|---|---|---|
| **P1 归档闭环** | 树级归档 + 工作区归档（schema 迁移）+ 归档视图 + 只读详情 + 首次引导 | 归档→消失→搜索到→查看详情→恢复→历史完整可继续对话；迁移在旧库幂等 |
| **P2 分页加载** | `store-changed` 事件 + store 改造 + 三类列表游标分页 + 折叠懒加载 + Recent 改造 | 1000 会话下启动只取 pinned + 首页；mutation 不再触发全量重载 |
| **P3 搜索与透传** | 服务端搜索（含归档分区）+ 远程目录 archived 透传 | 未加载的行可被搜到 |
| **P4 按需** | agent `list_sessions` 分页、TUI/移动端、渲染虚拟化、自动归档 opt-in | — |

P1 独立可交付（归档不依赖分页）；P2 是性能主体；P3 依赖 P2 的 store 形态。

## 10. 边界情况汇总

| 情况 | 处理 |
|---|---|
| 崩溃残留的 stale run 行挡住归档 | 活跃核验前先走 `settle_interrupted_run_from_agent` 收敛 |
| fork 血缘成环 | 复用 `thread_delete_closure` 的环保护；`buildTree` 另有断边兜底 |
| fork 跨工作区 | 闭包遍历天然覆盖；确认对话框数量如实包含 |
| 其他端在已归档会话里继续跑 | 不复活；归档视图显示活动指示；审批仍可操作 |
| TUI/CLI 在已归档工作区目录开新会话 | 工作区自动恢复（容器唤醒） |
| rail 收起态下首次归档 | 引导退化为 toast，不写布局变更 |
| `prefers-reduced-motion` | 关动画，气泡照出 |
| 打开不在已加载窗口内的归档详情 | `get_thread` 按需取；active 槽与重选逻辑容忍 |
| 归档视图里删除 | 与活跃区删除一致：树级级联 + 确认 + stop-first |

## 11. 开放问题

1. **从归档组中拆出单条恢复**：一期不做（恢复也是树级）。若确有需求，加"从归档组中拆出此会话"的例外操作——现有树代码能正确画出混合状态，技术上没有障碍。
2. **跑完自动归档**：仅以显式 opt-in 形式考虑（见 3.3）。
3. **TUI/CLI 是否尊重归档**：一期不尊重（归档是 GUI 组织层概念，不下沉进 agent session，避免双向同步）。
4. **老会话的"建议归档"列表**：后续可接 `managed_cleanup` 的调度机制，做 N 天不活跃的归档建议，而非自动归档。
