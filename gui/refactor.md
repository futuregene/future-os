# FutureOS GUI 重构计划

本文件是重构的活动清单(逐项勾选)。来源:2026-06 的四维度代码审查(模块结构 / UI 设计系统 / 命名 / Rust 后端)。

原则:**分批小改、每批独立可验证**(`tsc` + `eslint` + `cargo check/clippy/test`),不破坏现有功能。未发布,允许直接改结构、删死代码。

图例:`[ ]` 待办 · `[~]` 进行中 · `[x]` 完成

---

## Batch 1 — UI 基础(token + cn）✅ 完成

- [x] `tailwind.config.js` 增加语义色 token:`success/danger/warning/info` 各 `{,-soft,-line}`、`accent-hover`、`accent-disabled`、`focus`、`ink-strong`、`diff-add`/`diff-remove`(+line)
- [x] 增加 `boxShadow.dialog`(替换内联 `shadow-[0_24px_60px_rgba(15,23,42,0.18)]`)
- [x] `lib/cn.ts` 接入 `tailwind-merge`:`cn = (...) => twMerge(parts.join(" "))`

## Batch 2 — UI primitives 抽取与统一(进行中)

- [x] 新增 `components/ui/Switch.tsx`(从 settings 迁出,retint 到 token),`SettingsPrimitives` 改为 re-export
- [x] 新增 `components/ui/TextInput.tsx` + `components/ui/Select.tsx`(烘入统一 focus ring);已替换 CustomProviderDialog/ModelsPage 的裸 input/select
- [x] 新增 `components/ui/Field.tsx`;CustomProviderDialog 改用
- [x] `components/ui/Button.tsx` 增加 `size`(sm/md)与 `danger-soft` 变体,retint 到 token;已替换 settings(CustomProviderDialog/ProvidersPage)的裸按钮
- [x] `Badge` retint 到 token + 增 `info` tone;ProvidersPage 状态徽章改用 `<Badge>`
- [x] `shadow-dialog` token,替换 Dialog/SettingsDialog 内联阴影
- [ ] **剩余**:替换 ReviewPanel/RunInspectPanel/NewConversation/Composer/ApprovalPrompt 等的裸 input/button/raw color(大面板,需逐个视觉确认)
- [ ] **剩余**:status→颜色统一走 `<Badge tone>`:合并 `reviewChangesetStatusClass`/`eventCategoryClass`/`formatErrorType.color`/ThreadHeader 徽章为同一 tone 词汇
- [ ] **剩余**:抽 `components/ui/DiffView.tsx`(ReviewPanel diff 渲染器)、`components/ui/CopyablePre.tsx`(RunInspectPanel)
- [x] 抽 `components/ui/Overlay.tsx`,Dialog/SettingsDialog 共用(去掉两份遮罩+Esc fork);WorkspaceModal 是面板内 absolute 模态,刻意保留

## Batch 3 — `threadStore` 拆分与改名 + invoke 边界统一(进行中)

- [x] providers 命令 + 接口移到 `integrations/agent/providers.ts`(修正"providers 放在 storage"的错位)
- [x] appSettings 命令 + `AppSettings` 接口移到 `integrations/storage/appSettings.ts`
- [x] 把 `threadStore.ts` 按域拆成 `app/files/threads/runs/review/artifacts.ts`;`threadStore.ts` 改为 barrel re-export(call site 零改动)
- [x] 统一 typed `invoke` 包装层 `integrations/tauri/invoke.ts`(集中错误归一化 `normalizeInvokeError`);`markdownReferences` 并入,所有 call site 迁移
- [x] 统一 Tauri 命令参数形状约定(结构化→`{input}`,单标量→具名键:`abort_run`/`get_or_create_chat_workspace`/`save_pasted_image`)
- [ ] **剩余(可选)**:把 barrel `threadStore.ts` 重命名/让 call site 直接 import 域模块 —— 评估为低收益高 churn(barrel 本身无问题),暂不做

## Batch 4 — `AppShell` 拆分

- [ ] 抽 `useThreadStore`(线程/工作区数据 + rename/delete/pin/restore/model mutation + bootstrap)
- [ ] 抽 `useAgentConnection`(modelOptions + 连接轮询 + `classifyAgentConnectionError`)
- [ ] 抽 `useApprovals`(pendingApprovals + activeApproval + 1.5s 轮询 + 自动审批引擎)
- [ ] 抽 `lib/useAsyncResource`(替换 9 处 cancelled-flag effect)与 `lib/usePolling`(替换 3+ 处轮询)
- [ ] AppShell 收敛为布局编排

## Batch 5 — 模块归位与命名(前端)

- [x] `components/layout/context-panel/` → `features/{runs,review,artifacts}/`(RunsPanel/RunInspectPanel/runDisplayFormatters→runs、ReviewPanel→review、Artifacts*/PdfPreview→artifacts);`contextPanelFormatters` → 中性 `runDisplayFormatters`;删 `ContextEmptyState` shim(直接用 `ui/EmptyState`);`ReviewBase` 类型反转下放到 `ReviewPanel`(layout 反向依赖 feature);`ContextPanel` 留 layout 仅作容器
- [x] 删除 `features/agent/MarkdownContent.tsx` shim,MessageBlock 改直接 import
- [x] `formatRunStatus` 二义性:`agentThreadUtils` 里那份是**死代码**,直接删除(clash 消除)
- [x] 重命名:`useAgentThreadController`→`useAgentThreadState`、`agentThreadUtils`→`agentMessageFormatters`、`referencePromptContext`→`buildReferencePrompt`、`features/agent/types`→`agentThreadTypes`
- [x] `integrations/agent`:合并 `futureAgentClient`+`models` → `agentClient.ts`,删死导出
- [ ] 引入 typed event bus(替换 window CustomEvent)

## Batch 6 — Rust 后端拆分

- [x] 抽 `fs_commands.rs`(open_path/read_text_file_preview/export_artifact_file/save_pasted_image + helpers)出 lib.rs(594→444 行)
- [x] 其余命令按域拆到 `commands/*.rs`(app/providers/settings/workspaces/threads/messages/runs/approvals/review/artifacts/references/agent + fs_commands→commands/files);`commands/mod.rs` flat re-export 供 `generate_handler!` 用裸名;`decide_approval`/`abort_run` 编排(+ is_stale_approval_error/is_agent_unavailable_error)下沉到 agent_bridge。lib.rs 446→80 行(仅 run() + 模块接线)
- [x] 从 `agent_bridge.rs` 抽出 `run_error.rs`(classify_run_error + 全部相关测试,1165→1009 行)
- [x] `agent_bridge.rs` → 目录;抽出 `agent_bridge/client.rs`(RpcCommand 构造 + endpoint + 图片编码 + command_id,1009→872)
- [x] `agent_bridge/mod.rs` 再拆 `stream.rs`(collect_agent_response + event_text/event_error)/`persist.rs`(全部 persist_* 投影 + event_value/value_string/compact_json + review_shape_for_tool);mod.rs 收敛为连接/会话编排(872→395 行)。顺手删了误挂在 notify_agent_approval_decision 上的 classify_run_error 陈旧文档
- [x] `store.rs` 根的 threads/workspaces/messages/runs 抽到独立模块(638→61 行,facade only;对齐已有按域拆分)
- [x] `store/support.rs` 拆 `db.rs`(连接基础设施 app_dir/db_path/connect/apply_schema + 杂项跨域查询 get_run/get_approval_request/get_message/...)/`util.rs`(纯工具 create_id/now_millis/expand_tilde/count_workspace_files/...);13 个 `*_from_row` 移入 `records.rs`(与 record 同文件);删 support.rs,各模块 `use super::support::*` → `super::db::*` + 由 `super::records::*` 覆盖 from_row
- [x] `store/models.rs` → `records.rs`(消除与 LLM "models" 撞名)
- [x] `store/review.rs` → `approvals.rs`(approval_requests + review_changesets 同域)
- [x] `store/markdown_refs.rs` → 目录;抽出 `markdown_refs/extract.rs`(纯解析 `futureos://` link/fence + percent_decode + 对应测试,1224→1035)
- [x] `markdown_refs/mod.rs` 再拆 `resolve.rs`(读侧:references→records + get_*_in_workspace)/`search.rs`(@-mention 搜索 + compact_search_text/reference_matches)/`sync.rs`(denormalized 表同步 + metadata/upsert);`short_id` 留 mod.rs 共享,测试留 mod.rs(import 两个被测内部 fn)。1035→183 行 facade(extract 201 / resolve 235 / search 333 / sync 328)

## Batch 7 — 跨切面清理(Rust)

- [x] 去掉每次 store 调用的 `initialize_app_store()`(53 处:启动时 `setup` 已初始化一次,函数体内不再重跑建表);各模块失效的 `use ...initialize_app_store` 一并清理
  - 注:**未**引入全局共享连接/`with_conn` 包裹 —— 评估后认为当前「每调用一个轻量 `connect()`」语义无需改变,把 50+ 函数体重写成闭包是高 churn 低收益,故有意不做(如需池化可后续单独立项)
- [x] 引入 `AppError`(`thiserror`,`src/error.rs`):`From<rusqlite/io/serde_json/String/&str>` 让 `?` 自动转换,**序列化为 string**(命令边界 `Result<T, AppError>` → 前端仍是 `string`,前端 0 改动)。全 crate `Result<_, String>` → `Result<_, AppError>`,`.map_err(|e| e.to_string())` 由 174 → 0。保留的 `.map_err`:`|_| "..."`(lock poison 等无信息错误)与 `|error| format!("...{error}")`(远端 gRPC 错误带上下文)
  - 子串反解:本地错误已结构化;`run_error::classify_run_error` 和 agent_bridge 的 `is_stale_approval_error`/`is_agent_unavailable_error` 反解的是**远端 agent 经 gRPC 返回的错误文本**,本质是字符串,有意保留(调用点 `&error.to_string()` 喂入)
- [x] 合并重复 helper:`artifact_type_from_path`(→ `store::artifact_type_from_path`)、`canonical_or_raw`(→ `git_review::canonical_or_raw`)
- [ ] ~~删除死模块 `store/approval_config.rs`~~ —— 经判断是 P2 审批模型的**有意保留脚手架**(schema 表已就位),不删,留待后续接入

---

## 验证命令

```bash
# 前端
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run && npm run build
# 后端
cd gui/src-tauri && cargo fmt --check && cargo clippy && cargo test
```

## 备注
- 每批改完跑上面验证;保持功能不变。
- 重命名穿插在所属模块的改动里做,减少二次 churn。
