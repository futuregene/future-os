# 上游：LoopX

> （[English](UPSTREAM.md)）`future-loop` crate（位于 `orchestration/loop/`）含有
> 从 **LoopX**（一个面向长程 AI Agent 工作的控制面）翻译、移植并做结构适配的代码。

- **上游仓库：** <https://github.com/huangruiteng/loopx>
- **上游作者：** Ruiteng Huang 与 LoopX 贡献者
- **上游许可：** Apache License, Version 2.0（见 `orchestration/loop/` 中的
  [`LICENSE`](LICENSE)）。LoopX 至 v0.4.7 的发行版以 MIT
  许可分发；v0.4.8 是首个 Apache-2.0 发行版（见 [`NOTICE`](NOTICE)）。

## 基线版本

- 本派生跟随 LoopX **v0.4.x** 线，至
  [`v0.4.8`](https://github.com/huangruiteng/loopx/releases/tag/v0.4.8)
  （commit `8c103dfecae0f4424ecb0b07bad7cbc5f0797d6d`），经上游维护者对本实现审阅
  确认（2026-08）。
- 首次引入 FutureOS：2026-08-06，commit `e2a8fb84`（PR #97）。

## 派生代码范围

LoopX 用 Python 编写；`future-loop` 是其控制面的原生 Rust 重实现。派生子系统包括：

- 确定性 should-run 决策内核及其决策子域（identity / boundary / frontier /
  monitor / stall / heartbeat / goal-boundary / primary-action）；
- 事件溯源状态账本、事件重放与 markdown 回填；
- 配额与槽位记账（run / agent / heartbeat）；
- 调度仲裁层及其处置；
- markdown 工作台与 sidecar 文件格式（`ACTIVE_GOAL_STATE.md`、lockfile、工作台
  布局）；
- `loopx` 风格的 CLI 命令面（控制台命令）；
- agent 注册 / 编排、claim / lease、门禁、监视器与备份 / 恢复。

源码中标记为 `LoopX: ...` 或 `` LoopX `<module>` `` 的注释把各个函数与行为对应到
上游相应模块。

## FutureGene 的修改

FutureGene 对其修改与新增原创部分持有版权，包括：

- Rust 原生实现本身（类型系统、存储、并发、含 Windows 的跨平台支持）；
- 面向 FutureOS agent 的 gRPC 执行桥接与 typed-RPC 线上契约（`future-rpc`
  类型化 payload）；
- 统一的 `future loop` CLI 及与 FutureOS TUI、桌面端应用、技能的集成；
- 上游没有对应物的特性（canary smoke、automation liveness、read-model 自愈、
  pid lockfile / 僵尸接管）；
- 项目本地状态布局与 FutureOS 目录约定。

## 关系

Future Loop 是 FutureGene 维护的**独立下游实现**。它**不是** LoopX 官方发行版，
也**未经** LoopX 项目认证或背书。与上游 LoopX 状态文件的兼容是尽力而为。
