# 变异测试采样 —— runner、证据与汇总

本目录是覆盖率战役所做的 `cargo-mutants` 采样：生成它们的脚本、保留下来的证据
都在这里。结论本身写在
[`docs/testing/mutation-report.md`](../../../docs/testing/mutation-report.md)。

已提交的内容（每次运行的 `out-*/` 目录由 runner 写出，**不**提交 —— 见本目录下
的 `.gitignore`）：

| 路径 | 说明 |
|---|---|
| `run-*.cmd` | 六个 Windows 驱动脚本，用于以分离进程启动 `cargo-mutants`（默认 copy 模式、`--gitignore true`、`--timeout 300`，从不使用 `--in-place`，不设 `CARGO_TARGET_DIR`） |
| `analyze-queue.py` | 逐变异体归因表：每个变异体**自己的**日志里到底哪些测试失败 |
| `verify-attribution.py` | 判定卫生检查：只靠修复前 flaky 测试才被杀死的「caught」 |
| `dump-fails.py` | 按名字筛选变异体的失败测试清单 |
| `compare-runs.py` | 主运行与幸存者复检的对比 |
| `summary.json` | 机器可读汇总（schema `future-cov100-mutation-summary-v1`）：范围、计数、逐个幸存者裁定、修订哈希存证 |
| `out-*-attribution.txt`、`out-*-hygiene.txt` | 报告引用的归因/卫生快照 |
| `out-*-baseline.log` | 三次**失败**的基线尝试（参数传递、磁盘写满），作为报告中教训的证据保留 |
| `revision-hashes-prerun.txt` | 运行前记录的三个被变异文件的 SHA-256 存证 |
| `policy-defaults-finding.md` | `channels/src/policy.rs` 一节背后的原始证据 —— 本目录**唯一**的 findings 说明文档，其余均为工具或数据 |

## 重跑

在仓库根目录、Windows 下：

```
scripts\measure\mutation\run-queue.cmd
```

驱动脚本会自己 `cd` 到仓库根目录，并把 `out-<run>/` 写在本目录旁边。
`TEMP`/`TMP` 必须在有空间的卷上 —— `cargo-mutants` 会把 `target/debug` 复制进
临时目录（每个并行任务约 1 GB），第一次尝试就因磁盘写满报 `os error 112`。
之后这样归因判定：

```
cd scripts/measure/mutation && python analyze-queue.py out-queue/mutants.out
```

## 大日志的用途

`out-*-baseline.log`（每个约 400 KB）是参数传递尝试失败的基线日志
（`--skip=` 在 cargo 的 `--` 之前会被拒绝；两个测试名位置参数也会被拒绝）。
保留它们是因为报告里的「arg plumbing」一节正是从中推导出来的；它们不属于任何分数。

## 约定

* 每个判定都必须通过该次运行的 `outcomes.json` 从变异体自己的日志推导 ——
  日志文件名上的 `_NNN` 后缀是运行顺序的去重计数，不是 `--list` 顺序。
* 只有失败断言能够观察到被变异行时才算 caught；`verify-attribution.py`
  会报告那些只依赖修复前 flaky 测试的判定。
* 永远不要设置 `CARGO_TARGET_DIR`，永远不要使用 `--in-place`。
