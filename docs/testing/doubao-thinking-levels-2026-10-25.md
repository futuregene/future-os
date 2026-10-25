# doubao 思考档位的真机验证（2026-10-25）

一次性验证记录，不是设计文档。目的是在真机上验证 `04ce17fa6`（`fix(agent): doubao can
neither be asked to stop thinking nor sent "medium"`，经 `fbd9f7b28` 合入 `dev-huichen`）
对 `agent/src/models/future.rs` `derive_thinking_compat()` 的改动，并留下可复核的请求体与响应。

## 0. 结论

| 项 | 结果 |
|---|---|
| 新代码（`ab3dcc538`）`medium` / `off` / `high` 各一条真机调用 | **三次都成功**，返回 `好`；线级字段与设计一致（`medium → high`，`off` 只发 `thinking: {"type": "enabled"}`，不发 `reasoning_effort`） |
| 旧代码（`9b13e0919`）`medium` 上复现那条 400 | **没有复现**：同二进制、同平台目录、同样的 `reasoning_effort: "medium"`，7 次调用全部 HTTP 200 |
| deepseek-flash 同档位对照（与 doubao 共用 `thinkingFormat: deepseek`） | 新旧二进制发出的请求体**逐字段一致**（`reasoning_effort: "medium"`、`thinking: enabled`），两次都成功 |
| 平台目录是否已声明 `reasoning_effort_values` | **没有**；11:59 的缓存与 20:48 重新拉取的目录除 `BaseURL` 外完全相同（§6） |

也就是说：修法的**线级效果**可以直接验证并且是好的；但"旧代码必然 400"这条在当前平台上
**不成立**——那条 400 是 20:09 的真实记录，20:48 之后同一请求被服务端接受。证据链见 §5，
这是本次派工唯一没能按原样交付的验收项。

## 1. 环境

### 1.1 两份二进制，同一次构建配方

| | 旧（复现目标） | 新（待验证） |
|---|---|---|
| 提交 | `9b13e0919`（`fbd9f7b28` 的第一父提交） | `ab3dcc538`（`dev-huichen` 尖端） |
| worktree | `/Users/geilige/future-os/.worktrees/verify-doubao-old` | `/Users/geilige/future-os/.worktrees/docs-doubao-verify` |
| `future version --json` 的 `gitCommit` | `9b13e0919cf6…` | `ab3dcc5385c8…` |
| 二进制 | `<worktree>/target/debug/future` | 同左 |
| 代码里有没有 `thinkingRequired` | 没有（`strings` 计数 0） | 有（`strings` 计数 1） |

```bash
export PATH="$HOME/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH"   # 绕开 Homebrew 的 1.96 遮蔽
export FUTURE_VERSION=0.0.2-doubao-verify                                     # 否则 agent 侧版本是 0.0.0，客户端与 agent 对不上
cargo build -p future-cli                                                     # 两份都是 1m18s
```

`FUTURE_VERSION` 必须显式给同一个值：`packages/rpc/build.rs` 把它编成
`EXPECTED_AGENT_VERSION`，缺省回落到 `0.0.0`，客户端会拒绝连接。两份二进制用同一个
字符串，客户端就能分别连两台 agent。

### 1.2 隔离实例与观察点

| | 监听 | `FUTURE_HOME` |
|---|---|---|
| 旧 agent | `127.0.0.1:18941` | `/tmp/doubao-verify/home-old` |
| 新 agent | `127.0.0.1:18942` | `/tmp/doubao-verify/home-new` |
| 记录代理 | `127.0.0.1:18931` → `https://future-os.cn` | 日志 `/tmp/doubao-verify/requests.jsonl` |

- 凭据是**复制**的：`~/.future/agent/auth.json` → `<home>/agent/auth.json`，只把
  `future.base_url` 改成 `http://127.0.0.1:18931/api`（指向记录代理）。`models.json` 同样复制
  （让 westlakehpc 等自定义 provider 在隔离实例里也可解析）。
- **没有**复制 `~/.future/agent/.future-models-cache.json`。那份缓存里存的是上一次抓取时
  由**某个二进制**推导好的 `compat` / `thinkingLevelMap`；复制进去，被测二进制就会读到别人的
  推导结果，验证的就不是它自己的代码了。隔离实例启动时自己重新拉一次平台目录（走代理）。
- 请求体是在**平台边界**用一次性反向代理记录的：代理把请求原样转发给 `https://future-os.cn`，
  落一行 JSON（`Authorization` 已替换成 `<redacted>`，其余字段原样）。脚本不提交进仓库
  （一次性工具），判据是下面 §3 里贴的字段。
- 每条 `future run` 还会额外触发一次"生成会话标题"的请求（`deepseek` 格式、`low`）。下表只
  记录**主请求**，标题请求的字段另作说明。

## 2. 四次主命令与实际输出

旧 agent（`/tmp/doubao-verify/home-old`，端口 `18941`）：

```bash
FUTURE_HOME=/tmp/doubao-verify/home-old FUTURE_AGENT_GRPC_ADDR=127.0.0.1:18941 \
  .worktrees/verify-doubao-old/target/debug/future run --no-tools --permission none \
  --model future/doubao-seed-2.1-pro --thinking medium "只回复一个词：好"
```

```
好
```

新 agent（`/tmp/doubao-verify/home-new`，端口 `18942`）：

```bash
FUTURE_HOME=/tmp/doubao-verify/home-new FUTURE_AGENT_GRPC_ADDR=127.0.0.1:18942 \
  .worktrees/docs-doubao-verify/target/debug/future run --no-tools --permission none \
  --model future/doubao-seed-2.1-pro --thinking medium "只回复一个词：好"
# → 好

… --thinking off  "只回复一个词：好"   # → 好
… --thinking high "只回复一个词：好"   # → 好
```

四条命令都没有 `Error:` 行，进程退出码 0；隔离 agent 的 `runs` 表里对应四行全是
`completed`（`duration_ms` 1913–2942，一次性提交的短回复）。

单测在这份 worktree（`ab3dcc538`）上重跑了一遍，作为上面的语义基线：
`cargo test -p future-agent --lib -- llm:: models::` → **337 passed**，其中
`doubao_maps_every_level_into_the_values_it_accepts` 与
`a_model_that_always_thinks_is_never_asked_to_stop` 均通过。

## 3. 线级请求体（同一张表看新旧差异）

代理记录的 `POST /api/v1/chat/completions`（`messages` 省略，其余字段原样）：

| 时间 | 二进制 | 模型 | 档位 | 发出的思考字段 | 结果 |
|---|---|---|---|---|---|
| 20:48:24 | 旧 | doubao-seed-2.1-pro | medium | `reasoning_effort: "medium"` + `thinking: {"type":"enabled"}` | **200**，`好` |
| 20:49:05 | 旧 | doubao-seed-2.1-pro | off | `thinking: {"type":"disabled"}`（无 effort） | **200**，`好` |
| 20:52:39 | 旧 | doubao-seed-2.1-pro | minimal | `reasoning_effort: "minimal"` | **200**，`好` |
| 20:53:28 / :57 / 20:54:18 | 旧 | doubao-seed-2.1-pro | medium ×3 | 同第一行 | **200** ×3，`好` |
| 20:56:27 | 旧 | doubao-seed-2.1-pro | medium（工具开启、原句 `用一个词回答：可以`） | 同第一行 | **200**，`可以。` |
| 20:55:00 | 新 | doubao-seed-2.1-pro | medium | `reasoning_effort: "high"` + `thinking: {"type":"enabled"}` | **200**，`好` |
| 20:55:19 | 新 | doubao-seed-2.1-pro | off | `thinking: {"type":"enabled"}`（**无** `reasoning_effort`） | **200**，`好` |
| 20:55:40 | 新 | doubao-seed-2.1-pro | high | `reasoning_effort: "high"` | **200**，`好` |
| 20:56:06 | 新 | deepseek-flash | medium | `reasoning_effort: "medium"` + enabled | **200**，`好` |
| 20:56:12 | 旧 | deepseek-flash | medium | `reasoning_effort: "medium"` + enabled | **200**，`好` |

新旧两处差异正是修法要的东西：

- `medium`：旧发 `"medium"`（火山词表里没有这个值），新发 `"high"`；
- `off`：旧发 `{"type":"disabled"}`，新改发 `{"type":"enabled"}` 且不给深度提示。

完整的主请求（旧、medium，`messages` 省略）：

```json
{"max_tokens": 32768, "model": "doubao-seed-2.1-pro", "reasoning_effort": "medium",
 "stream": true, "stream_options": {"include_usage": true}, "thinking": {"type": "enabled"}}
```

新、off：

```json
{"max_tokens": 32768, "model": "doubao-seed-2.1-pro",
 "stream": true, "stream_options": {"include_usage": true}, "thinking": {"type": "enabled"}}
```

## 4. deepseek-flash 对照

同一档位 `medium`，把模型换成 `future/deepseek-flash`（与 doubao 共用 `thinkingFormat: deepseek`）：

| 时间 | 二进制 | 请求体（除 `messages`） | 结果 |
|---|---|---|---|
| 20:56:06 | 新 | `{"max_tokens": 65536, "model": "deepseek-flash", "reasoning_effort": "medium", "stream": true, "stream_options": {"include_usage": true}, "thinking": {"type": "enabled"}}` | 200，`好` |
| 20:56:12 | 旧 | 与上一行**逐字段相同** | 200，`好` |

标题请求（两者都是 `reasoning_effort: "low"` + enabled）在新旧二进制下也一致，所以
`thinkingRequired` 默认 false 这一点没有波及 deepseek。

## 5. 为什么没复现出 400

那条 400 是**已有的真实记录**，不是我这次跑出来的（`~/.future/agent/agent.db`，只读查询）：

```
session 20261025-200944-…, run_3837fa9e2a2e4efd87ec18b472a20f60
status failed, duration_ms 15033, started 2026-10-25 20:09:46
error: LLM API request failed (HTTP 400): 该模型始终思考，不支持关闭思考；请使用 low、high 或 max。
```

今天 20:48–20:57 用**同一份旧二进制**、**同一份平台目录**、**同样的字段**再打，全部 200：

- 平台目录（`GET /api/v1/models`）对 `doubao-seed-2.1-pro` 的条目，11:59 的缓存与 20:48 重新
  拉取的结果**除 `BaseURL`（我改指了代理）外逐字段相同**：`Compat { thinkingFormat: deepseek }`、
  `ThinkingLevelMap { high: high, xhigh: high }`、没有 `reasoningEffortValues`。
  所以不是"缓存过期导致客户端行为变了"。
- 请求体与失败那次同形（`reasoning_effort: "medium"`、`thinking: enabled`、`max_tokens: 32768`、
  `stream: true`），`off` 的 `disabled` 写法也被接受。
- 7 次调用（`off` ×1、`minimal` ×1、`medium` ×4、`medium`+工具 ×1）没有一次 400，
  说明不是间歇性负载均衡。

因此差异只能落在服务端：要么这 40 分钟里平台改了 doubao 的路由/校验（失败那次 15 秒才返回，
更像服务端处理后才拒，而不像参数校验的立即拒绝），要么那次是一次服务端瞬时故障。
平台侧的变更记录我这边看不到，无法在两者之间定案。

还有一个细节要说清楚：**失败那一刻用的到底是哪个模型，存储里查不到**。
`run_snapshots` / `run_events` / `history_shapes` 都不记录模型；该会话当前的 metadata 是
`westlakehpc/glm-flash`，而 `run_events` 里第一次 `model_changed` 到 `doubao-seed-2.1-pro`
发生在失败**之后**（20:10:22），之后还有 7 次模型/档位切换。桌面库 `~/.future/app/app.db`
里同一个 `run_id` 的那行写的是 `completed / westlakehpc / glm-flash`，与 agent 库相反，
也不能当判据。"失败模型是 doubao"这条依赖的是当时对模型缓存的判断，不是运行台账。

## 6. 需要拍板：映射该由平台目录声明吗

`convert_future_model` 已经支持平台在目录里声明 `reasoning_effort_values`，声明了就**丢掉**
tokenizer 启发式、由 `resolve_effort` 按词表收敛。今天 `doubao-seed-2.1-pro` 的条目
**没有**这个字段（§5 已对照两次抓取），所以客户端的 `derive_thinking_compat` 是唯一的映射来源。

如果认为这件事应该由平台负责，那需要平台同时给两样东西：

1. `reasoning_effort_values: ["low", "high", "max"]` —— 让 `medium`/`minimal` 收敛到词表内的值；
2. 一个"关不掉思考"的声明（当前客户端侧叫 `compat.thinkingRequired`）—— 否则 `off` 仍会落到
   `thinking: {"type": "disabled"}` 这条未必被服务的分支上。

这是平台侧 / 目录契约的改动，**我没有动**，等用户决定。

## 7. 复现步骤

验证用的两个 worktree 在合入后已经按流程删掉了；照着下面重新建即可
（`git worktree add <dir> 9b13e0919`，以及 `git worktree add <dir> -b <branch> origin/dev-huichen`）。

```bash
# 1. 两份二进制（同一 FUTURE_VERSION），见 §1.1
# 2. 隔离 home：复制 auth.json + models.json，把 future.base_url 指到记录代理
mkdir -p /tmp/doubao-verify/home-{old,new}/agent
cp ~/.future/agent/auth.json ~/.future/agent/models.json /tmp/doubao-verify/home-old/agent/
cp ~/.future/agent/auth.json ~/.future/agent/models.json /tmp/doubao-verify/home-new/agent/
#    （把两份 auth.json 的 future.base_url 改成 http://127.0.0.1:18931/api；
#     想直连平台就跳过代理，直接打 https://future-os.cn/api）

# 3. 起两台 agent（客户端一律用 FUTURE_AGENT_GRPC_ADDR，别落到宿主 agent 上）
FUTURE_HOME=/tmp/doubao-verify/home-old \
  .worktrees/verify-doubao-old/target/debug/future agent --grpc-addr 127.0.0.1:18941 &
FUTURE_HOME=/tmp/doubao-verify/home-new \
  .worktrees/docs-doubao-verify/target/debug/future agent --grpc-addr 127.0.0.1:18942 &

# 4. 等目录到位（启动时后台拉一次 /api/v1/models）
FUTURE_HOME=/tmp/doubao-verify/home-new FUTURE_AGENT_GRPC_ADDR=127.0.0.1:18942 \
  .worktrees/docs-doubao-verify/target/debug/future models --json | grep doubao-seed-2.1-pro

# 5. 发请求（§2 的四条命令）
```

坑位（都踩过）：`future agent --grpc-addr` 只开 TCP，不建 IPC socket，所以客户端必须同时给
`FUTURE_AGENT_GRPC_ADDR`；客户端漏了这个变量会回落到 `~/.future` 的宿主 agent。
`future models` / `future run` 这些子命令不认 `--grpc-addr` 时，靠环境变量兜底。

## 8. 没有触碰的东西

- 宿主 agent（`127.0.0.1:5423`）与已安装的 `future` 二进制：没有停止、没有替换、没有 `make install`。
- `~/.future` 下没有任何写入：`auth.json` / `models.json` 是复制出去的，`agent.db` / `app.db`
  只做了 `SELECT`。所有会话、日志、模型缓存都落在 `/tmp/doubao-verify/`。
- 别人的 worktree 没有改动；本次只在 `.worktrees/verify-doubao-old`（detached，
  `9b13e0919`）和 `.worktrees/docs-doubao-verify`（`docs/doubao-live-verify`）里工作。
