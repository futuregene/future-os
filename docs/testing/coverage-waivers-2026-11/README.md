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
| [host.md](host.md) | host（rust, llvm-cov） | 90（21 `W1`-`W7` + 20 `W8` + 33 `W9`-`W12` + 1 `W13` + 15 `W14`-`W17`） | `host/src/**` 的 lib lane（`cargo llvm-cov -p future-host --lib`） | lane 2464，其中 90 条在此；**358 OPEN**（`spawn_host` 与其 `perform` 臂已由实测关闭，剩客户端角色的一半，见 §4.1） |
| [tui.md](tui.md) | tui（rust, llvm-cov） | 2 | `tui/src/workbench/live.rs` 的两处 | 见该文件头部 |
| [cli.md](cli.md) | cli（rust, llvm-cov） | 53（16 行登记 + 79 行一类关闭符） | `relay/`（cli、server、room 的 HTTP 与注册表）、`prompt` 的行回退与 OS 失败臂、`commands/remote_peers` 的 add/list/rename、`commands/remote` 的报告与面板臂、`headless/agent` 的 `supervise` | **266**（49 行登记、130 行工作项、8 行待产品决定，均不是豁免）；另见 [cli-safari-gate-flake.md](cli-safari-gate-flake.md) |
| [mobile.md](mobile.md) | mobile（jest） | 8 | `mobile/src` | 见该文件头部 |

`mobile.md` 与 `tui.md` 的权威数字在各自文件头部，本索引不复述——复述过的数字迟早会不一致。

## 2026-11 web lane 最终口径（结案）

跑在 `origin/dev-huichen@49f82af67`，报告：`coverage/2026-11/after/web2/lane-audit.md`。

| | 数字 |
|---|---:|
| lane 起点（`coverage/2026-11/after/web/lcov.info`） | **430** |
| lane 现在（`DA:0`） | **95**（48 个文件） |
| 已登记豁免（本轮逐条审计） | **83**（74 `W2` + 7 `W8` + 2 `W5`） |
| **OPEN：需要产品加 seam 的** | **0** |
| 高成本夹具（已归类，未写） | **12** |
| 未归类（必须为 0） | **0** |
| 同一轮的套件 | 403 files / 5647 tests 全绿 |

`audit-lane.py` 的两道检查（仍读 `DA:0`、引用的构造仍在原位）在这条 lane 上是
**83/83 通过**：没有一行陈旧，也没有一行在已被覆盖的情况下仍被声称。

### 本 lane **不能**声明 per-line 100%，原因写在这里

口径 A 的判据是 `DA:0 = 0`。这条 lane 读 **95**，所以它**不满足**，而两条"看起来能满足"
的路都是假的：

1. **把豁免加进覆盖**：83 条是 83 个论证，不是 83 行被跑过的代码；"覆盖 + 豁免 = 分母"
   是算术恒等式，与代码是否被行使无关。
2. **把剩下 12 行也登记成豁免**：其中四条尚未被证明为死码——第四批的实测**推翻**了当初的
   乐观判断（挂载够不到的 adopt 行、手搓 entry 够不到的 abort 回退、根本不存在的"全禁用
   菜单"）。今天登记就是"断言披着引用的外衣"。

它可以声明的，也是这个目录存在的理由：**没有静默缺口**（95 行 = 83 已登记 + 12 已归类，
未归类 0）、**没有陈旧行**（83/83 双重复核）、**没有为覆盖让步产品**（未加 seam、未导出、
未改签名）。

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
