# Shadow Review 开发计划

本计划把 [SHADOW_REVIEW_DESIGN.md](./SHADOW_REVIEW_DESIGN.md) 拆成可执行任务。任务编号 `P{phase}-{nn}`，每条标注**文件 / 依赖 / 验收**。设计章节用 `§` 引用。

> 总体 GUI 路线图见 [PLAN.md](./PLAN.md)；本文件只覆盖 Shadow Review 子系统，对应 PLAN.md 的 P3 Review Workflow 深化。

## 实现状态（冒烟通过）

Phase 1 已实现并冒烟通过：发一轮 workspace prompt → before/after 影子快照 → 固化 diff → Review 面板展示真实「上一轮变更」，Git 与非 Git Workspace 均可用。后端 26 测试 + 8 shadow 测试全绿、clippy 零警告；前端 tsc/eslint/vitest 全绿。含 `shadow_review_lifecycle_smoke` 端到端测试与 DB 迁移回归测试。

落地的产品决策（覆盖原设计 §3）：

- **diff 只做 unified 模式**；split 双栏暂不做。
- **去掉 viewed 状态**（按钮、计数、持久化全部移除）。
- **去掉 Git changes 的文件搜索框**与「Current base」「untracked」行，精简垂直空间。
- 文件**默认收起**，顶部一个「全部展开 / 全部收起」切换按钮（Git changes 与上一轮变更两视图一致）。

**Phase 2 可靠性已完成**（后端 26 测试含端到端断言全绿）：

- **B1** §6.2：异常返回时轮询 Agent `isStreaming` 确认停笔后再拍 after（`wait_for_agent_idle`）。
- **B2** §6.6：启动时后台线程恢复被中断的 Run（有 before 无 after/无 changeset）→ 标 `cancelled` + 生成 `confidence=recovered` 的 changeset。
- **B3** §12.3：每 Thread 保留最近 10 个 changeset,finalize 后自动 prune（删 DB 行 + shadow refs）+ `git gc --auto`。
- **B4** §13：敏感文件（`.env`/`*.pem`/`*.key`/`id_rsa`…）变化作为 metadata-only 行展示「敏感文件发生变化，内容未保存」,不存内容。
- **B6** §8.4：启动一致性检查——快照 commit 丢失则标 snapshot `failed`（派生 `unavailable`）。
- **B5**：rename/copy 检测已随 `--find-renames --find-copies` 默认开启。
- 配套:A3 二进制 size/MIME、A4 omitted 计数也已固化并测试。

**Phase 3 性能 + 清理已完成的部分**：

- **C1** §6.1：finalize 拆成「同步 capture after（guard 内）+ 异步 materialize（spawn_blocking，只读 diff）」,去掉 IPC 阻塞。后端经 `APP_HANDLE.emit("review-updated")` → AppShell `listen` 桥接到 typed event bus → Review 面板自动刷新（替代了前端乐观 emit）。
- **C3** §11：`getGitReview` 整树 diff 只在 Review tab 激活时跑;tab/能力判断改用更便宜的 capabilities。
- **E1**：删除旧 apply/discard 流的死代码（`listReviewChangesets`/`updateReviewChangesetStatus`/`listReviewFileChanges` 前端封装 + Tauri 命令 + store fn + `UpdateReviewChangesetStatusInput`）。`StoredReviewChangeset` 类型保留（markdown embed 仍用）。

仍延后：

- **C2** fingerprint cache —— 持久化 shadow index 已实现增量(stat 缓存跳过未改文件),等价覆盖,不再单独做。
- **C4** 大文件 diff 按需加载 —— 已有 2 MiB/10000 行截断兜底,边际收益,暂不做。
- **E2** markdown `futureos://review/<id>` 只打开 Review tab、不定位具体 changeset —— 新模型只展示「上一轮变更」,无可选 changeset,记为已知行为。
- **monorepo 子目录 pathspec**（P1-11 §14.5）——子目录 Workspace 当前判为非 Git、走「上一轮变更」，不显示原生 Git changes。
- DiffView split 双栏（产品已决定暂不做）。

## 0. 约定

**验证（每个任务完成必跑）**

```bash
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run
# 涉及 Tauri 后端：
cd gui/src-tauri && cargo fmt --check && cargo clippy && cargo test
```

- 提交用 `git commit --no-gpg-sign`；视觉改动需在 `make run-gui` 实机确认。
- schema 变更同步改 `gui/ER.md`（CLAUDE.md 要求）。
- 颜色只用 `COLOR.md` token；Tauri 调用走 `integrations/tauri/invoke.ts`；跨组件事件走 `lib/futureEvents.ts`。

**新增后端模块（§9）**

```
gui/src-tauri/src/shadow_review.rs
gui/src-tauri/src/shadow_review/{repository,snapshot,diff,policy}.rs
```

**适用范围（§14.6）**：第一阶段只对 `thread.mode = workspace` 启用；普通 Chat 继续 Artifacts，不接入。

---

## Phase 1：核心闭环

目标：Run 前后真实快照 → 立即固化 diff → 「上一轮变更」可看（Git / 非 Git 两档），移除旧推测逻辑与 auto git-init。

### 工作流 A — 数据模型

#### P1-01　Schema 迁移
- **目标**：新增 `review_snapshots` 表（§8.1）；`review_changesets` 扩展列（§8.2，含 `source_kind/workspace_id/before_snapshot_id/after_snapshot_id/binary_files/omitted_files/completeness/confidence/overlapped/error_message`）；`review_file_changes` 扩展列（§8.3，含 `previous_path/binary/before_size/after_size/mime/diff_truncated/omission_reason`）。
- **文件**：`src-tauri/src/store/schema.rs`、`gui/ER.md`
- **依赖**：无
- **验收**：建表/加列幂等（顺序 migration）；`cargo test` 过；ER.md 同步。

#### P1-02　Store CRUD
- **目标**：snapshot 读写（按 `run_id+phase` UNIQUE）、changeset 读写（含 `getLastRunReview` 所需的「最新结束 Run」查询）、file_changes 批量写；记录结构补字段。
- **文件**：新增 `src-tauri/src/store/review_snapshots.rs`；扩展 `store/approvals.rs`（或新 `store/review.rs`）、`store/records.rs`、`store/mod.rs`
- **依赖**：P1-01
- **验收**：单测覆盖「最新结束 Run」选择、零变化 changeset、overlapped 回标。

### 工作流 B — 影子仓核心（Rust，可与 A 并行）

#### P1-03　repository.rs
- **目标**：shadow repo 路径（`~/.future/app/review/<workspace-id>/`，§5.1）、`git init` + **config 写入**（`autocrlf=false`/`symlinks`/`fsmonitor=false`/`untrackedCache`/`manyFiles`/`index.version=4`/`index.threads`，§5.2）、workspace 锁（§12.1）、ref 管理、持久化 shadow index 读写、Git Workspace 的 `alternates` 共享 + 真实仓 index 种子（§5.2）。`null_device_path()` 封装（§5.5）。
- **文件**：`src-tauri/src/shadow_review/repository.rs`
- **依赖**：无
- **验收**：init 后 config 正确；alternates 文件写对；锁串行化；**不触碰真实仓** index/refs/objects（单测断言）。

#### P1-04　policy.rs
- **目标**：ignore（尊重 `.gitignore`/`.ignore`、内置默认排除、`core.excludesFile=null`、§5.5 三个边界：同步真实仓 `info/exclude`/drop 新 ignore/超大 untracked 排除）；**候选集口径**的大小限制（§5.5，约束本轮候选集非整树）；敏感路径（§13 基础排除）；非 Git **体积红线**评估（20k/512MiB，早退计数，§6.7）；两档可靠性策略开关。
- **文件**：`src-tauri/src/shadow_review/policy.rs`
- **依赖**：P1-03
- **验收**：大 monorepo 少量改动不触发 partial；非 Git 超红线返回 `unsupported_too_large`；敏感/ignored 计数正确。

#### P1-05　snapshot.rs
- **目标**：`capture_snapshot(phase)`（§5.4）——copy 持久化 index → 候选集（`diff-files`+`ls-files`）→ policy 过滤 → 只 stage 候选集 → `write-tree` → tree 相同复用 commit 否则 `commit-tree` → 更新 ref → 写 snapshot metadata（P1-02）→ 原子换回 index。两档（git seed / 非 git 自有）。
- **文件**：`src-tauri/src/shadow_review/snapshot.rs`
- **依赖**：P1-03、P1-04、P1-02
- **验收**：新增/修改/删除/rename/symlink 都进 tree；未改文件不被哈希（计数/耗时断言）；首轮全量、次轮增量。

#### P1-06　diff.rs（固化）
- **目标**：`materialize_diff(before,after)`（§7.1）——**一次** `git diff --unified` + **一次** `--numstat`，内存切分按文件写 `review_file_changes`（diff 截断置 `diff_truncated`、二进制只记 metadata）、聚合 changeset 总计（含 `binary_files`/`omitted_files`）。`retry` 从 commit 重算（§10.4）。
- **文件**：`src-tauri/src/shadow_review/diff.rs`
- **依赖**：P1-05、P1-02
- **验收**：500 文件 Run 只 fork 两次 git；二进制无 diff 有 size/mime；rename 带 `previous_path`。

### 工作流 C — 生命周期集成（agent_bridge）

#### P1-07　Guard 覆盖 finalization
- **目标**：`PromptSessionGuard` 上移到外层 `agent_prompt`，覆盖 `inner` + after snapshot（§6.1）。
- **文件**：`src-tauri/src/agent_bridge/mod.rs`（现 guard 在 `agent_prompt_inner` 第 117 行）
- **依赖**：无（与 P1-08 协同）
- **验收**：after 落盘前下一轮 prompt 不会进入同 session。

#### P1-08　before / after hook
- **目标**：before 在 `agent_prompt_inner` 的 `prompt_command` 前（§6.1，参考 mod.rs:177）；after 在外层 `agent_prompt` 于 `inner().await` 返回后（success/error 同路径）调用 snapshot+materialize，再做失败状态投影、再返回前端。非 Git 走简化（拿不到 terminal event 即 `unavailable`，§6.7）。
- **文件**：`agent_bridge/mod.rs`（`agent_prompt`/`agent_prompt_inner`，line 77–198）
- **依赖**：P1-05、P1-06、P1-07
- **验收**：completed/failed/cancelled 都生成 changeset；`waiting_approval` 不提前 after（§6.4）；真实 Git 状态不变。

#### P1-09　Overlap 检测
- **目标**：after finalization 时按 `review_snapshots` 时间窗相交判定，set `overlapped=1` 并回标对端（§12.5）。
- **文件**：`agent_bridge/mod.rs`、`store/review*`（P1-02）
- **依赖**：P1-02、P1-08
- **验收**：同 workspace 两 thread 并发 → 双方标 overlapped；串行不标。

#### P1-10　移除旧推测 + auto git-init
- **目标**：停用 `ensure_review_change` 的 tool-start 投影（§14.3/§14.4）；移除 `create_workspace`/`ensure_workspace_git` 的 auto `git init`（§14.3）；`git_review.rs` 改为只读检测。清空旧 `review_changesets`/`review_file_changes` 开发数据。
- **文件**：`agent_bridge/persist.rs`（`review_shape_for_tool`/`persist_tool_start`）、`commands/workspaces.rs`（第 28/47 行）、`git_review.rs`（`ensure_git_init`）
- **依赖**：无（前端切换 P1-15 前完成）
- **验收**：新建 Workspace 不再生成 `.git`；不再有 tool-start changeset。

### 工作流 D — Tauri API（依赖 B/C）

#### P1-11　Git changes + monorepo 子目录
- **目标**：保留/改名 `get_git_review`（§10.2）；支持 monorepo 子目录（`git_root`/`workspace_path`/`workspace_pathspec`，§14.5）。
- **文件**：`src-tauri/src/git_review.rs`、`commands/review.rs`、`lib.rs`
- **依赖**：P1-10
- **验收**：子目录 workspace 判为 Git，`Git changes` 用 pathspec 只显示子目录变化。

#### P1-12　Capabilities 命令
- **目标**：`getWorkspaceReviewCapabilities`（§10.1）——`isGitWorkspace`/`views`/`defaultView`/`changePreview`；非 Git 调 policy 红线（缓存，可手动刷新）。
- **文件**：`commands/review.rs`、`lib.rs`
- **依赖**：P1-04、P1-11
- **验收**：Git→`ready`+双视图；非 Git 小目录→`ready`+仅 last_run；非 Git 超大→`unsupported_too_large`。

#### P1-13　getLastRunReview / retryRunReview
- **目标**：`getLastRunReview`（返回 `RunReview`：changeset/files/run/snapshotStatus/confidence/overlapped，§10.3）；`retryRunReview`（§10.4）。
- **文件**：`commands/review.rs`、`store/review*`、`lib.rs`
- **依赖**：P1-02、P1-06
- **验收**：区分「无 Run / 无变化 / unavailable / incomplete」；retry 仅 commit 在时生效。

### 工作流 E — 前端（依赖 D）

#### P1-14　invoke 封装 + 类型
- **目标**：`integrations/storage/review.ts` 加 `getWorkspaceReviewCapabilities`/`getLastRunReview`/`retryRunReview`；`types.ts` 加 `WorkspaceReviewCapabilities`/`RunReview` 及 file-change 新字段。
- **文件**：`src/integrations/storage/review.ts`、`storage/types.ts`
- **依赖**：P1-12、P1-13
- **验收**：`tsc` 过；命令参数遵循 `{ input }`/具名键约定。

#### P1-15　ReviewPanel 数据源拆分
- **目标**：`ReviewPanelState`（capabilities/activeView/gitChanges/lastRunReview，§11）；进入先拉 capabilities；Git 默认 `git_changes` + 下拉；非 Git 强制 `last_run`；Run terminal 后连 capabilities 一起刷新（§11 规则 1–8）。前端「上一轮变更」改读 `getLastRunReview`，弃用旧 `listReviewChangesets`。
- **文件**：`src/features/review/ReviewPanel.tsx`
- **依赖**：P1-14
- **验收**：切 Thread/视图不串数据；非 Git 不出 Git 控件。

#### P1-16　DiffView 增强
- **目标**：新增 split diff、二进制文件专用行、全部展开/收起（§3.1/§3.3/§14.2）。
- **文件**：`src/components/ui/DiffView.tsx`、`features/review/` 子组件
- **依赖**：无（UI 独立，集成在 P1-15）
- **验收**：unified/split 切换；二进制显示 size/mime + 「不支持文本 diff」。

#### P1-17　状态横幅 + 目录过大 + viewed 持久化
- **目标**：§3.6 状态横幅（overlapped/recovered/partial/incomplete/unavailable）、§3.7「目录过大」态、viewed 至少跨面板重挂保留（§14.2）。
- **文件**：`features/review/ReviewPanel.tsx`、相关展示组件
- **依赖**：P1-15
- **验收**：各状态文案/行为符合 §3.6/§3.7；viewed 重挂不丢。

#### P1-18　ContextPanel 接线
- **目标**：tab 选择改为 (mode, isGit) 三态（chat→Artifacts；workspace+git→Review 双视图；workspace+非 git→Review last_run）；`last_run` 仅在 Run terminal/changeset 更新刷新，去掉 1.5s 全量重算（§11）。
- **文件**：`src/components/layout/ContextPanel.tsx`（现 1.5s 轮询）、`lib/futureEvents.ts`（必要时加事件）
- **依赖**：P1-12、P1-15
- **验收**：非 git workspace 显示 Review（而非 Artifacts）；last_run 不再 1.5s 轮询。

### Phase 1 完成标准（§15）
- 新增/修改/删除（含 `bash` 改的）文件均显示；failed/cancelled 仍显示已落盘变化。
- 用户真实 Git index/`.git`/refs/objects 不变；真实仓事后 gc/移动不影响已固化 diff。
- 大 monorepo 少量改动不被标 `partial`；非 Git 超大目录走「目录过大」。
- 同 workspace 并发 Run 双方标 `overlapped`。

### Phase 1 关键路径
```
P1-01 → P1-02 ─────────────────────────┐
P1-03 → P1-04 → P1-05 → P1-06 ──────────┤
                              P1-07 ────┤
                                        ├→ P1-08 → P1-09
P1-10（独立，尽早）                       │
P1-11 → P1-12 ───────────┐               │
P1-06 + P1-02 → P1-13 ───┴→ P1-14 → P1-15 → P1-17
P1-16（独立）──────────────────────┘    P1-15 → P1-18
```
并行建议：A 与 B 同步起步；C 等 B 的 snapshot/diff（P1-05/06）；E 等 D。P1-10/P1-16 可随时插入。

---

## Phase 2：Git Workspace 可靠性

> 仅 Git Workspace；非 Git 保持 §6.7 简化档。

- **P2-01 Terminal 信号 / isStreaming**：§6.2 异常连接处理；bridge 补读 `get_state.isStreaming`（agent 已暴露，`agent/src/rpc/mod.rs:209`）。文件：`agent_bridge/mod.rs`（`get_state` 解析 396–402）、`agent_bridge/client.rs`。
- **P2-02 partial / incomplete 细分**：§5.5 超限文件 metadata + `partial`；after 失败 `incomplete`。文件：`shadow_review/{snapshot,diff,policy}.rs`。
- **P2-03 重启恢复**：§6.6 `confidence=recovered`；`update_run_status` 仅作恢复兜底。文件：启动恢复逻辑 + `store/runs.rs`。
- **P2-04 Retention + GC**：§12.3 每 Thread 留 10、删旧 refs/投影、空闲 `git gc --prune`。文件：`shadow_review/repository.rs` + 定时任务。
- **P2-05 敏感文件策略**：§13 凭证文件只记 metadata 不存 blob/diff。文件：`shadow_review/policy.rs`。
- **P2-06 rename/copy 调优**：§7.1 阈值校准。
- **P2-07 一致性检查**：§8.4 启动校验 commit 存在、清理孤儿 refs、无效 changeset 标 unavailable。
- **P2-08 历史遗留 `.git`**：§14.4 提示用户处理 auto-init 残留（不自动清）。

---

## Phase 3：性能

- **P3-01 fingerprint cache**：§12.4 大 Workspace 持久化指纹增量扫描。
- **P3-02 monorepo 父级 `.gitignore` 合并**：§14.5。
- **P3-03 后台低优先级 snapshot**：降低 after 对 IPC 的阻塞（§6.1 解耦方案：guard 下异步 + `review:changeset_ready` 事件）。
- **P3-04 大文件 diff 按需加载**：突破固化时展示截断。
- **P3-05 Git changes 文件事件刷新 + 缓存**：§11 去掉 1.5s 全树 diff。

---

## 测试映射（§16）

| 用例 | 覆盖任务 |
| --- | --- |
| Shadow 不改真实 Git index/refs/objects | P1-03、P1-08 |
| bash 改的文件进上一轮变更 | P1-05 |
| 真实仓 gc/移动后 diff 仍在 | P1-06 |
| 大 monorepo 不误判 partial | P1-04 |
| 非 Git 超红线→目录过大 | P1-04、P1-12、P1-17 |
| 并发 Run 双标 overlapped / 串行不标 | P1-09 |
| completed/failed/cancelled 均生成 changeset | P1-08 |
| waiting_approval 不提前 after | P1-08 |
| 零变化 Run 空 changeset，不回退更早 Run | P1-02、P1-13 |
| 不同 Thread changeset 隔离 | P1-13、P1-15 |

---

## 待确认产品决策（§17，实现前定）

1. 「上一轮变更」严格指最新结束 Run（建议：是）。
2. 非 Git Workspace 是否保留 Artifacts 入口（建议：保留）。
3. ignored 文件是否进 snapshot（建议：尊重 `.gitignore`）。
4. 敏感文件是否存内容（建议：否）。
5. retention 数量（建议：10）。
6. Git Workspace 默认视图（建议：`Git changes`）。
7. 非 Git 体积红线阈值（建议：20k 文件 / 512 MiB）。
8. ✅ 已定：并发 Run 允许并发 + `overlapped` 标记，不强行串行。
