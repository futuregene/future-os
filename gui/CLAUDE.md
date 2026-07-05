# GUI 开发指南（`gui/`）

FutureOS 桌面应用：Tauri + React + TypeScript，前端 `src/`，Tauri 后端 `src-tauri/`（Rust），通过 gRPC 连到仓库根的 agent。整个 monorepo 的架构/build 见**仓库根 `CLAUDE.md`**；本文件只管 `gui/`。

## 文档地图（按需精读，别整篇拉进 context）

| 文档 | 内容 | 何时查 / 改 |
|---|---|---|
| `PRODUCT.md` (~21KB) | 产品定位、模块边界、工作对象语义、桌面体验 | 改产品行为/加功能/确认领域语义时**查**；产品决策变了才**改** |
| `ER.md` (~33KB) | 数据对象与关系、表清单、schema 设计决策 | 改 store/数据流时**查**；schema 变更必须同步**改** |
| `COLOR.md` (4KB) | 配色语义 token + 用法速查 | 选色/改样式时**查**；加/改 token 才**改** |

> `PRODUCT.md`/`ER.md` 较大：用 `Read` 的 `offset/limit` 读下面目录里的**对应章节**，不要整篇载入。

### 章节速查
- **PRODUCT.md**：§1 定位 · §2 模块边界 · §3 产品原则 · §4 工作对象（4.1 Workspace / 4.2 Chat / 4.3 Message / 4.4 Run / 4.5 Tool / 4.6 Approval / 4.7 Review / 4.8 Artifact / 4.9 Research / 4.10 Data / 4.11 Skill / 4.12 Attachment）· §5 桌面体验（5.1 三栏 / 5.2 左导航 / 5.3 对话区 / 5.4 右上下文 / 5.5 配色 / **5.6 设置:Provider/模型/登录**）· §6 Agent 工作流 · §9 路线
- **ER.md**：§2 关系总览 · §3 命名约定 · §4 对象（4.1 Workspace … 4.8 Approval Request / 4.9 Review Changeset / 4.10 Review File Change（含 **Shadow Review 扩展**：`review_snapshots` 表 + changeset/file_change 扩展列）… 4.20 Object Reference）· §5 第一版表清单 · §6 关键设计决策（**6.8 影子仓「上一轮变更」** / **6.9 Provider/模型/登录配置**）

> **Shadow Review**（Run 级「上一轮变更」）的产品语义见 PRODUCT.md §4.7，数据模型见 ER.md §4.10，设计取舍见 ER.md §6.8。改动影子仓 / 快照 / changeset 相关代码（`src-tauri/src/shadow_review/`、`store/review_snapshots.rs`）前先读这三处。

> **Provider / 模型 / FutureGene 登录**：产品行为见 PRODUCT.md §5.6，存储与登录实现见 ER.md §6.9，字段校验见 PLAN.md「自定义 Provider 字段校验」。改动 `agent_providers.rs` / `auth_store.rs` / `future_login.rs` / `commands/login.rs` 前先读这几处。

## 代码结构（`src/`）

- `components/layout/` — `AppShell`（布局编排）+ `ContextPanel`；`hooks/` 放 AppShell 的领域 hook：`useThreadStore` / `useAgentConnection` / `useApprovals`
- `components/ui/` — 通用展示组件（`Badge` / `DiffView` / `CopyablePre` / `TextInput` / `Select` / `Button` / `Overlay` …），无业务逻辑
- `features/{agent,review,runs,artifacts,research,settings,markdown}/` — 按域的业务组件
- `integrations/` — 与 Tauri 后端的边界：`tauri/invoke.ts`（typed invoke 唯一入口）、`agent/`、`storage/`（`threadStore.ts` 是 barrel，域模块在同目录）
- `lib/` — 无业务依赖的工具：`usePolling` / `useAsyncResource` / `futureEvents`（typed event bus）/ `cn` / `clipboard` / `date` / `platform` / `useDismissableLayer` / `windowDrag`

## GUI 开发原则（长期记忆）

1. **颜色**：只用 `COLOR.md` 的语义 token，不写裸 Tailwind 色（`blue-300`…）。状态徽章用 `<Badge tone>`；分类色（事件类别 / 错误子类型）是有意例外。
2. **Tauri 调用**：所有 `invoke` 统一走 `integrations/tauri/invoke.ts` 的 `invokeCommand`，不要直接 `invoke`。命令参数：结构化输入用 `{ input }`，单标量用具名键。（其他 `@tauri-apps/api` 能力——事件 `listen`、dialog、`convertFileSrc`、window/webview——`invoke.ts` 不封装，可按需直接 import。）
3. **跨组件事件**：用 `lib/futureEvents.ts` 的 typed `emitFutureEvent` / `onFutureEvent`，不裸用 `window` CustomEvent。
4. **异步 / 轮询**：取消安全的加载用 `lib/useAsyncResource`，轮询用 `lib/usePolling`（别手写 `cancelled` flag effect 或 `setInterval`）。轮询里改连接/状态时**不要每个 tick 闪 `checking`**——静默重试，拿到结果才改状态。
5. **AppShell 状态**：按域抽进 `components/layout/hooks/` 的领域 hook，AppShell 只做布局编排；hook 用同名解构暴露给 AppShell 以减少改动面。
6. **数据**：schema 变更同步更新 `ER.md`；前端 store 改动注意后端 `src-tauri/src/store/`（按域分文件）的对应。
7. **后端错误**：Tauri 命令返回 `Result<_, AppError>`（`thiserror`），**序列化为字符串**，前端按字符串处理。后端散色已 token 化/AppError 化，别回退到 `.map_err(|e| e.to_string())`。
8. **审批（v2 文件式 + 三档）**：审批对象是**文件路径访问**，规则存在 `${WS}/.future/approval_rule.json` 与 `~/.future/approval_rule.json`，agent 直接读；GUI 经可信路径代写（`approval_rules.rs` + `commands/approvals.rs`）。审批分三档（`app_settings.approval_tier`：`manual`/`sandbox`(仅 macOS)/`off`），session 建立时经 `set_sandbox_policy { tier }` 下发。`approval_requests` 表保留结构化 `action_payload` / `sandbox_boundary` / `save_suggestion` 字段撑审批卡片。语义见 `APPROVAL_PLAN.md` / `SANDBOX_PLAN.md` / `ER.md §4.8`。（旧 P2 `approval_config.rs` 脚手架与三张预留表已于 2026-07-05 删除。）

## 验证（改完必跑）

```bash
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run
# 涉及 Tauri 后端时另跑：
cd gui/src-tauri && cargo fmt --check && cargo clippy && cargo test
```

GPG 签名在非交互终端会失败；提交用 `git commit --no-gpg-sign`。改色等视觉改动需在 `make run-gui` 里实机确认。
