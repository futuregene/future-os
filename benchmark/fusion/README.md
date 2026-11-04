# Fusion reasoning benchmark

本目录同时服务两个目标：改进 Fusion 的讨论设计，以及积累可以复现的公开对比数据。
目前只比较 **本机保存的 Fusion 配置**与 **Future DeepSeek Flash `max`**。
DeepSeek Flash 的本机目录没有 `xhigh`，经用户确认使用最高可用等级 `max`；
脚本会检查实际能力声明，不接受不支持的等级回退成 `default`。

这是无外部工具的推理评测。模型不能浏览、读文件、运行代码或查询标准答案。
评分程序在生成结束后读取标准答案，不向模型反馈正确性。

## 当前题库与执行顺序（2026-11-04）

用户已暂停此前的基线采集，取消未完成请求，固定以下两个独立题包。
后续不再使用 ZebraLogic；原有题包与成绩保留为历史证据。
此前的“14 错 + 6 对”及“7 道 BBEH 错题 + 7 道 Omni 错题”选题方案已被下面的 10 题方案替代。

| 题包 | BBEH mini | Omni-MATH | 用途 |
| --- | --- | --- | --- |
| `suites/fusion-10.json` | 7 道基线错题 + 1 道对题 | 1 道基线错题 + 1 道对题 | 固定 8 错 + 2 对，先用于 Fusion 优化 |
| `suites/max-1000.json` | 完整 mini 460 题 | 随机 540 题 | 后续另起 DeepSeek Flash max 基准 |

执行顺序是**先用 10 题优化 Fusion，再启动新的 1,000 题基线**。
创建题包不会调用模型，不会启动夜间评测。两个题包及其运行目录分别保留，
重跑 1,000 题不修改 10 题的题目、答案、选择清单或历史基线。

10 题的题目和原成绩哈希已经冻结；本机历史基线由此前两次运行的原始记录离线组合为
`runs/2026-11-04-fusion-10-baseline`，未重新生成答案。正确对照题按固定种子
`future-fusion-dev10-v1` 的哈希顺序在每个数据集已完成且答对的记录中各取一道。
具体选题与暂停记录保存在被忽略的 `.local/challenge-selection.json`。
原成绩和回答保留在本地，不随 Git 提交。

1,000 题题包使用固定种子 `future-deepseek-max-1000-20261104-v1`。
BBEH mini 的 460 个 ID 全部纳入；Omni 从整数答案子集的 1,962 题中，先排除已发现的
`omni:383`（题面/参考解答有疑点）与 `omni:2865`（依赖缺失题图），再无放回随机取 540 题。
排除发生在这轮生成任何答案之前，ID 记录在选择清单中；不根据本轮成绩换题。

## 当前题包的运行命令

先按安装章节准备 Python 环境，并使用已构建的 CLI 启动隔离 Host。
`--binary` 及 `--source-root` 应指向实际二进制和对应的源码树；下例使用本机保留的二进制副本。
在一个终端运行：

```sh
python3 benchmark/fusion/runtime.py --binary benchmark/fusion/.local/bin/future
```

然后在另一个终端，只运行 10 题的 Fusion，引用固定历史基线：

```sh
python3 benchmark/fusion/bench.py run --suite benchmark/fusion/suites/fusion-10.json --arms fusion --baseline benchmark/fusion/runs/2026-11-04-fusion-10-baseline --binary benchmark/fusion/.local/bin/future --runtime benchmark/fusion/.local/runtime.json --out benchmark/fusion/runs/fusion-10-v1
```

优化完成后，另起 1,000 题单模型基线。以下命令需主动执行，准备题包时不会自动运行：

```sh
python3 benchmark/fusion/bench.py run --suite benchmark/fusion/suites/max-1000.json --arms single --jobs 4 --http-502-retries 3 --binary benchmark/fusion/.local/bin/future --runtime benchmark/fusion/.local/runtime.json --out benchmark/fusion/runs/deepseek-max-1000-v1
```

每次 `--out` 使用新目录；中断后用原参数与原输出目录加 `--resume`，不重跑已完成题目。
默认题包是 Fusion 10，默认 arm 是 `fusion`；运行单模型基线应显式传 `--arms single`。

**502 重试策略**：仅当已接受 run 的终态账本确认上游 HTTP 502 时，才在新会话中
重新提交同一道原题。默认最多追加 3 次，等待 2、4、8 秒；可通过 `--http-502-retries`
调整次数，`--http-502-retry-delay` 调整初始等待，指数退避最多等待 30 秒。
答错、解析失败、HTTP 400 等其他结果不会触发这项重试。

每题的每次尝试保存在 `attempts/attempt-N/`，汇总结果位于该题目录的 `result.json`。
所有失败、回答、会话/运行 ID 与用量均保留；最终结果累计已记录的尝试用量，并标记
失败尝试可能存在未返回的用量。502 次数用尽后，保留该题失败并继续余下题目；
其他类型的执行错误仍暂停补新题供排查。模型自身的重试也保留在实际调用记录中。

重新建立同一范围的 1,000 题题包时使用新的输出路径：

```sh
python3 benchmark/fusion/bench.py plan --counts bbeh=460,omni=540 --exclude omni:383,omni:2865 --seed future-deepseek-max-1000-20261104-v1 --out benchmark/fusion/suites/max-1000-reproduced.json
```

## 安装

在仓库根目录执行（Python 3.9+）：

```sh
python3 -m venv benchmark/fusion/.venv
benchmark/fusion/.venv/bin/python -m pip install -r benchmark/fusion/requirements.txt
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py check-suite
```

Windows 使用 `.venv\Scripts\python.exe` 替代 `.venv/bin/python`。
复现已提交的冻结题包不需要下载完整题库。依赖只安装到本目录的虚拟环境，
不修改仓库 npm/Cargo 依赖，不安装模型权重。

如需完整题库或重新抽样，再执行以下命令（需要访问 GitHub Raw 与 Hugging Face）：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py prepare
```

`sources.lock.json` 固定上游 Git/Hugging Face revision，`data/manifest.json`
记录每个下载文件的 URL、字节数和 SHA-256。`prepare` 复用本地缓存；`run`
使用冻结题目文件时校验题包、来源锁及每道题目/答案哈希；使用完整缓存时校验下载文件哈希。
更新题库应主动更新 lock、下载到新缓存并建立新题包。

| 数据集 | 安装内容 | 当前评分支持 |
| --- | --- | --- |
| [ZebraLogic](https://huggingface.co/datasets/WildEval/ZebraLogic) | `grid_mode`，1,000 题 | 完整网格精确比较，允许行列重排 |
| [BBEH](https://github.com/google-deepmind/bbeh) | 官方 mini 460 题、官方评分器、README、LICENSE | 使用固定版本官方评分函数 |
| [Omni-MATH](https://huggingface.co/datasets/KbsdJames/Omni-MATH) | 完整 4,428 题 | 首轮仅使用 1,962 道最终答案为整数字面量的题 |
| [MuSR](https://github.com/Zayne-sprague/MuSR) | 三个领域的官方数据与 README | 选择题答案精确比较 |

本轮按用户要求不纳入 GPQA 与 LiveBench。MuSR 可通过 `plan --datasets musr` 建立独立题包。

## GitHub 提交范围

仓库提交下载及评测脚本、依赖版本、来源锁定、评测协议、离线回归测试，以及
`suites/` 中的冻结题包。每个题包由两个文件组成：`<name>.json` 保存原始选择清单
（题目 ID、分层和题目/答案哈希），`<name>.cases.json` 保存对应题干、标准答案、
评分器类型和来源信息。标准答案仅由离线评分使用，不发送给模型。
克隆后可直接校验并运行冻结题包；执行 `prepare` 则从固定来源恢复完整题库缓存。

| 路径 | 是否提交 | 内容 |
| --- | --- | --- |
| `*.py`、`requirements.txt`、`ruff.toml`、`tests/` | 是 | 下载、执行、评分、审计、汇总和回归代码 |
| `sources.lock.json`、`suites/*.json` | 是 | 固定来源、冻结题号/哈希，以及对应题干与标准答案 |
| `third_party/bbeh/` | 是 | 固定版本的官方评分器、许可证及来源哈希 |
| `README.md`、`PROTOCOL.md`、`.gitignore` | 是 | 使用方法、评测规则和忽略策略 |
| `data/` | 否 | 完整题干、标准答案、上游评分器/说明及下载清单 |
| `runs/` | 否 | 模型回答、评分、耗时、用量、逐步转录和执行源码快照 |
| `reports/`、`results/` | 否 | 汇总、图表和可分享导出，即使已经脱敏也不提交 |
| `.local/` | 否 | 隔离 Host、数据库、认证副本、日志和模拟验证结果 |
| `.venv/`、Python 缓存、`*.log` | 否 | 本机依赖与工具运行产物 |

公共忽略策略放在 `benchmark/.gitignore`，适用于后续其他 benchmark；
Fusion 的 `data/` 下载缓存由本目录 `.gitignore` 排除。
每次提交前检查 `git status --short --untracked-files=all benchmark`。
不使用 `git add -f` 绕过这些规则。
新运行目录还会生成忽略全部内容的本地标记，即使 `--out` 误填到其他位置也不会混入普通提交。
仓库内的汇总导出仅允许放在被忽略的 `reports/`，也可以导出到仓库外。

## 历史首轮：基线先跑，冻结错题，再单跑 Fusion

2026-11-04 首轮使用两阶段开发流程。第一阶段随机抽取 ZebraLogic 100 题、BBEH mini
100 题、Omni-MATH **整数答案子集** 200 题，共 400 题。各数据集独立按
`SHA256(seed, question_id)` 排序后取前 N 题，无放回；这一路径不做难度分层配额。
`baseline-400.json` 及配套题目文件已经冻结，种子为
`future-deepseek-baseline-400-20261104-v1`。不纳入 MuSR。

```sh
# 仅在建立一个全新题包时执行；已有题包禁止覆盖。
python benchmark/fusion/bench.py plan --counts zebra=100,bbeh=100,omni=200 --seed future-deepseek-baseline-400-20261104-v1 --out benchmark/fusion/suites/baseline-400.json

# 使用上文隔离 Host，只生成 DeepSeek Flash max 的回答。
python benchmark/fusion/bench.py run --suite benchmark/fusion/suites/baseline-400.json --arms single --jobs 4 --runtime benchmark/fusion/.local/runtime.json --out benchmark/fusion/runs/deepseek-max-400 --freeze-out benchmark/fusion/suites/fusion-challenge-20.json
```

每个回答结束后写入逐题证据并更新 `summary.json`、`results.csv`、`report.md`。
这轮只调单模型，不生成 Fusion 回答。四路并发不会改变总题数；若需顺序运行，使用
`--jobs 1`。可用 `--binary` 指定固定二进制，`--source-root` 指定该二进制对应的源码树；
运行清单会保留二进制哈希、源码快照及源码树是否存在未提交改动。

`--resume` 配合原来的参数和 `--out` 续跑：已有最终成绩（包括失败成绩）原样保留；
有 `ack.json` 的未完成题只重新等待同一个 run，不重新提交。已经创建目录却没有
`ack.json` 的题要求人工检查，避免在提交结果未知时重复调用。
续跑会验证题包、二进制、执行脚本、模型能力和运行参数未变；Fusion 还校验讨论配置。
服务或协议错误会停止继续提交新题，已接受的题继续落盘。排查后才执行续跑。

全部 400 题都有终态记录后，`--freeze-out` 自动按固定种子从**已完成且成功解析**的
回答中抽取 **14 道错误 + 6 道正确**，再冻结 20 题的顺序。服务失败、超时及解析失败
保留在基线总成绩的分母中，但不用于填补“推理错题”名额。若不足 14 错或 6 对，
命令会报告不足，保留全部基线，不偷偷换题或重跑回答。也可单独执行：

```sh
python benchmark/fusion/bench.py freeze --baseline benchmark/fusion/runs/deepseek-max-400 --out benchmark/fusion/suites/fusion-challenge-20.json
```

若需要修正评分器，先停止控制进程的新题提交，等已接受请求结束，再建立离线评分修订：

```sh
python benchmark/fusion/bench.py regrade-baseline --source benchmark/fusion/runs/deepseek-max-400 --out benchmark/fusion/runs/deepseek-max-400-v2
```

它不调用模型，原运行目录、原成绩和原脚本快照保持不动；新目录保存修订后的判分及原始证据哈希。
若原基线尚未完成，对新目录使用 `run ... --out ...-v2 --resume`，复用已有回答并继续剩余题目。
2026-11-04 首次运行发现 ZebraLogic 允许的 JSON 对象行被错误地按字段名比较；
修正后同时接受数组行和以列名为键的对象行，未知/缺少列仍报解析失败。
本次修订没有改变题目、提示词、模型或回答，不把格式误判列入推理错题。

20 题选择清单固定原基线清单和每个结果的 SHA-256；完整模型输出与成绩仍保存在被忽略的
`runs/`，不能提交。后续比较只跑 Fusion：

```sh
python benchmark/fusion/bench.py run --suite benchmark/fusion/suites/fusion-challenge-20.json --arms fusion --baseline benchmark/fusion/runs/deepseek-max-400 --runtime benchmark/fusion/.local/runtime.json --out benchmark/fusion/runs/fusion-challenge-v1
```

`--baseline` 在任何模型生成前验证题干、标准答案、提示词、评分器、实际模型/思考等级和
冻结结果哈希。报告引用原有 DeepSeek 成绩，明确标记为历史基线，不再调用单模型；
基线原始用量和耗时也标记为历史数据，不算本轮新增花费。审计跳过历史会话，避免到新 Host
查询旧会话。新的单独基线只允许一遍，引用比较也只允许一次 Fusion 回答。

这 20 题是专门寻找改进空间的开发题库：基线 **6/20（30%）由选题规则决定**。
主要关注 14 道错题中修复多少，以及 6 道对题中退步多少；不把这个题库的正确率差异
解释为总体能力提升。报告抑制该选题样本的总体置信区间。判断基线原有能力应看第一阶段
400 题总体和分数据集成绩。

## 小测试

已冻结的 `suites/smoke.json` 包含三道题：ZebraLogic、BBEH mini、Omni-MATH 各一题。
这个题包用于检查实际模型调用、协议和判分，不用于宣称整体推理能力领先。
它不是按模型成绩挑选的样例，也不应在看到结果后更换题目。

抽样规则：先按题库自带分层字段分组，每组按 `SHA256(seed, question_id)` 排序，
再按固定顺序轮流取题；BBEH mini 无子任务字段，作为一个整体抽样。
Omni 的整数答案过滤发生在抽样前，报告必须注明这个范围。
选择清单保持原始题目 ID、分层及题目/答案哈希；配套 `.cases.json` 提交被冻结题目的
题干与标准答案，方便在不下载完整数据的情况下复现。两个文件必须通过哈希校验。

重新生成相同小题包：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py plan
```

建立更大的开发题包（先检查成本，再运行）：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py plan --per-dataset 4 --out benchmark/fusion/suites/dev-12.json
```

`plan` 同时生成选择清单及配套题目文件，不包含任何模型输出。提交前可离线校验：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py check-suite --suite benchmark/fusion/suites/dev-12.json
```

## 运行真实模型

先使用当前源码构建的 `target/debug/future`。脚本不自动编译或更换正在运行的产品实例。
在第一个终端启动隔离 Host：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/runtime.py
```

它读取本机当前 Fusion 设置，只复制模型调用需要的认证与配置到 `.local/instance-*`，
不复制日常会话、任务或讨论历史，不修改原 `FUTURE_HOME`，并选择新的回环端口。
关闭它会删除复制的认证/模型配置；保留的评测数据库仅用于本地复盘。
不要分享 `.local`。如果源数据库存在 WAL 且源 Agent 无法查询，脚本会停止，
避免读到旧设置。此时先启动原 Host，再重试。

在第二个终端执行：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py run --runtime benchmark/fusion/.local/runtime.json --out benchmark/fusion/runs/my-smoke
```

也可省略 `--runtime`，直接连接已启动的 Host；这种方式会在该实例中创建评测会话。
`--binary` 可指定其他已构建的 CLI。每次 `--out` 必须是新目录，不覆盖旧结果。
当前单模型基线按上文策略重试已确认的 HTTP 502；历史运行及 Fusion 不因此回填成绩。
Agent/Host 自身的阶段修正和上游重试仍按实际策略执行，
这些尝试应保留在运行证据中。`--repeats 3` 可运行三次，重复次数不会被统计成更多独立题目。

运行前冻结配置、模型能力目录、二进制版本、脚本副本、Python/平台信息和数据清单。
每题每组启动新的会话。`--arms single` 只运行基线；`--arms fusion --baseline ...`
只生成 Fusion 回答。兼容的 `--arms both` 顺序执行同题两组；`--jobs` 控制题目并发。
每个题对开始前检查 Fusion 配置是否漂移。

本次本机设置：

| 角色 | 模型 | 思考等级 |
| --- | --- | --- |
| 参与者 P1 | `future/deepseek-flash` | `high` |
| 参与者 P2 | `future/glm-5.3-flash` | `high` |
| 参与者 P3 | `future/qwen3.7-plus` | `high` |
| 主持人 | `future/deepseek-flash` | `minimal` |
| 单模型基线 | `future/deepseek-flash` | `max` |

生成策略 3 不设置步骤或整场固定总时长：`step_timeout_secs=0`、`run_timeout_secs=0`，
`idle_timeout_secs=300`。连续 300 秒没有新增推理或正文才终止对应子请求；心跳、usage、状态通知
及只读回放不算内容进展。旧设置读取时升级，但模型、思考等级和历史讨论快照保持原值。
Fusion 子请求也不再受原 30 分钟 HTTP 总时限截断，普通会话的传输配置不随之改变。
以每次运行的有效配置快照为准。

2026-11-04 修复版已移除同一提供商最多两名参与者的并发限制。
三名参与者在分析、评议阶段均同时派发，模型和思考等级不变。
推理流断开时，未结束的推理块在消息持久化封口前结束，避免重试再次封口同一内容。
原有内容长度及 SHA-256 校验继续执行。

在已构建的 CLI 上执行本地模拟集成验证（不调用付费模型）：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/verify_runtime.py --check-idle
```

该脚本启动回环 SSE 服务和隔离 Host，要求同一提供商三路分析请求全部到达后才放行，
并检查三人分析及评议完成；另模拟两种长推理断流路径，验证 1,310,720 字节 UTF-8
原文、重试完成及同一会话后续请求。结果写入 `.local/verification-*/verification.json`。
另核验实际系统提示包含日常 Fusion 规则且工具选择仍是自动模式；
空闲看门狗在隔离夹具中缩短为 2 秒：持续推理输出 2.5 秒仍应完成，仅发送心跳的参与者应被终止且不重试。
这个缩短值仅写入夹具自己的设置，真实评测仍用 300 秒。模拟服务首帧延迟 300 毫秒让 CLI 订阅接入；
该脚本不用于衡量真实模型速度。
本地不运行单元测试，新增 Rust 回归测试由 CI 执行。

## 如何读结果

`runs/<name>/` 保存：

- `manifest.json`：题包、配置、源码/二进制版本、数据来源与执行参数。
- `harness/`：执行脚本副本，避免 Git 尚未提交时无法追溯评测代码。
- `implementation/`：关键执行源码快照，清单记录各文件与实际 CLI 二进制 SHA-256。
- 每题每组的 `prompt.txt`、`ack.json`、`output.json`、`result.json`。
- Fusion 的 `discussion.json`、完整公开阶段发言分页、主会话工具记录及用量。
- `summary.json`、`results.csv`、`report.md`：离线汇总与逐题对比。

**日常使用配置的主指标**：两组最终回答的正确率。
两组主 Agent 均为 DeepSeek Flash `max`，使用产品默认系统提示词，关闭无关项目上下文。
Fusion 组通过普通 `/fusion` 前缀启用功能，模型按日常共用提示词自然选择工具；
不使用强制 `tool_choice`，不添加“第一步必须调用”“只能调用一次”等评测专用指令。
另外记录 Fusion 启动率、实际讨论次数、完成情况、讨论结论评分及最终回答与结论的差异。
未调用时即使最终答对，也不能据此宣称 Fusion 参与或带来了推理收益。

旧轮次采用“讨论结论为主指标、一次讨论、主 Agent 转述”的协议，其结果与执行脚本原样保留，
不会用新指标回填旧成绩。更换提示词及时间策略后属于新一轮系统对比，不能把全部差异
解释成某一个修改的因果效果。

会话关闭所有普通工具，但权限使用 `all`，因为 `none` 连 Fusion 内部工具也会拒绝；
实际工具记录必须符合单模型零调用、Fusion 外层若有调用只能是 `fusion`、内层零外部调用，
并核验空 `read_scope`。自然发生的零次或多次讨论都保留，不为凑调用数重新生成答案。
若主模型流中断导致工具在执行前取消，保留对应工具回执并标记 `not_started`；
这类调用尝试不算已完成讨论，也不应因缺少讨论记录而覆盖主模型最终答案。

输出必须包含 `FINAL_ANSWER:`。网格题明确给出列名 schema，不泄露格子中的答案。
缺少答案、格式失败、超时或调用失败都保留在分母中，同时单独列出错误与解析失败。
不使用裁判模型补救答案提取，避免引入另一个模型的推理能力。

过程指标包括：第一轮到评议轮的纠错/改错次数、正确少数意见保留情况、第一轮全错后最终答对情况。
日常配置仅对单次讨论且完整原题逐字传入的情形计算这些自动过程指标；子问题不与整题标准答案比较。
这是一项保守的可比较性条件，不用于判断改写是否忠实。无法可靠解析的阶段回答不当作错误立场，
同时保留全部转录供人工分析。

用量保留提供商报告的 token、缓存、reasoning token 和 credit cost；缺失意味着未知。
`costCny` 是运行时字段名，Future 的实际来源可能是平台 `credit_cost`；不自行换算为人民币或美元。
内核与外层用量分别保存再相加；缺失外层花费时不把内核花费冒充整场花费。
原始步骤记录保留修正、失败及重试。耗时包含真实路由和综合开销，
并发运行可能有资源竞争；这次比较不声称等 token、等算力或等花费。

只重新生成报告，不调用模型：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py report benchmark/fusion/runs/my-smoke
```

在关闭隔离 Host 前，可只读核验实际模型等级、工具记录、讨论配置和终止用量：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py audit benchmark/fusion/runs/my-smoke --runtime benchmark/fusion/.local/runtime.json
```

审计不提交问题、不生成答案、不改变评分；补取的证据与原始 `result-observed.json`
分开保留。超时/取消时提供商可能未返回完整用量，此时汇总明确标记用量可能不完整，
不把已记录部分当成全部花费。

导出可审阅的汇总（只写本地文件，不发布到外部）：

```sh
benchmark/fusion/.venv/bin/python benchmark/fusion/bench.py report benchmark/fusion/runs/my-smoke --public-out benchmark/fusion/reports/my-smoke
```

导出 JSON、CSV、Markdown 和本次执行的脚本快照，不含问题/答案正文、会话 ID、本机路径或认证。
不完整试跑、标有 `INVALID.md` 的试跑和已经存在的导出目标都会被拒绝。

CLI 的事件流有独立的 300 秒空闲限制。评测器遇到该只读等待错误时重新订阅
同一个 session/run，不重发题目、不重新生成答案。`--timeout 0`（默认）不增加评测器总时限；
也可显式设置正数秒数。读取窗口到期只是只读重连，不等于模型无进展超时。
日常配置不会为满足评测协议而中止主 Agent 的正常回应；失败讨论仍记为失败，
其后的独立回答也不被归为成功的 Fusion 讨论。旧轮次中止外层补答的行为只保留在其脚本快照中。
只读重连次数保存在 `output.json`。模型本身的其他失败不自动重跑。

## 设计改进与公开数据

请阅读 [评测与发布协议](PROTOCOL.md)。开发题可以反复诊断；正式对外数据应使用提前冻结、
未用于调提示或改协议的保留题，并同时公布范围、样本数、重复次数、错误率、用量和延迟。
仅三题的结果只能作为透明的试跑记录或案例展示，不能推断整体提升百分比。

本机试跑记录保留在被忽略的 `reports/` 中，克隆仓库后不会包含历史成绩。
模型原始响应、汇总成绩、完整上游题库缓存、认证、环境与运行缓存均被 `.gitignore` 排除；
`suites/` 中被冻结的题目资产应提交。
`--public-out` 只生成可供人工分享的本地导出，其名称不表示应提交或自动发布。

静态检查：

```sh
benchmark/fusion/.venv/bin/ruff check benchmark/fusion
benchmark/fusion/.venv/bin/ruff format --check benchmark/fusion
```

遵守仓库约定，本机不运行单元测试；这项工作的真实模型试跑属于手工集成评测。
仓库不再提供 benchmark 专用 GitHub Actions 工作流；静态检查、题包校验及真实模型评测
通过本机命令手动执行。`tests/` 中的离线回归文件保留。
常规 `ci.yml` 的规则保持不变：纯 `benchmark/` 改动不会安排 Rust、TypeScript、Android
或 iOS 编译和测试，仍保留轻量的影响范围检测与主干必需的 `CI result` 汇总检查。
