# 决策内核回归基线（P0 快照）

> （[English](snapshots.md)）拆分前 `src/decision.rs`（696 行）的逐字节快照。它是
> G-1 内核模块化的冻结基线：`decision.rs` 被拆分为 `src/decision/` 子域模块
> （identity / boundary / frontier / monitor / stall / heartbeat recommendation /
> goal boundary / primary action），本快照锚定逐字段 packet 一致性回归。

## 快照文件

| 文件 | 说明 |
|---|---|
| `decision.rs.pre-split` | 拆分前 `src/decision.rs`（696 行）的逐字节副本，即完整的拆分前决策内核 |

## 基线元数据

- **源 commit**：`85372a53a0b61f57ba492894788249fb66315b94`
  （分支 `claude/loop-orchestrator`，"full lifecycle process + quota packet
  parity"，2026-08-05）
- **SHA-256**：`4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165`
- **行数**：696
- **范围**：`decide`/`decide_for`（should-run 决策编译）、`complete_todo` 义务
  （successor / no-follow-up）、monitor 到期轮询/退避、replan
  （成功/失败/停滞/验收缺口），以及 `packet()` 组装（约 40 字段
  `ShouldRunPacket` + 交互契约通道 + 子契约）。
- **基线测试**：11 个文件的 76 个契约测试，快照时全绿（2026-08-06）。

## 回归如何强制

快照由 `tests/decision_split_regression.rs` 消费：

1. **拆分回归（权威）**：拆分前内核作为仅测试的 legacy 模块逐字编译
   （`tests/legacy/decision_pre_split.rs`，仅机械变换），与重构后的内核在 20 个
   覆盖每条决策路径的夹具上并排运行；序列化 packet 逐字段比对（递归 JSON，
   wall-clock/UUID 掩码）。唯一允许的差异是 G-2/G-11 调度仲裁记录。
2. **出处守卫**：`generated_legacy_module_is_derived_from_snapshot` 从
   `decision.rs.pre-split` 重新派生 legacy 模块并验证快照无一行丢失——快照无法
   静默漂移。
3. **哈希检查**（带外）：
   ```sh
   shasum -a 256 orchestration/loop/snapshots/decision.rs.pre-split
   # 期望 4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165
   ```

## 更新策略

- 快照是**冻结**的：仅当拆分回归全绿且 packet 字段/enum 面与快照语义一致时才
  覆盖它；随后更新本 README 的源 commit 与哈希字段。
- 持久回归锚点是 `tests/decision_split_regression.rs`（23 个测试）；快照本身
  的存在是为了在源文件消失后仍能验证拆分前的逻辑。
