# coverage-waivers-2026-11 — 目录索引

2026-11 覆盖率战役的四条 lane，**每条 lane 一个文件**，一行一条豁免。
这里只放"测不到的行"和它们为什么测不到；**能写的测试不写在这里**，写进代码。

`docs/testing/waiver-ledger.md` 里这个战役只占**一行**（2026-11 campaign），指向本目录——
那条行曾经每次 lane 落地都要改一次，三份并行的分支因此冲突了五次。现在数字只维护在
本目录的文件里，账本那一行只说"见目录"。

## 文件

| 文件 | lane | 行数 | 覆盖范围 | 最新 `DA:0` |
|---|---|---:|---|---|
| [web.md](web.md) | web（desktop vitest），布局与交互组 | 37（33 `W2` + 4 `W8`） | `desktop/src/components/layout/`、`features/vibe/`、`features/agent/`、`features/palette/`、`features/tasks/`、`terminal/`、`artifacts/`… | lane 102，其中 37 条在此 |
| [web-remote.md](web-remote.md) | web（desktop vitest），远端 / 设置组 + 收口批次 2-3 采纳的 16 条 | 42（38 `W2` + 2 `W5` + 2 `W8`） | `desktop/src/features/remote/`、`features/remote-peer/`、`features/settings/`；行 27-42 来自 lane 审计（见文件内说明） | 42 条在此 |
| [tui.md](tui.md) | tui（rust, llvm-cov） | 2 | `tui/src/workbench/live.rs` 的两处 | 见该文件头部 |
| [mobile.md](mobile.md) | mobile（jest） | 8 | `mobile/src` | 见该文件头部 |

`mobile.md` 与 `tui.md` 的权威数字在各自文件头部，本索引不复述——复述过的数字迟早会不一致。

## 2026-11 web lane 收口审计（2026-11-01）

跑在 `origin/dev-huichen@12b89ee7e`，报告见
`coverage/2026-11/after/web2/lane-audit.md`：

| | value |
|---|---:|
| `DA:0`（整条 web lane） | **95** 行 / **48** 文件 |
| 已在 `web.md` + `web-remote.md` 登记 | **83**（web.md 40 + web-remote 42） |
| 已登记行的审计结果 | **83 / 83 通过**（仍为 `DA:0`，引用的构造仍在原位） |
| 未登记但仍为 `DA:0` | **12**（可覆盖 9 / 暂缓（AppShell 脚手架）4 / 待定 1） |
| **未归类（必须为 0）** | **0** |

结论：**没有静默缺口**。12 行工作项不等于 100 %，它们是可写测试、待定的边界或被评估为暂缓的一次脚手架；
"覆盖 + 豁免 + 已归类工作项 = 全部行"这一条成立。

## 规矩

1. **一行一个 falsifier。** 每条豁免必须能说出"什么改动会让它跑起来"（正对照）和
   "钉住它的用例名"。说不出来的不算证据。
2. **读到 `1` 的行要删。** 豁免行对应的代码一旦被覆盖，行不是"过时"而是**错误**；
   审计脚本会把这种情况报成失败。
3. **产品代码不动。** 删守卫、把私有函数导出给测试用，都不允许——覆盖变化只能是
   结果，不能是手段。
4. **工作项不进本目录的豁免表。** 还没写的测试记在 lane 的审计报告里（例如
   `lane-audit.md` 的三分类清单），一旦登记成豁免就必须同时给出机制与正对照。

## 重跑

```bash
# web lane（desktop vitest）
cd desktop && npx vitest run --coverage --coverage.reporter=lcov && cd ..
python3 coverage/2026-11/tools/lcov-missed.py desktop/coverage/lcov.info --top 40

# 审计全部登记行 + 未归类计数（web lane）
python3 coverage/2026-11/after/web2/audit-lane.py \
    --lcov desktop/coverage/lcov.info \
    --docs docs/testing/coverage-waivers-2026-11 \
    --root desktop/src \
    --work-items coverage/2026-11/after/web2/lane-audit.md
# → registered rows: 83  pass: 83  fail: 0
# → UNACCOUNTED (must be 0): 0
```

指标口径（metric A）：行计数只看 LCOV 的 `DA:<line>,0`；istanbul / llvm-cov 的
summary 百分比不是验收数字。
