# 分页回复不再走 base64：`read_binary_v1` 载体

本记录是一次「分页读取的块字节怎么送」的改动，以及决定它值不值得做的测量。
它不是产品文档；`docs/testing/` 本身豁免双语配对规则，中文版是按请求另写的。

## 1. 运行身份

| 项 | 值 |
| --- | --- |
| 仓库 | `future-os`，worktree `.worktrees/page-encoding` |
| 分支 | `perf/remote-page-encoding`，从 `origin/dev-huichen` 切出 |
| 基线提交 | `df43a6230`（`Merge branch 'perf/remote-protocol' into HEAD`） |
| 机器 | macOS 26.6.2，Apple M3 Ultra |
| 工具链 | `rustc 1.97.0 (2d8144b78 2026-07-07)`，即 `rust-toolchain.toml` 钉住的版本——通过 `~/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin` 调用，因为 `PATH` 上的 Homebrew `cargo`/`rustc`（1.96.0）会遮蔽 rustup 且不读这个钉版本 |
| node | v26.3.0 |

§3 数字的产生命令：

```bash
# 字节数与单块 CPU（确定性，不开 socket）
cargo test --release -p future-remote-interop --features network-tests \
    --test page_encoding -- --nocapture

# 环回墙钟时间，两种载体、同一页
cargo test --release -p future-remote-interop --features network-tests \
    --test page_encoding -- --ignored --nocapture

# 真实加密链路上的正确性，两种载体
cargo test --release -p future-remote-interop --features network-tests --test page_encoding
```

`--release` 对 CPU 数字是硬要求：debug 构建里 base64 解码是另一个数量级，会把
它夸大 40 倍以上。

## 2. 线缆上改了什么

### 2.1 改动前

一个分页块的链路：

```
页字节 → base64url（× 4/3，且是 gzip 无法建模的字母表）
       → JSON 字符串字段 `readChunk.data`
       → 整个回复信封 serde_json::to_vec
       → gzip（≥ 32 KiB，由 `reply_gzip_v1` 协商）
       → ChaCha20-Poly1305 封装（+28 头、+16 tag）
       → NATS 发布
```

base64 这一步不是多余的：JSON 文档要携带任意字节，唯一的落点就是 JSON
*字符串*，而回复体当时就是一份 JSON 文档。可它下面那一层本来就是二进制——
封装后的记录就是字节，NATS payload 也是二进制安全的——所以这个 33% 的膨胀在
那一层没有换到任何东西。

### 2.2 改动后

声明了 `read_binary_v1` 的客户端（且只有它）收到的是帧：

```
"FBR1" | u32 大端头长度 | 头部（JSON） | 块字节（原样）
```

头部仍是同样的回复信封（`type`/`success`/`data`/`error`），`data` 是
`{"readChunk": {"id","offset","nextOffset","totalBytes"}}`。变的是字节的
**载体**，不是页的形状：读者要钉住并校验的字段一个都不少仍然是 JSON，消失的
只有 `data`（base64 字符串）。帧随后按同一套策略 gzip——现在压缩器看到的是它
能建模的文本，这正是省下来的部分；gzip 并没有被去掉（0.5 MiB 的 JSON 文本仍会
压到约 180 KB）。

实现要点，每条对应一个线缆决定：

- `packages/remote-protocol/src/json.rs`——帧的写与读
  （`encode_binary_frame`、`split_binary_frame`），用读者本来就在执行的
  `MAX_REMOTE_JSON_BYTES`（1 MiB）设界。四个不可能开启 JSON 文档、也不是 gzip
  magic 的 ASCII 字节，于是读者靠它自己分辨载体，从不需要被告知。
- `packages/remote-protocol/src/lib.rs`——`READ_BINARY_FEATURE`。
- `packages/remote-protocol/src/handshake.rs`——写进 `SERVER_FEATURES` 公布。
- `packages/remote-server/src/services.rs`——`ReplySink::send_binary(header,
  body)`；它的**默认实现**就是旧的 base64 页，于是没有连接在背后的 sink（测试
  替身、别的嵌入方）产出的字节和从前一模一样。
- `packages/remote-server/src/commands.rs`——`HandshakeState::binary_pages` 与
  `gzip_replies`/`lean_events` 完全同构，由 `apply_declared_features` 从声明里
  置位；`NatsReply::send_binary` 只在该位为真时成帧，否则退回 base64 页。
  单飞缓存存的是它实际发出的那份编码负载，所以重放的命令 id 重放的是同一种
  载体。协商出来的 gzip 标志从单飞包装层挪进了 `handle_command`——每条路径都必经
  的那一个函数；起因见 §3.3。
- `packages/remote-server/src/reply.rs`——
  `encode_binary_reply_payload_with_gzip`，与 JSON 编码器共用尺寸闸、32 KiB
  阈值和「绝不膨胀」判断。
- `host/src/remote_host/read_pages.rs`——一页变成 `Page { header, body }`：
  host 根本不再决定载体，于是 `PagedReply` 和 `get_read_chunk` 都把字节与 JSON
  分开交给 sink。
- `packages/remote-client/src/session.rs`——`Reply::{Data, Page}`；解码靠形状
  （先解压，再看 magic），所以同一个客户端读两种载体，没学过这个能力的 host
  照旧工作。
- `packages/remote-client/src/runtime.rs`——`PageChunk` 在跑任何校验之前，先把
  「字节来自帧」与「字节来自 base64」折成同一个形状，快照断言只写一遍。
- `packages/remote-client/src/{readPages.ts,remoteJson.ts,client.ts}`——TS
  客户端（手机与桌面）解帧，把体字节挂在解析后的回复旁边（不可枚举，既有断言
  看不到它），并且只在桌面确认里公布过该能力时才声明。

## 3. 测量

下表的页大小都是 `get_session_entries` 回复的 JSON 字节（真实回复信封），按
传输自己的 192 KiB 切块，超过自己 32 KiB 阈值就 gzip，再加每条消息固定的 44
字节封装开销。字节数取的是生产编码器算出的**编码后回复**长度，正是这次改动能
动的部分；封装开销恒定且两边相同。§3.2 再把它与 socket 上的实测对账——用被测
代码自己算出来的数字只算半个测量。

三种语料，因为 base64 的代价是内容的属性：

- **散文**——英文样子的助手长文，gzip 约 2.8×。日常情况。
- **工具输出**——shell 记录、路径与退出码页脚，重复度高。
- **hex 标识符**——id/哈希密集的文本，gzip 很难建模。

### 3.1 同一页、两种载体的线缆字节

| 语料 | 页 | 块数 | base64 | 帧 | 省下 |
| --- | --- | --- | --- | --- | --- |
| 散文 | 525 072 B | 3 | 184 736 B | 127 690 B | **30.9 %** |
| 散文 | 4 199 547 B | 22 | 1 478 526 B | 1 019 707 B | **31.0 %** |
| 工具输出 | 528 354 B | 3 | 83 385 B | 38 309 B | **54.1 %** |
| 工具输出 | 4 197 784 B | 22 | 658 580 B | 306 663 B | **53.4 %** |
| hex 标识符 | 524 825 B | 3 | 442 656 B | 249 381 B | **43.7 %** |
| hex 标识符 | 4 197 969 B | 22 | 3 542 455 B | 1 995 275 B | **43.7 %** |

读法：**31 %–54 %**，这个跨度是内容造成的，不是软件。base64 在内容压不动时最
贵（hex），在 gzip 本来就能吃下文本时最便宜（散文）。4 MiB 那几行与 0.5 MiB
的孪生行相差不到 0.1 个百分点，这说明收益是按块计的，与页长无关。

### 3.2 同一页在 socket 上的实测

客户端与 broker 之间放一个计数 TCP 代理，在命令发出前、回复到达后各取一次计数，
窗口里只有这次读取。host 保持自己的直连：两端都走代理会让每个方向都报出整段对话
的量，这也是本测量第一次尝试在一个 4.2 MB 的页上两个方向各读到约 5.4 MB 的原因。

| 载体 | broker → client | client → broker（请求） |
| --- | --- | --- |
| base64 | 1 480 668 B | 7 844 B |
| 帧 | 1 021 918 B | 7 844 B |
| 省下 | **31.0 %** | 0（按设计） |

socket 与 §3.1 相差 0.05 个百分点以内；请求方向两边逐字节相同——这次改动只动
回复，客户端那 22 次拉块两边是完全一样的命令。

### 3.3 这次测量顺手发现的一个缺陷：分页块从来没有被 gzip 过

§3.2 第一次跑出来 base64 是 5.41 MB，而编码器算的是 1.48 MB。差距不是测量假
象：`get_read_chunk` 是唯一故意跳过单飞回复缓存的那条命令（避免为每个块留存第
二份拷贝），而协商出来的 gzip 标志是由那个包装层安装的——于是首块之后的每一个块
都是**未压缩**发出去的。在 4 MiB 的页上实测：22 块里只有 1 块被 gzip，其余 21 块
每块 262 KB 的 base64，而不是约 90 KB。自 `reply_gzip_v1` 落地以来，对所有客户端
都是如此，不只是本记录关心的这个载体。

现在这个标志放在 `handle_command`——每条命令路径都必经的那一个函数——而不是放在
包装层。§3.2 的数字是修好之后测的；对照旧行为，base64 臂是 5.41 MB、帧臂是
4.06 MB（也就是 25 % 的算术上界，因为什么都没压），那才是这次改动连同这个修复
一起替换掉的状态。

### 3.4 单块 CPU（release，192 KiB 块）

| 步骤 | base64 载体 | 帧载体 |
| --- | --- | --- |
| 读者：解一块 | 156 445 ns | 891 ns |
| host：编一块 | 190 972 ns | 105 ns |

帧的「解码」是一次切片加一次小 `serde_json` 解析；base64 的是切分 + 262 KB 的
`serde_json` 反转义 + base64 解码。按 4 MiB 一页（22 块）算，省掉约 3.4 ms 读者
CPU 与 4.2 ms host CPU。手机比这台机器慢，且它的 base64 路径没有 SIMD 垫背——
但本记录只测了一台机器，不从它外推手机数字。

### 3.5 真实加密链路上的环回时间，4 MiB 页 / 22 块

同一页、同一客户端、同一 broker、同一封装通道，只有夹具的载体不同。两次采样，
每轮三轮（第 0 轮含连接预热）。下面是在 §3.3 的 gzip 修复生效之后测的——两块都被
压缩，所以 host 干得更多、线缆上更少：

| 采样 | 载体 | 第 0 轮 | 第 1 轮 | 第 2 轮 |
| --- | --- | --- | --- | --- |
| 1 | base64 | 56.3 ms | 29.9 ms | 26.1 ms |
| 1 | 帧 | 46.1 ms | 25.9 ms | 19.2 ms |
| 2 | base64 | 57.1 ms | 30.0 ms | 26.0 ms |
| 2 | 帧 | 44.6 ms | 24.7 ms | 19.2 ms |

热轮次：采样 1 从 28.0 → 22.6 ms（−19 %），采样 2 从 28.0 → 22.0 ms（−21 %）。
**这不是网络测量。**环回链路没有 RTT 也没有带宽上限，这些数字里剩下的是编码
工作、解码工作和拷贝——也就是从外面看到的 §3.4。在真实链路上起作用的是
§3.1–§3.2 的字节减少量，而那样的链路本轮没有测（见 §5）。

§3.5 的每一次运行还断言重组出来的页与快照**逐字节相同**
（`serde_json::to_vec(&read) == page`），这才让两臂成为同一件工作，而不是两件
不同的任务。

## 4. 为什么是 31 %，不是 44.6 %

被替换掉的 44.6 % 来自一个 0.5 MiB 的页：它的 JSON 在 base64 之前 gzip 到
23.7 KB、之后 42.8 KB——也就是一个压缩了 **22×** 的页。那不是转录文本的样子；
在那种语料上，gzip 几乎吃掉了原始字节的全部冗余，而 base64 又把其中大部分破
坏掉，那是这项改动的最好情况，不是典型情况。

按真实形状的文本测，诚实的区间是 **31 %–54 %**（§3.1），而散文——会话页的常见
情况——落在区间下沿。写成规则：收益等于 4/3 膨胀量减去 gzip 本来就能从 base64
字母表里捞回的那部分，所以高熵内容最大、自然语言最小。

## 5. 没有测的东西，以及原因

- **手机。**TS 客户端有测试覆盖（见 §6），但没有计时；上面的数字是 M3 Ultra 上
  的 Rust。手机 CPU 在 base64 上比在 memcpy 上慢得更多，方向是一致的，但本记录
  不声称任何手机数字。
- **弱网。**本仓库没有 RTT/丢包夹具，而那里真正的问题（往返次数 vs 字节）是另
  一项改动。环回时间按 CPU 汇报，不按链路汇报。
- **用户真实会话。**这些页是在真实回复形状与尺寸区间（0.5 MiB 与 4 MiB，正是会
  触发分页的那两档）里生成的，不是从某个已存会话里抓的。抓真实会话更忠实，本
  任务没有这样的样本可用。

## 6. 不许改变、并且有断言的语义

这些是正确性而非性能，每一条都有会红的测试：

| 语义 | 钉在哪里 |
| --- | --- |
| 首条回复钉住不可变快照；`id`、`totalBytes`、`offset`、`nextOffset` 每块都校验 | `readPages.test.ts`（owner/offset/total）、`runtime.rs` 的 `store_chunk`、`host/src/remote/commands.rs` 的载体测试 |
| 即便偏移相同，来自另一份快照的块也要拒绝 | `readPages.test.ts` 的 binary 快照用例、`store_chunk` 的 id 校验 |
| 页还没结束就出现过短的块，两种载体都拒绝 | `readPages.test.ts`（base64 与 binary 各一）、`store_chunk` |
| 乱序块写入不相交区间；`READ_CONCURRENCY = 4` 有界批次；失败不放大成第二轮 | `readPages.test.ts`（未改动，仍全绿） |
| `MAX_READ_BYTES = 16 MiB` 与 ≤ 192 KiB 的单块上界 | `readPages.ts`、`runtime.rs`、`host/read_pages.rs` |
| 未声明的客户端照旧收到逐字节相同的 base64 页 | `host/src/remote/commands.rs::paged_reads_switch_carrier_only_for_a_connection_that_declared_it`（声明前 / 声明后 / 撤销后） |
| 已声明的客户端若被喂 base64 页，仍能读 | 同一测试的 base64 臂；`an_undeclared_connection_receives_the_base64_page` |
| 忽略 `chunkedRead` 的旧 desktop 不受影响 | 页只对 `chunkedRead` 生成，那份代码从不设它 |
| 帧头部在构建期就受读者 1 MiB 上界约束 | `remote-protocol/src/json.rs` |
| 已声明连接的**块**回复与其它回复一样被 gzip（§3.3 的缺陷） | `host/src/remote/commands.rs` 的载体测试在解压后断言帧 magic；interop 的 socket 测量在页不再压缩时会红 |

host 侧那条是承重的：它跑真实的桌面 host、真实的分页传输与真实的回复路径，并
断言同一份快照在声明前、成帧后、撤销声明后三次都重组出**相同的字节**。

## 7. 跑过的闸门

| 闸门 | 结果 |
| --- | --- |
| `cargo test -p future-remote-protocol --lib` | 通过（28） |
| `cargo test -p future-remote-server --lib` | 通过（174，5 ignored） |
| `cargo test -p future-remote-client --lib` | 通过（66） |
| `cargo test -p future-remote-interop --features network-tests` | 通过（全部 target，含新增 `page_encoding` 套件） |
| `cargo test -p future-host --lib` | 分两次跑通：`--skip agent_bridge`（1442 通过，0 失败，10 ignored）与 `-- agent_bridge`（376 通过，0 失败）；两者合在一个二进制里并行跑会死锁，见 §8 |
| 改过的 crate 上 `cargo fmt --check`、`cargo clippy --all-targets` | 干净 |
| `npx vitest run`（remote-protocol）、`npx jest --runInBand`（remote-client） | 通过（57 / 667） |
| `make check-docs` | 见 §8 |

## 8. 套件备注

- **完整的 `future-host --lib` 在这台机器上用一个二进制并行跑会死锁，改动前也是
  这样。**成批的 `agent_bridge::*` 家族（`import`、`approval`、`headless`、
  `models`、`observer`）各自报「running for over 60 seconds」且几乎没有进展：它们
  排在一把进程级夹具锁后面，而 HOME↔mock-agent 的加锁顺序让持锁者永远不放手。
  拆成两次都又绿又快：`-- --skip agent_bridge` → 1442 通过 / 0 失败 / 10 ignored，
  用时 417 s；`-- agent_bridge` → 376 通过 / 0 失败，用时 60 s；
  `agent_bridge::import::tests` 单独跑 48 通过、3.6 s。这些用例都不碰本改动涉及的
  remote 栈，死锁也是 `.future/memory/testing-gotchas.md` 里已记录的那一个；这里
  如实记录而不是绕过它——一个不能整包跑的套件本身就是一条发现。
- 首次整包跑确实红了一个用例，而且它是本改动的真实后果、不是抖动：
  `remote::commands::bridge_tests::handshake_roundtrip_activates_the_bridge` 钉住
  确认报文里 `features` 的精确数组，协议夹具
  （`packages/remote-protocol/fixtures/protocol.json`）又从两种语言各钉一次。两者
  都已更新为包含 `read_binary_v1`——这正是本记录要改的那份公布清单。
