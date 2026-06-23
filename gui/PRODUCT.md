# FutureOS 产品说明

## 1. 产品定位

FutureOS 是一个以桌面 GUI 为主体验的 Agent 工作空间，面向需要持续推进复杂任务的人：软件工程、科研、数据分析、文档写作、报告生成和自动化调试都应该能在同一套工作对象里完成。

FutureOS 的核心目标是让 AI 的工作过程可检查、可恢复、可审查。用户不是只看到最终答案，而是能看到 Agent 读取了什么、运行了什么命令、等待哪些审批、产生了哪些文件和 artifact，以及之后如何继续这段工作。

第一阶段的重点是桌面体验、workspace 绑定的 Agent 执行、审批闭环和科研工作流的基础对象。

## 2. 当前模块边界

当前仓库采用扁平模块结构，不再保留源项目的 `apps/desktop` 或 `packages/agent-core` 路径。

- `gui/`：Tauri + React + TypeScript 桌面 GUI，是主产品入口。
- `agent/`：Rust `future-agent` gRPC 服务，负责会话、LLM streaming、工具执行、审批和事件投影。
- `cli/`：当前仓库已有的 TypeScript CLI，负责 auth、agent、channel、tools、skills、tui 等管理能力。
- `tui/`：当前仓库已有的 TypeScript TUI。
- `channel/`：Rust channel 模块，作为独立构建目标保留。

GUI 不把 Agent 作为 Tauri crate dependency 编进桌面进程。运行时 GUI 通过 `FUTURE_AGENT_GRPC_ADDR` 连接 `future-agent` gRPC 服务，默认地址是 `127.0.0.1:50051`。

源项目中的 Rust CLI 没有迁入。本仓库的 `cli/` 仍是最终 CLI。

## 3. 产品原则

### 3.1 GUI 是主体验

桌面 GUI 应该提供最完整、最细腻的体验，包括对话、workspace、后台程序状态、工具执行结果、即时审批、review、artifacts 和长任务进度。GUI 不应该只是终端包装器，而是把 Agent 的工作过程渲染成可理解、可操作的界面。

### 3.2 Agent 工作必须可检查

用户应该能够检查：

- Agent 做了什么计划。
- 它调用了哪些工具。
- 它读取了哪些文件。
- 它执行了哪些命令。
- 哪些文件发生了变化。
- 哪些步骤失败了。
- 哪些操作需要审批。
- 哪些任务可以继续恢复。

### 3.3 对话是工作对象

对话不是临时 chat log，而是可恢复、可继续、可管理的工作对象。FutureOS 同时支持普通 Chat 和绑定到具体目录的 Workspace 对话。

- Chat：系统自动创建临时 workspace，适合快速开始。
- Workspace 对话：用户选择本地目录，适合长期项目和真实文件工作。

每个 Thread 都必须使用独立 Agent session，避免不同 Chat 或 Workspace 对话共享运行上下文。

### 3.4 CLI 是辅助入口

CLI 用于登录、服务管理、终端自动化和调试。CLI 不复制 GUI 的完整体验，也不替代 GUI 的产品主线。CLI 创建的任务是否进入 GUI 需要后续单独定义，第一阶段不作为默认同步目标。

### 3.5 系统不只服务于编码

FutureOS 要支持编码，但不能只被设计成代码工具。科研、文献整理、数据分析、报告写作和 artifact 管理都是同级产品场景。Research 是第一阶段优先优化的非编码工作流。

## 4. 核心工作对象

### 4.1 Workspace

Workspace 是一个项目或工作上下文，可以对应本地项目目录、科研目录、数据分析目录、写作目录，或 FutureOS 自动创建的临时目录。

Workspace 下可以有多个子对话。删除某个 workspace 对话只删除对话本身，不删除 workspace 目录和目录内文件。

### 4.2 Chat

Chat 是不显式绑定用户目录的普通对话。为了让工具、脚本和 artifacts 有稳定运行空间，每个 Chat 背后都会自动创建一个临时 workspace。

普通 Chat 的文件产物需要可检查、可清理、可转入 Research。清理普通 Chat 时，如果存在 artifacts 或临时 workspace 文件，需要提示用户确认。

### 4.3 Message

Message 是对话中的消息，不只是一段文本。它未来可能包含 markdown、图片、文件引用、附件、tool result、diff 和 artifact 引用。

第一阶段输入框附件入口只支持图片，用户附加的图片会作为多模态输入传给模型理解。附件是否落盘到工作目录、是否登记为 Artifact，按对话类型（普通 Chat / Workspace 对话）判断，详见 4.12 Attachment。

### 4.4 Run

Run 是一次 Agent 执行，通常由一条用户消息触发。Run 在 GUI 中以“后台程序”呈现，记录状态、模型、开始和结束时间、错误信息、工具调用过程和输出结果。

用户应该能够看到当前后台程序是否正在执行、是否失败、是否等待审批、是否可以终止、重试或恢复。右侧 Runs 面板只展示后台程序列表和结果状态，不展示完整 event、tool payload 或长输出细节，避免把右侧变成调试日志集合。

后台程序列表的产品规则：

- 正在运行、排队中、等待审批的程序使用蓝色小点。
- 已完成、失败、已取消的程序使用灰色小点。
- 运行中的程序可以强制终止，终止前必须二次确认。
- 终止会调用 Agent abort，并把对应 run 标记为 `cancelled`。
- 如果 run 正在等待审批，终止时要同步取消该 run 下的 pending approval，避免 composer 上方残留无效审批。
- 已完成程序展示结果状态：成功、失败或已取消。
- 已完成、失败、已取消的程序可以一键清空；清空只删除 run 及其事件、tool、approval、review 关联，不能删除用户消息或 artifact 本体。

### 4.5 Tool Call

Tool Call 是 Agent 对外部能力的一次调用。默认工具集是长期固定约束：`read`、`bash`、`edit`、`write`。

目录浏览和搜索不新增 `ls` 或 `grep` 工具，而是通过 `bash` 执行系统能力，并由审批和可观察 UI 保证用户能理解风险。

GUI 中的工具调用应展示短标题、状态、耗时、路径或命令摘要。命令详情以单行截断展示，hover 时可以看到完整内容。

### 4.6 Approval

Approval 解决“是否允许执行”。它发生在高风险操作真正执行之前。

当前策略：

- workspace 内安全只读操作可自动放行。
- 简单只读 shell 命令可自动放行，例如 `pwd`、`ls`、`find`、`rg`、`grep`、`head`、`tail`、`cat`、`wc`、`sed`。
- 自动放行只适用于简单命令；复杂 shell、重定向、命令替换、链式执行和高风险操作需要审批。
- workspace 外读取需要审批。
- workspace 外写入、编辑、删除需要审批。
- 文件写入、文件修改、网络访问、数据访问、大范围批量修改都应该进入可审查路径。

GUI 负责展示审批请求并把 allow / deny 决策回传给 Agent。

审批体验是产品关键路径，必须遵守以下规则：

- 审批不放在右侧上下文面板中，不作为历史 tab 操作。
- 当前待处理审批显示在中间对话区底部、composer 上方，位置必须比普通上下文信息更醒目。
- UI 同时最多只展示一个待审批项；Agent 后端也应串行等待审批，当前审批未决时不继续执行后续危险操作。
- 审批不超时，始终等待用户明确允许或拒绝；Agent / GUI 不应因为固定 HTTP、SSE idle 或事件收集 timeout 结束正在等待审批的 run。
- 审批操作只保留左侧拒绝和右侧允许两个主按钮。
- 支持键盘快捷操作：`Esc` 拒绝，`Cmd/Ctrl + Enter` 允许一次。
- requested action 预览应 JSON pretty print、自动换行、高度随内容自适应，但最大不超过窗口高度的三分之一，超过后内部滚动。
- Agent 或 GUI 重启后，遗留的 pending approval 必须自动转为 `cancelled`，不能继续显示为可点击的有效审批。
- 如果用户点击一个后端已不存在的旧 pending approval，GUI 应把本地状态转为 `cancelled`，不能向用户暴露 “not pending” 之类内部错误。

### 4.7 Review

Review 解决“改了什么”。它不同于 Approval。Approval 是执行前允许与否，Review 是执行后查看变更。

Review 第一优先级是 Git workspace 的工作树 diff 审查。只要当前 workspace 是 Git 仓库，Review 面板就应该基于真实 `git diff HEAD` 展示当前未提交变更，包括新增、删除、修改和未跟踪文本文件，并展示文件级 additions / deletions。

无 Git 的 workspace 不展示 Review 入口。普通 Chat 或临时 workspace 产生的文件不走 Git diff 审查，而是进入 Artifacts 管理。

Git workspace 中不展示 Artifacts 入口。Git 仓库里的文件由 Git 管理，右侧 Review 负责展示文件变更，不能再把同一批文件重复投影成 Artifacts。

长期目标中，Review 还需要覆盖 markdown、文档、表格等文本类 artifact 的变更审查；但第一版产品语义必须先保持清晰：Git workspace 看 Review，非 Git workspace 看 Artifacts。

### 4.8 Artifact

Artifact 是 Agent 或用户在工作过程中产生的可复用产物，包括文档、报告、表格、图表、diff、命令输出、文献摘要、实验计划和数据分析结果。

Artifact 在非 Git workspace 和普通 Chat 场景中可以关联文件系统；在普通 Chat 场景中可以由 FutureOS 自动管理。

当 workspace 是 Git 仓库时，文件写入、编辑和新增不自动创建 Artifact。用户应该通过 Git Review 查看和管理这些变更，避免同一个文件同时出现在 Git diff 和 Artifacts 两套系统中。

用户在普通 Chat 中上传的图片附件也作为 Artifact 管理：图片会被保存进对应（临时）工作目录，并登记为 Artifact，便于后续查看、转入 Research 或复用。Workspace 对话中上传的图片附件不保存到工作目录、不创建 Artifact，只作为多模态输入传给模型。详见 4.12 Attachment。

### 4.9 Research Resource

Research Resource 是科研工作中的资料对象，包括论文、网页、笔记、摘要、方法表、数据集说明、证据链和实验计划。

Research 不是单纯搜索入口，而是面向科研工作的材料和证据空间。用户应该能把对话中的 artifacts、链接、文件和摘要转入 Research。

### 4.10 Data Source

Data Source 是 FutureOS 可以访问的数据或凭证入口。它可以服务科研、分析、报告和工程场景，不只代表数据库连接。

Data 管理任务相关的数据源和访问凭证；模型 provider key 属于模型或应用设置，二者必须分开。

### 4.11 Skill

Skill 是 Agent 可使用的能力单元，可以是用户级、workspace 级或系统级能力。Skill 应该有清晰说明、适用场景和启用状态。

Research、Data、Skill 的边界：

- Research 是材料。
- Data 是数据入口。
- Skill 是能力。

### 4.12 Attachment

Attachment 是用户在对话输入框中附加、供模型理解的内容。第一阶段附件只支持图片。

附件来源与上传方式：

- 第一阶段只接受图片（如 jpg、jpeg、png、gif、webp、bmp、svg），非图片文件不进入附件。
- 支持三种上传方式：点击输入框的附件按钮、复制粘贴、拖拽文件。
- 每一轮对话最多附加 4 个文件，超出时不再继续添加。
- 所有图片附件都会作为多模态输入传给模型，让模型理解图片内容。

附件是否落盘到工作目录、是否登记为 Artifact，按对话类型判断：

- 普通 Chat（没有用户选择的 workspace，背后是临时 workspace）：附件被视为 Artifact，保存到临时工作目录，并登记为 Artifact，同时作为多模态输入传给模型。
- Workspace 对话：附件不保存到工作目录、不创建 Artifact，只作为多模态输入传给模型，避免污染用户的项目目录。

无论哪种情况，附件都必须传给模型；区别只在于是否落盘到工作目录、是否登记为 Artifact。

## 5. 桌面体验

### 5.1 三栏布局

GUI 采用三栏结构：

```text
左侧导航栏      中间对话区      右侧上下文面板
```

左侧负责功能入口、workspace 和 thread 导航。中间负责 conversation 和即时审批。右侧负责后台程序、review 和 artifacts 等可检查上下文。

右侧面板不能抢占用户注意力，也不能承载需要用户立即响应的审批操作。普通后台刷新不应该造成闪烁或频繁切 tab。

### 5.2 左侧导航

左侧导航应支持：

- New Chat。
- Research。
- Data。
- Skill。
- Workspace 列表。
- Workspace 下子对话。
- Chat 列表。
- Settings。
- 展开、收起和归档显示。

置顶只影响所属列表。Workspace 对话的置顶只影响该 workspace 下的子对话排序，普通 Chat 的置顶只影响 Chat 列表。

### 5.3 中间对话区

中间区域展示用户消息、assistant 流式输出、计划、工具调用、命令预览、错误状态和 follow-up 交互。

输入框应保持底部悬浮，模型选择位于输入框区域。发送消息时 GUI 创建 Run，写入用户消息，连接 Agent stream，并把事件投影到数据库和 UI。

输入框支持图片附件，三种上传方式：附件按钮、复制粘贴、拖拽文件；每一轮对话最多附加 4 个文件。附件是否落盘按对话类型判断，详见 4.12 Attachment。

当 Agent 请求审批时，审批卡片应插入在 composer 上方，而不是右侧面板。审批卡片属于当前对话的即时交互层；用户做出允许或拒绝前，Agent run 保持等待状态。

### 5.4 右侧上下文面板

右侧面板用于检查当前 Thread 的运行上下文，但不承载即时审批。顶部不显示独立标题和当前 session / thread name；使用一个 dropdown 作为当前视图标题，并根据 workspace 类型切换可用视图。

当前重点：

- Runs：展示后台运行程序列表，区分运行中和已完成程序，支持终止运行中程序和清空已完成程序。每个程序卡片主内容只展示 tool input JSON 中的 `command` 字段，按真实命令文本显示，不展示完整 JSON、转义文本、模型名或 `Program <id>` 这类内部编号；没有 `command` 的 run 不进入 Runs 列表。
- Git workspace：只展示 Runs 和 Review。Review 基于真实 Git diff；Artifacts 不展示。
- 非 Git workspace / 普通 Chat：只展示 Runs 和 Artifacts。Review 不展示。
- Review：展示当前 Git workspace 的工作树变更审查，包含文件列表、统计和 diff。
- Artifacts：展示当前 Thread 或 Workspace 的产物，仅用于非 Git workspace / 普通 Chat。

右侧面板应可以收起。收起后只保留轻量入口，不影响主对话阅读。

Runs 面板明确不做以下事情：

- 不展示完整 run event timeline。
- 不展示完整 tool payload。
- 不展示长 stdout / stderr。
- 不展示审批历史和审批操作。

这些细节属于后续专门的调试 / timeline / review 视图，不能污染日常后台程序列表。

### 5.5 配色与设计 token

GUI 的颜色统一走 `gui/tailwind.config.js` 里定义的**语义 token**（中性/表面、强调/交互、状态三件套、diff、阴影），组件里不直接写 Tailwind 原生具名色。状态徽章统一用 `<Badge tone>` 组件；用颜色区分**并列种类**（事件类别、错误子类型）的地方是有意的例外。

配色清单、用法速查与反模式见 [`gui/COLOR.md`](COLOR.md)——新写或改组件选色时以它为准。

## 6. Agent 工作流

一次典型流程：

1. 用户打开 FutureOS。
2. 用户创建 Chat 或选择 Workspace 对话。
3. GUI 创建 Thread、Message 和 Run。
4. GUI 通过 gRPC 调用 `future-agent`。
5. Agent 进行 LLM streaming。
6. Agent 根据模型输出执行 `read`、`bash`、`edit`、`write`。
7. 高风险操作进入 Approval，Agent 停止在当前工具调用处等待用户决策，不超时。
8. GUI 在 composer 上方展示当前审批；用户允许后 Agent 继续，用户拒绝后该危险操作失败并反馈给 Agent。
9. GUI 展示文本增量、后台程序状态、工具活动摘要和结束状态。
10. Run 完成后 assistant message、run events、tool calls、tool outputs 和 approval 记录被持久化。

Agent 工具执行默认以当前 session cwd 为 workspace 边界。普通 Chat 使用系统创建的临时 workspace，Workspace 对话使用用户选择的目录。越界访问不允许静默执行。

## 7. 构建与发布

根 `Makefile` 是当前推荐入口：

- `make build`：构建 agent、tui、cli、gui。
- `make lint`：运行 Rust fmt check、TS lint 和 GUI stylelint。
- `make check-gui`：运行 GUI lint、stylelint 和 frontend build。
- `make run-agent`：启动 `future-agent` gRPC 服务。
- `make run-gui`：启动 GUI dev app。
- `make package-gui`：构建桌面 bundle。

macOS 下 `.app` bundle 已支持 Tauri 构建和 ad-hoc signing。DMG 依赖系统 `hdiutil`，需要在真实 macOS session 或 CI 中验证；受限 sandbox 中可能出现 `hdiutil: create failed - 设备未配置`。

## 8. 第一阶段非目标

以下内容暂不进入第一阶段：

- 不引入根 `package.json` workspace。
- 不引入根 `Cargo.toml` workspace。
- 不迁入源项目 Rust CLI。
- 不把 Agent 编进 GUI Tauri crate。
- 不让 CLI 默认复制 GUI 的完整 chat 体验。
- 不做复杂多 Agent 服务 UI。
- 不做完整 PDF 阅读器和精细标注。
- 不做复杂数据分析工作台。
- 不做 Skill marketplace。

## 9. 产品路线

近期产品路线按稳定性优先：

1. 对话、Workspace、Run、Tool Call、Approval 的稳定闭环。
2. 运行控制：终止、重试、恢复。
3. Review 基于真实 diff 的审查体验。
4. Artifact 和统一 `@` 引用。
5. Research / Data / Skill 从入口变成可用模块。
6. CLI 和 TUI 在核心模型稳定后继续补齐高级工作流。
