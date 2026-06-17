# FutureOS GUI 后续开发计划

## 1. 当前基线

这份计划只记录 GUI 仍未完成或需要继续增强的部分。已经完成的迁移、构建整理、Agent 拆分、gRPC 接入、动态模型列表、右侧后台程序面板、composer 上方即时审批、Git workspace Review 基础面板、非 Git workspace Artifacts 基础面板、右侧面板闪烁修复、macOS `.app` 打包验证等内容不再放入任务池。

当前 GUI 已具备的基础能力：

- `gui/` 已作为独立 Tauri + React 模块存在。
- GUI 通过 `future-agent` gRPC 服务发送 prompt、接收事件、提交 approval 决策和 abort 请求。
- 模型列表从 agent 获取，不再使用前端硬编码列表。
- Chat / Workspace thread 可创建、恢复、重命名、置顶、归档、删除。
- 普通 Chat 会使用系统创建的临时 workspace。
- 右侧面板已按 workspace 类型动态切换：Git workspace 展示 Runs / Review，非 Git workspace 和普通 Chat 展示 Runs / Artifacts；审批操作不在右侧面板中。
- Runs 已收敛为后台程序列表，支持运行中蓝点、完成态灰点、运行中终止、完成态清空。
- Approval 已移到 composer 上方即时卡片，同一时间只展示一个待审批项，不超时，支持 `Esc` 拒绝和 `Cmd/Ctrl + Enter` 允许一次。
- Review 已基于真实 `git diff HEAD` 展示 Git workspace 工作树变更，包含文件列表、统计和文本 diff；无 Git workspace 不展示 Review。
- Artifacts 可展示、软删除、转入 Research；Git workspace 不展示 Artifacts，文件变更由 Git Review 管理。
- Research 可展示当前 workspace 下的 resource 列表。
- Data、Skill、Settings 仍是入口级占位。

## 2. P0：运行控制与失败恢复

目标：让日常使用中的 Agent run 可中断、可重试、可恢复，避免用户遇到长任务或失败任务时只能重开对话。

### 2.1 Terminate 当前 Run

当前基线：

- Runs 面板已提供运行中程序的 `Terminate` 入口。
- 终止前已有二次确认。
- GUI 已调用 agent abort 能力，并把 run 状态更新为 `cancelled`。
- 如果 run 正在等待审批，终止会同步取消该 run 下 pending approval，避免 composer 上方残留无效审批。
- Runs 面板已提供 `Clear finished`，清理已完成、失败、已取消的 run 及其事件 / tool / approval / review 关联，但保留用户消息和 artifact 本体。

仍需增强：

- pending assistant message 需要在终止后给出更明确的取消文案。
- 左侧 thread running indicator 在极端竞态下仍需要更多回归测试。
- 终止后如果 Agent 已经进入长时间外部命令，仍需要确认 abort 对底层进程的真实中断能力，而不只是更新本地状态。

### 2.2 Retry Last User Message

- 对失败或取消的 run 提供 retry。
- retry 复用上一条 user message 和附件引用。
- 新建 run，不覆盖旧 run 记录。
- 失败原因保留在旧 assistant message 或 run events 中。

### 2.3 Continue / Resume

- 对中断、连接失败或工具执行失败后的 run 提供继续入口。
- Continue 应显式提示 Agent 基于当前 thread 和 workspace 状态继续。
- 需要避免重复执行已完成的危险操作。

### 2.4 Agent 断开恢复

- GUI 检测 gRPC 连接失败、agent 未启动、agent 中途断开。
- Composer / Runs 面板显示可操作错误状态。
- 提供重新连接提示，而不是只在 assistant message 中展示异常文本。

## 3. P1：Tool Call 与 Run Timeline 升级

目标：让用户能快速看懂 Agent 正在做什么、做完了什么、失败在哪里。

### 3.1 Run Timeline / Debug View

- 注意：右侧 Runs 面板已经明确收敛为后台程序列表，不再承载完整 timeline、tool payload 或长输出。
- 后续如果需要 `run_events` timeline，应新增专门 Debug / Inspect 视图，不能塞回右侧日常 Runs 列表。
- 将 `run_events` 渲染成更清晰的 timeline。
- 区分 text、thinking、tool call、approval、review、error、agent end。
- 避免主要依赖 JSON 摘要。
- 支持展开查看原始 payload，便于调试。

### 3.2 Tool Call 卡片

- Shell：展示命令、cwd、exit status、stdout、stderr。
- Read：展示路径、读取摘要、越界提示。
- Write/Edit：展示目标路径、变更摘要、Review 入口。
- 默认折叠长输出，避免撑开对话区和右侧面板。
- 对运行中 tool 显示明确 pending 状态和耗时。

### 3.3 Tool Output 存储策略

- 大 stdout/stderr 不应无限塞入 UI。
- 长输出需要截断预览，并保留完整内容的打开方式。
- 需要明确 tool output 与 artifact 的转换规则。

## 4. P2：Approval 策略与审批体验

目标：让“是否允许执行”成为稳定、可解释、可审计的闭环。

当前基线：

- Approval 操作已从右侧上下文面板移到 composer 上方。
- UI 同时只展示一个待审批项，取最早 pending approval。
- Agent 审批等待已取消 30 秒超时，当前审批未决时保持等待。
- GUI 事件收集和 Agent run 外层不再因审批等待触发固定超时。
- 操作按钮只保留左侧拒绝和右侧允许一次。
- 支持 `Esc` 拒绝，`Cmd/Ctrl + Enter` / `Ctrl + Enter` 允许一次。
- requested action 预览已自动换行、pretty JSON，并限制最大高度为窗口三分之一。
- GUI 启动时会把上次进程遗留的 pending approval 标为 `cancelled`。
- 如果后端已经没有该 pending approval，点击旧审批时 GUI 会转为 `cancelled`，不暴露内部错误。

### 4.1 Approval 类型补齐

需要系统化覆盖：

- workspace 外读取。
- workspace 外写入。
- 文件删除。
- 复杂 shell。
- 网络访问。
- 数据源访问。
- 大范围批量修改。

### 4.2 Approval Payload 结构化

每个 approval request 应提供：

- 操作类型。
- 工具名和 tool call id。
- cwd。
- 目标路径、命令、URL 或数据源。
- 风险摘要。
- 原始 requested action。

### 4.3 Approval UI 升级

- 详情区分摘要和完整 payload。
- 允许 / 拒绝后展示更明确的决策状态。
- Agent 断开、run 已结束时给出明确提示。
- 支持从 approval 跳转到对应 tool call / run。

禁止回退：

- 不要把审批操作放回右侧面板。
- 不要恢复审批超时。
- 不要同时展示多个待审批操作。
- 不要把审批历史作为右侧默认 tab。

### 4.4 审批策略回归测试

- workspace 内安全只读 shell 自动放行。
- workspace 外读取需要审批。
- workspace 外写入需要审批。
- 复杂 shell 需要审批。
- 拒绝后 run 不应继续执行该危险操作。

## 5. P3：Review 正式闭环

目标：把“执行前审批”和“执行后审查”彻底分开，Review 应基于真实变更而不是粗粒度事件投影。

### 5.1 真实 Diff 生成

当前基线：

- Git workspace 已直接读取真实 `git diff HEAD`。
- 已展示每个变更文件的 additions / deletions。
- 已包含未跟踪文本文件的新增 diff 预览。
- 无 Git workspace 不展示 Review。

仍需增强：

- 支持按本轮 run 聚合 changeset，而不只是展示整个工作树。
- 支持选择 base，例如 upstream、merge-base 或用户指定 commit。
- markdown、代码、普通文本继续走统一文本 diff，但需要更好的 hunk 定位和行号。

### 5.2 Review UI 专门化

- 当前右侧 Review 已展示文件列表、每文件 diff、统计和状态。
- 后续从右侧最小视图升级为专门审查视图。
- 支持 viewed / applied / discarded 状态。
- 后续再考虑 apply / revert 的真实文件操作。

### 5.3 Artifact Review

- 仅适用于非 Git workspace / 普通 Chat 的 artifacts。
- Git workspace 不走 Artifact Review，文件变更由 Git Review 管理。
- 文本类 artifact 支持 diff 和审查状态。
- 文档、表格、多模态 artifact 先保留摘要审查，不进入复杂细粒度预览。

## 6. P4：Artifact 与 Chat 清理

目标：让 Agent 产物可管理、可转移，普通 Chat 的临时 workspace 可安全清理。

### 6.1 Artifact 详情与打开

- Artifacts 面板支持打开文件。
- 支持更完整的详情视图。
- inline 内容和 file 内容统一预览体验。
- 长内容支持复制、打开、导出。

### 6.2 Artifact 存储策略

- 小内容 inline 存储。
- 大内容写入文件，数据库保存路径和摘要。
- 写入 Git workspace 内的文件不自动创建 Artifact。
- 写入非 Git workspace / 普通 Chat workspace 内的文件可以创建 Artifact。
- 写入 workspace 外部的文件不计入 Artifacts。
- 明确 `content_storage = inline | file` 的使用边界。

### 6.3 普通 Chat 清理流程

- 删除前展示 artifacts 和临时 workspace 文件摘要。
- 支持保留、导出或转入 Research。
- 清理状态从 `pending_cleanup` 推进到真实文件处理。
- 清理完成后更新 workspace cleanup status。

## 7. P5：Research 第一版可用化

目标：Research 不只是展示 artifact 转入结果，而是成为科研材料管理入口。

### 7.1 Resource 创建

- 手动添加 URL。
- 添加本地文件。
- 添加手动笔记。
- 从 Artifact 转入时允许编辑标题、类型和摘要。

### 7.2 Resource 详情

- 展示标题、类型、来源、摘要、metadata。
- 支持打开源文件或 URL。
- 支持软删除或归档 resource。

### 7.3 Collection 管理

- 默认 collection 已可隐式存在，后续需要 UI。
- 支持创建、重命名、归档 collection。
- Resource 可归入 collection。

## 8. P6：Data / Skill / Settings

目标：把当前占位入口推进到可用产品模块。

### 8.1 Data

- Data Source 列表。
- CSV / TSV 数据源登记。
- MySQL 只读数据源登记。
- Credential 元信息管理。
- Agent 访问数据源前进入 approval。

### 8.2 Skill

- 展示已安装 skills。
- 展示 skill 名称、描述、来源和启用状态。
- 支持用户级启用 / 禁用。
- 支持 workspace 级启用 / 禁用。
- Agent prompt 构建时读取启用状态。

### 8.3 Settings

- Agent 地址设置或连接状态展示。
- 默认模型和默认 thinking level。
- 应用级偏好。
- 本地数据目录和版本信息。

## 9. P7：Markdown 渲染与工作对象嵌入架构

目标：FutureOS 的 markdown 必须兼容标准 markdown，同时能原生承载 Artifact、Run、Tool Call、Approval、Review、Research Resource 等工作对象。markdown 不只是文本展示层，而是 Agent 工作流的可读、可恢复、可引用的表达格式。

### 9.1 设计原则

- 标准 markdown 内容必须继续可读、可复制、可导出。
- FutureOS 扩展语法必须有纯文本 fallback，离开 GUI 后也不应变成不可理解的乱码。
- message 原文仍保存为 markdown，不把 React 节点或 UI 状态写进 message content。
- 对象关系写入结构化表，例如 `object_references`，不要只靠字符串解析恢复业务关系。
- 渲染时先解析 markdown，再批量解析 FutureOS 引用，最后按组件注册表渲染。
- 大对象不直接塞入 markdown，例如长 stdout、完整 artifact 文件、完整 diff 只以引用方式嵌入。

### 9.2 Markdown 分层

渲染管线分为四层：

1. 标准 Markdown 层
   - CommonMark / GFM 基础语法。
   - heading、paragraph、list、task list、table、blockquote、code fence、link、image、inline code、bold、italic。

2. FutureOS Inline Reference 层
   - 用于一句话中的轻量对象引用。
   - 例如 artifact chip、run chip、research resource chip。
   - 目标是像普通链接一样可读，但在 GUI 中渲染为对象 chip。

3. FutureOS Block Embed 层
   - 用于独立块级对象展示。
   - 例如 Artifact 卡片、Run timeline 摘要、Tool output 摘要、Review diff 摘要。

4. Runtime Projection 层
   - 根据对象 id 从本地 store 批量加载最新状态。
   - Run / Approval / Tool Call 这类动态对象可以随 polling 或事件刷新。
   - markdown 本身只表达“引用哪个对象、用什么视图展示”。

### 9.3 建议扩展语法

Inline reference 使用可读链接形式，保证标准 markdown 环境下仍可理解：

```md
请参考 [artifact:实验计划](futureos://artifact/artifact_123) 和 [run:上次分析](futureos://run/run_456)。
```

GUI 渲染规则：

- `futureos://artifact/<id>` 渲染为 Artifact chip。
- `futureos://run/<id>` 渲染为 Run chip。
- `futureos://tool/<id>` 渲染为 Tool Call chip。
- `futureos://approval/<id>` 渲染为 Approval chip。
- `futureos://review/<id>` 渲染为 Review chip。
- `futureos://research/<id>` 渲染为 Research Resource chip。

Block embed 使用 fenced directive，保证普通 markdown 阅读器会把它当作代码块或可读文本处理：

````md
```futureos-artifact
id: artifact_123
view: card
```

```futureos-run
id: run_456
view: timeline
```

```futureos-review
id: review_789
view: diff-summary
```
````

第一版只需要支持 `id` 和 `view`。后续可增加 `title`、`collapsed`、`range`、`focus` 等字段。

### 9.4 AST 与渲染架构

前端需要把当前 `MarkdownContent` 从手写 parser 升级为插件式架构：

```text
raw markdown
  -> markdown parser
  -> FutureOS directive parser
  -> reference collector
  -> batch resolver
  -> render registry
  -> React nodes
```

核心对象：

- `MarkdownDocument`
  - `raw`: 原始 markdown。
  - `nodes`: 标准 markdown AST + FutureOS extension nodes。
  - `references`: 从 inline link 和 block embed 收集到的对象引用。

- `FutureReference`
  - `targetType`: `artifact | run | tool | approval | review | research | file | data | skill`。
  - `targetId`: 对象 id。
  - `label`: 用户可见文本。
  - `view`: `chip | card | timeline | diff-summary | output-summary`。

- `ReferenceResolution`
  - 批量从 Tauri store command 取回对象快照。
  - 缺失对象返回 typed missing state，而不是让渲染崩掉。

- `RenderRegistry`
  - `paragraph`、`heading`、`code` 等标准节点使用 markdown renderer。
  - `future_reference` 使用 `ReferenceChip`。
  - `future_artifact` 使用 `ArtifactEmbed`。
  - `future_run` 使用 `RunEmbed`。
  - `future_tool` 使用 `ToolCallEmbed`。
  - `future_review` 使用 `ReviewEmbed`。

### 9.5 Store 与数据关系

message 存储策略：

- `messages.content` 保存原始 markdown。
- `messages.content_type` 继续使用 `markdown` 或 `mixed`。
- `object_references` 保存消息与对象的结构化关系。
- `reference_targets` 注册可引用对象，供 Composer `@` 搜索和 markdown resolver 共用。

对象嵌入不复制大内容：

- Artifact embed 只存 artifact id，渲染时读取 artifact record。
- Run embed 只存 run id，渲染时读取 run、events、tool calls。
- Review embed 只存 changeset id，渲染时读取文件列表和 diff 摘要。
- Tool output 超长时只渲染摘要，并提供打开完整输出入口。

### 9.6 Agent 输出约束

需要给 Agent 明确 markdown 输出规则：

- 普通回答使用标准 markdown。
- 需要引用产物时优先输出 `futureos://` 链接。
- 需要嵌入完整工作对象时输出 fenced directive。
- 不直接把长 stdout、完整 diff、大文件内容塞进回答。
- 生成 artifact 后，assistant message 中只引用 artifact，artifact 内容进入 artifacts 存储。

### 9.7 兼容与安全

- 默认不渲染 raw HTML。
- 外部 URL 需要安全打开策略。
- `futureos://file/...` 不应绕过 workspace 权限。
- 缺失对象显示 Missing reference，而不是删除原文。
- 导出 markdown 时保留原始 markdown 和 futureos 链接。

### 9.8 第一阶段已完成

- 已将聊天消息渲染切换到 `features/markdown` 运行时。
- 已支持 heading、paragraph、list、code fence、inline code、bold、普通 link 等基础 markdown 节点。
- 已支持 `futureos://artifact/<id>` 和 `futureos://run/<id>` inline chip。
- 已支持 `futureos-artifact` block card 和 `futureos-run` block summary。
- 已新增 `resolve_markdown_references` Tauri command，前端按消息批量解析 artifact/run 引用。
- 已在消息落库时提取 artifact/run 引用，并写入 `reference_targets` / `object_references`。
- 已补 Rust 单元测试覆盖引用抽取与引用关系写入。

### 9.9 第二阶段已完成

- 已补充 blockquote、task list、italic、image 等常用 markdown 节点。
- 已扩展 `futureos://tool/<id>`、`futureos://approval/<id>`、`futureos://review/<id>`、`futureos://research/<id>` inline reference。
- 已扩展 `futureos-tool`、`futureos-approval`、`futureos-review`、`futureos-research` block embed。
- 已为 Tool Call、Approval、Review、Research Resource 增加后端 resolver、引用索引 metadata 和前端 summary renderer。
- 已更新引用抽取测试，覆盖 artifact/run/tool/approval/review/research。

### 9.10 第三阶段已完成

- 已补充 table 和 thematic break 渲染。
- 已增强 fenced code 解析，支持 backtick / tilde fence、缩进容错和 fence info string。
- 已增强 inline link / image 解析，支持转义字符、带括号 URL、title 和 FutureOS `?view=` 查询参数。
- 已修复重复 Markdown 节点造成的 React key 冲突风险。
- 已将 FutureOS 引用解析限制在当前 workspace，并过滤已删除 artifact。
- 已修复 percent encoded 中文 id 的 UTF-8 解码问题。
- 已接入 `unified` + `remark-parse` + `remark-gfm`，标准 Markdown 解析切换到 mdast/GFM 管线。
- 已将 FutureOS inline reference 和 fenced directive 作为 mdast 后处理扩展保留。
- 已支持 nested list、strikethrough、GFM table、task list 等由 GFM parser 提供的结构化解析。
- 已新增 Vitest 前端测试入口，覆盖 Markdown parser、MarkdownContent renderer、FutureOS inline reference / block embed、reference-style link/image、GFM table/task list/nested list 和 raw HTML 安全降级。
- 已将 FutureOS 引用解析升级为 React 原生数据驱动模型：`MarkdownContent` 注册引用，Run / Tool / Approval / Artifact / Review 等对象通过共享 reference store + `useSyncExternalStore` 订阅更新。
- 已把 ContextPanel 和当前 run 刷新结果写入共享 reference store，Markdown 中引用同一对象的 chip/card 会随 store 快照自动重渲染。

### 9.11 下一阶段交付

- 补齐代码高亮和更完整的导出策略。
- 继续补齐更多 store 更新来源，例如 Research resource、跨窗口 Tauri event bridge 和更细粒度 tool output 更新。
- 为 Artifact embed 增加打开详情、复制路径、预览文件等交互。
- 持续完善 Agent 输出约束，要求产物、运行、审批、review 优先以 FutureOS markdown 引用表达。
- 为 Markdown renderer、Composer、ContextPanel、Approval、Artifact 转 Research 继续补前端测试。

## 10. P8：统一引用与 Composer

目标：让用户能在聊天中主动引用已有对象，Agent 获取上下文时更可控。

### 10.0 已完成

- 已新增 workspace-scoped reference search command，可搜索 Artifact、Run、Tool Call、Approval、Review、Research Resource。
- Composer 已支持 `@` 搜索弹层，支持键盘上下选择、Enter/Tab 插入、Escape 关闭。
- 选中引用后会插入标准 FutureOS Markdown 链接，例如 `[artifact:Poem](futureos://artifact/artifact_123)`。
- New Chat 在选择 workspace 后已支持 `@` 搜索。
- 发送给 Agent 的 prompt 已包含引用对象摘要上下文，消息原文仍保持原始 markdown。
- 已补后端测试，覆盖从 workspace 对象搜索 reference target。

### 10.1 Reference Target

- 注册 Workspace File、Data Source、Skill。
- 支持 workspace scope 和 global scope。
- 将搜索 command 从当前对象表查询升级为统一 registry/cache，并处理对象删除后的失效状态。

### 10.2 Composer `@` 搜索

- 支持 mouse hover selection 和更明确的空状态。
- 支持引用预览和删除已插入引用。

### 10.3 Prompt 注入

- 将 workspace file / data source / skill 引用对象转成 Agent 可理解上下文。
- 增加明确读取权限和缺失对象提示。
- 引用对象在消息中可点击回看。

## 11. P9：自动化测试与发布可靠性

目标：让 GUI 迭代可以更放心地持续推进。

### 11.1 前端测试

- 已接入 Vitest，`npm test` 可运行前端单元测试。
- 已覆盖 Markdown parser、MarkdownContent renderer 和 FutureOS 引用投影的基础回归。
- Composer 行为测试。
- Markdown renderer 更多交互状态测试。
- ContextPanel polling、后台程序列表、终止二次确认和清空已完成程序测试。
- Approval 决策状态测试。
- Artifact 转 Research 测试。

### 11.2 Tauri / Agent 集成测试

- mock agent 返回模型列表。
- mock agent stream 返回 text/tool/approval/end。
- 验证 run events、tool calls、approval requests 的投影。
- 验证 composer 上方 approval prompt 的 `Esc` / `Cmd+Enter` 快捷键。
- 验证旧 pending approval 在 GUI 重启后自动转为 `cancelled`。
- 验证 assistant markdown 中的 Artifact / Run 引用能写入 `object_references`。

### 11.3 打包验证

- macOS `.app` 继续保持可构建和可签名。
- DMG 在真实 macOS session 或 CI 中验证。
- GitHub Actions 上传 artifact 路径保持有效。

## 12. 当前不进入近期范围

- 根 npm workspace。
- 根 Cargo workspace。
- 迁入源项目 Rust CLI。
- 将 Agent 编译进 GUI。
- 多 Agent 服务复杂切换 UI。
- 完整 PDF 阅读器和精细标注。
- 复杂数据分析工作台。
- 多数据库写入。
- Skill marketplace。
- 复杂文档、表格、多模态 artifact 的细粒度 review。
