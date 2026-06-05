# FutureOS 开发计划

## 1. 当前状态

FutureOS 已完成从源项目 GUI 到当前仓库的合并整理。当前结构是：

- `gui/`：Tauri + React + TypeScript 桌面 GUI。
- `agent/`：Rust `future-agent` gRPC 服务。
- `cli/`：TypeScript CLI，继续沿用当前仓库实现。
- `tui/`：TypeScript TUI。
- `channel/`：Rust channel 模块。

源项目的 `apps/desktop` 已扁平迁移为 `gui/`。源项目的 `packages/agent-core` 改造已迁移进当前 `agent/`，但没有新增 `packages/agent-core` 目录。源项目 Rust CLI 没有迁入。

## 2. 已完成

### 2.1 GUI 合并

- 迁入 React/Vite/Tauri 桌面端源码。
- 迁入 `src-tauri`、icons、capabilities、frontend 配置和 npm lockfile。
- 将 `PRODUCT.md`、`ER.md`、`PLAN.md`、ESLint、Stylelint 配置放到 `gui/` 根目录。
- 将源项目 `tsconfig.base.json` 放到 `gui/tsconfig.base.json`，避免在仓库根目录引入 npm workspace。
- GUI 运行时通过 `FUTURE_AGENT_GRPC_ADDR` 连接 `future-agent`，默认 `127.0.0.1:50051`。

### 2.2 Agent 改造

- 将 agent run loop 拆到 `agent/src/agent/run_loop.rs`。
- 将 LLM helper 拆到 `agent/src/llm/helpers.rs`。
- 将 gRPC session、protocol、approval、commands、prompt 逻辑拆到 `agent/src/rpc/*`。
- 保持 crate 名称和路径为当前 `future-agent` / `future_agent`。
- 迁移 workspace-scoped tool 行为：工具执行以 session cwd 为 workspace 边界。
- `read`、`write`、`edit`、`bash` 使用 workspace 路径解析。
- workspace 外读取、写入和复杂 shell 操作进入审批。
- 安全只读 workspace 操作自动放行。
- GUI 所需事件已投影到 run events、tool calls、tool outputs、approval requests。
- LLM tool-call streaming 增加 idle watchdog，避免模型工具调用尾包不完整时卡住。
- `build.rs` 增加 `protoc` 缺失时复用 checked-in generated proto 的 fallback。

### 2.3 GUI 修复

- 工具调用活动展示命令或路径摘要。
- 命令详情单行截断，hover 可查看完整命令。
- Approval 决策按钮增加处理中状态，避免重复点击。
- Approval 新增时右侧面板自动切到 Approvals。
- 后台刷新不再清空右侧 context 数据，避免右侧面板闪烁。
- 只有待审批数量从 0 变为大于 0 时才自动切 tab，避免持续抢焦点。

### 2.4 构建整理

- 根 `Makefile` 增加 `install-gui`、`build-gui`、`lint-gui`、`stylelint-gui`、`run-gui`、`package-gui`、`check-gui`、`clean-gui`。
- `make build` 覆盖 agent、tui、cli、gui。
- `make lint` 覆盖 Rust fmt check、TS lint 和 GUI stylelint。
- `.gitignore` 覆盖 GUI node_modules、dist、Tauri target、bundle 产物和本地 SQLite 数据。
- 新增 `.github/workflows/desktop-build.yml`，从仓库根 checkout，在 `gui/` 下执行 Node 和 Tauri build。

### 2.5 macOS 打包

- `npm run tauri:build -- --bundles app` 可生成 macOS `.app`。
- `gui/src-tauri/tauri.conf.json` 已配置 macOS ad-hoc signing identity。
- `.app` bundle 可通过 `codesign --verify --deep --strict`。
- DMG 依赖系统 `hdiutil`，在受限 sandbox 中可能失败，需要在真实 macOS session 或 CI 上继续验证。

## 3. 当前验收基线

推荐本地检查命令：

```sh
git diff --check

cd agent
cargo fmt --check
cargo check
cargo test

cd ../gui
npm run lint
npm run stylelint
npm run build

cd src-tauri
cargo fmt --check
cargo check
```

可选回归：

```sh
cd channel
cargo check

cd ../cli
npm run build

cd ../tui
npm run build
```

运行验证：

```sh
make run-agent
make run-gui
```

然后在 GUI 中发送消息，确认可以创建 session、收到 `text_chunk` / `agent_end`，并能展示 tool activity 和 approval。

## 4. P0：稳定性收口

目标：让当前 GUI + Agent 可以作为日常测试入口持续使用。

任务：

1. Run 控制
   - Cancel 当前运行。
   - Retry last user message。
   - Continue / Resume interrupted run。
   - Agent 断开、审批超时、工具失败时给出可操作恢复入口。

2. Approval 策略补齐
   - 梳理 shell、文件写入、文件删除、网络、数据访问、大范围批量修改。
   - 为每种审批类型提供结构化 payload。
   - GUI 展示操作类型、影响范围、风险摘要和完整详情。
   - Approval 决策后 run 状态与 UI 状态保持一致。

3. Tool Call UI
   - Shell：命令、cwd、stdout、stderr、exit status。
   - Read：路径、摘要、越界提示。
   - Write/Edit：目标路径、摘要、Review 入口。
   - 默认折叠长输出，避免撑开对话区。

4. 右侧面板可靠性
   - 后台 polling 不造成闪烁。
   - 切 Thread 时清理旧数据。
   - 只有真实新事件才触发用户可见状态变化。
   - Runs、Approvals、Review、Artifacts 的 loading 和 empty state 统一。

5. macOS DMG 验证
   - 在真实 macOS session 或 GitHub Actions 上验证 DMG。
   - 如果 `hdiutil` 仍失败，记录系统日志并调整 Tauri bundler 配置。

## 5. P1：Review 与 Artifact 闭环

目标：把“允许执行”和“审查变更”彻底分开。

任务：

1. Review changeset
   - 基于真实文件 diff 生成 changeset。
   - 记录每个文件的新增、删除、修改统计。
   - 支持本轮 run 的变更汇总。

2. Review UI
   - 从右侧 tab 的最小列表升级为专门审查界面。
   - 展示文件列表、diff、状态和摘要。
   - 支持 viewed / applied / discarded 状态。

3. Artifact
   - 小内容 inline 存储。
   - 大内容文件存储。
   - Artifacts 面板支持预览、打开文件、软删除。
   - Artifact 转入 Research 的交互补齐。

4. 普通 Chat 清理
   - 删除前展示 artifacts 和临时 workspace 文件摘要。
   - 支持保留、导出或转入 Research。
   - 清理状态从标记推进到实际文件处理。

## 6. P2：Research / Data / Skill 第一版

目标：让 FutureOS 从通用 Agent 聊天进入科研和复杂工作流工作台。

任务：

1. Research
   - Collection 列表和默认集合。
   - Resource 列表、详情、URL、本地文件、手动笔记。
   - 从 Artifact 转入 Research。
   - 在聊天中通过 `@` 引用 Research Resource。

2. Data
   - Data Source 列表。
   - CSV / TSV 数据源登记。
   - MySQL 只读数据源登记。
   - Credential 元信息管理。
   - Agent 只读查询审批。

3. Skill
   - 内置 Skill 列表。
   - Skill 详情说明。
   - 全局启用 / 禁用。
   - Workspace 级启用 / 禁用。
   - Agent prompt 构建时读取启用状态。

4. 统一引用
   - 注册 Artifact / Research Resource / Workspace File / Data Source / Skill。
   - Composer 输入 `@` 时弹出搜索。
   - 发送消息时写入 `object_references`。
   - Agent prompt 注入被引用对象的摘要、路径和权限信息。

## 7. P3：CLI / TUI / 自动化

目标：在核心 GUI 工作流稳定后，让 CLI 和 TUI 成为同一产品模型的辅助入口。

任务：

- CLI 增加 `ask`、`run`、`chat list`、`chat resume`、`review` 等高级命令。
- 明确 CLI 创建任务是否同步到 GUI。
- TUI 在 GUI、Agent、数据模型稳定后再产品化。
- 多 Agent 服务 UI 后置，不打断当前单服务闭环。

## 8. 暂不进入近期范围

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

## 9. 当前风险

- DMG 构建受 macOS 环境影响较大，需要在真实系统或 CI 里最终确认。
- Approval 类型需要继续系统化，否则容易出现 GUI 能审批但 Agent 策略不完整的问题。
- Review 目前仍偏事件投影，需要尽快切到真实 diff。
- 长任务取消、恢复和失败重试还不完整。
- Research / Data / Skill 已有产品边界，但 UI 和 command 仍需继续补齐。
