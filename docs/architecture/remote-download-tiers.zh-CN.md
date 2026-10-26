# 远程下载分档：桌面不是手机

> [English](remote-download-tiers.md)
>
> 决定日期：2026-10-26。源码基线：`89a18748d`，以及 `708959a23`（*让拒绝文案说出真正卡住的限制*）的文案修正。

## 缺陷

一台作为远程客户端的桌面机，从已配对的 host 保存文件时被拒绝：

> This file type is not available on mobile; view it on desktop.

文件是 12.5 MB 的 `app-debug.apk`，而这句话两处都不对：类型并不是真正的限制（大小才是，10 MiB），并且报错的那个面板**本身就在桌面端**。

一套策略在服务两个客户端。10 MiB 上限与类型白名单是手机侧的规则，对手机是对的：预览要在 10 MiB 预算内解码，移动端阅读器只能处理固定几种类型，且一次点击可能发生在计费网络上。桌面没有这些理由——它只是要把字节写到用户选定的路径——却仍被按手机规则衡量，因为 `download_prepare` 没有办法说明是谁在请求。

## 现在的规则

`download_prepare` 的 `mode` 表明**客户端意图**，host 据此选择限制（[`files.rs`](../../host/src/remote_host/files.rs) 的 `DownloadTier`）：

| 变体 | 客户端 | 上限 | 类型白名单 |
|---|---|---|---|
| `preview`（及空 mode） | 手机，应用内预览 | 10 MiB | 有——host 只提供手机阅读器能路由的类型 |
| `original` | 手机，保存/分享 | 10 MiB | 有 |
| `download` | 桌面，保存副本 | 512 MiB | 无 |

手机侧刻意不变：`MAX_FILE_BYTES`、`is_mobile_download_allowed`、`mobile/src/remote/fileTypes.ts` 的类型表，以及所有面向手机的文案。未知或拼写错误的变体**不是**通往桌面档的入口——只有明确表示桌面保存的那一个变体才是，因为发出无法识别变体的手机不能因此悄悄获得 512 MiB。

桌面档只保留一条拒绝：源必须是普通文件。目录、设备节点、socket 或 FIFO 没有可流式传输的大小，也不构成「一份副本」，两档都拒绝。

### 为什么是 512 MiB

这个数字不是新引入的，也不是第二次猜测：拉取侧本来就拒绝组装超过 `MAX_DOWNLOAD_BYTES` = 512 MiB 的下载（[`transfer.rs`](../../packages/remote-client/src/transfer.rs)），host 以 `DESKTOP_MAX_FILE_BYTES` 重导出该常量，而不是另写一份。host 档高于客户端会让它准备好客户端必然失败的传输；低于客户端则会拒绝客户端本可完成的文件。512 MiB 覆盖远程桌面真正会被索要的东西——安装包、日志包、数据库转储——同时仍能挡住误点一个目录级的产物或一段视频。上限是 2 的幂，文案标为 `MB`，与文件列表既有的标法一致（12.5 MiB 显示为 `12.5 MB`）：用词跟用户正在看的那一行对齐，而不是引入第二套习惯。

### 两种拒绝，两句话

| 档位 | 文案 |
|---|---|
| 手机，超上限 | `The file is over the 10 MB download limit.` |
| 手机，阅读器无法处理的类型 | `This file type is not available on mobile; view it on desktop.` |
| 桌面，超上限 | `The file is over the 512 MB desktop download limit.` |

先判大小再判类型，让手机被告知它真正能处理的那件事。手机按 `download limit` 子串选择本地化文案，所以桌面那句同样包含该短语，但明确点名 desktop：两种拒绝不能被互相报告成对方。

## 没有改变的部分

- **手机**：阈值、白名单、文案逐字一致。`prepare_preview`（移动预览路径）仍按 `MAX_FILE_BYTES`。
- **桌面预览**：仍走手机方言（`preview`，失败回退 `original`）。预览是在本应用内渲染，其缓存按条数与 TTL 有界而非按大小——这里刻意没有一并放开。
- **传输协议本身**：分块、拉取、哈希校验与 `read_binary_v1` 分页通道均未改动；变的只是 `download_prepare` 按哪一套限制衡量。

## 旧 host

桌面请求 `download`，若 host 回答 `UNSUPPORTED_VARIANT`（[`remote-protocol`](../../packages/remote-protocol/src/files.rs) 中受匹配的字符串），则改请 `original`——今天的请求、今天的限制、今天的结果。因此新桌面连旧 host 的行为与今天完全一致，代价是多一次请求；旧 host 原本会提供的文件不会被拒。

只对那一种拒绝重试。把任何失败都按 `original` 重试会把桌面自己的上限文案换成手机的，并把两个不同的问题报成一个，因此该判定单独有一条测试。

## 验收，2026-10-26

macOS arm64、仓库固定工具链（1.97），对进程内的 host 桥接（真实 `download_prepare`、真实分块拉取、客户端自己的哈希校验）：

| 检查 | 结果 |
|---|---|
| 桌面保存 12.5 MiB 的 `app-debug.apk` | 成功，落盘字节与源逐字节一致（`a_desktop_save_takes_a_file_the_phone_must_refuse`、`a_desktop_download_of_an_installer_is_not_held_to_the_phones_limits`） |
| 同一文件走手机模式（`original`） | 仍被拒，`The file is over the 10 MB download limit.`，不变 |
| 超过桌面上限的文件 | 用桌面自己的句子拒绝；同一文件在手机模式下按手机规则拒绝（`a_desktop_download_has_its_own_ceiling_and_says_so`） |
| 目录在两档下 | 均拒绝 |
| 旧 host（无 `download` 变体） | 回退到 `original`；两侧都钉住——host 精确输出 `UNSUPPORTED_VARIANT`，客户端只对该串重试（`only_a_missing_variant_falls_back`） |
| 上限单一来源 | `DESKTOP_MAX_FILE_BYTES == MAX_DOWNLOAD_BYTES`（`the_desktop_ceiling_is_the_one_the_puller_enforces`） |

**未在此验证**：两台真实机器经真实中继、拉取接近 512 MiB 上限的文件。没有部署任何 host——替换现役二进制是另一道需审批的步骤——所以上面的数字是一个进程内的结果，不是网络上的结果。该设计本身有两项成本，值得在那次测试前知道：host 在传输进行期间会暂存一份完整副本（客户端取消即释放，成功的保存就会取消；失败或放弃的传输才等 30 分钟 TTL），拉取端会在落盘前把整个文件组装进内存，因此一次满上限的保存是桌面 512 MiB 内存加上 host 512 MiB 临时磁盘。

### 跨进程、跨真实 broker，2026-10-26

第二次运行把 host 桥放进一个 OS 进程，把桌面的客户端半边（`remote_peer::transfer::download_for_desktop` 与 `write_to_path`，也就是 Tauri 命令调用的两个函数）放进另一个，中间是走 TCP 的真实 `nats-server`：12.5 MiB 安装包保存成功且 `shasum` 一致，release 构建 94 ms；64 MiB 文件 437 ms；600 MiB 与 512 MiB + 1 字节都用桌面自己的句子拒绝；手机的模式与类型规则仍逐字拒绝各自的用例；用 `b594ac2bc` 构建的 host 确实回 `Unsupported download variant.` 并被再问一次。具体数字、原文串以及仍未测量的项见 [`docs/testing/remote-download-tiers-live-2026-10-26.zh-CN.md`](../testing/remote-download-tiers-live-2026-10-26.zh-CN.md)。

## 复现

```bash
cargo test -p future-host --lib remote_peer::transfer   # 保存副本与回退
cargo test -p future-host --lib remote_host::files      # 分档与文案
cd desktop/src-tauri && cargo test --lib commands::remote_peer   # 命令层
```
