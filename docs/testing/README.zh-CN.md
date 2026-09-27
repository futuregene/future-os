# 测试任务记录

本目录是逐模块覆盖率战役（coverage campaign）的任务记录：某个模块或应用
子树如何测量、还剩哪些未覆盖行、登记了哪些 waiver 及理由、跑了哪条验收
gate。这些是 agent 的工作记录，不是面向用户的产品文档——它们也是 `docs/`
下唯一被刻意豁免双语配对要求的子树（`scripts/docs/check-docs.py` 中的
`PAIR_EXEMPT`），因此只有英文。

## 目录内容

| 记录 | 内容 |
| --- | --- |
| `module-*.md` | 每个 Rust crate 或应用子树一篇：运行身份、测量命令、未覆盖行、各维度证据、waiver 台账、未解决项 |
| `desktop-*.md` | 桌面各子树（agent、shell、panels、settings、packages）的同类记录 |
| [review-rust.md](review-rust.md)、[review-frontend.md](review-frontend.md) | 试图证伪 waiver 的对抗性复核 |
| [weak-test-audit.md](weak-test-audit.md)、[disabled-test-audit.md](disabled-test-audit.md)、[weak-test-fixes.md](weak-test-fixes.md) | 弱测试与被禁用测试的审计，以及随后的修复 |
| [mutation-report.md](mutation-report.md) | 变异测试战役：范围、逐 mutant 判定、存活 mutant 的裁定（runner 与证据在 `scripts/measure/mutation/`） |
| [platform-coverage.md](platform-coverage.md) | 每条未测量行以哪个平台为准 |
| [dimension-matrix.md](dimension-matrix.md) | 从各模块记录中摘引的 boundary / error-path / concurrency / property / platform-cfg / serialization 证据索引 |
| [waiver-ledger.md](waiver-ledger.md) | waiver 注册表与类别政策 |

每篇记录的第 §1 节（`## 1.`）给出运行身份（机器、修订、套件状态）与精确的
测量命令。要重跑某条 gate，请从对应模块的 §1 复制命令——不要凭记忆重建：
这些参数是关键项（例如 `--no-fail-fast` 与私有的 `CARGO_TARGET_DIR`）。

## 这些记录的用途

- 源码注释引用具体章节，因此章节号是引用目标——要保持稳定，或至少保持其
  含义。例如：`orchestration/loop/src/webui/server.rs` 与
  `orchestration/loop/tests/*.rs` 引用 `module-loop.md`；`tui/src/app.rs`
  引用 `module-tui.md`；桌面测试引用 `desktop-*.md`。
- 战役 gate 只检查结构：每个存在未覆盖行的文件，必须在路径同一行注明
  waiver 类别。理由是否成立由复核者负责——未被证伪的 waiver 记为证据，
  被推翻的则返工。
- waiver 类别及相关政策（`unreachable-by-construction`、
  `unreachable-in-this-environment`、`attribution-artifact`、
  `platform-unmeasured`；`OPEN` / `not waived` 一律视为未豁免）定义在
  [waiver-ledger.md](waiver-ledger.md)；平台相关行以
  [platform-coverage.md](platform-coverage.md) 为准。

每篇记录正文只保留仍然有效的内容——现行 waiver、未解决项、gate 命令与操作
说明。被取代的历史快照移入 `docs/archives/testing/`，原处留一行指针。

## 相关

- 用户文档索引：[docs/README.md](../README.md)
- 覆盖率与性能测量工具：`scripts/measure/`；常设的 ratchet 门禁是
  `scripts/measure/coverage.sh --check` 与
  `scripts/measure/coverage_ratchet.py`
- 文档门禁及其规则：`scripts/docs/check-docs.py`
