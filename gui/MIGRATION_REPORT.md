# FutureOS GUI 合并迁移报告

日期：2026-06-05

## 迁移目标

本次迁移将原项目 `/Users/tao/Documents/FutureOS` 合并到 `/Users/tao/workspace/future-os`，并按约定把原桌面端变成 `future-os` 下的独立 `gui` 模块。

关键约束：

- `FutureOS/apps/desktop` 扁平迁移为 `future-os/gui`。
- 保留 `future-os/cli` 的 TypeScript CLI，不引入或替换为原项目的 Rust CLI。
- 将原 `FutureOS/packages/agent-core` 近两天的 agent 改造迁移到 `future-os/agent`。
- 不引入根级 npm workspace 或根级 Cargo workspace，保持现有 `agent`、`channel`、`cli`、`tui`、`gui` 独立构建。

## GUI 模块迁移内容

新增目录：`gui/`

迁入内容：

- 从 `FutureOS/apps/desktop` 迁入 React/Vite/Tauri 桌面端：
  - `src/`
  - `src-tauri/`
  - `index.html`
  - `package.json`
  - `package-lock.json`
  - `vite.config.ts`
  - `tailwind.config.js`
  - `postcss.config.js`
  - `tsconfig.json`
- 从原项目根目录迁入 GUI 相关文档和规范：
  - `PRODUCT.md`
  - `ER.md`
  - `PLAN.md`
  - `eslint.config.mjs`
  - `stylelint.config.mjs`
  - `tsconfig.base.json`

未迁入内容：

- `FutureOS/apps/desktop/dist`
- `FutureOS/node_modules`
- `FutureOS/target`
- `FutureOS/apps/cli`
- `FutureOS/packages/agent-core` 作为目录本身
- `FutureOS/packages/protocol`

结构调整：

- 原 `apps/desktop` 的路径被扁平化为 `gui` 根目录，不保留 `gui/apps/desktop`。
- `gui/tsconfig.json` 从原来的 `../../tsconfig.base.json` 改为 `./tsconfig.base.json`。
- `gui/eslint.config.mjs` 的 Tailwind 配置路径从 `apps/desktop/tailwind.config.js` 改为 `tailwind.config.js`。
- `gui/eslint.config.mjs` 的 lint 文件范围从 `apps/desktop/src/**/*.{ts,tsx}` 改为 `src/**/*.{ts,tsx}`。
- `gui/stylelint.config.mjs` 的 ignore 路径从 `apps/desktop/dist/**` 改为 `dist/**`，并忽略 `src-tauri/**`。
- `gui/package.json` 增加 GUI 自包含 lint/stylelint 所需依赖，不依赖原项目根级 package workspace。
- `gui/package-lock.json` 重新生成并清理了原 monorepo workspace 条目。
- 新增 `gui/src-tauri/Cargo.lock`，用于锁定 Tauri 后端 Rust 依赖。

GUI 运行关系：

- GUI 不直接依赖 CLI 包。
- GUI 的 Tauri 后端通过 gRPC 连接 `future-agent`。
- 默认 agent 地址仍通过 `FUTURE_AGENT_GRPC_ADDR` 控制；未设置时使用 `127.0.0.1:50051`。

## Agent 改造迁移内容

原项目中 `FutureOS/packages/agent-core` 对应当前仓库的 `agent`。本次没有新增 `packages/agent-core` 目录，而是把 agent-core 的改造迁入现有 `agent/`。

### 1. Agent loop 模块化

新增：

- `agent/src/agent/run_loop.rs`

调整：

- `agent/src/agent/mod.rs` 不再承载完整 run loop 实现。
- 大量 streaming、turn loop、tool call 执行相关逻辑被拆到 `run_loop.rs`。
- 保留 `Loop` 结构体和公开入口，降低对现有调用方的影响。

影响：

- agent loop 逻辑更接近原 `agent-core` 的模块拆分。
- 后续修改 streaming/tool 执行逻辑时入口更清晰。

### 2. LLM streaming helper 拆分

新增：

- `agent/src/llm/helpers.rs`

调整：

- `agent/src/llm/mod.rs` 移出部分 helper/解析逻辑。
- 保留 OpenAI-compatible SSE streaming 客户端能力。

影响：

- LLM 模块体积明显下降。
- streaming chunk、tool call、reasoning/thinking 相关辅助处理更集中。

### 3. RPC server 拆分

新增：

- `agent/src/rpc/approval.rs`
- `agent/src/rpc/commands.rs`
- `agent/src/rpc/protocol.rs`
- `agent/src/rpc/session.rs`
- `agent/src/rpc/session_prompt.rs`

调整：

- `agent/src/rpc/mod.rs` 从单个大文件拆成多个职责模块。
- `mod.rs` 现在主要负责 AppState、导出类型、共享 helper 和 gRPC command dispatch 的挂接。

职责变化：

- `protocol.rs`：`RpcCommand`、`RpcResponse`、SSE event/broadcaster 类型。
- `session.rs`：`ServerSession` 状态、session-level 配置和基础方法。
- `commands.rs`：`handle_command_internal` 和命令分发。
- `session_prompt.rs`：prompt 执行、agent streaming、session cwd 注入、工具事件广播。
- `approval.rs`：tool approval gate、approval decision 相关逻辑。

影响：

- GUI 需要的 session scoped event stream、approval request、tool activity 投影由这些模块承载。
- RPC 代码可维护性提升，后续扩展 GUI 事件和审批流时更容易定位。

### 4. Workspace-scoped tool 执行

修改：

- `agent/src/tools/mod.rs`

新增能力：

- `with_workspace_scope(workspace, future)`：在指定 workspace 中运行工具调用。
- `approve_outside_path(path)`：允许已审批的 workspace 外部路径。
- task-local `TOOL_SCOPE`：为异步 tool execution 保存当前 workspace 和已审批外部路径。
- `workspace_path()`：统一解析相对路径、绝对路径和 `~/` 路径。
- `ensure_workspace_access()`：阻止未审批的 workspace 外路径访问。

工具行为变化：

- `bash`：
  - 以 session cwd 作为 `current_dir`。
  - 设置 `HOME` 和 `PWD` 为当前 workspace。
- `read` / `write` / `edit` / `ls`：
  - 路径统一通过 workspace scope 解析。
  - 默认只能访问 session workspace 内路径。
- `grep`：
  - 默认在 workspace 下执行。
  - 指定 path 时先解析到 workspace path。

新增测试：

- `scoped_workspace_maps_tilde_paths_to_workspace`
- `scoped_workspace_rejects_unapproved_absolute_outside_write`
- `loop_workspace_scope_blocks_unapproved_absolute_write_from_model_tool_call`

影响：

- GUI 中每个 thread/session 的 agent 操作会被限制到对应 workspace。
- 降低 agent 在桌面端误写用户其他路径的风险。
- 为 GUI 的 approval/review 面板提供更合理的安全边界。

### 5. Session prompt 和 GUI 事件支持

修改：

- `agent/src/rpc/session_prompt.rs`

迁入能力：

- prompt 执行前确保 session cwd 存在。
- 每次 prompt 根据当前 cwd、日期、工具列表重建 system prompt。
- 增加 prompt guideline：
  - 当用户要求创建、保存、写入或修改文件时，必须使用可用工具执行，工具成功后再描述文件变化。
- agent loop 执行包裹在 workspace scope 内。
- 为工具调用准备阶段设置 `prepare_tool_call` hook，用于规范化工具参数路径。
- 广播 GUI 需要的 tool/event payload 字段，包括：
  - `tool_name`
  - `tool_id`
  - `tool_args`
  - `error`
  - `stopReason`
  - `usage`

影响：

- GUI 能将 agent events 投影成 tool activity、review changes、approval requests、artifacts 等 UI 状态。
- agent 的工具路径和 session workspace 绑定。

### 6. Approval 相关能力

新增：

- `agent/src/rpc/approval.rs`

迁入能力：

- Approval gate。
- Approval request/decision 数据结构。
- tool call 需要审批时向 event stream 发送 approval request。
- GUI 决策后通过 `approval_decision` command 回传。

影响：

- GUI 可以展示待审批工具调用，并通过 approval decision 让 agent run 继续执行。
- 当前产品规范已将审批操作从右侧 approvals panel 移到 composer 上方即时审批卡片；右侧面板不再承载审批操作或审批历史。

### 7. build.rs 的 protoc fallback

修改：

- `agent/build.rs`

新增能力：

- 构建时先检查 `protoc` 是否存在。
- 如果没有 `protoc`，但 `agent/src/grpc/generated/proto.rs` 已存在，则复用 checked-in generated proto。
- 如果没有 `protoc` 且 generated proto 不存在，则构建失败并提示安装 protobuf。

影响：

- 本地和 CI 环境在缺少 `protoc` 时更容易通过 agent 构建。
- 本次验证中该 fallback 已实际生效。

### 8. 其他 agent 文件变化

修改：

- `agent/src/main.rs`
- `agent/src/grpc/mod.rs`
- `agent/src/auth/mod.rs`

说明：

- 这些文件跟随 `agent-core` 迁移做了小范围适配。
- 主要目的是对齐拆分后的 RPC/session 类型、移除旧 mock/stale 逻辑，并保持 gRPC API 与现有 `proto/future.proto` 一致。
- `proto/future.proto` 本次没有改动。

## 构建和 CI 调整

### Makefile

新增 GUI 相关目标：

- `install-gui`
- `build-gui`
- `lint-gui`
- `stylelint-gui`
- `check-gui`
- `run-gui`
- `package-gui`
- `clean-gui`

调整：

- `make build` 现在包含 `build-agent build-tui build-cli build-gui`。
- `make lint` 现在包含 `lint-agent lint-tui lint-cli lint-gui stylelint-gui`。
- `make clean` 会调用 `clean-gui`。

### .gitignore

新增忽略项：

- `gui/src-tauri/target/`
- `gui/src-tauri/gen/`
- Tauri bundle 产物：`*.app`、`*.dmg`、`*.msi`、`*.AppImage`
- `gui/node_modules/`
- `gui/dist/`
- `gui/.vite/`
- SQLite/本地 app 数据：`.future/`、`*.db`、`*.sqlite` 等

### GitHub Actions

新增：

- `.github/workflows/desktop-build.yml`

工作流行为：

- tag `v*` 或手动触发时构建桌面端。
- matrix 覆盖 macOS、Windows、Linux。
- Node 依赖安装在 `gui/` 下执行。
- npm cache 使用 `gui/package-lock.json`。
- Tauri build 在 `gui/` 下执行 `npm run tauri:build`。
- artifact 路径为 `gui/src-tauri/target/release/bundle/**`。

### CLAUDE.md

更新内容：

- 记录 `gui/` 是 Tauri/React GUI 模块。
- 记录 GUI 通过 Tauri 后端连接 `future-agent` gRPC。
- 补充 GUI 的 build/lint/run/package/check 命令。

## 验证结果

已通过：

```bash
git diff --check
cd agent && cargo fmt --check
cd agent && cargo check
cd agent && cargo test
cd channel && cargo check
cd cli && npm install
cd cli && npm run build
cd tui && npm install
cd tui && npm run build
cd gui && npm ci
cd gui && npm run build
cd gui && npm run lint
cd gui && npm run stylelint
cd gui/src-tauri && cargo check
```

验证说明：

- `agent cargo check` 在本机缺少 `protoc` 的情况下通过 checked-in generated proto fallback 成功。
- `channel cargo check` 需要 `protoc`，因此安装了 Homebrew `protobuf` 后验证通过。
- `cli` 和 `tui` 初次构建前需要安装各自 npm 依赖，安装后构建通过。

## 当前状态和后续建议

当前迁移保留为未提交工作区改动，尚未 stage 或 commit。

建议后续提交前检查：

```bash
git status --short
git diff --check
make build
make lint
```

建议后续人工验证：

1. 启动 agent：

   ```bash
   make run-agent
   ```

2. 启动 GUI：

   ```bash
   make run-gui
   ```

3. 在 GUI 中创建/选择 workspace，发送一条消息，确认：

   - 能创建 agent session。
   - 能收到 `text_chunk` / `agent_end`。
   - 工具调用能显示在 activity/review/approval 相关面板中。
   - 文件写入默认限制在当前 workspace 内。
