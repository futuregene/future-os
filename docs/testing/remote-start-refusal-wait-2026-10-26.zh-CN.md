# 26 秒的启动序章，配了一个 30 秒的窗口

（`cli/tests/remote_start_refusal.rs::a_broker_that_refuses_the_credential_is_reported`）

一篇一次性记录，不是设计文档：失败在 runner 日志里的原样、为什么那个等待同时在测量宿主、
改法、两次证明"新写法仍会失败"的实验，以及同类但没动的那些固定秒数。以下数据都是实测的，
命令写全，每一行都能重跑。

## 0. 结论

| | |
|---|---|
| 观测到 | Windows CI，run 40267289153 **第 1 次**，job 121392429655（`Rust Windows test / tui-cli`，2026-10-26）：子进程起来 30 秒后仍未打印 `Remote connection stopped`；同一次 run 第 2 次、同一个 commit 通过 |
| 原因 | 等待窗口把子进程自己的启动装进去了：内嵌 agent 起来花了 **26.2 秒**，窗口关闭时**一条 `Remote:` 都没有**——远程 supervisor 还没开始跑 |
| 改法 | 等待改为以**结局**收束（出现该行 / 子进程结束 / 到兜底上限），不再以时长收束；`TERMINAL_CAP` = 300 秒是防卡死的兜底，不是预算 |
| 断言 | 一个字没放松：同一行、同一个 `ServiceAuthorization` 类别、同样的非零退出、同样的平台/broker/补偿 worker 副作用 |
| 证明 | 人为把序章拖到 45 秒：**改前红（49.25 秒）、改后绿（49.84 秒）**；人为让终态永不出现：**300.27 秒以"timed out with the child still running"失败** |
| 另外改了 | 同一文件里另外三处同形等待（同样是在 `spawn` 之后预算"序章之后才有的行"）——§5 |
| 没动 | 八处 `Remote: Ready`（那是就绪预算，序章本身就是被等的对象）、十八处本来就发生在它之后的等待，以及一处有 17 倍余量的轮询——§5 |
| 仍可能误报 | 序章慢到观测值 ~11 倍以上时会报"超时"（是失败，不是假通过）——§6 |

## 1. 失败当时的样子

`gh run view --job 121392429655 --log`——测试 panic 时打出的整份转录只有六行，里面的时间戳
就是证据：

| 时间（UTC） | 子进程打印的东西 |
|---|---|
| 04:15:19.704 | 测试目标开始跑（`Running tests\remote_start_refusal.rs`） |
| 04:15:21.249458 | `FutureOS: host starting version=0.0.0-e4505c9+local pid=2296 mode=auto log=…\desktop.log` |
| 04:15:21.255 | `startup progress is logged to … and mirrored in this terminal.` / `FutureOS host - foreground mode.` |
| — | **26.165 秒，stdout 与 stderr 都没有任何输出** |
| 04:15:47.414666 | `INFO Agent startup: loading the model catalogue and credentials` |
| 04:15:47.491584 | `future-agent: no model configured yet — starting the gRPC server anyway …` |
| 04:15:47.493241 | `INFO session storage: opening database=…\.future\agent\agent.db` |
| 04:15:51.25 | 测试自己的 30 秒窗口（`spawn` + 30 秒）在这里关闭 |
| 04:16:40.099 | `panicked at cli\tests\remote_start_refusal.rs:855:5: the refused start must be reported by name; transcript:`，后面就是上面那几行 |

于是有两点，整个诊断就是这两点：

1. **30 秒全花在远程 supervisor 出现之前。** 转录里**一条 `Remote: …` 都没有**。那些行由
   headless 入口的状态轮询打印，而那个轮询在 agent 起来之后才开始；没有它，就说明入口还没
   走到"取凭据、连 broker"那段代码——broker 从头到尾没见过这次被拒绝的连接，也就是测试要等的
   事件根本没被尝试过。
2. **窗口关闭时距最后一条阶段日志只有 3.8 秒，还在序章里面。** 那 26.2 秒的空白属于 agent
   启动：它是宿主入口打完横幅之后干的第一件事，紧跟着的几行也正是它的阶段自述。序章真正的
   结束点日志里没有（窗口先关了），所以 26.2 秒是它的**下界**。

我看不到的东西：runner 上的 `desktop.log`（`…\Temp\fos-refused-7568` 目录随临时 runner 一起没了）
会说清那 26.2 秒具体花在哪一步。镜像出来的这几行足够支撑"窗口在序章内关闭、被拒的连接尚未被
尝试"这个结论，但不足以拆解 26.2 秒，本文也不做这个声称。

被掩盖掉的那一步：**同一次 run 的第 2 次、同一 commit、同一个测试二进制，通过了**（重跑的
job 121397859138 用了 8 分 56 秒，第 1 次是 10 分 36 秒）。那是重跑了一次负载条件，不是关于
产品行为的证据——这正是重跑不能算修的原因。

## 2. 为什么那个窗口其实在测量 runner

这条测试的意图是"broker 拒绝凭据时，**要报出来**"。它的等待是
`wait_for("Remote connection stopped", Duration::from_secs(30))`，从 `spawn` 开始计时，
于是窗口里装了三样都不是断言对象的东西：

- 内嵌 agent 的启动（上面那 26.2 秒）；
- 第一次向平台取凭据；
- broker 的拒绝与入口的报告。

只有最后一项是被测的产品行为。一个把无界序章也装进去的窗口，会在**宿主慢**的时候红——实际
发生的正是这件事：同一份代码在同一 commit 上前后几分钟都绿。

## 3. 改法

对"断言的是**会不会**出现某一行"的等待，增加
`ChildRun::await_line(needle, cap) -> Verdict`：

| 终态条件 | 结论 |
|---|---|
| 转录里出现该行 | `Reported` |
| 子进程结束（排水后的转录里没有该行） | `EndedWithoutReport` |
| 到了 `cap` 且子进程仍活着 | `TimedOut` |

`TERMINAL_CAP`（300 秒）只是兜底，它的文档注释自己写明了这一点，并写下本文依据的那次测量。
`Verdict::describe()` 把结局写进 panic 信息——因为"到 300 秒还在启动"和"结束了但没说"是两个
不同的发现，裸 `false` 会把这个区别抹掉。

拒绝凭据那条测试现在是：

```rust
let verdict = child.await_line("Remote connection stopped", TERMINAL_CAP);
let exit = match verdict {
    Verdict::Reported => child.wait_for_exit(TERMINAL_CAP),
    Verdict::EndedWithoutReport | Verdict::TimedOut => {
        let _ = child.child.kill();
        child.child.wait().ok()
    }
};
```

`kill` 那条分支正是"卡死的子进程会以**超时**失败"而不是把测试永远堵在 `wait()` 里的原因——
旧代码那个 `child.child.wait()` 完全没有上界。下面每一条断言都保持原样，其中一条还变强了：
"这次运行必须结束"现在有了界，而不是"终究会结束"。

## 4. 两次证伪实验

两次都用一段临时补丁（不入提交、不在 diff 里）让被拉起的子进程先 sleep 再 `exec`，这样序章
像满载 runner 一样慢，而 pid、管道和信号目标仍然是子进程本身：

```rust
let mut command = if let Ok(delay) = std::env::var("FUTURE_FIXTURE_PROLOGUE_DELAY") {
    let mut command = Command::new("sh");
    command.arg("-c").arg(format!(
        "sleep {delay}; exec \"{}\" host",
        env!("CARGO_BIN_EXE_future")
    ));
    command
} else { … };
```

### 4.1 序章远超原来的 30 秒窗口

同一个慢子进程（45 秒），先跑改前的文件，再跑改后的文件：

```bash
FUTURE_FIXTURE_PROLOGUE_DELAY=45 cargo test -p future-cli --test remote_start_refusal \
    a_broker_that_refuses_the_credential_is_reported -- --exact
```

| 测试版本 | 结果 | 墙钟 |
|---|---|---|
| 改前（`git show HEAD:…`） | **FAILED** —— `the refused start must be reported by name; transcript:` | 49.25 s |
| 改后（本次改动） | **ok** | 49.84 s |

同一个子进程、同一个产品，只有等待方式不同：旧窗口在第 30 秒耗尽时子进程还在序章里；新等待
在约 47 秒看到那份报告并通过。

对第二处改动做同一实验（它原本也是同样的 30 秒形状）：

```bash
FUTURE_FIXTURE_PROLOGUE_DELAY=45 cargo test -p future-cli --test remote_start_refusal \
    a_retry_that_cannot_be_classified_ends_the_worker_and_rearms_it -- --exact
```

| 版本 | 结果 | 墙钟 |
|---|---|---|
| 改前 | **FAILED** —— `the first start must report the retryable failure; transcript:` | 30.01 s |
| 改后 | **ok** | 53.82 s |

### 4.2 终态永不出现的子进程

```bash
FUTURE_FIXTURE_PROLOGUE_DELAY=400 cargo test -p future-cli --test remote_start_refusal \
    a_broker_that_refuses_the_credential_is_reported -- --exact
```

```
test a_broker_that_refuses_the_credential_is_reported has been running for over 60 seconds
thread 'a_broker_that_refuses_the_credential_is_reported' (256611630) panicked at cli/tests/remote_start_refusal.rs:954:5:
assertion `left == right` failed: the refused start must be reported by name (timed out with the child still running); transcript:
test result: FAILED. 0 passed; 1 failed; 0 measured; 13 filtered out; finished in 300.27s
```

兜底确实在生效：300.27 秒、退出码 101、原因写清楚——既不是挂死，也不是通过。（子进程由超时
分支 kill 掉，所以夹具的 `Drop` 没有东西要收，测试二进制正常退出。）

## 5. 同类盘点，以及为什么只改了四处

这里的"同类"指的是一件具体的事：**窗口从 `spawn` 开始，长度按事件本身定，于是把子进程的
序章也装了进去。** 序章是无界的宿主工作，其后的那个事件才是有界的产品工作。

### 5.1 改了（4 处）

| 测试 | 等的行 | 改前 | 改后 |
|---|---|---|---|
| `a_broker_that_refuses_the_credential_is_reported` | `Remote connection stopped` | 30 s | 结局判定 + 300 s 兜底 |
| `a_retry_that_cannot_be_classified_ends_the_worker_and_rearms_it` | `Remote: Reconnecting` | 30 s | 结局判定 + 300 s 兜底 |
| `a_reconnect_refreshes_the_credential_and_rebuilds_the_slot` | `rebuilding generation (1/3)` | 40 s | 结局判定 + 300 s 兜底 |
| `a_reconnect_whose_rebuild_cannot_connect_is_retryable` | `rebuilding generation (1/3)` | 40 s | 结局判定 + 300 s 兜底 |

第一处是观测到的失败。其余三处是同一文件里同一句话——一行"子进程第一次启动尝试之后才可能有"
的行，从 `spawn` 起预算 30–40 秒，也就是在 26.2 秒序章之上只剩 4–14 秒余量；其中两处用同一个
实验证明了"红→绿"（§4.1）。两处一模一样的 `rebuilding generation (1/3)` 只改一处、留一处，
反而是随意的。

### 5.2 没动，理由（同文件另有 27 处等待）

| 分组 | 处数 | 为什么不动 |
|---|---|---|
| `Remote: Ready`，`READY_BUDGET` = 90 s | 8 | 序章**正是**被等的对象，而 90 秒是带自身记录理由的就绪预算（`INCIDENTS F.105`）。它是观测序章的 3.4 倍，而这条 flake 的 30 秒只有 1.2 倍；且这里失败意味着"子进程根本没起来"，与本文修的那条断言是不同的主张 |
| `Remote: Ready` 或显式的 bridge/book 轮询之后 | 18 | 这些窗口已经不包含序章，它们的时长只反映产品延迟（`critical task stopped` 60/90 s、`automatic bridge reconnect failed` 90 s、`[AU001]` 20 s、`invalid_remote_credential` 40 s、`server failure episode started` 10/25 s、`wait_for_exit` 30 s、answer/book 文件轮询 20/30 s、`rebuilding generation (3/3)` 40 s） |
| `an_unreadable_revoke_queue_is_reported_and_the_refusal_still_stands` 里的 `no answer within 60s` | 1 | 形状相同（从 `spawn` 起算），但 60 秒对 26.2 秒序章仍留下约 34 秒给约 2 秒的工作——17 倍余量，不是 1.2 倍。把它也抬到 300 秒兜底，会让"请求真的没人应答"这种真实失败变成五分钟的等待，而没有任何证据表明需要这样 |

### 5.3 `cli/tests/` 与 `tests/` 的其它文件

按"拉起子进程、再给事件定时长"的形状查过，没有同源的，所以一处没动：

| 文件 | 固定等待 | 为什么是另一类 |
|---|---|---|
| `cli/tests/headless_cli.rs` | `wait_for(…, 300 s)` 等 `Not signed in to FutureOS`、`Remote: Stopped`、`Remote: Reconnecting`、`Platform account signed in` | 已经是本文落地的形状——对"必须出现的行"用宽兜底。`wait_for_exit`/`wait_for_signal_exit` 约束的是已经起来的子进程 |
| `cli/tests/headless_cli.rs` | 15 秒等 agent 就绪标记、8 秒等退出 | 进程内对一个已经启动的子进程做监督，不是"序章 + 事件"的窗口 |
| `cli/tests/bin.rs` | 30 秒等 config 向导退出等 | 那个子进程里没有内嵌 agent、没有 broker，30 秒比它的工作量高出两个数量级 |
| `cli/tests/relay_nats_auth.rs` | 15 秒等 `nats-server` 监听 | 外部二进制绑端口，不是我们入口的序章 |
| `cli/tests/remote_unpair_prompt.rs` | `BUDGET` = 60 秒等 `[y/N]` | pty 下的 CLI 提示，没有 agent 启动；60 秒等一次提示读取 |
| `tests/remote-interop/tests/*.rs` | `tokio::time::timeout(…, 5 s)` | 进程内异步调用，走脚本化的 socket——没有拉起的子进程，也没有序章 |

## 6. 仍可能误报的情形，以及本文没有覆盖的部分

- **序章慢到观测值的 ~11 倍以上**，新等待会在 300 秒报超时。那是**失败**而不是假通过，并且会
  打印子进程走到了哪里——但仍然是一条因宿主慢而红的 CI。另一个选择（一个很大的固定数）性质
  相同而信息更少，所以这个上限继续当兜底、不当预算：真出现这种红，要 revisit 的是
  `TERMINAL_CAP` 这个数，就在它文档注释里那段测量的旁边。
- **只重新测量了这个测试二进制的等待。** 26.2 秒来自 2026-10-26 的一次 Windows run；本文没有
  测序章时间的分布，所以某条 lane 的序章若远慢于 `READY_BUDGET`（90 秒），那八处
  `Remote: Ready` 仍会失败——这是刻意的，它意味着"子进程没起来"。
- **Windows runner 没有在本地复现。** 实验是用 sleep 代替慢序章：它证明的是**等待**对序章多长
  不敏感，而不是这台 Windows 宿主对文件里其余部分是够快的。
- **仍有两处等待把序章当作隐式前提**——那八处 `Remote: Ready`——并被有意保留为就绪预算（§5.2）。
- **本次没有验证**内嵌 agent 的序章能否在 Windows 上变短（那 26.2 秒本身）。本文讲的是不要
  在产品断言里测量它，不是让它变快。
