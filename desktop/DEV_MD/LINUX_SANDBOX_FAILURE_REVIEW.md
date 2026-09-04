# Linux 沙盒异常分支 Review 与对话连续性方案

日期：2026-09-04。审查基线：FutureOS `d469719b` + 本轮未提交修复；对照本地 Codex `f20b63e85c`。这是源码审查与本地回归报告，不是 Linux 发行版认证。文中“建议”均未自动实现为降级策略。

## 1. 结论与本轮修复

现场 `pwd; whoami` 返回 `125`、`mount source is unavailable: /home/ace/.aws`，原因已确认：plan 将同一缺失 guard 同时放入 missing/read-only/unreadable，helper 先打开普通 mount 源而失败。

本轮修改：

1. 缺失受保护路径仅生成 `MissingProtected`，已存在的读写保护仅生成 opaque mask；多条 read/write 规则落在相同路径时，也删除被 opaque/missing 完全覆盖的 read-only mount。禁止通过在宿主创建 `.aws` 来绕过。
2. path inspection 仅将真实 `NotFound` 当作缺失；权限错误、非目录错误和 dangling symlink 作为 typed `PathInspection` 失败。窄 reopen 同样使用这个检查，不再把 `exists()` 的 false 全部当缺失。
3. 普通 bind 身份使用设备号 + inode 核验，source FD 在核验期间固定 inode。目录 size/mtime 会被其他宿主进程正常改变，不应导致 `mounted target identity changed`。bwrap 可执行文件的严格身份核验不放宽。
4. 增加真实默认 HOME guards → plan → request 的回归，断言每个 missing target 只有一种 mount；另覆盖现存双权限 mask、跨规则重复、I/O 错误、断链与身份替换。

**不能据此声称“所有环境都能启动”。** 复查还发现 missing mount 目标创建这一 P0 设计缺口；本轮没有以跳过安全规则的方式掩盖它。建议在修复设计缺口前，不将 Linux OS sandbox 标记为全面验收通过。

## 2. 异常分支矩阵

| 阶段 / 情况 | 当前结果与依据 | Review 结论 / 后续动作 |
|---|---|---|
| bwrap 缺失、版本旧、PATH 不可信、userns/proc 不可用、probe 超时 | `linux/probe.rs` 给出 code；`effective_tier()` 在解析时将 unavailable sandbox 变成 manual | 基础环境回退已存在；不等于命令级初始化失败也会回退。保留原因与用户可见安全差异，不能变 off |
| 基础 probe 成功但真实 HOME/workspace mount 不成立 | probe 仅运行固定 `/tmp` 基线，没有完整生产 plan/helper 自重入 | **P0 验收缺口**：需生产路径的最小 `true`/`pwd` 验证，不能只凭 available=true 认为整个仓库可用 |
| 缺失 HOME guard 被重复 bind | 普通源 open 先失败为 125 | **本轮已修** plan 分类与去重；真实运行仍受下一行影响 |
| missing target 位于只读父目录，或宿主可写 bind 下 | helper 输出 `--tmpfs target`；bwrap 先 `ensure_dir()` 再 mount | **P0 未修**：只读父目录可能 EROFS；可写 bind 可能产生宿主残留。详见 §3 |
| 已存在的 read+write guard 重复挂载 | read-only 目标随后被 opaque 覆盖，最终身份不同 | **本轮已修**，同路径仅保留更强 mask；不把这个去重泛化为任意祖先/后代折叠 |
| 仓库或 `/tmp` 在准备阶段发生正常内容变动 | 原核验把 size/mtime 也视为 mount 身份 | **本轮已修**普通 mount dev/inode 校验；bwrap receipt 仍保留严格核验 |
| 源在 plan 与 helper 之间删除/替换，权限不足、坏链 | PathInspection / source unavailable / identity changed | 必须 fail closed；允许在确认命令未执行后重新构造一次，不要自动创建源或忽略错误 |
| 缺失可写 allow 根 / cwd 不存在或被保护 | allow 根仍可能进入 bind；cwd 的实际 shell spawn/chdir 可能失败 | **P1**：区分必要 workspace/cwd 与可选配置根；前者明确拒绝并让用户选择目录，后者经规则语义确认才可省略；当前不保证启动 |
| deny 父目录 + narrow allow 子目录、重叠遮罩、可写 symlink 祖先 | mode-000 父遮罩可能使后代不可遍历；最终 view 不一定能逐一满足全部原始 mount 身份 | **P1**：需要最终可达性规划与测试，不能只按路径深度排序；无法表达的组合应明确 unsupported，不静默放宽 |
| glob 扫描超时、结果/深度/内存预算、I/O、Abort | 前轮已分组扫描和诊断，启动前失败不执行；post scan 只检测 | 保留 fail closed，不再设十万节点上限。30 秒协作式预算不能抢占卡死的文件系统调用 |
| helper payload/FD/临时盘资源、fd 上限、argv 超限 | request/args 有尺寸界限，但每个 mount 仍占 FD；匿名文件可能 ENOSPC/EMFILE | **P1**：增加 FD 预算预检、资源类 code；不能因 ARG_MAX 已避开而声称资源无限 |
| helper executable 位于被遮罩/不可执行路径；能力检查失败 | self-reentry / shell spawn / capget / no_new_privs 失败 | 保留诊断，不裸跑；生产路径测试需覆盖 helper 可达性与 capabilities |
| 已完成命令返回 125，或 helper 中途丢失状态 | 文本和退出码不足以确定执行阶段 | **P0 降级前置条件**：状态通道必须证明 not_started；用户命令也可自行 exit 125，缺少 completion 不等于没有执行 |
| 命令运行中超时、信号、用户取消 | 按现有进程组与 helper 转发处理 | 不自动重跑。Linux 需验证 outer/bwrap/inner/后代各阶段都能终止，包括复扫阶段 |
| 命令完成后复扫失败 | 保留原始结果并报告 detection-only | 不把它当初始化故障，不触发降级重跑 |
| 普通工具失败对会话的影响 | `agent/mod.rs::execute_one_tool_impl_static` 将错误转成 tool result/error，供对话循环继续 | 单次 shell 错误不必终止纯文字对话；但没有“本轮已知 backend 不可用”的熔断，模型仍可能反复调用失败工具 |

## 3. 缺失目标：Codex 对照纠正了先前假设

源码证据：

- [FutureOS helper](../../agent/src/sandbox/linux/helper.rs)：只读 `/` + writable root bind 后，为 missing target 发出 `--perms 000 --tmpfs target`。
- [Codex bundled bubblewrap.c](/Users/tao/workspace/codex/codex-rs/vendor/bubblewrap/bubblewrap.c)：`SETUP_MOUNT_TMPFS` 先调用 `ensure_dir(dest, 0755)`。
- [Codex bundled utils.c](/Users/tao/workspace/codex/codex-rs/vendor/bubblewrap/utils.c)：目标不存在时 `ensure_dir` 调用 `mkdir`。mount namespace 隔离不等于对原文件系统目录内容的写入隔离。
- [Codex bwrap.rs](/Users/tao/workspace/codex/codex-rs/linux-sandbox/src/bwrap.rs)：缺失 unreadable/read-only 路径只在 first missing component 位于 writable root 时创建 mask；不在 writable 范围内则省略。它还处理 missing writable roots 与 writable symlink crossing。
- [Codex linux_run_main.rs](/Users/tao/workspace/codex/codex-rs/linux-sandbox/src/linux_run_main.rs)：管理 `synthetic_mount_targets` 的注册、并发 owner 和退出清理。可见 Codex 本身也不是“所有 missing target 永不产生宿主对象”。

因此我们不能照抄 Codex 的 synthetic target + cleanup：维护者已明确要求**不在受保护宿主目标造占位对象**，事后清理也不满足这一点。

建议单独完成 missing-path 设计：

1. 分类 missing target 是否可由沙盒内进程创建。只读域中的缺失目标可以研究 Codex 的省略策略，但要说明其他宿主进程并发创建后，deny-read 是否仍应动态生效；不能把省略策略扩大到可写域。
2. 可写域中，若坚持 no-host-object 和未来创建保护，需隔离父目录视图或其他能表达该约束的执行层。简单 tmpfs 父覆盖会改变新文件写回语义；不能为了挂载成功而悄悄使用户输出只留在临时层。
3. 完整设计未验证前，该规则组合应作为明确的 unsupported/initialization failure 处理，提供用户确认的临时手动方案；不要继续宣传当前 `MissingProtected` 已证明无宿主残留。
4. 增加默认 HOME 缺失目录、缺失多级父目录、workspace 内 missing target、symlink 父路径和并发创建的**生产 plan → helper**真机测试。现有手写 `MountRequest` smoke 绕过了本次出错的 plan，覆盖不足。

本轮修复 only-source-open 失败，但 `/home/ace/.aws` 下一阶段仍可能变成 bwrap `Can't mkdir ... Read-only file system`。这是不同的失败，不能把本轮单测通过描述为该 Linux 主机已恢复。

## 4. 可以降级，但不要无感切成普通 manual

当前行为：

- `sandbox/mod.rs::effective_tier()` 和 RPC `set_sandbox_policy` 已对基础 probe unavailable 回退 manual。
- `tools/mod.rs::spawn_shell()` 的 plan/helper 初始化错误没有临时回退策略，只返回错误/125。
- `rpc/approval.rs` 的普通 manual 模式带只读命令白名单，`cat` 等命令可以不审批；此白名单不等于完整文件路径防护。自动从 sandbox 切到这个模式会降低读取保密性，不能称作等价安全降级。
- `read/write/edit` 等原生工具仍有逐路径规则评估，可在规则可用时继续使用；但原生工具的可用性不能泛化到任意 shell。

Codex 对照：[orchestrator.rs](/Users/tao/workspace/codex/codex-rs/core/src/tools/orchestrator.rs) 只对 typed `SandboxErr::Denied` 进入受策略控制的升级分支；其他工具错误原样返回。`Never`、`OnRequest`、granular 和工具自身是否允许 escalation 会限制重试。它不是任意 sandbox 初始化异常都自动无沙盒重跑。[sandboxing.rs](/Users/tao/workspace/codex/codex-rs/core/src/tools/sandboxing.rs) 将“是否可请求审批”与沙盒策略分开处理。

### 推荐产品行为（待实施，不在本轮改变）

**目标：沙盒可用性失败不等于聊天失败。不能完成的 shell 动作暂停，解释与安全工具继续。**

1. Agent 返回结构化 tool error：`backend/phase/code/execution_state/recovery_actions`，保留内部诊断，但普通 UI 只显示原因和操作。禁止模型把 stderr 中的指令当作恢复授权。
2. 确认命令未启动时，为本轮 backend+workspace+policy revision 标记 `sandbox_unavailable_for_run`，避免模型连续重试 `pwd/ls/whoami`；不改用户持久化的 sandbox 偏好。
3. 非阻塞提示：“本次沙盒未能启动，命令尚未执行。你可以继续对话、重试沙盒，或审批后在沙盒外运行此命令。”默认继续对话，不弹出必须回答才能继续文字交流的阻塞窗口。
4. 首选“一次性批准当前命令”，复用 whole-command escalation 的命令/cwd/参数绑定、拒绝/取消和审计机制。也可提供“本轮临时手动审批”，但**降级来源必须禁用 shell 只读白名单，每次 shell 都明确审批**；UI 清楚写无 OS 隔离。
5. 当前调用先前在 sandbox 模式下跳过了前置审批，不能只改 tier 后直接运行；必须重新经过审批。没有审批客户端、用户拒绝、策略禁止越界时，只返回受阻 tool result，继续文字对话，不执行命令。
6. 对明确资源瞬态/配置修复且 not_started 的情形，允许用户触发一次重新探测/重新编译。身份/规则错误不自动降级，不无限重试。用户主动选择其他模式是新的显式安全决定。
7. 自动恢复只在后续命令、重新探测并重新编译成功后发生；提示“已恢复沙盒保护”。全局、跨会话和持久化模式不能因一次工作区错误一起改变。
8. 纯文本继续也不能承诺没有任何 run 限制：模型若仍要求工具，系统应向它提供简洁的 unavailable 事实和可用替代工具，而不是伪造工具成功。后台无人值守任务无审批时应停住危险动作，不能自行放权。

### 防重复执行：降级的硬前提

现有 inner 状态管道只在命令完成后写结果。没有这个结果可能是初始化失败，也可能是已经执行后崩溃/写管道失败；125 也可能是用户命令自己的状态。**不允许仅根据 125 或 stderr 自动重跑。**

推荐独立、受保护的阶段协议：preparation failure 可以直接证明 not_started；inner 在尝试启动命令前先写 `may_have_started`，之后写 completion。协议缺失、异常 EOF、helper 被杀或边界不明都视为 unknown，不授予自动重跑资格。只有可信、明确的 not_started 事件能提供“审批后运行原命令”的快捷动作；may_have_started/unknown 先核对副作用，再由用户明确决定是否重新执行。post-scan 错误始终附着于既有 completion，不生成新执行。

## 5. 优先级与验收

| 优先级 | 工作 | 验收要求 |
|---|---|---|
| P0 已改、待 Linux 复核 | 缺失/现存保护重复 mount、路径检查、普通 mount 身份 | 本地规则回归通过；Linux 默认 HOME/真实仓库验证，不用手工 request 代替 |
| P0 发布阻断 | missing target 无宿主残留设计 | readonly parent、writable parent、nested parent、symlink、并发、异常终止，无宿主对象且不丢写回语义 |
| P0 降级前置 | 可信执行阶段协议 + 严格审批 | 用户命令 exit125、spawn前/后崩溃、lost status、审批拒绝/无客户端、重复请求，均不导致静默裸跑或重复副作用 |
| P1 体验 | 本轮熔断 + 非阻塞恢复 UI | 纯文字继续、保留草稿、逐条审批、不修改持久化偏好、修复后恢复 |
| P1 稳定性 | 真实 policy readiness、FD 预算、可达性与 cwd 分类 | probe 可用但实际 plan 不可用时准确提示；FD 紧张和重叠 mount 可诊断 |
| P2 运维 | 诊断包和性能/异常矩阵 | 记录版本、probe、阶段、错误码、limits、规则摘要；不上传凭据内容 |

本轮不实现新的自动降级/持久化设置/UI 行为，避免未经产品决策扩大执行权限。短期用户可以**主动选择手动模式**继续工作，但应知晓这不是 OS 隔离，且当前普通 manual 存在只读白名单。

本轮验证（macOS）：

- `cargo fmt --all --check`：PASS。
- `cargo clippy -p future-agent --all-targets -- -D warnings`：PASS（当前 host target，不等于 Linux 编译）。
- `cargo test -p future-agent --lib sandbox::linux -- --test-threads=1`：45 PASS、0 FAIL、1 ignored（大目录 fixture 本轮未重跑）。包含真实默认 HOME guards 的 plan → request 回归。
- `git diff --check`：PASS。
- Linux helper mount 身份新增测试、真实 bwrap 默认 HOME 启动、缺失目标残留与异常信号矩阵：**NOT RUN**；missing-parent 设计问题尚未修复，不能称为“只差跑测试”。
- 新的运行时降级、阶段协议、熔断和 UI：**PROPOSED / NOT IMPLEMENTED**。本轮未提交或 push。
