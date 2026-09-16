# cli-rust-port——最终验收报告（P4-final）

> 本文是历史快照 [cli-rust-port — Final Acceptance Report (P4-final)](./cli-rust-port-acceptance.md)（2026-08-07，commit `d43460c1`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

> 历史迁移快照：下方结果、CLI 帮助奇偶与源路径适用于记录的候选/日期，不适用于后续版本。当前 CLI 默认值与帮助已经演进（包括 IPC 与工具权限）；请查阅 [CLI 指南](../../wiki/en/CLI.md) 并对照当前源码重新验证。

状态：**已验收**——于 commit `1069f227`（分支 `claude/cli-rust-port`，`origin/main` 的合并，含 PR #112 typed-RPC 线上契约）上于 2026-08-07 验证。
范围：`cli/`（TS）→ `cli/`（Rust crate，bin `future`）的 1:1 TypeScript→Rust 移植；
TS CLI 在本 PR 中退役，`cli/rust/*` 提升为 `cli/`。
参数 / 帮助文本 / 输出 / 退出码字节一致，带移植的单元测试 + 一个 golden 测试工具（Rust CLI vs 记录的 TS golden）。

> 合并后注记（PR #112 集成）：`cli/` 现在从 `future-rpc` crate（#112 引入的唯一 proto 代码生成所有者）消费生成的 proto 代码，而不是拥有自己的生成副本——`src/generated/` 与 build.rs 的 proto 部分已移除；`rpc.rs` 导入 `future_rpc::proto`。已对新的 typed-payload agent 验证线上兼容（163/163 diff，含 live-gRPC `agent` 场景）。

---

## 1. 验证结果（全部门禁绿）

| 门禁 | 命令 | 结果 |
|---|---|---|
| Golden 工具（Rust vs 记录 TS golden） | `make test-cli-diff`（`cli/tests/diff-ts-rust.sh`） | **163 通过 / 0 失败 / 0 跳过** |
| cli-rust 单元测试 | `cargo test -p cli-rust` | **194 通过 / 0 失败** |
| Workspace 测试 | `cargo test --workspace` | **1729 通过 / 0 失败**（含 future-rpc） |
| Workspace clippy（CI 标志） | `cargo clippy --workspace --all-targets -- -D warnings`（rustup 1.97.0） | 干净 |
| 格式 | `cargo fmt --check` | 干净 |
| TS CLI typecheck | `npx tsc --noEmit`（经 `make lint-cli`） | 干净（未改 TS） |

差分工具以 `FUTURE_VERSION=0.0.0-diff+local` 重建**两个** CLI（TS 构建镜像 `make build-cli`：`npm run gen-version` + `npm run build` + `bun build --compile`），从隔离的 `$WORK` 副本运行两者（无 `future-agent` 同级，免疫并发重建冲突），先做烘焙版本健全检查，然后按 argv 逐字节比较 stdout / stderr / 退出码。

### 语料清单（163 用例，15 场景）

| 场景 | 用例数 | 覆盖 |
|---|---|---|
| `static` | 55 | 全部帮助文本（两种 auth 组帮助变体）、`--version`/`-v`/`version`、虚假组、run 解析怪癖（no-prompt 退出 1、invalid-flag 退出 0） |
| `home:none` / `home:auth` / `logout` / `home:badjson` | 13 | 各种 auth.json 状态下的 auth 状态/凭据/登出、账户 profile/balance（含损坏 JSON 回退） |
| `http` | 26 | 账户 profile/balance、`auth login --url`/`--url=`、针对 mock MCP (SSE) 的 tools list/describe/call（含验证 + 错误翻译）、`init`（空目录） |
| `http:errors` | 2 | 401 错误正文翻译 |
| `init:linked` / `init:blocked` | 2 | 幂等重链接 + 被阻止的 `.future/bin/future` 路径 |
| `agent` | 17 | 专用 gRPC 端口上的真实 `future-agent`：agent status/models/session list/info/rename/delete（+`--json`） |
| `doctor` | 1 | 完全受控环境（假 bin 目录、死 gRPC） |
| `skills` / `skills:installed` | 13 | mock 目录 + 确定性下载 zip 上的 list/install/install-builtin/uninstall/update |
| `agentdown` | 2 | 死 gRPC 端口，**stderr-前缀** 模式（见 §2） |
| `browser` | 32 | mock CDP 端点 + 脚本化 WebSocket：status/start(already-running)/tabs(select/new/close)/open/snapshot/click/type/press/screenshot/scroll/console |

语料累计增长：90（P4）→ 131（+P2 run/tools/skills/mcp）→ 163（+P3 browser CDP）。

---

## 2. 最终分歧列表（已知 / 已接受）

除传输错误*措辞*（网络栈相关，与 P4 第 1 轮发现同类）外，字节一致处处成立。全部记录在 `cli/src/rpc.rs`（模块文档）与工具头部。

1. **gRPC 传输错误文本**——agent 宕机时 tonic（"transport error"）vs grpc-js（"14 UNAVAILABLE: …"）。仅 agent-down 路径受影响；工具的 `agentdown` 模式对这两个用例比较退出码 + stdout 字节 + stderr **前缀**（`Error:`）。其余全部为 `exact`。
2. **HTTP 传输错误文本**——不可达 `--url` 上 reqwest vs node-fetch 措辞。语料未覆盖（所有 `auth login --url` 用例指向可达 mock）；按设计排除。
3. **对不可达端点的 `browser status`**——Bun（"Unable to connect…"）vs reqwest 措辞。语料改为通过 `snapshot --endpoint http://127.0.0.1:1` 覆盖固定消息的 `ensureBrowser` 错误（字节一致），以及 already-running `status` 路径（字节一致）。

### 按设计排除在语料外（工具头部文档化）

- 对 live agent 的 `future run`——`agent_end` 携带可变 `duration_ms` / 事件 id（非确定性）；本地 run 路径与死 agent 传输错误**已**覆盖。
- `browser start` 拉起真实 Chrome——非确定性；只对 already-running 路径做 diff。
- 实时网络技能下载——以确定性 mock zip 替代。

---

## 3. 交付内容（`claude/cli-rust-port` 上的提交，领先 origin/main）

- `10462696` P0 脚手架：`index.ts main()` 的 1:1 分发、逐字帮助文本（两种 auth 组帮助变体）、谓词 + 桩、utils/constants/types、Output sinks、tonic/prost gRPC 客户端、17 个单元测试。
- `c891de5a` + `57b59922` P1：rpc.rs + reqwest 上的 init/auth/account/models/agent status/session/doctor；auth.json 辅助；test_env ENV_LOCK；help golden 测试；`preserve_order`；58 个单元测试。
- `0092147c` + `e79bb960`（+`caebf63d` 脚手架）P4：diff 工具 + mock 平台服务器；make 目标 `test-cli-diff`；90/90。
- `d7bc948f` P2：run/tools/skills/mcp 主体（流式事件、SSE MCP、工具目录、错误翻译、浏览器工具面 + 配置 v1→v2）；100 个单元测试；语料 131/131。
- `99ebcaab` P3：浏览器子系统——chromium CDP WebSocket 会话后端 + Safari 路径 + 选择器/输入/截图；语料 163/163。
- `d5fa284a` 语料细化：`snapshot --endpoint <unreachable>` 覆盖固定消息的 ensureBrowser 错误（status 传输措辞分歧）。
- `1069f227` `origin/main` 合并（#112 typed-RPC）：cli 消费 `future-rpc` crate（生成 proto 退役），Makefile `test-cli-diff` 获得 `node-workspace` 前置（根 npm install + 为 TS 侧构建 future-rpc/ts——两者均在 2026-08 仅 Rust pass 中退役），验收数字刷新（1729 workspace）。

## 4. 重跑门禁

```bash
make test-cli-diff            # ~5-7 min（重建两个 CLI）
make test-cli-rust            # cargo test -p cli-rust
rustup run 1.97.0 cargo clippy --workspace --all-targets -- -D warnings
rustup run 1.97.0 cargo test --workspace
```

注：Homebrew cargo 忽略 `rust-toolchain.toml`——总是在 `rustup run 1.97.0` 下运行。在工具外检查 `--version` 输出时，unset/覆盖泄漏的 `FUTURE_VERSION`。
