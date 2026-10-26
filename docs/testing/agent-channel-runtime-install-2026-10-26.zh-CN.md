# 同一进程里的两个安装者与一份 Agent 通道 runtime（2026-10-26）

一次性实测记录，不是设计文档：复现、带行号的根因、修法，以及测试覆盖到哪、没覆盖到哪。
下列所有数字都在本机实测；命令都写出来了，每一行都能重跑。

## 0. 摘要

| | |
|---|---|
| 状态 | 每个入口都用 `set` 写同一个进程级 Agent 通道 driver runtime 的 `OnceLock`，**并且**把"已经配过"这个错误包在各自的 `Once` 里 `expect` |
| 效果 | 同一进程里第二个安装者会 **panic**；panic 发生在它自己的 `Once` 内，于是那个 `Once` 被毒化，之后该进程里每一次安装都因与第一次无关的原因失败。Desktop 套件的一次运行因此变成 364 个用例里 **38 个失败** |
| 根因 | `configure_channel_runtime`（`host/src/agent_client.rs:224`）把"已经配过"报成 `Err`；三个调用方——Desktop 入口（`desktop/src-tauri/src/agent_bridge/client.rs:10`）、CLI 入口（`cli/src/headless/mod.rs:841`）、共享测试夹具（`host/src/test_support/agent.rs:336`）——各自在自己的 `Once`/`OnceLock` 里 `expect` 它 |
| 修法 | 在 `future-host` 里提供唯一的幂等安装口，两种结果都算成功（`Installed` / `AlreadyInstalled`）；`OnceLock` 是唯一仲裁者；赢家把 runtime 永久 park，输家关掉自己那份。不再有任何调用方 `expect` 安装结果 |
| 覆盖 | 3 条新测试（`future-host` 2 条，Desktop 入口 1 条） |
| 未覆盖 | 并发竞态本身——测试钉住的是安装**契约**，不是线程调度（见 §5.2） |

这个通道 runtime 问题**不是** CI run
[40262600990](https://github.com/futuregene/future-os/actions/runs/40262600990)
变红的原因。那次红是 clippy 的参数个数 lint（已由随后落地的 `72950dd26` 修掉）和 macOS runner 的 pty 耗尽（§7）。
写这份记录的理由不是解释那次红，而是这一类毒化会让**以后每一次**红都不可信。

## 1. 复现

这个 bug 需要**同一进程里有两个安装者**，而这正是测试二进制的常态：`desktop/src-tauri`
的 dev-dependencies 打开了 `future-host` 的 `test-support` feature（`desktop/src-tauri/Cargo.toml:45`），
所以 `agent_bridge::test_support::mock_agent` 装的夹具（`host/src/test_support/agent.rs`）
与 Desktop 入口自己的安装跑在同一个地址空间里。谁先 `set` 谁赢，另一个 panic。

```bash
# 修前，分支基线 ed012b3a9
ulimit -n 10240                       # 本机软限是 256，不提会让无关的 fd 密集用例假红
cargo test --locked -p futureos --lib
```

| | 修前（`ed012b3a9`） | 修后（本分支） |
|---|---|---|
| `cargo test -p futureos --lib` 运行次数 | 5 | 9 |
| 红 | **3** | 2（都是无关原因，见 §1.2） |
| 单次红里的失败数 | 38 | — |
| 出现通道 runtime panic 或 `Once` 毒化的运行 | **5 次里 3 次** | **9 次里 0 次** |
| 绿 | 2 × 364 通过 | 7 × 365 通过 |

主 panic 与它引发的连锁（取自一次红，原文）：

```
thread 'commands::agent::tests::get_agent_status_forwards_the_agents_phase' panicked at
  desktop/src-tauri/src/agent_bridge/client.rs:10:14:
  entry configures the Agent channel runtime once: Message("Agent channel runtime already configured")

thread 'commands::agent::tests::agent_status_requires_the_running_agent_version_to_match' panicked at
  desktop/src-tauri/src/agent_bridge/client.rs:8:17:
  Once instance has previously been poisoned
...
test result: FAILED. 326 passed; 38 failed; 0 ignored; 0 measured; 0 filtered out
```

38 个里有 37 个带着 `client.rs:8` 的 `Once instance has previously been poisoned`——
**第一次** panic 毒化了 `Once`，之后该进程内每一次入口安装都栽在毒上，而不是栽在真实问题上。

### 1.1 为什么 CI 没见过

抓最近 8 个 `Rust macOS test / desktop` job 的日志（`gh api .../actions/jobs/<id>/logs`），
`previously been poisoned` 与 `Agent channel runtime already configured` **各 0 次**。
GitHub runner 只暴露 3–4 核，本机 32 核：同时起跑的测试线程多得多，两个安装者更容易撞上。
也就是说这是"快机器容易打到"的调度竞态，不是 Desktop 专属、也不是 CI 专属或某台机器的特性。

### 1.2 修后那两次红是另一回事（负载相关）

两次都检查过本修复的特征：`poisoned` 与 `already configured` **各 0 次**。其中一次完整抓下：

```
test skills_bootstrap::tests::the_bootstrap_spawns_the_cli_through_the_shell_plugin ... FAILED
  ... CLI discovery timed out. Install a matching CLI beside Desktop or correct PATH;
      no fallback CLI was executed.; skipping built-in skill bootstrap
  panicked at desktop/src-tauri/src/skills_bootstrap.rs:229:
  the spawned CLI must have run its `init`
```

这是机器有负载时 3 秒的 CLI 发现预算超时（红的两次跑了 98 s / 104 s，绿的 64–67 s），
发生在本次改动没碰的子系统。另一次那 4 个失败的用例名没抓到，同样那两项特征都是 0。

`cargo test -p future-cli --lib` 有一个同族的 flake：两次都是 1399 里 1398 过，失败的永远是
`commands::mcp_client::tests::process::the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest`，
报的是假 MCP 服务（`/bin/sh -c`，管道接 `sed`）那边
`/bin/sh: line 8: sed: command not found`——即子进程继承到的环境丢了 `PATH` 里的条目。
单独用同一个 `PATH` 跑 4/4 全过，所以触发条件是这个子进程 spawn 的同时，套件里别的测试在调
`std::env::set_var`（`cli/src/test_env.rs` 的 `EnvGuard`）：`set_var` 改的正是 `spawn` 会复制的
进程 `environ`，这也正是它在 Rust 2024 里被标成 unsafe 的原因。属既有问题，本次改动不 spawn
任何进程，也没修它。

## 2. 根因

注册表就是一个进程级槽：

```rust
static CHANNEL_RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

pub fn configure_channel_runtime(handle: tokio::runtime::Handle) -> Result<(), crate::Error> {
    CHANNEL_RUNTIME
        .set(handle)
        .map_err(|_| "Agent channel runtime already configured".into())
}
```

`OnceLock::set` 本身是好仲裁：原子，且恰好一个调用方赢。缺陷在于"输家被告知了什么"和"三个调用方拿它做了什么"：

| 安装者 | 修前代码 | 输掉的安装做了什么 |
|---|---|---|
| Desktop 入口 | `client.rs:6-11`：`static INITIALIZED: std::sync::Once` → `configure_channel_runtime(agent_channel_runtime()).expect("entry configures the Agent channel runtime once")` | **在 `Once::call_once` 内 panic** → 该 `Once` 在此后整个进程里都是毒化的 |
| CLI 入口 | `cli/src/headless/mod.rs:838-843`，同一形状 | 同上 |
| 共享测试夹具 | `host/src/test_support/agent.rs:325-344`：`INSTALLED: OnceLock<()>`，`if agent_channel_runtime().is_ok() { return; }`，再 `.expect("fixture installs its channel runtime once")` | 在 `OnceLock::get_or_init` 内 panic |

两个设计错误，都修了：

1. **把"已经安装"当成错误。** 它是第二个安装者的正常结果，报成 `Err` 就等于邀请别人写上面那个 `expect`。
2. **每个安装者在仲裁者前面各自维护了第二套状态机**（`Once` / `OnceLock<()>`）。
   两个状态机管一个槽就会打架——这里是"我的 `Once` 还没跑过"而槽已经满了——打架的结果就是 panic。
   只有注册表就够了。

夹具其实**写对了**那道检查（`if agent_channel_runtime().is_ok() { return; }`）仍然会输：
那道检查和 `set` 不是一个原子步骤，Desktop 的安装挤进中间就会把它 panic 掉。

## 3. 修法

一个口、一个仲裁者、三个调用方同一个形状。生产路径"谁提供 runtime"的语义**没变**：
`future-host` 仍然拒绝构造 fallback，`agent_channel_runtime()` 仍是 `pub(crate)`，
没人安装时仍然返回"Entry must configure the Agent channel runtime…"。
`docs/architecture/headless-package-development.md:388` 那条不变量仍然成立。

### 3.1 安装口（`host/src/agent_client.rs`）

```rust
pub enum RuntimeInstall { Installed, AlreadyInstalled }        // 两个都是成功

pub fn install_channel_runtime(handle: tokio::runtime::Handle) -> RuntimeInstall {
    install_into(&CHANNEL_RUNTIME, handle)                     // set() 就是仲裁
}

pub fn channel_runtime_installed() -> bool { ... }

/// 每个安装者都用的形状：没人装过才构建；只有本次调用赢了才把 runtime 留着跑。
pub fn provide_channel_runtime(
    driver_thread: &str,
    build: impl FnOnce() -> tokio::runtime::Runtime,
) -> RuntimeInstall {
    if channel_runtime_installed() { return RuntimeInstall::AlreadyInstalled; }
    let runtime = build();
    match install_channel_runtime(runtime.handle().clone()) {
        RuntimeInstall::Installed => { /* park 一个线程在 block_on(pending()) */ }
        loser => { runtime.shutdown_background(); loser }
    }
}
```

- **没有"失败"这种结果。** 类型只有两个变体、没有 `Err`；调用方想给正常情况套 `expect` 也没得套。
- **`OnceLock` 是唯一仲裁者。** 之所以不保留前面那个 `Once`：这份记录里的故障模式就是**两套**状态机打架，
  再放一套进去等于把坑挖回来。`OnceLock::set` 本身原子且幂等，
  重复调用之所以便宜是因为 `provide_channel_runtime` 先查 `channel_runtime_installed()`——
  输的那次连 runtime 都不构建（有测试钉住，§5）。
- **留与丢的配对只写在一处。** 赢家必须把 runtime 永久 park（`block_on(pending())`），输家必须关掉自己那份：
  赢家让自己的 runtime 被 drop 会杀掉共享通道的 h2 driver task；
  而输家也不能简单地 drop 自己刚建的 runtime，因为 `install_channel_runtime` 可从 async 代码
  （Desktop 的 `get_agent_readiness`）到达，在那种上下文里 drop 多线程 runtime 会 panic。
  所以丢弃走 `Runtime::shutdown_background`——tokio 文档明确写了它可以在另一个 runtime 内部 drop。
  这两半都写在 `provide_channel_runtime` 里，任何入口都不必自己把这个配对写对；
  不变量写在安装口的文档注释里，因为它是这个设计能成立的全部理由。

### 3.2 三个调用方

| 文件 | 修后 |
|---|---|
| `desktop/src-tauri/src/agent_bridge/client.rs` | `install_channel_runtime() -> RuntimeInstall` 就是 `provide_channel_runtime("agent-channel-runtime", build)`；`Once` 删除，原来那段有价值的注释（为什么 runtime 必须永不关闭）挪到 builder 闭包上。crate 内 6 个调用点不变——它们忽略返回值 |
| `cli/src/headless/mod.rs` | 同一形状，`agent_channel_driver()` 返回 runtime 而不是 handle |
| `host/src/test_support/agent.rs` | `provide_channel_runtime("fixture-agent-channel", build)`；`INSTALLED: OnceLock<()>` 与手写的那道检查都删了 |

**不变**的行为：每个进程仍然恰好一个 driver runtime，跑在一个永不返回的线程上，无论哪个安装者赢；
输掉的安装者现在返回一个值，而不是 panic。

## 4. 修前修后实测

| | 修前 | 修后 |
|---|---|---|
| 同进程第二次安装 | panic（`Agent channel runtime already configured`），发生在调用方的 `Once` 内，随后毒化 | 返回 `AlreadyInstalled`，什么也不改 |
| 之后再一次安装 | 栽在毒化的 `Once` 上（`Once instance has previously been poisoned`） | 返回 `AlreadyInstalled` |
| `cargo test -p futureos --lib`（32 核，5 / 9 次） | 5 次里 3 次红，每次 38 个失败 | 9 次里 0 次出现通道 runtime 特征；7 次绿 × 365，2 次红见 §1.2 |
| `cargo test -p future-host --lib` | — | 1828 通过，0 失败，10 忽略（468 s） |

## 5. 测试

| 测试 | 位置 | 钉住什么 |
|---|---|---|
| `the_first_install_wins_and_the_second_reports_already_installed` | `host/src/agent_client.rs`（单元，私有槽） | 先 `Installed` 后 `AlreadyInstalled`，不 panic，槽已填充——用结果取代了原来的 `Err` |
| `a_losing_installer_neither_builds_nor_parks_a_runtime` | 同上 | 输掉的那次**不执行** build 闭包（一旦被调用就 panic）："已经装过"意味着*什么也不做*，不是"先建再扔掉" |
| `a_second_installer_is_a_normal_outcome_not_a_panic` | `desktop/src-tauri/src/agent_bridge/client.rs`（单元，真实进程级槽） | 另一个安装者先注册，然后**入口连装两次**：都是 `AlreadyInstalled`，都不 panic。跑到测试末尾本身就是"没 panic"的断言 |

两条 `host` 测试用自己私有的 `OnceLock`（`install_into`/`provide_into` 把槽作为参数），
所以无论同二进制里别的测试先装了什么，它们都是确定性的。
Desktop 那条故意用**真实**的进程级槽，因为这正是缺陷所在的现场。

### 5.1 证伪过，不只是绿

两条测试都对着修前的语义跑过（把 `set` 失败恢复成 `panic!("Agent channel runtime already configured")`，
并去掉"已安装就提前返回"，也就是旧 `Err` + `expect` 那套的行为）：

| 测试 | 修前语义 | 有修复时 |
|---|---|---|
| `the_first_install_wins_and_the_second_reports_already_installed` | **FAILED** —— `panicked at host/src/agent_client.rs:294: Agent channel runtime already configured` | ok |
| `a_losing_installer_neither_builds_nor_parks_a_runtime` | **FAILED** —— 同一 panic | ok |
| `a_second_installer_is_a_normal_outcome_not_a_panic` | **FAILED** —— `panicked at host/src/agent_client.rs:294: Agent channel runtime already configured` | ok |

证伪用的改动事后已删除，并用 `shasum -a 256 -c` 验证文件字节复原，所以上面记的数字来自修好的源码。

### 5.2 测试没覆盖到哪

- **只覆盖安装契约，不覆盖线程调度竞态。** 没有测试让两个安装者并发起跑再断言交错：
  在一个进程里赢家由 `OnceLock::set` 决定，测试无法强行制造"输掉的那个调度"。
  两次顺序安装是最强的确定性陈述——而修前整轮套件的红（§1）说明真正会发生的就是这种顺序形状：
  夹具与入口相撞，而不是两个线程同时撞。
  两线程竞态额外能覆盖的，是输家那条 `shutdown_background` 丢弃路径，本机没有任何测试走到它。
- **Desktop 入口的丢弃路径。** 同理：要让入口输，得有一次夹具安装恰好落在
  `channel_runtime_installed()` 与 `OnceLock::set` 之间，无法按需复现。
- **`future-cli` 的入口。** 它与 Desktop 入口是同一段三行形状，但没有测试往它里面安装；只有编译和 clippy 覆盖它。

## 6. 改动的文件

| 文件 | 改动 |
|---|---|
| `host/src/agent_client.rs` | `configure_channel_runtime` → `install_channel_runtime`（`RuntimeInstall`），新增 `channel_runtime_installed` / `provide_channel_runtime`，两条单元测试 |
| `desktop/src-tauri/src/agent_bridge/client.rs` | 删除 `Once`；安装改为 `provide_channel_runtime`；一条单元测试 |
| `cli/src/headless/mod.rs` | 删除 `Once`；安装改为 `provide_channel_runtime` |
| `host/src/test_support/agent.rs` | 删除 `INSTALLED: OnceLock<()>` 与手写的检查 |

## 7. 同一次 CI 里的 macOS `openpty: Device not configured`

**不同源**，本次也没修。`Rust macOS test / desktop` job
[121378735419](https://github.com/futuregene/future-os/actions/runs/40262600990/job/121378735419)
只有一个用例失败：`terminal::manager::tests::global_capacity_is_enforced_across_conversations`，
而且是在*创建*第 33 个 PTY 时挂的：

```
create session: Spawn("SPAWN_FAILED: openpty: failed to openpty:
  Os { code: 6, kind: Uncategorized, message: \"Device not configured\" }")
```

errno 6 是 `ENXIO`，macOS 的 `openpty(3)` 明确写着
`[ENXIO] The sysctl(3) limit "kern.tty.ptmx_max" number of pseudo-terminal devices was reached.`——
系统级 PTY 上限，不是这段代码的逻辑：

- 该用例故意持有 32 个活 PTY（`MAX_SESSIONS = 32`），再断言第 33 个被 429 拒绝；
  它死在那个第 33 个会话的**建立阶段**，根本没走到自己的断言；
- 同二进制里相邻的 PTY 用例全过；同一个用例在最近 14 次 `Rust macOS test / desktop` job 里从未失败，
  Linux/Windows 的 desktop job 也从未失败；
- 本机采样（每 300 ms 一次）在整个套件终端密集段落里峰值到 85，之后回落到基线——
  没有泄漏，也没有单调增长；
- 同一用例在本分支上单跑 3/3 通过、随整轮套件 9/9 通过。

所以这是"runner 的 `kern.tty.ptmx_max` 相对于核数偏小"造成的并发峰值 flake，属于 runner 侧容量问题。
真要修需要知道 runner 的上限（从这里拿不到），或者显式把 PTY 密集测试串行化；两者都超出本次任务的范围。
正确的应对是重跑。列在这里是为了别把那次 CI 的两种失败混为一谈。

## 8. 环境

| | |
|---|---|
| 机器 | macOS 26.6.2 (25G83)，Apple silicon，32 核 |
| 基线提交 | `ed012b3a9`（`origin/dev-huichen`）；分支 `fix/agent-runtime-idempotent` |
| 工具链 | 仓库 pin 的 `1.99.1`（`rust-toolchain.toml`），经 `rustup` 安装；Homebrew 的 `rustc`（1.96.0）未进 `PATH` |
| 测试环境 | `FUTURE_VERSION=0.0.2-ci-record`、`ulimit -n 10240`（本机软限 256，会让无关的 fd 密集用例假红） |
| 检查 | `cargo fmt -p future-host -p future-cli -p futureos -- --check`、`cargo clippy -p future-host --all-targets -- -D warnings`、`cargo clippy -p future-cli --all-targets -- -D warnings`、`cargo clippy -p futureos --all-targets -- -D warnings`、`cargo test -p future-host --lib`、`cargo test -p futureos --lib` ×9、`cargo test -p future-cli --lib` |
