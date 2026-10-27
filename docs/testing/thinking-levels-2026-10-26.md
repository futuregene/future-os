# thinkingLevels 成为唯一思考档位来源（2026-10-26）

> **2026-10-27 补充**：档位词表不再是“约定”。当模型的访问路径不接受我们的档位拼写时，
> 模型用 `compat.reasoningEffort` 声明它实际接受的值，档位按单调规则映射上去
> （例如 DeepSeek V4 的 `medium → 50`）。`thinkingLevels` 因此读作“模型提供的档位”，
> 未声明词表的模型行为与本篇完全一致。见
> [reasoning-effort-vocabulary-2026-10-27.md](reasoning-effort-vocabulary-2026-10-27.md)。

一次性实现与验证记录。设计结论：**`thinkingLevels` 就是 wire 值列表**；`default` 是隐式的
第一档，表示“不发送任何 thinking/effort 参数”；`off` 表示显式关闭；其余值原样发送。
`compat.reasoningEffortValues`、`thinkingLevelMap`、`supportsReasoningEffort`、
`thinkingRequired`、`resolve_effort` 与 tokenizer 档位启发式全部删除；`reasoning` 字段保留，
但运行期不再参与任何决策。

## 1. 契约

- 生效列表 = `["default"] + normalize(declared)`；`normalize` 做 trim、去空、去重、去掉保留的
  `default`、按固定展示顺序排序（off → minimal → low → medium → high → xhigh → max，未知 wire 值排在已知值之后并保持声明顺序）、上限 16 项 / 每项 32 字符。
- 未声明（缺失或空数组）= 只有 `default`；客户端在 `thinkingLevels` **字段缺失**时回退旧六档，
  且绝不向旧 agent 发送字面量 `default`。
- 存储/请求：不在列表内的档位收敛为 `default`（不再按深度就近吸附）。
- `reasoning`：仍出现在 `models.json`、`list_models`、proto 中给旧客户端，行为零依赖。
- 自定义 provider 的写入路径用 presence 位区分“老客户端未提供（保留）”与“显式清空”。

## 2. 数据

- future-server `config/models.yaml`：平台模型从上游继承 `thinking_levels`（17 个平台模型 /
  18 条上游声明）。doubao 为 `["low","high","max"]`（无 `off`，服务端拒绝关闭思考），
  取代原来的 `thinkingRequired` + 逐档映射启发式。
- 内置目录：`reasoning=true` 的 2406 条声明 `["minimal","low","medium","high"]`（即历史上实际
  发送的标准值），`reasoning=false` 不声明；删除 `tlm_json`。

## 3. 验证结果

| 范围 | 命令 | 结果 |
|---|---|---|
| agent 单元 | `cargo +1.97.0 test -p future-agent --lib` | 2478 passed；13 个失败全部是 seatbelt/sandbox 与审批测试（当前 harness 沙箱禁止 `sandbox-exec`，模块未被本改动触碰） |
| agent 集成 | `cargo test -p future-agent --test cli_smoke --test service_startup --test http_client --test sqlite_startup --test sqlite_storage --test session_load_test --test compaction_persist` | 全部通过（25+1+1+15+1+3+15） |
| agent clippy/fmt | `cargo +1.97.0 clippy -p future-agent --all-targets -- -D warnings`、`cargo fmt -p future-agent` | clean |
| 生成器 | `python3 scripts/tests/test-generate-models.py` | 7 tests OK（含 thinking_levels 规则、回填幂等） |
| future-server | `cargo test -p future-api-service` | 245 passed, 0 failed, 1 ignored |
| host | `cargo +1.97.0 test -p future-host agent_providers` | 60 passed |
| desktop | `npm run lint`（eslint+tsc）、`npx vitest run` | tsc/eslint clean；382 files / 5301 tests passed |
| mobile | `npm run typecheck`、`npm run lint`、`npx jest --runInBand` | clean；150 suites / 2895 tests passed |
| remote-protocol | typecheck + vitest + eslint | 62 tests passed，clean |
| TUI | `cargo +1.97.0 test -p future-tui` | 1210 passed；19 个失败全部是 PTY/terminal（沙箱禁止 `openpty`，terminal 模块未改动） |
| CLI | `cargo +1.97.0 test -p future-cli` | 库/单元测试全过；2 个 config-wizard 集成测试因 PTY 被沙箱拒绝 |
| channels | `cargo +1.97.0 test -p future-channel` | 全部通过（1503 lib + 集成） |
| 跨 crate | `cargo +1.97.0 check --workspace --exclude futureos --all-targets` | Finished，无 error/warning |
| proto | `REGENERATE_PROTO=1 cargo build -p future-rpc` | 生成物与 proto 一致（无新增 diff） |
| 端到端（隔离 home） | `future agent --grpc-addr 127.0.0.1:18999` + `future models --json` | 自定义模型 `["low","high"]` → `thinkingLevels: ["default","low","high"]`；未声明 → `["default"]` |

环境限制（不是代码问题）：

- 当前 harness 沙箱禁止写 `~/.cargo` 未解包目录、`~/.npm/_cacache`、`~/.expo`、`sandbox-exec`
  和 PTY；以上失败项都来自这些限制。
- `futureos`（Tauri GUI crate）无法在本沙箱编译：需要把 `block-buffer` 解包进 `~/.cargo`，
  被沙箱拒绝；`desktop/src-tauri` 本次未改动，`future-host` 编译/测试/clippy 均通过。

## 4. 兼容与发布顺序

1. **future-server 先上线**（`/v1/models` 带 `thinking_levels`），再发客户端。旧 agent 忽略新字段。
2. 旧客户端：新 agent 仍返回 `reasoning`；旧客户端按自己的六档过滤声明列表。
3. 旧 `models.json`：`thinkingLevelMap` / `compat.reasoningEffortValues` 等被忽略；未声明
   `thinkingLevels` 的自定义模型只剩 `default`，需在 Providers 页重新选择（接受的破坏性变更）。
4. 新会话默认 `default`；存量档位不在列表时收敛为 `default`。
5. 模型选择器打开会触发 Future 目录后台刷新（TTL 10 分钟），旧缓存最多在下次打开时更新。

## 5. 已知限制

- 只有开关、没有深度参数的协议（`zai`、`reasoning-split`）中，档位只表达“开/关”；
  数据应只声明该协议能表达的档位。
- Anthropic 手动模式的 budget 仍有固定映射（`max`→24000、未知非空值→8000、`default`/`off`→0）。
- 自定义 provider 编辑器提供已知集合（off…max）；手写 `models.json` 允许任意 wire 值。
