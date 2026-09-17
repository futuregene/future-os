# FutureOS 文档

> `docs/` 目录导航索引。链接均在本仓库内解析；面向用户的 wiki 见 [docs/wiki/](wiki/en/Home.md)（en）与
> [docs/wiki/zh/Home.md](wiki/zh/Home.md)（zh）。
>
> `docs/` 下每篇文档均为双语（`name.md`=英文，`name.zh-CN.md`=中文）；
> `scripts/check-docs.py` 强制校验目录归属、双语配对、链接与代码围栏。
> `archives/` 下的历史审计文档保留原始时间与 commit 边界。

## 指南（`guide/`）

| 文档 | 内容 |
|---|---|
| [构建与安装](guide/build-and-install.zh-CN.md)（[en](guide/build-and-install.md)） | 前置条件、各平台工具链（macOS / Linux / Windows）、`make` 目标、GUI 打包、`future-loop` 安装、技能安装 |
| [TUI](guide/tui.zh-CN.md)（[en](guide/tui.md)） | 终端界面（`future-tui`）：斜杠命令、快捷键、设置 |
| [渠道配置](guide/channels-config.zh-CN.md)（[en](guide/channels-config.md)） | `~/.future/channels/config.json` 统一参考（agent / Feishu / DingTalk 块、默认值） |
| [无头桌面](guide/desktop-headless.zh-CN.md)（[en](guide/desktop-headless.md)） | 前台 `futureos-headless` 启动、终端登录/配对二维码与链接、Ctrl+C、无 GUI 服务器构建与排障 |
| [目录布局](guide/directory-layout.zh-CN.md)（[en](guide/directory-layout.md)） | `~/.future/` 下各目录职责（agent、channels、TUI、GUI、loop） |
| [移动端延迟诊断](guide/mobile-latency-diagnosis.md) | 移动端端到端延迟测量与诊断（暂缺中文） |
| [截图工具](guide/screenshots.zh-CN.md) | 用演示数据渲染真实桌面端 / 手机端界面，在无显示器环境产出截图、功能示意图与图文文档 |

仓库根 [README](../README.zh-CN.md)（[en](../README.md)）为入口；
[wiki](wiki/zh/Home.md) 是面向用户的应用指南。

## 架构（`architecture/`）

| 文档 | 内容 |
|---|---|
| [Loop 控制面](architecture/loop-control-plane.zh-CN.md)（[en](architecture/loop-control-plane.md)） | `future-loop` — 目标/todos/门禁/监控、should-run 内核、配额、事件溯源、交付闭环、多 agent、supervisor/worker 消息、web dashboard |
| [长程证据账本](architecture/long-run-evidence-ledger.zh-CN.md)（[en](architecture/long-run-evidence-ledger.md)） | 长程 loop 目标问责记录 — 墙钟、花费、验证结果、每个已关闭目标的显式边界 |
| [SQLite 迁移](architecture/sqlite-migration.zh-CN.md) | Agent/Desktop SQLite 存储布局与迁移策略（暂缺英文） |
| [回复结果语义](architecture/response-outcomes.zh-CN.md) | 回复结束的结果语义（停止 / 拒答 / 过滤 / 暂停）（暂缺英文） |
| [loop/](architecture/loop/README.md) | `future-loop` crate：[架构](architecture/loop/ARCHITECTURE.md)（en/zh）、[上游来源](../orchestration/loop/UPSTREAM.md)、[决策内核快照](architecture/loop/snapshots.md) |
| [共享包](architecture/packages.md) | `packages/` 约定：npm/workspace 包与 crate 边界 |
| [RPC crate](architecture/rpc.md) | `packages/rpc` — protobuf 线上契约（唯一事实来源） |

## 内部文档（`internals/`）

各模块工作文档，原分散于 `desktop/DEV_MD/`、`mobile/docs/`、`tui/`、
`packages/`、`orchestration/loop/` 与 `tests/`。

- [desktop/](internals/desktop/PRODUCT.md) — 产品语义、数据模型（`ER.md`）、颜色、沙箱（macOS/Windows/Linux）、连接与远程、嵌入式终端、压缩（原 `desktop/DEV_MD/`；文档地图见 `desktop/CLAUDE.md`）
- [mobile/](internals/mobile/README.md) — 移动端构建/TestFlight、iOS 平台对齐、流式同步性能/审计、鸿蒙兼容（原 `mobile/README.md` + `mobile/docs/`）
- [tui/](internals/tui/tests.md) — TUI 测试框架约定（原 `tui/tests/README.md`）
- [Desktop NATS 桥](internals/desktop-nats.md)（原 `desktop/nats/README.md`）
- [Windows 安装/更新](internals/desktop-windows.md)（原 `desktop/src-tauri/windows/README.md`）
- [Provider 协议测试](internals/provider-protocol.md)（原 `tests/provider-protocol/README.md`）

## Wiki（面向用户的应用指南）

- 中文: [首页](wiki/zh/Home.md), [安装](wiki/zh/Installation.md),
  [快速开始](wiki/zh/Quick-Start.md), [使用 FutureOS](wiki/zh/Using-FutureOS.md),
  [设置](wiki/zh/Settings.md), [审批与沙箱](wiki/zh/Sandbox.md), [手机远程](wiki/zh/Remote.md), [技能](wiki/zh/Skills.md),
  [命令行工具](wiki/zh/CLI.md), [FAQ](wiki/zh/FAQ.md),
  [飞书](wiki/zh/Feishu.md), [钉钉](wiki/zh/DingTalk.md),
  [模型目录](wiki/zh/Models.md) *(自动生成)*
- English: [Home](wiki/en/Home.md), [Installation](wiki/en/Installation.md),
  [Quick Start](wiki/en/Quick-Start.md), [Using FutureOS](wiki/en/Using-FutureOS.md),
  [Settings](wiki/en/Settings.md), [Sandbox](wiki/en/Sandbox.md), [Remote](wiki/en/Remote.md), [Skills](wiki/en/Skills.md),
  [CLI](wiki/en/CLI.md), [FAQ](wiki/en/FAQ.md),
  [Feishu](wiki/en/Feishu.md), [DingTalk](wiki/en/DingTalk.md),
  [Models](wiki/en/Models.md) *(auto-generated — do not edit by hand)*

## 发布包内附说明（`dist/`）

这些文件在打包时逐字复制为发布包中的 `Readme.txt`（macOS / Windows /
Linux 便携包）。它们是**活文档**——只与打包流水线同步修改；发布工作流引用
的就是这个确切路径，因此不经同步修改
`.github/workflows/build-{macos-signed,windows-signed,linux}.y*ml` 就不得改名该目录。

- [readme-macos.txt](dist/readme-macos.txt) / [en](dist/readme-macos-en.txt)
- [readme-windows.txt](dist/readme-windows.txt) / [en](dist/readme-windows-en.txt)
- [readme-linux.txt](dist/readme-linux.txt) / [en](dist/readme-linux-en.txt)

## 历史档案（`archives/`）

历史审计快照；其结论仅适用于记录的日期/commit，不代表当前源码。保留
溯源用途——不要为对齐当前代码而改写。

- [bughunt/](archives/bughunt/README.md) — bug 狩猎证据与修复记录
  （agent / apps / cli / loop-tui），含模型输出溯源 JSON
- [verification/](archives/verification/errors-outdated-missing.zh-CN.md) —
  文档↔源码核验快照（事实清单、错误/过时/缺失清单、沙箱/e2ee/延迟审计）

## 维护者文档（`maintainers/`）

- [wiki-prompt.md](maintainers/wiki-prompt.md)（[中文](maintainers/wiki-prompt.zh-CN.md)）—
  （重新）生成 wiki 页面的提示词；定义范围、风格与页面清单。

## 审计（`audits/`）

预留目录，供后续文档↔代码审计报告使用。

## 文档如何保持正确

- `docs/wiki/{en,zh}/Models.md` 由 `make generate-models`
  （scripts/generate_models.py）生成——切勿手工编辑。
- wiki 页面按 [wiki-prompt.md](maintainers/wiki-prompt.md) 的范围撰写：
  macOS/Windows/Linux 桌面、Android/iOS 远程、各平台沙箱、无独立 TUI 页、
  CLI 名为 `future`。传输细节归入排障/CLI 与仓库指南。
- 变更声明须对照当前源码核验并同时更新两种语言。
  历史核验记录保留原始证据，而非永久 PASS。
- [文档检查](../scripts/check-docs.py)：`make check-docs`（或
  `python3 scripts/check-docs.py`）强制校验目录归属、双语配对、本地链接、
  wiki 目标与代码围栏。两种模式：
  - 默认——问题即错误；`BILINGUAL_PENDING` 中的条目允许存在并只报告数量，
    因此迁移途中的文档树仍可校验。
  - `--strict-pending`——终验模式：债务清单必须为空，即每篇文档确实都有两种语言。
  - `--scope docs/guide,docs/architecture`——只报告这些路径下的问题，便于分片工作。
    过滤依据是问题**所属文件**，而不是报错文本。
- `docs/` 下的每个 `.md` 都需要两种语言（新增的 `docs/` 子目录自动继承该要求；
  `docs/wiki/` 按 `en/`+`zh/` 配对，`docs/dist/` 按 `-en` 后缀配对）。
  例外为 `WHITELIST` / `EXTRA_PAIR_SCOPED` 中列出的文件。
- `make test-docs-check` 运行该校验器自身的回归测试，含反向用例
  （一旦校验器不再能发现问题，测试就会失败）。
- 当前没有任何流程自动运行它：**未接入 CI，也未接入 `make lint`**。
  若要接入流水线，应当作为一次明确的决定。
