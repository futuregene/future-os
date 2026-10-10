# 通道框架的测试覆盖

通道框架对测试的承诺，以及少数刻意不覆盖的行。

## 口径

按行统计，取 llvm-cov 的 `DA:<line>,0` 记录，而不是百分比汇总。一行被覆盖，
指的是有测试执行了它；不用任何代理指标。

```bash
bash scripts/measure/chan-cov.sh                          # 整个 crate：报告 + 未覆盖清单
bash scripts/measure/chan-cov.sh --check FILE [FILE...]    # 指定文件有未覆盖行就失败
python3 scripts/measure/chan-missed.py [子串]              # 分组列出未覆盖行
python3 scripts/measure/show-lines.py FILE LINE [LINE...] # 带上下文打印这些行
```

脚本会消毒环境（`CARGO_HOME`、`CARGO_TARGET_DIR`、`HOME`）并加独占锁：
变量泄漏会改变插桩单元哈希，llvm-cov 随即把所有行报成"从未执行"；并发跑插桩
构建则会把机器拖垮。

下面的行号来自某一次测量快照；crate 变化后行号会漂移。依赖具体行号前请先重跑
上面的命令。

## 规则

新代码应当每一行都被测试执行。这里的"覆盖"指有测试能因这一行写错而失败——
不是被无关测试顺带走到，也不是为了数字好看而把行弄成不可达。

确实覆盖不了的行属于以下三类之一，并且写在这里，而不是藏起来：

1. **构造上不可达**——类型已经排除了该情况，代码只是守卫。能删就删；保留时
   （协议边界、防御性分支）要说明为什么类型系统表达不了。
2. **本环境不可达**——需要某个平台、一个坏掉的 socket，或测试台无法确定性制造
   的真实外部服务。
3. **归属假象**——宏展开旁边的收尾大括号或 span 结尾。llvm-cov 会给它单独一个
   区域，于是即便相邻语句已覆盖，它仍读作 `0`。下面每个假象行都注明了证明该
   代码确实执行过的测试。

## 框架内未覆盖的行

在通道 crate 上测得。共 22 行，分四组。

### 环境所致（8）

| 行 | 为何测试无法执行 |
|---|---|
| `transport/webhook.rs` 149–151 | `accept()` 失败分支。需要监听套接字坏掉或进程耗尽文件描述符——无法在进程内确定性复现；强行制造（耗尽 fd）又会拖累并行运行的其他测试。 |
| `providers/imessage.rs` 282 | `sender()` 的 `!platform_supported()` 分支。覆盖在 macOS 上测量，该判断恒真。 |
| `providers/imessage.rs` 391 | `send()` 在 `run_osascript` 之后的收尾。走到它会真的执行 `osascript`，发出真实 iMessage。 |
| `providers/slack.rs` 934–936 | `#[cfg(not(test))] webhook_test_slot`。被测量的产物是测试构建，这段本身不在其中。 |

### 构造上不可达（4）

| 行 | 原因 |
|---|---|
| `providers/slack.rs` 733、`providers/mattermost.rs` 527 | `let Some(message) = stream.next() else { bail!("… closed by the platform") }`。已用实验验证：服务端**不发**关闭帧直接断开时，流并不会结束——客户端在读取分支报 `Err(Protocol(ResetWithoutClosingHandshake))`；正常关闭则得到 `Ok(Close)`，由它自己的分支退出。因此这一 `None` 分支经由该客户端不可达；保留是因为 `Stream::next` 的类型是 `Option`。 |
| `providers/slack.rs` 976 | 测试专用助手里的 `unreachable!("tests dial plain ws only")`：测试传给它的是明文套接字，TLS 分支取不到。 |
| `outbox.rs` 210 | `Outbox::context` 中"未实现"的守卫。按 id 解析通道要经过注册表，而注册表不带任何 planned 通道，所以守卫不会触发。同一守卫在启动器里**是**被覆盖的——那里可以直接把定义传进去（见下）。 |

### 顺序与环境（1）

| 行 | 原因 |
|---|---|
| `providers/mattermost.rs` 552 | 在"读到帧"与"写回回复"之间死掉的套接字上写 pong。测试台无法给这两件事排序：事先已死的套接字会先在认证写入处失败（已覆盖）；会话正在读取时到达的 RST 又先由读取分支报出。现有的 reset 测试断言了同样的用户可见结果（会话报告套接字不可写）。 |

### 归属假象（9）

以下都是大括号或 span 结尾；相邻语句已被表中所列测试覆盖。

| 行 | 是什么 | 代码确实执行的证据 |
|---|---|---|
| `lib.rs` 172–173、190–191 | 飞书与钉钉 supervisor 的 `inspect_err` 闭包 | 两个桥在出错时重试、在关停时返回 `Ok`，所以该闭包只在桥被中途 abort 时执行。测试套件里没有这种中途 abort。 |
| `lib.rs` 386 | 状态刷写失败分支的收尾 `}` | `the_flusher_survives_an_unwritable_snapshot` 让刷写失败并断言刷写器继续运行。 |
| `lib.rs` 514 | `starting_publishes_a_state_for_every_channel` 状态断言中的 `Some(Running)` 操作数 | 快照在 supervisor 运行之前取得，因此每个启用的行都是 `Starting`，第二个操作数从不求值。保留它是为了让该断言在这一点变化后仍然成立。 |
| `providers/slack.rs` 763、903 | webhook 里 `if let Some(event)` 之后、以及派发 spawn 之后的 `}` | `the_events_webhook_verifies_signatures_and_answers_the_challenge` 会等待被接受消息的确认反应，而 `dispatch_event` 只在事件走完桥的管道后才发出它。 |
| `providers/qq.rs` 626 | 心跳分支的收尾 `}` | `heartbeats_echo_the_last_sequence_and_stop_after_a_missed_ack` 与 `an_ack_clears_the_heartbeat_flag_and_the_connection_survives` 断言了心跳帧与被清掉的 ack 标志。 |

## 不在本次范围内的行

飞书与钉钉的桥（`channels/src/feishu/`、`channels/src/dingtalk/`）以及 agent
gRPC 客户端（`channels/src/grpc_client.rs`）另有 **25 或 26** 行未覆盖——这个数字会自己
变动一行，原因值得知道：`feishu/feishu_ws.rs:254` 是 pong 发送里的 `warn!`，而它的测试
**刻意**容忍了竞态的两种先后顺序（"要么 pong 发送先报 warn，要么读取先报错"）；只有当
发送那一边先发生时这行才会执行。所以测到 47 和测到 48 都可能是对的，那里冒出一行并不等于
某项改动引入了它。它们早于通道框架、不属于它；列在这里只是为了避免把整个 crate 的数字误
当成框架欠账。

## 让守卫可测的接缝

`start_all` 把每个已注册通道的去向判定放在 `entry_action` 里，它接收"一个定义"
而不是伸手去查注册表。这正是让"启用了但未实现 → 报告，而不是静默跳过"可以被
测试的原因，且不必让本构建真的带一个 planned 通道：
`an_enabled_channel_that_is_not_built_is_reported_and_skipped` 自己构造了一个。

`pump_lines` 接受任意 `BufRead`，所以它的读取失败分支由
`a_read_error_ends_the_pump_instead_of_retrying_it` 覆盖，而不必去破坏测试自己的
标准输入。
