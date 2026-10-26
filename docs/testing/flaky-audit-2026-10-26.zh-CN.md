# flaky 测试审计，以及仍然活着的那两条（2026-10-26）

一次性记录，不是设计文档。任务是「修复所有的 flaky 测试」；这份记录说明它最终落成了什么——
每一条有记录的 flake 都对着**代码**核对（而不是对着它自己的记录），其中仍然复现的两条在根上修掉，
其余明确留下并写明理由。

## 0. 摘要

| | |
|---|---|
| 起点 | `.future/memory/testing-gotchas.md` 与 `docs/testing/flaky-*.md`（6315 行）里记为未修的 **13** 条 |
| 早前工作已修好 | **5** 条 —— 记录过时了，代码里已经有修复（其中两条还带强制守卫测试） |
| 仍然复现，本次修掉 | **2** 条 —— `mcp_client` 的假 MCP 服务器，和 Desktop bootstrap 的 CLI 探测 |
| 本机不复现 | **1** 条 —— `future-rpc` 的 endpoint 租约竞态（12/12 全绿） |
| 环境问题，不是测试缺陷 | **2** 条 —— macOS PTY 上限、Linux runner 的 `database is locked` |
| 仅 CI、本机无复现 | **3** 条 —— 如实记为未动 |

本次修的两条形状相同：**测试的结果由被测代码之外的东西决定**——一条是继承来的进程环境，
另一条是墙钟。两条都不是靠放宽断言修的，都是靠去掉那个依赖修的。

## 1. 按实测的审计表

「记录里写着未修」不等于「现在还坏」。下表每一条都在树里核对过，不是抄记录。

| 记录中的 flake | 本 tip 的结论 | 证据 |
|---|---|---|
| Desktop 的 HOME↔MOCK 加锁顺序反转（lib 套件整体死锁） | **2026-10-17 已修** | `host/src/remote/test_support.rs:430` 写明了顺序；`settings.rs:420` 是先 MOCK 后 HOME；`no_body_takes_the_home_guard_before_the_mock_lock`（2162 行）扫描 `src/` 全部 191 个文件，任何反序函数体都判红，且自带对照测试 |
| `cli/tests/bin.rs` 的 agent 单例锁竞态 | **已修** | 每个 spawn 的 agent 都有独立 `HOME`（`cli/tests/bin.rs:20-30`，63/85/128/143/147/200 处 `tempdir()`），正是记录里给的「根治方向」 |
| mobile `useSkillRecommendation`「今天已推荐过」（本地午夜） | **已修** | 测试改为 `import { today }`（`skillRecoBudget.ts:32`），用 `getFullYear/getMonth/getDate` 拼**本地**日历日；记录里的 bug（`toISOString().slice(0,10)` = UTC 日）已消失，测试里还留着点明它的注释 |
| `agent/tests/cli_smoke.rs` 的 profile 计时器 vs bind 日志竞态 | **靠消除竞态修好** | `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint` 不再断言日志行的先后；它断言 `~/.future/run` 目录及其 `0o700` 权限，计时器抢不过它 |
| `safari_manager::start_launch_success_against_fake_driver` 的 10s 窗口 | **已放宽** | 就绪 deadline 现在是 20s、外层 timeout 30s（`safari_manager.rs:930-936`） |
| `future-rpc` 的 local endpoint 租约（`AddrInUse`） | **不复现** | 本机 32 核上 `cargo test -p future-rpc --lib` 12/12 全绿（原记录量到 9/30 与 16/30） |
| macOS 的 `openpty: Device not configured` | **环境问题** | `kern.tty.ptmx_max` 触发的 `ENXIO`；测试在创建第 33 个 PTY 时挂，没走到自己的断言。最近 15 次 macOS desktop job 里只挂 1 次；本机 9/9 全绿 |
| `DatabaseBusy: database is locked`（Linux CI） | **环境问题** | 挂在 `TestHome` 构造，即夹具而非被测对象 |
| desktop backend 的 `chunk_pull` 500ms 预算 | **仅 CI，未动** | 本机无复现；记录里「重跑」的建议仍然成立 |

## 2. 活着的 flake 之一——假 MCP 服务器依赖了别的测试正在改写的 `PATH` 里的 `sed`

### 2.1 症状

在 32 核机器上连续两次跑完整 `cargo test -p future-cli --lib`，都是 1398/1399，
挂的永远是同一个测试：

```
---- commands::mcp_client::tests::process::the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest
panicked at cli/src/commands/mcp_client.rs:1032:18:
the handshake is a normal one: "the MCP server 'chatty' failed its handshake:
  the MCP server sent an unreadable frame while answering 'initialize':
  expected value at line 1 column 23
--- server stderr (last lines) ---
stderr-07
...
/bin/sh: line 8: sed: command not found"
```

同一个用例单跑 4/4 全过。同一个构造也出现在
`a_poisoned_stderr_tail_is_tolerated_by_the_drain_and_the_reader` 那条。

### 2.2 根因

假服务器是 `/bin/sh -c` 脚本，它靠管道把帧喂给 `sed` 来取请求 id：

```sh
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
```

`sed` 是从**继承来的** `PATH` 里解析的。`ServerProcess::start`
（`cli/src/commands/mcp_client.rs:350`）spawn 时不 `env_clear`，所以子进程拿到的是
spawn 那一刻对 `environ` 的一份拷贝——而 `std::env::set_var` 在 Rust 2024 里之所以是
`unsafe`，正是因为这份拷贝会与它竞争。libtest 并行跑同一个二进制里的测试，其中好几个会替换
`HOME`/`PATH`/`FUTURE_HOME`（`crate::test_env::EnvGuard`），而
`mcp_client::tests::process` 这个模块**一次都没**取 `lock_env()`（0 处）。拷贝被撕坏、丢了
`/usr/bin` 时，`sed` 就不可解析；于是 id 展开为空，应答变成
`{"jsonrpc":"2.0","id":,"result":{}}`，客户端把它当成不可读帧拒掉——正是日志里那句
`expected value at line 1 column 23`。

### 2.3 修法

这个 crate 里每个假服务器都改用 shell 参数展开取 id（其中一处取 path），因此
**完全不需要外部二进制**：

```sh
id=${line#*\"id\":}
id=${id%%,*}
```

共十处：`cli/src/commands/mcp_client.rs` 各 1+1 处；`cli/src/commands/mcp_servers.rs`
8 处（外加 1 处字符串 `path`）。现在 `grep -rn "sed -n" cli/src` 为空。

### 2.4 证伪

这个依赖可以直接证明：用服务器自己的脚本，配一个没有 `/usr/bin` 的 `PATH`（即拷贝被撕坏的状态）

| 脚本 | `PATH=<空目录>` |
|---|---|
| 旧的（`sed -n …`） | `/bin/sh: sed: command not found` → `id=[]` → 应答不是合法 JSON |
| 新的（参数展开） | `id=[3]` —— 正确，而且完全没有 `PATH` |

（对照很重要：用 `env -i` 即 `PATH` **未设置**时，旧脚本照样能跑，因为 `sh` 会回落到编译期默认
`PATH`。只有「设置了但不对」的 `PATH` 才能复现这个 flake——那正是被撕坏的 `environ` 给的形态，
也正是单跑从不出现的原因。）

| | 修前 | 修后 |
|---|---|---|
| 完整 `cargo test -p future-cli --lib` | 2 次跑 2 次红，1398/1399 | **3 次跑 3 次绿，1399/1399** |
| `-- commands::mcp_`（81 条） | — | 5 次跑 5 次绿 |

## 3. 活着的 flake 之二——Desktop bootstrap 被 3 秒探测时钟决定

### 3.1 症状

在有负载的 32 核机器上跑完整 `cargo test -p futureos --lib`（98s 与 104s，绿的那几次 64–67s），
`skills_bootstrap` 里三个用例同时挂，原因都是同一句：

```
isolated skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin failed:
  FutureOS: CLI .../bin/future: CLI discovery timed out. Install a matching CLI
    beside Desktop or correct PATH; no fallback CLI was executed.; skipping built-in skill bootstrap
  the spawned CLI must have run its `init` (drain loop exit ".../init-ran")
```

### 3.2 根因

发现 CLI 的探测在 `PROBE_DEADLINE`（3s，`host/src/cli_tool.rs:180`）后杀掉子进程。而每个
bootstrap 测试都把 `PATH` 指向夹具目录，然后等那个夹具 shell 脚本回答
`future version --json`——于是测试的结果成了「繁忙机器的调度器 vs 墙钟」的赛跑，而不是 bootstrap
的性质。

这个 crate 其实**早就有**防竞态的 seam，它的文档注释把这个道理讲得一字不差（「一旦有 deadline
参与，那些断言就由繁忙机器的调度器和这个时钟的赛跑决定，而不是由被测代码决定」）。但它被门在
**`#[cfg(test)]`** 下，而 Desktop 是把 `future-host` 当**依赖**编进自己的测试二进制的，那里
`cfg(test)` 为假——于是入口测试够不到这个 seam，成了唯一还在赛跑的地方。

### 3.3 修法

seam 现在与 crate 其他夹具同门：`#[cfg(any(test, feature = "test-support"))]`，
`ProbeDeadlineGuard` 公开并加了 `without_deadline()`。两个断言「探测抓到了什么」的 Desktop 函数体
把它打开：

| 位置 | 为什么需要 |
|---|---|
| `skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin` | 它的断言就是「被发现的 CLI 跑过 `init`」 |
| `skills_bootstrap::block_fixtures::host_fixture` | 三个 case（解释器消失、stop 杀掉活子进程、退出无法确认）都先要探测成功 |

断言 **kill** 的那条——`a_silent_cli_is_killed_and_reported_as_a_timeout`——保留生产预算，
3s 这个常量正是由它钉住的。生产行为不变：不带 `test-support` 时这个 seam 不存在。

它刻意仍是 **thread-local**：libtest 每个测试一个线程，且 bootstrap 的正文跑在 re-exec 出来的
子进程里，所以在正文里打开它够不到别的测试的探测（`docs/testing/flaky-tauri.md` 第 27 条就是
这个泄漏类）。

### 3.4 证伪

把「无 deadline」临时换成 1ns deadline，**逐字**复现了原始 CI 失败——同样的两行、同一个断言：

```
test skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin ... FAILED
  FutureOS: CLI .../bin/future: CLI discovery timed out. ...; skipping built-in skill bootstrap
  the spawned CLI must have run its `init` (drain loop exit ".../init-ran")
```

换回 `without_deadline()` 即绿。这证明该测试原本由时钟决定，而这次改动是去掉那个依赖，
不是把它藏起来。

| | 修前 | 修后 |
|---|---|---|
| 完整 `cargo test -p futureos --lib` | 6 次跑 2 次红，每次 3–4 个 `skills_bootstrap` 失败 | **3 次跑 3 次绿，365/365** |
| `-- skills_bootstrap::`（12 条） | — | 3 次跑 3 次绿 |

## 4. 刻意没有改的

- **macOS 的 `openpty: Device not configured`**（§1）。`ENXIO` 是 `kern.tty.ptmx_max`，系统级
  上限：该测试故意持有 32 个 PTY，死在第 33 个，没走到断言。最近 15 次 macOS desktop job 挂 1 次、
  本机 9/9 全绿；要修得知道 runner 的上限或把 PTY 密集测试串行化，两者在这台机器上都拿不到证据。
- **`future-rpc` 的 endpoint 租约。** 本机 12/12 全绿，没有可对着修的对象——改了就是猜。原记录的
  测量仍然作为历史成立。
- **`database is locked`、500ms chunk-pull 预算。** 仅 CI、本机无复现，任何改动都是推测。
- **`mcp_servers` 的加锁方式。** 它的测试彼此之间一致地取 `lock_env()`；脚本改动已经让它们对
  `PATH` 那一半免疫，而在没有复现的情况下做整套环境串行化重构并不划算。

### 4.1 需要说明的边界

让假服务器不依赖 `PATH` 修的是**这些**测试。它没有修掉那个普遍隐患：CLI 测试二进制会一边改
进程级环境、一边 spawn——`std::env::set_var` 在 Rust 2024 里是 `unsafe` 的原因就是这个，而
`mcp_client::tests::process` 至今不取 `lock_env()`。将来那里若出现「spawn 出来需要某个继承变量的
东西」，同一个洞会再开一次。根治方向是测试不再改环境（像 `ServerSpec.env` 已经允许的那样把值传进
spec），或把每次 spawn 都串行到 `lock_env()` 之后；两者都比这份记录大。

## 5. 改动的文件

| 文件 | 改动 |
|---|---|
| `cli/src/commands/mcp_client.rs` | 两个假服务器改用参数展开取 id，不再用 `sed` |
| `cli/src/commands/mcp_servers.rs` | 八个假服务器同样处理，外加 `path` 的提取 |
| `host/src/cli_tool.rs` | 探测 deadline 的 seam 在 `test-support` 下也可用；`ProbeDeadlineGuard` 公开并加 `without_deadline()` |
| `desktop/src-tauri/src/skills_bootstrap.rs` | 两个断言「抓到了什么」的函数体打开该 seam |

## 6. 环境

| | |
|---|---|
| 机器 | macOS 26.6.2 (25G83)，Apple silicon，32 核 |
| 基线提交 | `f7ed9bb84`（`origin/dev-huichen`）；分支 `fix/flaky-tests` |
| 工具链 | 经 `rustup` 安装的仓库 pin `1.99.1`（Homebrew 的 1.96 未进 `PATH`） |
| 测试环境 | `FUTURE_VERSION=0.0.2-flaky`、`ulimit -n 10240` |
| 检查 | `cargo fmt -p future-host -p future-cli -p futureos -- --check`；`cargo clippy --locked -p future-host -p future-cli -p futureos --all-targets -- -D warnings`；`cargo test -p future-cli --lib` ×3；`cargo test -p futureos --lib` ×3；`cargo test -p future-rpc --lib` ×12 |
