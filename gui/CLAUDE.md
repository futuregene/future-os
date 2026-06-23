# GUI 开发指南（`gui/`）

FutureOS 桌面应用：Tauri + React + TypeScript，前端 `src/`，Tauri 后端 `src-tauri/`（Rust），通过 gRPC 连到仓库根的 agent。整个 monorepo 的架构/build 见**仓库根 `CLAUDE.md`**；本文件只管 `gui/`。

## 文档地图（按需精读，别整篇拉进 context）

| 文档 | 内容 | 何时查 / 改 |
|---|---|---|
| `PRODUCT.md` (18KB) | 产品定位、模块边界、工作对象语义、桌面体验 | 改产品行为/加功能/确认领域语义时**查**；产品决策变了才**改** |
| `ER.md` (27KB) | 数据对象与关系、表清单、schema 设计决策 | 改 store/数据流时**查**；schema 变更必须同步**改** |
| `COLOR.md` (4KB) | 配色语义 token + 用法速查 | 选色/改样式时**查**；加/改 token 才**改** |

> `PRODUCT.md`/`ER.md` 较大：用 `Read` 的 `offset/limit` 读下面目录里的**对应章节**，不要整篇载入。

### 章节速查
- **PRODUCT.md**：§1 定位 · §2 模块边界 · §3 产品原则 · §4 工作对象（4.1 Workspace / 4.2 Chat / 4.3 Message / 4.4 Run / 4.5 Tool / 4.6 Approval / 4.7 Review / 4.8 Artifact / 4.9 Research / 4.10 Data / 4.11 Skill / 4.12 Attachment）· §5 桌面体验（5.1 三栏 / 5.2 左导航 / 5.3 对话区 / 5.4 右上下文 / 5.5 配色）· §6 Agent 工作流 · §9 路线
- **ER.md**：§2 关系总览 · §3 命名约定 · §4 对象（4.1 Workspace … 4.8 Approval Request … 4.20 Object Reference）· §5 第一版表清单 · §6 关键设计决策

## 代码结构（`src/`）

- `components/layout/` — `AppShell`（布局编排）+ `ContextPanel`；`hooks/` 放 AppShell 的领域 hook：`useThreadStore` / `useAgentConnection` / `useApprovals`
- `components/ui/` — 通用展示组件（`Badge` / `DiffView` / `CopyablePre` / `TextInput` / `Select` / `Button` / `Overlay` …），无业务逻辑
- `features/{agent,review,runs,artifacts,research,settings,markdown}/` — 按域的业务组件
- `integrations/` — 与 Tauri 后端的边界：`tauri/invoke.ts`（typed invoke 唯一入口）、`agent/`、`storage/`（`threadStore.ts` 是 barrel，域模块在同目录）
- `lib/` — 无业务依赖的工具：`usePolling` / `useAsyncResource` / `futureEvents`（typed event bus）/ `cn` / `clipboard` / `date` / `ids`

## GUI 开发原则（长期记忆）

1. **颜色**：只用 `COLOR.md` 的语义 token，不写裸 Tailwind 色（`blue-300`…）。状态徽章用 `<Badge tone>`；分类色（事件类别 / 错误子类型）是有意例外。
2. **Tauri 调用**：统一走 `integrations/tauri/invoke.ts` 的 `invokeCommand`，不直接 import `@tauri-apps/api`。命令参数：结构化输入用 `{ input }`，单标量用具名键。
3. **跨组件事件**：用 `lib/futureEvents.ts` 的 typed `emitFutureEvent` / `onFutureEvent`，不裸用 `window` CustomEvent。
4. **异步 / 轮询**：取消安全的加载用 `lib/useAsyncResource`，轮询用 `lib/usePolling`（别手写 `cancelled` flag effect 或 `setInterval`）。轮询里改连接/状态时**不要每个 tick 闪 `checking`**——静默重试，拿到结果才改状态。
5. **AppShell 状态**：按域抽进 `components/layout/hooks/` 的领域 hook，AppShell 只做布局编排；hook 用同名解构暴露给 AppShell 以减少改动面。
6. **数据**：schema 变更同步更新 `ER.md`；前端 store 改动注意后端 `src-tauri/src/store/`（按域分文件）的对应。
7. **后端错误**：Tauri 命令返回 `Result<_, AppError>`（`thiserror`），**序列化为字符串**，前端按字符串处理。后端散色已 token 化/AppError 化，别回退到 `.map_err(|e| e.to_string())`。
8. **审批 P2 脚手架（预留，未接线）**：结构化 action payload / 沙盒边界 / 自动审批规则引擎是为未来预留的——后端 `src-tauri/src/store/approval_config.rs` + schema 预留表（`sandbox_config` / `approval_policy_config` / `approval_rules`）已就位但未暴露 Tauri command。设计细节见 `ER.md §4.8 Approval Request` 与 git history（原 `P2_APPROVAL_MODEL.md`）。碰审批前先读这些，别误删脚手架。

## 验证（改完必跑）

```bash
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run
# 涉及 Tauri 后端时另跑：
cd gui/src-tauri && cargo fmt --check && cargo clippy && cargo test
```

GPG 签名在非交互终端会失败；提交用 `git commit --no-gpg-sign`。改色等视觉改动需在 `make run-gui` 里实机确认。
