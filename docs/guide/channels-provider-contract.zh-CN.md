# 通道 provider 契约

本页是新增通道时的契约。桥本身——去重与陈旧过滤、访问策略、会话 → session 路由、
每会话排队、流式回复、审批回信路由、分片、节流与重试——已经写好且共享。
**provider 只承载平台知识，别的都不管。**

请先读 `channels/src/providers/cli.rs`：它是一个完整可用的 provider，约 150 行，
也是其余 provider 应遵循的形态。面向使用者的部分见
[通道 provider 参考](channels-providers.zh-CN.md)。

## provider 由什么构成

一个 provider 由「两半 + 元数据」组成，全部放在 `channels/src/providers/` 的一个文件里：

```rust
pub static DEFINITION: ChannelDefinition = ChannelDefinition { /* 元数据 */ };

pub fn provider() -> Box<dyn Provider> { Box::new(MyChannel) }
```

* `ChannelDefinition`——标识、能力、消息上限、成熟度、最小配置样例、外部依赖。
  启动器与 `future channel list` 都读它，因此一个通道只在一处描述。
* `Provider`——`definition()`、`sender(&ctx)`、`run(ctx)`，可选 `probe(&ctx)`。
  `run` 是监听端；`sender` 构建出站半边（`run`、`future channel test` 与
  `future channel send` 都会用），因此构建过程不得启动长跑任务。
* `ChannelSender`——`send_text()`，可选 `edit_text()`、`typing()`、`react()`。

在 `channels/src/providers/registry.rs` 加一条注册（声明 + 工厂）即可。框架其它
部分无需改动。

## 入站

监听端把平台事件解析成 `bridge::Inbound`，逐条交给桥：

```rust
let outcome = ctx.handle(inbound, sender.clone()).await;
```

这一次调用就是整条链路。**不要**自己实现去重、策略检查、会话创建、排队或流式；
如果发现正在写其中之一，请停下来——要么接口缺了东西（请上报），要么该逻辑属于桥。

`Inbound` 携带桥需要的信息：

| 字段 | 说明 |
|---|---|
| `message_id` | 平台消息 id，桥据此去重（按通道加命名空间）。 |
| `sender.id` | 策略比对的平台稳定用户 id。 |
| `conversation` | 会话 id、可选 `thread_id` 与 `ChatKind`。线程是独立会话，因此也是独立 agent session。 |
| `text` | 纯文本，平台标记已剥离。 |
| `media` | 附件。图片字节放 `data`（无法下载时放 `url`）；桥会把字节落到该通道的数据目录，并把图片转成模型输入。 |
| `addressed_to_bot` | 被 @ 或本就是私聊时为 true；群策略据此判断。 |
| `created_at_ms` | 平台时间戳，单位 Unix **毫秒**。早于新鲜度窗口的消息会按回放丢弃，因此不要自己编造。 |
| `raw` | 未改动的平台载荷，留给之后需要的平台特有字段。 |

## 出站

平台返回消息 id 时，`send_text` 返回 `Some(id)`；否则返回 `None`，桥就不会尝试渐进编辑。

只有当定义里声明了 `Capabilities::edit` 时才实现 `edit_text`。默认实现会明确拒绝，
对「无法改写消息」的通道而言这是诚实的答案。

不要在 provider 内做分片、节流、重试或标记转义，只有一个例外：平台自己的**方言**
（例如 Telegram 的 MarkdownV2）。桥已经按 `DEFINITION.max_text_len` 与 `length_unit`
在字符边界上切分，且不会把代码围栏切成两半。

`typing` 与 `react` 是尽力而为：失败只记日志，绝不能中断一个回合。

## 生命周期

* `run(ctx)` 在通道停止或失败时返回。监督者随后按指数退避重启，因此请返回**致命**
  错误，可恢复错误在自己的循环里处理。
* 传输就绪后调用 `ctx.mark_running()`，这样 `future channel status` 才会停止报
  `starting`。
* 值得上报的可恢复失败调用 `ctx.mark_failed("原因")`。
* 收到 `ctx.shutdown().notified()` 要尽快退出。
* 下载或记忆的内容都放在 `ctx.data_dir()` 下；绝不写入用户家目录或仓库。
* `probe(ctx)` 支撑 `future channel test <id>`：发出能证明凭据可用的最小真实请求，
  返回一行摘要（`"connected as @bot"`）。没有发出请求就不要报成功。

## 配置

通道在 `~/.future/channels/config.json` 中的块：

```jsonc
{
  "providers": {
    "my-channel": { "enabled": true /* ...你自己的字段... */ }
  }
}
```

框架从同一个块里读 `enabled` 与访问策略键（`dm_policy`、`dm_allowlist`、
`group_policy`、`group_allowlist`、`require_mention`）。自己的结构体每个字段都加
`#[serde(default)]`，并且**不要**加 `#[serde(deny_unknown_fields)]`——策略键不是你的。
用 `ctx.config::<MyConfig>()?` 读取，字段格式错误时错误信息会点出通道名。
让 `DEFINITION.config_example` 与实际读取保持一致，并在
[通道 provider 参考](channels-providers.zh-CN.md) 中记录该配置块：在那里为该通道加一节，
并让 `DEFINITION.docs` 指向它（带锚点，写英文页路径
`docs/guide/channels-providers.md#my-channel`）。`docs` 就是
`future channel list --json` 报出来的值，所以指向一个不存在的路径等于"看起来像文档、
其实哪里也去不了"——`every_channel_documents_itself_somewhere_that_exists` 会在任何
通道的目标（含锚点）不存在时让构建失败。

## 错误

平台语义明确的错误自己分类；其余交给共享助手（`transport::http::ErrorClass`、
`delivery::is_permanent_error`）。

* 尊重 `Retry-After`——HTTP 助手已经处理。
* 机器人被封、会话被删、凭据无效都是永久错误。
* 除非平台提供幂等键，否则不要重试可能重复产生用户可见消息的发送。
* **把分类写进消息文本**（`ErrorClass::label`）。持久化队列只存文本、不存别的，
  所以没进文本的分类等于丢了——它会退回去匹配平台自己的措辞，而那套措辞并不
  认识你刚分类过的错误码。永久的 `channel_not_found` 与队列里的 "channel not
  found" 没有一个共同词，于是会被重试到次数上限。要像 Signal 的
  `a_classified_failure_is_readable_by_the_delivery_queue` 那样断言它。

## 测试

测试那些「平台特有且容易写错」的部分：

* 载荷与事件解析，包括你刻意忽略的形状；
* 你自己方言的分片边界（标记转义、按字节计长的协议、UTF-16 计数）——通用分片器已由
  `transport::text` 覆盖；
* 寻址规则（提及判定、回复机器人、私聊）；
* webhook 型 provider 的签名校验（正确、错误、缺失）；
* 错误分类，包括该分类确实到达持久化队列（对真实消息文本调用
  `delivery::is_permanent_error`）；
* 配置默认值，以及配置块格式错误时给出可读错误。

用 `crate::test_support`（`temp_dir`、`home_lock`、`spawn_mock_grpc`、`spawn_http`、
`spawn_ws`），不要直接访问网络。测试绝不访问真实平台，也绝不访问 agent：不可达地址
（`http://127.0.0.1:1`）正是桥自身测试保持确定性的办法。端口用 `127.0.0.1:0` 而不是
固定端口，避免并发测试进程互相干扰。

宁可选一个「逻辑写错就会失败」的测试，也不要写一个把实现照抄一遍的测试。

## 基本规则

* **依据平台公开 API 文档实现。** 不要从其它 chat-bridge 项目复制代码、注释、标识符
  命名或常量表。
* **不要提及其它项目的名字**，无论代码、注释、测试还是文档。描述平台行为，而不是
  「谁还实现过」。
* 跨平台：不硬编码 `/`、`~`、`\` 或 shell 语法。用 `ctx.data_dir()`、`PathBuf` 与
  平台中立写法。
* 注释写**为什么**——平台怪癖、顺序约束、限流——不写**是什么**。

## 在共享检出里工作

多个 provider 可能在同一检出里并行编写，因此有两点：

* `cargo` 编译整个 crate，同侪写了一半的文件可能让你的
  `cargo test -p future-channel <过滤>` 失败。等对端能编译后重跑；失败文件不属于你时，
  那不是你的缺陷，但要在交接里说明，而不是无限重试。
* 绝不 `git stash`、`git checkout --`、`git clean`、`git reset`：它们作用于共享工作树，
  可能丢掉别人的成果。只读检查（`git show`、`git diff -- <你的文件>`）没问题。

## 测量覆盖率

新增 provider 代码要求逐行覆盖，而不是只看汇总百分比：

```bash
bash scripts/chan-cov.sh                     # 全 crate：报告 + 未覆盖行
bash scripts/chan-cov.sh --check channels/src/providers/my-channel.rs
python3 scripts/chan-missed.py providers/my-channel   # 单文件的未覆盖行号
```

`--check` 只判定你列出的文件，因此同侪在途文件不会判你失败。目标是未覆盖行为 0；确实
无法在单测中触达的行（操作系统失败路径、无法调用的平台 API、仅在安装订阅者时才求值的
tracing 参数），请说明原因以及为何不可注入，而不是删代码或弱化断言。
