# 桌面"另存为"在真实链路上实测 — 2026-10-26

> [English](remote-download-tiers-live-2026-10-26.md)

## 问题与答案

桌面下载分档的改动（`feat/dl-tiers`，合并为 `4981f4e99`）此前只在**一个进程里**跑过：
真实的 host `download_prepare`、真实的块拉取、客户端自己的哈希校验，但两端在同一个地址空间、
共用一个进程内 broker（见 `docs/architecture/remote-download-tiers.md` 的"Acceptance"）。
没有人真正从"插座另一边"的 host 上下载过一个大文件。这一页做的就是这个。

**答案：报告的缺陷在真实链路上已经消失，手机侧一个字节都没有动。**

- 12.5 MiB 的安装包（`app-debug.apk`，13 107 200 字节，25 块）由桌面成功落盘，
  `shasum -a 256` **与源文件一致**，耗时 **94 ms**（release 构建；debug 构建约 2.0 s，见 §5.3）。
  64 MiB 文件（128 块）同样成功，437 ms。
- 600 MiB 与 512 MiB + 1 字节的文件**被拒绝**，且提示是桌面自己的：
  `The file is over the 512 MB desktop download limit.`
- **同一个** 12.5 MiB 文件走手机模式，仍然被拒绝，文案不变：
  `The file is over the 10 MB download limit.`
- 手机的类型白名单对桌面同样放开：1 MiB 的 `.bin` 桌面能保存，手机仍以
  `This file type is not available on mobile; view it on desktop.` 拒绝。
- 对接**旧 host**（用 `b594ac2bc` 构建，那时还没有这个 mode）只多一次请求：旧 host 回
  `Unsupported download variant.`，桌面改问 `original`，下载完成且哈希一致；同一个 host 上
  那个 12.5 MiB 安装包得到的仍是今天那句手机文案——今天的行为，逐字不变。

架构页里有一句**说错了**，已改正（见 §5.1）：下载成功时 host 的暂存副本**不会**留到 30 分钟
TTL，客户端完成即取消、host 立即删除。另有一处用户可见的不一致按"未解决"记录（§5.2）：
同一个安装包点**预览**仍然是手机那句 10 MB 文案，这是当初就定下的设计。

## 1. 运行身份

| 项目 | 值 |
| --- | --- |
| 仓库 | `future-os`，worktree `.worktrees/dl-tiers-accept` |
| 分支 | `test/dl-tiers-accept`，从 `origin/dev-huichen`（`ed012b3a9`）切出 |
| 被测 host 代码 | `dev-huichen` = `b594ac2bc` + `feat/dl-tiers` 合并（`4981f4e99`） |
| 旧 host | worktree `.worktrees/dl-tiers-oldhost`，detached 于 `b594ac2bc`（`4981f4e99^1`），测试 harness 改了一行（§4） |
| 机器 | Mac Studio，Apple M3 Ultra，96 GB，macOS 26.6.2（25G83） |
| 工具链 | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`，即 `rust-toolchain.toml` 钉住的 channel——经 `~/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin` 调用，因为 `PATH` 上的 Homebrew `cargo`/`rustc`（1.96.0）会遮蔽 rustup 且不读 pin |
| broker | `/opt/homebrew/bin/nats-server` **v2.15.0**，客户端端口 `127.0.0.1:64881`，监控 `127.0.0.1:64882`，存储与日志都在 `/tmp/dltiers` 下 |
| 进程 | host 子进程、client 子进程、`nats-server`；每次运行都用 `HomeGuard` 的临时 HOME，操作员 `~/.future` 既没读也没写 |
| 构建目录 | `/tmp/dl-tiers-target`（新代码）、`/tmp/dl-tiers-old-target`（旧代码） |

harness 是新增的 `host/src/remote/download_tiers_network.rs`（`#[cfg(test)]`，`#[ignore]`），
形状照抄 `remote/real_network.rs`：测试带 `DLTIERS_ROLE=host|client` 重新 exec 自己，于是两端是
不同的 PID，之间只有 broker 一条通道。

```bash
# broker（自己的存储、自己的端口）
nats-server -a 127.0.0.1 -p 64881 -m 64882 -sd /tmp/dltiers/nats -l /tmp/dltiers/logs/nats.log &

# 新 host + 桌面客户端，一条命令（orchestrator 自己拉起两个子进程）
cd .worktrees/dl-tiers-accept
export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"
CARGO_TARGET_DIR=/tmp/dl-tiers-target HOME=/tmp/dltiers/home \
  DLTIERS_DIR=/tmp/dltiers/new DLTIERS_NATS_URL=nats://127.0.0.1:64881 \
  cargo test -p future-host --lib --release -- --ignored --exact --nocapture \
    --test-threads=1 remote::download_tiers_network::desktop_download_tiers_over_the_network
```

源文件由 orchestrator 造，不靠夹具：12.5 MiB 的 `app-debug.apk`（确定性伪随机字节）、
64 MiB 的 `database-dump.bin`（同）、1 MiB 的 `.txt` 与 `.bin`、600 MiB 稀疏 `disk-image.img`、
512 MiB + 1 字节的稀疏 `edge.bin`。两个稀疏文件只需要**声明长度**——host 在复制之前就拒绝了。

## 2. 哪些是真的，哪些不是

| | |
| --- | --- |
| **真实** | `nats-server` 进程与 TCP；生产 host 桥（`remote::install_host` → 生产 supervisor、命令循环、传输循环）；真实的 `download_prepare` 命令，因而真实的分档检查、路径检查、普通文件检查；真实的块拉取；客户端自己的哈希校验；桌面自己的 `write_to_path` 落盘；磁盘上的真实文件；两个独立 OS 进程 |
| **真实，但不是 GUI** | 客户端半边是 `remote_peer::transfer::download_for_desktop` 与 `write_to_path`，跑在自己的进程里——正是桌面 `remote_peer_download_file` 命令调用的两个函数。Tauri 命令壳（参数类型、`AppError`）由它自己的进程内测试覆盖（`cargo test --lib commands::remote_peer`），选保存路径的面板由 `desktop/src/features/remote-peer/*.test.tsx` 覆盖；**没有驱动 webview 和保存对话框** |
| **桩** | host 背后的 agent 是 harness 的脚本化 mock agent：可用性会被广播（`agentAvailable: true`），但下载不涉及任何对话。配对平台的 HTTP 跳和 `real_network` 一样被绕过——v2 邀请在本地签发 |
| **这里没覆盖** | 两台物理机、公网中继、TLS/WebSocket 传输、手机 App 本体（§6） |

客户端报告的 host 能力共 20 项，含 `e2ee_v2`、`file_transfer_v1`、`file_download_v2`、
`session_files_v1`。

## 3. 三次下载（及追加检查）

release 构建，一次运行，harness 写出的报告（`/tmp/dltiers/final/client.json`）：

| # | 请求 | 源文件 | 结果 | 落盘字节 | 块数 | 客户端墙钟 | SHA-256 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 桌面另存为，mode `download` | `app-debug.apk` | **成功** | 13 107 200 | 25 | **94 ms** | 源 = 落盘（`448a2be1…`） |
| 2 | 桌面另存为，mode `download` | `database-dump.bin` | **成功** | 67 108 864 | 128 | **437 ms** | 一致 |
| 3 | 桌面另存为，mode `download` | `disk-image.img`（600 MiB） | **拒绝** | — | — | — | — |
| 4 | 桌面另存为，mode `download` | `edge.bin`（512 MiB + 1 B） | **拒绝** | — | — | — | — |
| 5 | 手机，mode `original` | `app-debug.apk` | **拒绝** | — | — | — | — |
| 6 | 手机，mode `original` | `toolchain.bin`（1 MiB） | **拒绝（类型）** | — | — | — | — |
| 7 | 桌面另存为，mode `download` | `toolchain.bin`（1 MiB） | **成功** | 1 048 576 | 2 | — | 一致 |
| 8 | 桌面**预览**，mode `preview` | `app-debug.apk` | **拒绝** | — | — | — | — |

以下文案取自客户端收到的 host 原文（`check_reply_success` 把 host 的 `error` 字段原样透出）：

| 检查 | host 的答复 |
| --- | --- |
| 3 | `The file is over the 512 MB desktop download limit.` |
| 4 | `The file is over the 512 MB desktop download limit.` |
| 5 | `The file is over the 10 MB download limit.` |
| 6 | `This file type is not available on mobile; view it on desktop.` |
| 8 | `The file is over the 10 MB download limit.` |

落盘结果另在 harness 之外核对一次——自己哈希自己只能证明自洽：

```bash
$ shasum -a 256 /tmp/dltiers/final/source/app-debug.apk /tmp/dltiers/final/saved/app-debug.apk
448a2be182a8c7ed2bb6314eda39d6735318d6aa9d78d2b22d8910e601950a71  .../source/app-debug.apk
448a2be182a8c7ed2bb6314eda39d6735318d6aa9d78d2b22d8910e601950a71  .../saved/app-debug.apk
```

第 8 项只是**观测**、不是断言：预览走手机方言属于既定设计（§5.2），记下来是因为"另存为成功"
的读者应当知道同一个文件两个控件的结论现在不同。

读数：13.1 MB / 94 ms 与 67.1 MB / 437 ms 都在环回链路上 ≈139–154 MB/s，所以**上限不是带宽
问题**——512 MiB 挡的是误点，按这个速率拉满整个上限也只是几秒的字节加上客户端 512 MiB 内存
（§5.4）。这次没有真实 RTT、没有丢包（§6）。

## 4. 旧 host

这里的 host 半边是**上一版代码的独立构建**：worktree `.worktrees/dl-tiers-oldhost`，detached 于
`b594ac2bc`，`CARGO_TARGET_DIR=/tmp/dl-tiers-old-target`。只改了它**测试 harness** 的一行——
`install_host(broker, "realnet-host", true)` → `false`——让它的 host 角色装脚本化 mock agent，
而不需要额外的真 `future-agent` 进程。那个 worktree 里没有任何产品文件被改动。

它以独立进程启动（它自己的 `REALNET_ROLE=host` 那一半），新桌面客户端用
`DLTIERS_EXTERNAL_HOST=1 DLTIERS_OLD_HOST=1` 接上去（orchestrator 此时等对方写出的 `host.json`，
不会去碰不属于它的目录）。

| 检查 | 观测 |
| --- | --- |
| 1 MiB `.txt` 用 mode `download` | 拒绝，`Unsupported download variant.` |
| 同一文件走桌面的正式路径 | **成功**，1 048 576 字节，SHA-256 与源一致，**122 ms**（含多出来的一次 prepare） |
| 12.5 MiB 安装包用 mode `download` | 拒绝，`The file is over the 10 MB download limit.`——今天的手机文案，也就是今天的结果（旧 host 本来也提供不了这个文件） |

所以回退路径不只是一个单元测试里判定：旧 host 确实只多一次请求，然后照常把文件发完；它提供不了
的文件仍然用原来那句话失败。

## 5. 发现

### 5.1 下载完成时暂存副本会被释放（文档写错了）

`docs/architecture/remote-download-tiers.md` 原先写 host 把文件"复制进传输目录保留到传输结束
（30 分钟 TTL）"。实测：TTL 是**失败**路径，不是常态。`download_cancel` 会删掉暂存副本，而客户端
在一次成功拉取之后**会**发 `download_cancel`（`Transfer::with_release`）。这些运行里 21 次成功下载之后，
暂存目录里**没有**任何 `original_*` 文件、也没有任何超过 1 MB 的文件——运行之前那里只有旧的预览
文件。架构页已改成：host 只在传输期间持有副本，取消（成功的另存为就会取消）即释放，失败或放弃的
传输才等 30 分钟 TTL。

这比旧说法对 host 磁盘成本的描述更准确，也值得写下来：如果真是"每次另存留 12.5 MB 半小时"，
那本身就会成为不做这个改动的理由。

### 5.2 未解决：同一个安装包，预览控件仍然回手机那句

在远程文件面板里点这个安装包不是保存，是**预览**，而 `remote_peer_preview_file` 发的是
`preview`——手机那一档，这是当初的决定（预览要在预算内解码并缓存，那一页明确没有放开）。
结果就是同一个文件在两个控件上结论不同：`Save as…` 成功，打开它却得到
`The file is over the 10 MB download limit.`

这不是回归（改动之前也是这个结果），本次也没有修：改预览档是另一个关于内存与缓存的决策，不是
"保存"的决策。记在这里，是为了下次有人报"还是提示 10 MB"时能对上正确的控件。

### 5.3 主导计时的构建配置，不是链路

同一个 12.5 MiB 下载：

| 构建 | 样本（ms） |
| --- | --- |
| `--release` | 97、93、89、94 |
| debug（`cargo test` 默认） | 2019、1976、2022、2154 |

12.5 MiB / 94 ms ≈139 MB/s；debug 数字 ≈6.5 MB/s。差距来自 SHA-256（host：暂存副本在发出去之前
整体哈希一次，而暂存本身就是一次完整 `std::fs::copy`；client：组装好的文件哈希一次）以及逐记录
AEAD，这些在 debug 构建里都没有优化。13 MiB 的环回字节两边都不是瓶颈——但**不带构建配置的耗时
会把这个特性讲错 20 倍**，所以两个都列出来。

### 5.4 设计已经点名的两项成本，这次从另一侧测量

- **客户端内存**：拉取端在 `write_to_path` 之前把整个文件组装进一个 `Vec`
  （`transfer.rs::fetch`）。到上限就是桌面进程里 512 MiB 内存；这次运行证明了它能工作，没有证明
  它便宜，而这里真正拉过的最大文件是 64 MiB。
- **host 磁盘**：一次另存会暂存完整副本（§5.1）——到上限是 512 MiB 临时磁盘，完成后释放。
- **host CPU**：暂存副本在传输开始前整体哈希一次，提供服务时再读一遍，所以上限大小的一次保存会
  在 host 上把文件读两次。

这些都不是本次改动引入的；手机那一档从来没到过让它们有意义的尺寸。

### 5.5 没有变的东西，是验证过的而不是假设的

- 分档那次提交没有触碰 `mobile/` 下任何文件（`git diff b594ac2bc..4981f4e99 -- mobile/` 为空），
  `MAX_FILE_BYTES` 在 `host/src/remote_host/files.rs` 与 `mobile/src/remote/files.ts` 里都仍是
  `10 * 1024 * 1024`，手机那两句在检查 5、6 里逐字回来。
- 桌面面板的两个另存为调用点（`useRemoteFiles.ts`、`RemoteFilesDialog.tsx`）都**不带** `variant`
  调 `downloadRemoteFile`，这正是选中桌面档的方式；desktop 的远程文件特性里已经没有任何尺寸/类型
  文案残留。
- 绝对路径仍然只被 FutureOS 凭据文件挡住（`files::ensure_path_allowed`）；分档放开的是**什么可以
  传输**，不是**什么可以读取**——已配对设备本来就能通过 agent 读这些文件。harness 正好走的是这个
  形状：一个不属于任何会话附件的绝对 `/tmp/...` 路径，也就是 host 早已允许的 markdown 链接形状。

## 6. 没有测的，以及原因

| 未覆盖 | 原因 |
| --- | --- |
| **两台物理机 + 公网中继** | 需要部署中继，生产上还要已登录账号；本次是一台机器上的两个进程走环回 TCP |
| **TLS / WebSocket NATS 传输** | 两端都只在 `#[cfg(not(test))]` 下调 `require_tls(true)`；测试二进制走明文 |
| **512 MiB 本身或接近上限** | 边界被测到了（512 MiB + 1 字节被拒），真正拉成功最大的是 64 MiB；拉满上限意味着客户端 512 MiB 内存 + host 512 MiB 暂存盘，留给两机运行 |
| **手机 App** | 手机方言按 mode 验证，没有上真机；也没有改动任何手机阈值或文案 |
| **桌面 webview、保存对话框、面板** | 驱动的是命令背后的两个函数，不是它的 GUI 外壳（§2） |
| **丢包 / 延迟 / NAT 重绑定** | 只有环回，与 `real-network.md` §7 相同 |

## 7. 跑过的门禁

| 门禁 | 结果 |
| --- | --- |
| 新 harness，release，新 host | 通过——1 个测试，1.36 s，报告见 §3 |
| 新 harness，release，旧 host | 通过——1 个测试，0.53 s，报告见 §4 |
| `cargo test -p future-host --lib remote_host::files` | 通过（44） |
| `cargo test -p future-host --lib remote_peer::transfer` | 通过（34） |
| `cargo fmt -p future-host --check` | 干净 |
| `cargo clippy -p future-host --all-targets -- -D warnings` | 干净 |
| `make check-docs` | 通过——检查 343 篇文档、0 对双语待补、provider 矩阵与代码一致 |

release 构建会打一条与本改动无关的警告：
`stripping debug info with rust-objcopy failed … Library not loaded: @rpath/libLLVM.dylib`
——钉住的工具链里 `rust-objcopy` 需要它旁边没有随附的 LLVM dylib。构建照样完成，只是跳过
debug-info strip。

## 8. 重跑旧 host 那一支

旧 host 必须是上一提交的构建，所以它从自己的 worktree 驱动：

```bash
# 1. 旧 host，自己的进程（harness 改一行：real_agent=false）
git -C /Users/geilige/future-os worktree add --detach .worktrees/dl-tiers-oldhost 4981f4e99^1
#    host/src/remote/real_network.rs: install_host(…, "realnet-host", true) → false
cd .worktrees/dl-tiers-oldhost
export PATH="$HOME/.rustup/toolchains/1.99.1-aarch64-apple-darwin/bin:$PATH"
CARGO_TARGET_DIR=/tmp/dl-tiers-old-target HOME=/tmp/dltiers/home \
  REALNET_ROLE=host REALNET_DIR=/tmp/dltiers/old REALNET_NATS_URL=nats://127.0.0.1:64881 \
  REALNET_AGENT_ADDR=127.0.0.1:1 REALNET_LOG_DIR=/tmp/dltiers/logs \
  cargo test -p future-host --lib -- --ignored --exact --nocapture --test-threads=1 \
    remote::real_network::real_network_two_process_host_and_client &

# 2. 新客户端接上去
cd .worktrees/dl-tiers-accept
CARGO_TARGET_DIR=/tmp/dl-tiers-target HOME=/tmp/dltiers/home \
  DLTIERS_DIR=/tmp/dltiers/old DLTIERS_NATS_URL=nats://127.0.0.1:64881 \
  DLTIERS_EXTERNAL_HOST=1 DLTIERS_OLD_HOST=1 \
  cargo test -p future-host --lib --release -- --ignored --exact --nocapture \
    --test-threads=1 remote::download_tiers_network::desktop_download_tiers_over_the_network
```

旧 host 在 orchestrator 写出 `client.done` 后退出（成功失败都会写），或到它自己的 300 s 上限。
