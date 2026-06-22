# FutureOS GUI Development Plan

更新时间：2026-06-18

本轮目标是先把 `P0 / P1 / P3 / P4 / P7` 做到主路径可用、能跑、不会阻塞手工测试。更细的完善项先放到后续，不再作为当前阶段完成条件。

## Current Baseline

GUI 当前已具备：

- `gui/` 独立作为 Tauri + React + TypeScript 模块运行。
- GUI 通过 `future-agent` gRPC 发送 prompt、接收事件、提交 approval 决策和 abort 请求。
- 模型列表从 agent 获取，不使用前端硬编码模型。
- Chat / Workspace thread 可创建、恢复、重命名、置顶、归档、删除。
- 普通 Chat 自动创建临时 workspace。
- 右侧面板按 workspace 类型切换：
  - Git workspace：Runs / Review。
  - 非 Git workspace 和普通 Chat：Runs / Artifacts。
- Runs 面板收敛为后台程序列表：
  - 只展示有 shell command 的后台程序。
  - 运行中蓝点，完成 / 失败 / 取消灰点。
  - 支持运行中终止、二次确认、终止失败提示、清空已完成程序。
  - 支持进入 Run Inspect。
- Approval 位于 composer 上方：
  - 同时只展示一个待审批项。
  - 不超时，始终等待用户允许或拒绝。
  - `Esc` 拒绝，`Cmd/Ctrl + Enter` 允许一次。
  - 输入框聚焦时不误触发审批快捷键。
  - GUI 启动时取消旧 pending approval。
- Agent run 主路径已具备基础防护：
  - thread 切换 / 组件卸载时清理流式轮询。
  - 在途请求不写错当前 UI 状态。
  - 同一 session 并发 prompt 会被拦截。
  - 失败 assistant message 提供 Retry / Continue。
  - Continue 会注入最近 run 的工具与关键事件摘要。
- Agent 连接状态已进入可用基线：
  - Header 展示 connected / checking / not running / model unavailable。
  - Composer 上方展示 agent 不可用提示和 Retry。
  - 提示 `make run-agent` 作为本地启动命令。
- Run Inspect 已进入可用基线：
  - 展示 run 状态、模型、时间、错误信息。
  - 展示 tool call、tool output、timeline。
  - tool input / output 支持复制，长 output 可展开。
  - timeline 支持分类过滤、摘要 / 原始 payload 切换、全文搜索。
  - tool 详情展示 command、cwd、path、exit status、duration、stdout、stderr 等可提取结构化字段。
- Review 已进入可用基线：
  - Git workspace 基于真实 `git diff` 展示工作树变更。
  - 支持文件级 additions / deletions、未跟踪文本文件 diff、hunk 行号。
  - 无 Git workspace 不展示 Review。
  - 支持 file search、viewed 进度、diff base 选择。
  - 支持 Working tree / Run changesets 切换。
  - Run changesets 可展示 run 写入 / 修改的文件列表、摘要和记录的 diff。
  - Changeset 支持 pending / applied / discarded 状态标记。
  - Review embed 可打开右侧 Review 并跳转 changeset。
- Artifacts 已进入可用基线：
  - 仅用于非 Git workspace / 普通 Chat；Git workspace 文件由 Review 管理。
  - workspace 外写入不计入 Artifacts。
  - 可展示、软删除、导出、打开文件、复制 path/content、转入 Research。
  - 支持 artifact 详情视图、inline 预览、文本文件预览、图片预览、PDF 预览。
  - 图片加载失败、类型不支持时显示 fallback。
  - PDF 预览支持页码导航、自适应缩放、Canvas 渲染。
- Markdown 已进入可用基线：
  - 使用 `unified` / `remark-parse` / `remark-gfm` 管线。
  - 支持常用 Markdown、GFM table/task list、code fence 复制。
  - 支持 `futureos://artifact|run|tool|approval|review|research/<id>` inline reference。
  - 支持 `futureos-artifact|run|tool|approval|review|research` fenced block embed。
  - 引用解析限制在当前 workspace，过滤已删除 artifact。
  - 引用状态使用 React 原生数据驱动模型更新。
  - 图片失败态和外链安全属性已补齐。
  - 代码块支持语法高亮（Shiki，28 种语言，GitHub Light 主题，行号显示）。
- Composer 支持 `@` 搜索并插入 FutureOS markdown 引用。
- 发送给 Agent 的 prompt 会注入引用对象摘要，消息原文仍保存 markdown。
- Store 已启用 SQLite `foreign_keys`、`busy_timeout`、`WAL` 和顺序 migration。
- Research 可展示当前 workspace resource，并支持从 markdown embed 跳转选中 resource。
- Data、Skill、Settings 仍是入口级占位。

## Priority Pass Status

### P0: Run Reliability And Recovery

当前阶段状态：主路径完成。

已具备：

- Retry / Continue。
- 失败 run 的可见错误信息。
- Agent 连接状态提示和 retry。
- agent 未运行时的启动提示。
- abort 基础链路和本地 run 状态更新。

后续细化：

- 更精确地区分 stream 断开、命令失败、模型失败。
- 验证 abort 是否真实终止底层 shell 子进程。
- 增加等待审批、LLM streaming、shell command 的中断自动化测试。

### P1: Run Inspect And Tool Output

当前阶段状态：主路径完成。

已具备：

- Run Inspect 详情视图。
- Tool call / output / timeline 展示。
- 搜索、复制、展开、事件过滤。
- command、cwd、path、exit status、duration、stdout、stderr 的结构化展示。

后续细化：

- 长 stdout/stderr 的专门 output storage。
- 二进制或不可 UTF-8 输出 fallback。
- Read / Write / Edit tool 的专属恢复动作和入口。

### P3: Review Workflow

当前阶段状态：主路径完成。

已具备：

- Git workspace 工作树 diff。
- diff base 选择。
- file search、viewed 进度、hunk 行号。
- Run changesets。
- pending / applied / discarded 状态标记。
- Review embed 跳转。

后续细化：

- 专门审查大视图。
- 按 run 筛选 changesets。
- 更完整的 additions / deletions / diff 记录。
- 真实 apply / revert 文件操作。
- applied / discarded 与真实工作树状态校验。

### P4: Artifacts And Chat Cleanup

当前阶段状态：主路径完成，PDF 预览已实现。

已具备：

- 非 Git workspace / 普通 Chat 的 artifact 列表。
- artifact 详情、打开、复制、导出、删除、转 Research。
- inline / text file / image / PDF 预览。
- preview fallback。
- Git workspace 不展示 Artifacts，workspace 外文件不计入 Artifacts。
- PDF 预览支持页码导航、自适应缩放、Canvas 渲染（Mozilla PDF.js）。

后续细化：

- 表格预览。
- PDF 缩放控制、全屏模式、文本选择和复制、搜索功能。
- 大 artifact 的文件存储策略和 metadata。
- 临时 workspace 删除前的清理摘要和保留 / 导出流程。
- 文件缺失、过大时的更细恢复操作。

### P4b: Attachments (Images)

目标：输入框图片附件作为多模态输入，落盘与 Artifact 归属按对话类型判断（详见 PRODUCT.md 4.12）。

产品规则：

- 第一阶段只支持图片附件（jpg / jpeg / png / gif / webp / bmp / svg），非图片不进入附件。
- 三种上传方式：附件按钮、复制粘贴、拖拽文件。
- 每一轮对话最多附加 4 个文件。
- 所有图片附件都作为多模态输入传给模型。
- 普通 Chat：附件保存到临时工作目录并登记为 Artifact，同时传给模型。
- Workspace 对话：附件不保存到工作目录、不创建 Artifact，只传给模型。

已具备：

- 落盘 / Artifact 归属按对话类型判定（`import_attachment_artifact` 对 Chat thread 落盘并登记 Artifact，Workspace 对话直传路径）。
- 三种上传方式：附件按钮（图片过滤）、复制粘贴（图片写入临时文件取路径）、拖拽文件（Tauri DragDrop 取 OS 路径）。
- 入口即限制图片类型；每轮最多 4 个文件。

后续细化：

- 粘贴 / 拖拽产生的临时图片文件的清理策略。
- 超出 4 个或非图片时的更明确用户提示。

### P7: Markdown And Object Embeds

当前阶段状态：主路径完成，代码高亮已实现。

已具备：

- 标准 Markdown + GFM。
- FutureOS inline reference 和 block embed。
- React 原生数据驱动引用刷新。
- code fence copy。
- 代码块语法高亮（Shiki，28 种语言，GitHub Light 主题，行号显示，异步加载）。
- Artifact / Run / Tool / Approval / Review / Research embed 基础交互。
- 图片失败态和外链安全属性。

后续细化：

- Tool / Review / Artifact embed 的更丰富摘要。
- 更多 store 更新来源和跨窗口 Tauri event bridge。
- Markdown export 策略。
- Agent 输出约束继续加强，鼓励优先输出 `futureos://` 引用。

### P2: Approval Model

当前阶段状态：数据模型升级完成，结构化展示就位。

已具备：

- 结构化 `action` payload（tool / category / command / paths / writes / deletes / scope）。
- `sandbox_boundary` 字段描述操作与沙盒边界关系。
- `reviewer` / `decision_scope` / `decision_source` 字段就位（当前固定 `user` / `once` / `user`）。
- `sandbox_config` / `approval_policy_config` / `approval_rules` 三张预留表。
- Agent 侧 `approval_policy.rs` 策略评估桩点。
- ApprovalPrompt 按 category 渲染结构化卡片，违反沙盒边界显示警告徽章。
- 单元测试覆盖 action 提取、sandbox_boundary 计算、policy_evaluator 桩。

后续细化：

- 沙盒执行（`sandbox_config` 接入 Agent，沙盒内自动通过）。
- 自动审批策略（`evaluate_policy` 实现规则匹配）。
- 决策范围扩展（session / always 按钮和规则缓存）。
- Settings UI（沙盒、策略、规则三个配置面板）。
- `auto_review` reviewer（审查 agent）。

详见 `gui/P2_APPROVAL_MODEL.md`。

## Next Priorities

当前优先 pass 已完成。下一阶段建议按以下顺序推进：

1. P5 Research resource 创建、详情、collection。
2. P6 Data / Skill / Settings 从占位变成最小可用。
3. P8 Composer 引用 UX 和统一 registry。
4. P9 自动化测试、migration 测试、打包可靠性。
5. P2 Approval 后续细化（沙盒执行、自动审批、Settings UI）。

## Out Of Current Scope

- 根 npm workspace。
- 根 Cargo workspace。
- 迁入源项目 Rust CLI。
- 将 Agent 编译进 GUI。
- 多 Agent 服务复杂切换 UI。
- 复杂数据分析工作台。
- 多数据库写入。
- Skill marketplace。
- 复杂文档、表格、多模态 artifact 的细粒度 review。
