# Future Goal（`future-goal`）

> （[English](README.md)）FutureOS 的 goal 控制面：持久化目标 / 任务 / 门禁 / 配额、
> 确定性 should-run 决策内核、事件溯源状态，以及 gRPC 执行桥接——产品化为
> `future goal` CLI。

文档：[`ARCHITECTURE.md`](ARCHITECTURE.md)（设计原则，[中文版](ARCHITECTURE.zh-CN.md)）
与 [`../goal-control-plane.md`](../goal-control-plane.md)（运维模型）。

## 许可与归属

`future-goal` crate（位于 `orchestration/goal/`）含有派生自
[LoopX](https://github.com/huangruiteng/loopx) 的代码，以
**Apache License, Version 2.0** 分发——见 [`LICENSE`](../../../orchestration/goal/LICENSE)、[`NOTICE`](../../../orchestration/goal/NOTICE)
与 [`UPSTREAM.md`](../../../orchestration/goal/UPSTREAM.md)，其中载明上游基线版本、派生代码范围以及
FutureGene 的修改。原创部分 Copyright 2026 LoopX contributors；修改与新增原创
部分 Copyright 2026 FutureGene。

Future Goal 是 FutureGene 维护的**独立下游实现**。它**不是** LoopX 官方发行版，
也**未经** LoopX 项目认证或背书。

FutureOS 其余部分以 MIT 许可分发——见仓库根目录。
