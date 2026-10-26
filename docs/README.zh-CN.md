# FutureOS 文档

> `docs/` 目录导航索引。链接均在本仓库内解析；面向用户的 wiki 见 [docs/wiki/](wiki/en/Home.md)（en）与
> [docs/wiki/zh/Home.md](wiki/zh/Home.md)（zh）。
>
> `docs/` 下每篇面向用户的文档均为双语（`name.md`=英文，`name.zh-CN.md`=中文）；
> `testing/` 下的 agent 任务记录是刻意保留的仅英文例外。
> `scripts/docs/check-docs.py` 强制校验目录归属、双语配对、链接、脚本路径引用与代码围栏。
> `archives/` 下的历史审计文档保留原始时间与 commit 边界。

## 指南（`guide/`）

| 文档 | 内容 |
|---|---|
| [构建与安装](guide/build-and-install.zh-CN.md)（[en](guide/build-and-install.md)） | 前置条件、各平台工具链（macOS / Linux / Windows）、`make` 目标、GUI 打包、`future-goal` 安装、技能安装 |
| [持续集成](guide/ci.zh-CN.md) ([English](guide/ci.md)) | 影响范围检测、依赖图、共享 runner、合并门禁与手动专项验证 |
| [TUI](guide/tui.zh-CN.md)（[en](guide/tui.md)） | 终端界面（`future tui`）：全屏布局、键鼠、命令面板能跑的命令，以及哪些已经接上 agent |
| [渠道配置](guide/channels-config.zh-CN.md)（[en](guide/channels-config.md)） | `~/.future/channels/config.json` 统一参考（agent / providers / Feishu / DingTalk 块、默认值） |
| [通道 provider 参考](guide/channels-providers.zh-CN.md)（[en](guide/channels-providers.md)） | 共享桥上的通道：能力矩阵、逐通道配置、成熟度、诊断、主动外发 |
| [通道 provider 契约](guide/channels-provider-contract.zh-CN.md)（[en](guide/channels-provider-contract.md)） | 新增通道：provider 契约、入站/出站边界、配置、错误、测试与覆盖率要求 |
| [Desktop Agent](guide/desktop-agent.zh-CN.md)（[en](guide/desktop-agent.md)） | 默认内置/外部服务、退出中断范围、CLI 接入、开发启动、日志与升级恢复 |
| [主机模式](guide/desktop-host.zh-CN.md)（[en](guide/desktop-host.md)） | 前台 `future host` 启动、平台登录二维码、用 `future remote devices add` 配对、Ctrl+C、无 GUI 服务器构建与排障 |
| [Headless 发布验收](testing/headless-release-acceptance.zh-CN.md)（[en](testing/headless-release-acceptance.md)） | 最终成功 CI、打包签名、真实安装升级、性能与同候选验收遗留清单 |
| [Headless unsigned 下载与结果](testing/headless-unsigned-results.zh-CN.md)（[en](testing/headless-unsigned-results.md)） | 实际 dev 包、公开地址、SHA-256、CLI/sidecar 和静态/旧 Linux 验证 |
| [目录布局](guide/directory-layout.zh-CN.md)（[en](guide/directory-layout.md)） | `~/.future/` 下各目录职责（agent、channels、TUI、GUI、goal） |
| [数据备份与恢复](guide/data-backup.zh-CN.md)（[en](guide/data-backup.md)） | `future data`：离线备份、校验、恢复到原 home、中断恢复后的 resume/rollback，以及启动围栏 |
| [会话历史召回](guide/session-history.zh-CN.md)（[en](guide/session-history.md)） | 只读的历史检索／条目读取、字节分页，以及压缩后的模型指引 |
| [移动端延迟诊断](guide/mobile-latency-diagnosis.zh-CN.md)（[en](guide/mobile-latency-diagnosis.md)） | 移动端端到端延迟测量与诊断 |
| [截图工具](guide/screenshots.zh-CN.md)（[en](guide/screenshots.md)） | 用演示数据渲染真实桌面端 / 手机端界面，在无显示器环境产出截图、功能示意图与图文文档 |

仓库根 [README](../README.zh-CN.md)（[en](../README.md)）为入口；
[wiki](wiki/zh/Home.md) 是面向用户的应用指南。

## 架构（`architecture/`）

| 文档 | 内容 |
|---|---|
| [Goal 控制面](architecture/goal-control-plane.zh-CN.md)（[en](architecture/goal-control-plane.md)） | `future-goal` — 目标/todos/门禁/监控、should-run 内核、配额、事件溯源、交付闭环、多 agent、supervisor/worker 消息、web dashboard |
| [长程证据账本](architecture/long-run-evidence-ledger.zh-CN.md)（[en](architecture/long-run-evidence-ledger.md)） | 长程 goal 目标问责记录 — 墙钟、花费、验证结果、每个已关闭目标的显式边界 |
| [定时任务](architecture/tasks.zh-CN.md)（[en](architecture/tasks.md)） | `future-tasks` — 可复用提示词 + 触发器（按计划或随时手动）、全权限运行、确定性 `next_due` 内核、运行台账与提示词版本历史 |
| [SQLite 迁移](architecture/sqlite-migration.zh-CN.md)（[en](architecture/sqlite-migration.md)） | Agent/Desktop SQLite 存储布局与迁移策略 |
| [精简会话存储开发计划](architecture/stream-storage-development.zh-CN.md)（[en](architecture/stream-storage-development.md)） | 待 review：规范分段正文、实时与持久化解耦、崩溃一致性、历史兼容与分阶段验收 |
| [Desktop 内置 Agent 开发计划](architecture/embedded-agent-development.zh-CN.md)（[en](architecture/embedded-agent-development.md)） | 进程内完整 Agent、服务与宿主生命周期、实例锁、退出清理、升级安装、开发工具及分阶段验收 |
| [Agent 启动仲裁与自愈](architecture/agent-supervision.zh-CN.md)（[en](architecture/agent-supervision.md)） | 同一数据根下 `future agent` 与 `future host` 谁启动 Agent：实例锁仲裁、让位而非失败、有界等待、崩溃熔断，以及 TUI 为何只连接（D11） |
| [回复结果语义](architecture/response-outcomes.zh-CN.md)（[en](architecture/response-outcomes.md)） | 回复结束的结果语义（停止 / 拒答 / 过滤 / 暂停） |
| [goal/](architecture/goal/README.md) | `future-goal` crate：[架构](architecture/goal/ARCHITECTURE.md)（en/zh）、[上游来源](../orchestration/goal/UPSTREAM.md)、[决策内核快照](architecture/goal/snapshots.md) |
| [共享包](architecture/packages.zh-CN.md)（[en](architecture/packages.md)） | `packages/` 约定：npm/workspace 包与 crate 边界 |
| [通道测试覆盖率](architecture/channels-test-coverage.zh-CN.md)（[en](architecture/channels-test-coverage.md)） | 通道框架的测试承诺什么，以及有意不覆盖的那几行代码及每行的原因 |
| [RPC crate](architecture/rpc.zh-CN.md)（[en](architecture/rpc.md)） | `packages/rpc` — protobuf 线上契约（唯一事实来源） |
| [远程下载分档](architecture/remote-download-tiers.zh-CN.md)（[en](architecture/remote-download-tiers.md)） | 桌面保存副本不是手机下载：按客户端分档（手机 10 MiB、桌面 512 MiB）、桌面档去掉了什么、旧 host 的受匹配拒绝文案及其验收证据 |

## 内部文档（`internals/`）

桌面应用与移动客户端的分模块工作文档：产品语义、数据模型、各平台沙箱、连接与远程行为、上下文压缩、移动端构建与平台对齐记录，以及 TUI 测试框架约定。

- [desktop/](internals/desktop/PRODUCT.md) — 产品语义、数据模型（`ER.md`）、颜色、沙箱（macOS/Windows/Linux）、连接与远程、嵌入式终端、压缩（文档地图见 `desktop/CLAUDE.md`）
- [compaction/](internals/compaction/compaction.zh-CN.md) — 运行时上下文压缩：两个策略及其参数、生产请求形状、开发指南（`compaction-development.zh-CN.md`）、检索接口，以及两个实验——闭卷（`compaction-closed-book-experiment.zh-CN.md`）与开卷（`compaction-open-book-experiment.zh-CN.md`）
- [mobile/](internals/mobile/README.md) — 移动端构建/TestFlight、iOS 平台对齐、流式同步设计与测量、鸿蒙兼容（带日期的审计与测量报告见 `archives/verification/`）
- [skill_reco/](internals/skill_reco/harness.zh-CN.md) — 技能推荐：[评测](internals/skill_reco/evaluation.zh-CN.md)、[运行框架](internals/skill_reco/harness.zh-CN.md) 与[集成方案](internals/skill_reco/integration-plan.zh-CN.md)（历史设计记录）
- [agent/](internals/agent/README.zh-CN.md) — gRPC 后端（`agent/`）的模块地图：turn loop、工具、LLM 层、会话、沙箱，以及改每一项前该读什么
- [host](internals/host.zh-CN.md) — `future-host`（无 GUI 的共享后端：store、agent bridge、tasks、remote、identity）的模块地图
- [Desktop NATS 桥](internals/desktop-nats.md)
- [Windows 安装/更新](internals/desktop-windows.md)
- [Provider 协议测试](internals/provider-protocol.md)

## TUI 设计（`tui/`）

| 文档 | 内容 |
|---|---|
| [Workbench TUI 规格](tui/workbench-spec.zh-CN.md)（[English](tui/workbench-spec.md)） | [终端界面](guide/tui.zh-CN.md) 背后的冻结契约：布局常量、鼠标与键盘约定、类型名与验收命令。它描述的界面已经实现；agent 适配层与 P1 项还没有 |
| [没进表的界面文案](tui/i18n-homeless-strings.zh-CN.md) （[English](tui/i18n-homeless-strings.md)） | 当 workbench 绘制了取自 `workbench/i18n.rs` 之外的文案时会失败的检查：它读什么、看不见什么、顺带冻住的既有纯英文文案，以及在引入这张表之前离开的分支要移植的清单 |

## Wiki（面向用户的应用指南）

- 中文: [首页](wiki/zh/Home.md), [安装](wiki/zh/Installation.md),
  [快速开始](wiki/zh/Quick-Start.md), [使用 FutureOS](wiki/zh/Using-FutureOS.md),
  [设置](wiki/zh/Settings.md), [审批与沙箱](wiki/zh/Sandbox.md), [手机远程](wiki/zh/Remote.md), [技能](wiki/zh/Skills.md),
  [定时任务](wiki/zh/Scheduled-Tasks.md),
  [命令行工具](wiki/zh/CLI.md), [FAQ](wiki/zh/FAQ.md),
  [飞书](wiki/zh/Feishu.md), [钉钉](wiki/zh/DingTalk.md),
  [模型目录](wiki/zh/Models.md) *(自动生成)*
- English: [Home](wiki/en/Home.md), [Installation](wiki/en/Installation.md),
  [Quick Start](wiki/en/Quick-Start.md), [Using FutureOS](wiki/en/Using-FutureOS.md),
  [Settings](wiki/en/Settings.md), [Sandbox](wiki/en/Sandbox.md), [Remote](wiki/en/Remote.md), [Skills](wiki/en/Skills.md),
  [CLI](wiki/en/CLI.md), [FAQ](wiki/en/FAQ.md),
  [Feishu](wiki/en/Feishu.md), [DingTalk](wiki/en/DingTalk.md),
  [Models](wiki/en/Models.md) *(auto-generated — do not edit by hand)*

两种语言还各有 `_Sidebar.md` / `_Footer.md` 导航页。

## 发布包内附说明（`dist/`）

这些文件在打包时逐字复制为发布包中的 `Readme.txt`：未签名的 macOS 测试
DMG、Windows 便携 zip 与 Linux 便携 tarball。它们是**活文档**——只与打包
流水线同步修改；发布工作流与 `scripts/build/` 下的脚本引用的就是这个确切
路径，因此不经同步修改
`.github/workflows/build-{macos-signed,windows-signed,linux}.y*ml` 就不得改名该目录。

- [readme-macos.txt](dist/readme-macos.txt) / [en](dist/readme-macos-en.txt)
- [readme-windows.txt](dist/readme-windows.txt) / [en](dist/readme-windows-en.txt)
- [readme-linux.txt](dist/readme-linux.txt) / [en](dist/readme-linux-en.txt)

## 历史档案（`archives/`）

历史审计快照；其结论仅适用于记录的日期/commit，不代表当前源码。保留
溯源用途——不要为对齐当前代码而改写。

- [bughunt/](archives/bughunt/README.md) — bug 狩猎证据与修复记录
  （agent / apps / cli / goal-tui），含模型输出溯源 JSON
- [verification/](archives/verification/errors-outdated-missing.zh-CN.md) —
  带日期的文档↔源码核验快照：事实清单、错误/过时/缺失清单、
  [文档↔代码不符审计](archives/verification/doc-code-mismatches.zh-CN.md)、
  沙箱/E2EE/延迟审计，以及移动端快照：[Markdown 展示审计](archives/verification/markdown-rendering-audit.zh-CN.md)、
  [浏览器同步测量](archives/verification/streaming-sync-browser-measurement.zh-CN.md)、
  [缓存重开测量](archives/verification/streaming-sync-warm-measurement.zh-CN.md) 与性能/问题报告、去身份测量 JSON
- [testing/](archives/testing/) — 被取代的覆盖率战役任务记录快照，整体移出
  `testing/`；同属非用户文档仅英文例外

## 维护者文档（`maintainers/`）

- [wiki-prompt.md](maintainers/wiki-prompt.md)（[中文](maintainers/wiki-prompt.zh-CN.md)）—
  （重新）生成 wiki 页面的提示词；定义范围、风格与页面清单。

## 审计（`audits/`）

预留给进行中的文档↔代码审计报告；已完成的带日期报告归档到
`archives/verification/`（例如 2026-09-16 的
[文档↔代码不符审计](archives/verification/doc-code-mismatches.zh-CN.md)）。

## 测试任务记录（`testing/`）

- [testing/](testing/README.zh-CN.md)（[en](testing/README.md)）— 逐模块覆盖率
  战役的任务记录：运行身份、测量命令、未覆盖行、各维度证据、waiver 与未解决项。
  面向 agent 的工作笔记，不是用户文档；按 `scripts/docs/check-docs.py` 中刻意的
  `PAIR_EXEMPT` 例外只有英文；被取代的快照保存在 `archives/testing/`。

## 文档如何保持正确

- `docs/wiki/{en,zh}/Models.md` 由 `make generate-models`
  （scripts/docs/generate_models.py）生成——切勿手工编辑。
- wiki 页面按 [wiki-prompt.md](maintainers/wiki-prompt.md) 的范围撰写：
  macOS/Windows/Linux 桌面、Android/iOS 远程、各平台沙箱、无独立 TUI 页、
  CLI 名为 `future`。传输细节归入排障/CLI 与仓库指南。
- 变更声明须对照当前源码核验并同时更新两种语言。
  历史核验记录保留原始证据，而非永久 PASS。
- [文档检查](../scripts/docs/check-docs.py)：`make check-docs`（或
  `python3 scripts/docs/check-docs.py`）强制校验目录归属、双语配对、本地链接、
  wiki 目标、代码围栏，以及现行文档中每个仓库内 `scripts/` 路径都真实存在
  （Windows 分隔符先归一化处理；文档中的 glob 必须至少匹配一个文件；
  `docs/archives/**` 作为冻结历史豁免）。两种模式：
  - 默认——问题即错误；`BILINGUAL_PENDING` 中的条目允许存在并只报告数量，
    因此迁移途中的文档树仍可校验。
  - `--strict-pending`——终验模式：债务清单必须为空，即每篇文档确实都有两种语言。
  - `--scope docs/guide,docs/architecture`——只报告这些路径下的问题，便于分片工作。
    过滤依据是问题**所属文件**，而不是报错文本。
- `docs/` 下的每个 `.md` 都需要两种语言（新增的 `docs/` 子目录自动继承该要求；
  `docs/wiki/` 按 `en/`+`zh/` 配对，`docs/dist/` 按 `-en` 后缀配对）。
  `docs/testing/` 与 `docs/archives/testing/` 作为非用户文档（覆盖率战役的
  agent 任务记录）被豁免；`BILINGUAL_PENDING` 债务清单必须为空。`docs/` 之外的
  Markdown 只允许 `WHITELIST` 中列出的路径；`EXTRA_PAIR_SCOPED` 要求其中少数
  文件也配对（`SECURITY*`、`THIRD_PARTY_NOTICES*`、`orchestration/goal/UPSTREAM*`）。
- `make test-docs-check` 运行该校验器自身的回归测试，含反向用例
  （一旦校验器不再能发现问题，测试就会失败）。
- 当前没有任何流程自动运行它：**未接入 CI，也未接入 `make lint`**。
  若要接入流水线，应当作为一次明确的决定。
