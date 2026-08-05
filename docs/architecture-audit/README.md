# FutureOS 架构审计（2026-08-05）

针对 FutureOS 的四个维度做的一次只读架构审计，四份报告独立成篇，可分别修复。

| 报告 | 主题 | 一句话结论 |
|---|---|---|
| [01-agent-guirust-boundary.md](./01-agent-guirust-boundary.md) | agent ↔ gui_rust 边界 | 泄漏且双向：影子 JSON 契约 + 7 条文件系统旁路 + 编译期源码 include |
| [02-guirust-guireact-boundary.md](./02-guirust-guireact-boundary.md) | gui_rust ↔ gui_react 边界 | 架构纪律好（invoke 单一入口、状态零重复），但类型全手工同步、agent 透传域无类型，已出现漂移 |
| [03-large-modules-split.md](./03-large-modules-split.md) | 超大模块与拆分建议 | 18 个候选：3 个强烈建议拆、9 个建议拆、6 个内聚不拆；含逐个文件的职责清单与拆分方案 |
| [04-react-rendering-performance.md](./04-react-rendering-performance.md) | React 渲染性能 | 4 个 HIGH 问题让既有性能投资失效；H1/H2 是一行级改动、全局级收益 |

**审计基准**：`dev @ 8aa82925`（2026-08-05）。调查实际在工作树 `8164b8e1` 上进行，两者树内容完全一致（diff 为空），报告中所有 file:line 在当前 dev 上可直接使用。

**方法**：4 个并行调查（gRPC/proto 契约与文件旁路、Tauri IPC 表面与类型契约、大文件函数级职责分析、流式热路径渲染反模式），全部结论带 file:line 证据；最关键的论断（`#[path]` include、proto `data` 字符串、`SessionState` 未使用、裸 Value 命令、唯一 memo、`handleFork` deps）已二次抽查验证。

**四份报告的交叉引用**：
- 报告 01 的"影子 JSON 契约"与报告 02 的"agent 透传域无类型"是同一根因在两个边界上的表现，修复时应一起考虑（proto 类型化 → Rust struct → TS 契约一条链）。
- 报告 03 建议拆分的 `threadRunProjection.ts`、`agentActivity.ts` 与报告 04 的 H1/M4/M5 在同一批文件里动刀，建议先做 04 的性能修复再做大拆分（或同一分支内按文件协调），避免互相冲突。
