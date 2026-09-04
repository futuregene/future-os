# Linux 沙盒异常分支 Review

日期：2026-09-04。审查基线：FutureOS `d469719b` + 后续修复 `d51963c9`；对照本地 Codex `f20b63e85c`。这是源码审查与本地回归报告，不是 Linux 发行版认证。

## 1. 结论与本轮修复

### 第二轮（同日）：bwrap `--args` 与 missing target 处理

现场第二轮错误 `bwrap: Can't mkdir /home/ace/.aws: Read-only file system`（exit 1，helper 报 `bubblewrap did not report command status`）。两个独立根因都已修复：

1. **`--args` 传输丢了 COMMAND（导致所有命令失败）**：`4de7004d` 引入 `bwrap --args <fd>` 后，helper 把 `--`、helper 路径和 helper 参数也写进了参数文件。bwrap 源码证实 `--args` 只递归解析 OPTIONS，遇到文件内的 `--` 就 `break`，文件内的 COMMAND 被丢弃，外层 argv 为空 → `usage` + exit 1。修复：参数文件只含 OPTIONS（到 `--chdir` 为止），COMMAND（`-- current_exe agent-helper-args`，只有 2–3 个短参数）回到 bwrap 命令行 argv。已用 bwrap 0.11.1 源码（`parse_args_recurse`、`bubblewrap.c:3021 argc<=0`）和最小实验验证。
2. **missing protected target 挂载设计缺口（上轮 P0）**：bwrap 挂载前必然 `ensure_dir()`（`SETUP_MOUNT_TMPFS`），只读父目录下 EROFS，可写父目录下产生宿主对象。维护者禁止在受保护宿主目标造占位对象，因此本轮改为：**缺失目标一律不挂载**，plan 收集进 `omitted_missing_protected_paths` 随 request 传给 outer helper；命令结束后对每个目标 `symlink_metadata` 重扫，路径出现即输出 detection-only 违规标记（新 kind `MissingProtectedCreated`）。这与既有 glob 语义一致：存在则硬遮罩，运行中出现则事后检测。
3. **新增生产路径端到端验证**：`production_plan_with_real_default_rules_starts_a_shell` smoke 测试用真实默认规则（真实 HOME guards + workspace globs）走 plan → prepare → helper → bwrap → `pwd; whoami` 全链路，断言请求中无 `MissingProtected` mount、省略列表随请求传递、命令成功。其余 6 个 smoke（RO 根、unreadable mask、信号、父子退出、fd:3 传输、省略后创建检测）在本机真 bwrap 下全部通过。
4. 顺手修复 `probe.rs::is_root_owned` 的 clippy `needless_return`（本机 clippy 版本比上轮验证环境新）。

**限制（明示）**：可写域内缺失 guard 现在是“省略 + 事后 detection-only 报告”，命令自身创建该路径时不会在运行中被阻止（写保护暂降为事后可见）。只读域内省略则命令本来就无法创建。硬执行仍需 §3 的隔离挂载点设计；本轮不宣称 missing target 已获运行时强保护。

### 第一轮：缺失 guard 重复分类

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
| 基础 probe 成功但真实 HOME/workspace mount 不成立 | probe 仅运行固定 `/tmp` 基线，没有完整生产 plan/helper 自重入 | **已补**：`production_plan_with_real_default_rules_starts_a_shell` 真实默认规则全链路 smoke（本机 bwrap 通过）；仍需在更多发行版上重跑 |
| 缺失 HOME guard 被重复 bind | 普通源 open 先失败为 125 | **本轮已修** plan 分类与去重；真实运行仍受下一行影响 |
| missing target 位于只读父目录，或宿主可写 bind 下 | 旧：helper 输出 `--tmpfs target`，bwrap 先 `ensure_dir()` 再 mount | **第二轮已修（省略 + 检测）**：缺失目标一律不挂载（EROFS/宿主残留都不可能发生）；命令后重扫，出现即 `MissingProtectedCreated` detection-only 标记。硬执行仍依赖 §3 隔离挂载点设计 |
| 已存在的 read+write guard 重复挂载 | read-only 目标随后被 opaque 覆盖，最终身份不同 | **本轮已修**，同路径仅保留更强 mask；不把这个去重泛化为任意祖先/后代折叠 |
| 仓库或 `/tmp` 在准备阶段发生正常内容变动 | 原核验把 size/mtime 也视为 mount 身份 | **本轮已修**普通 mount dev/inode 校验；bwrap receipt 仍保留严格核验 |
| 源在 plan 与 helper 之间删除/替换，权限不足、坏链 | PathInspection / source unavailable / identity changed | 必须 fail closed；允许在确认命令未执行后重新构造一次，不要自动创建源或忽略错误 |
| 缺失可写 allow 根 / cwd 不存在或被保护 | allow 根仍可能进入 bind；cwd 的实际 shell spawn/chdir 可能失败 | **P1**：区分必要 workspace/cwd 与可选配置根；前者明确拒绝并让用户选择目录，后者经规则语义确认才可省略；当前不保证启动 |
| deny 父目录 + narrow allow 子目录、重叠遮罩、可写 symlink 祖先 | mode-000 父遮罩可能使后代不可遍历；最终 view 不一定能逐一满足全部原始 mount 身份 | **P1**：需要最终可达性规划与测试，不能只按路径深度排序；无法表达的组合应明确 unsupported，不静默放宽 |
| glob 扫描超时、结果/深度/内存预算、I/O、Abort | 前轮已分组扫描和诊断，启动前失败不执行；post scan 只检测 | 保留 fail closed，不再设十万节点上限。30 秒协作式预算不能抢占卡死的文件系统调用 |
| helper payload/FD/临时盘资源、fd 上限、argv 超限 | request/args 有尺寸界限，但每个 mount 仍占 FD；匿名文件可能 ENOSPC/EMFILE | **P1**：增加 FD 预算预检、资源类 code；不能因 ARG_MAX 已避开而声称资源无限 |
| helper executable 位于被遮罩/不可执行路径；能力检查失败 | self-reentry / shell spawn / capget / no_new_privs 失败 | 保留诊断，不裸跑；生产路径测试需覆盖 helper 可达性与 capabilities |
| bwrap `--args` 参数文件同时携带 OPTIONS 与 COMMAND | bwrap 递归解析只接受 OPTIONS，文件内 `--` 后的 COMMAND 被丢弃，外层 argv 为空 → usage + exit 1 | **第二轮已修**：参数文件只含 OPTIONS；COMMAND 回到 bwrap 命令行 argv（短参数不触及 ARG_MAX 初衷）。源码 + 最小实验验证 |
| 已完成命令返回 125，或 helper 中途丢失状态 | 文本和退出码不足以确定执行阶段 | 用户命令也可自行 exit 125，缺少 completion 不等于没有执行；不能仅据此自动重跑，应先核对可能的副作用 |
| 命令运行中超时、信号、用户取消 | 按现有进程组与 helper 转发处理 | 不自动重跑。Linux 需验证 outer/bwrap/inner/后代各阶段都能终止，包括复扫阶段 |
| 命令完成后复扫失败 | 保留原始结果并报告 detection-only | 不把它当初始化故障，不触发自动重跑 |
| 普通工具失败对会话的影响 | `agent/mod.rs::execute_one_tool_impl_static` 将错误转成 tool result/error，供对话循环继续 | 单次 shell 错误不必终止纯文字对话；模型可主动申请单次脱沙盒执行，仍需用户审批，见产品文档 §4.6 |

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
3. 完整设计未验证前，该规则组合应作为明确的 unsupported/initialization failure 处理；不要继续宣传当前 `MissingProtected` 已证明无宿主残留。
4. 增加默认 HOME 缺失目录、缺失多级父目录、workspace 内 missing target、symlink 父路径和并发创建的**生产 plan → helper**真机测试。现有手写 `MountRequest` smoke 绕过了本次出错的 plan，覆盖不足。

本轮修复 only-source-open 失败，但 `/home/ace/.aws` 下一阶段仍可能变成 bwrap `Can't mkdir ... Read-only file system`。这是不同的失败，不能把本轮单测通过描述为该 Linux 主机已恢复。

## 4. 优先级与验收

### 审批路径展示补充

审批诊断路径提取在原有 `Operation not permitted` 之外，增加 Linux 常见的
`Permission denied`、`Read-only file system`。支持单/双/弯引号中的绝对路径、
含空格路径，以及 shell 的 `程序: line N: /目标: 错误` 格式，保持去重、原序、最多 5 项。
相对路径不假定为工作区内路径；URL、缺失文件/helper 初始化错误不据此推断访问目标。
这些路径只是报错涉及的目标，不代表整条命令访问的全部文件，也不是授权边界。

新增 Linux 解析仅用于展示：不修改 denial 判定、脱沙盒审批/重跑触发条件，
也不扩大“在工作区永久允许”的规则建议来源（仍保留原有 `Operation not permitted` 来源）。
模型主动申请脱沙盒、没有失败输出或无法识别格式时，仍可能没有可展示的路径；
本次未新增 UI 空状态或结构化访问目标协议。

| 优先级 | 工作 | 验收要求 |
|---|---|---|
| P0 已改、待 Linux 复核 | 缺失/现存保护重复 mount、路径检查、普通 mount 身份 | 本地规则回归通过；Linux 默认 HOME/真实仓库验证，不用手工 request 代替 |
| P0 发布阻断 | missing target 无宿主残留设计 | readonly parent、writable parent、nested parent、symlink、并发、异常终止，无宿主对象且不丢写回语义 |
| P1 稳定性 | 真实 policy readiness、FD 预算、可达性与 cwd 分类 | probe 可用但实际 plan 不可用时准确提示；FD 紧张和重叠 mount 可诊断 |
| P2 运维 | 诊断包和性能/异常矩阵 | 记录版本、probe、阶段、错误码、limits、规则摘要；不上传凭据内容 |

产品决定：不新增命令级自动回退、临时手动模式、熔断或恢复 UI；使用既有的模型主动申请单次脱沙盒审批流程，见 [PRODUCT.md §4.6](PRODUCT.md#46-approval)。这不改变现有基础 probe 不可用时的设置处理，也不代表上述沙盒缺陷已修复。

本轮验证（macOS）：

- `cargo fmt --all --check`：PASS。
- `cargo clippy -p future-agent --all-targets -- -D warnings`：PASS（当前 host target，不等于 Linux 编译）。
- `cargo test -p future-agent --lib sandbox::linux -- --test-threads=1`：45 PASS、0 FAIL、1 ignored（大目录 fixture 本轮未重跑）。包含真实默认 HOME guards 的 plan → request 回归。
- `git diff --check`：PASS。
- Linux helper mount 身份新增测试、真实 bwrap 默认 HOME 启动、缺失目标残留与异常信号矩阵：**NOT RUN**；missing-parent 设计问题尚未修复，不能称为“只差跑测试”。

第二轮验证（本机 Linux，bwrap 0.11.1，真机）：

- `cargo test -p future-agent --lib sandbox::linux -- --test-threads=1`：53 PASS、0 FAIL、1 ignored。新增：只读祖先/嵌套/遮罩父目录/reopen 下的省略回归、request 省略字段往返与校验、violation 新 kind 回归。
- `cargo test -p future-agent`：1651 PASS、2 FAIL（`models::future::cache_save_and_concurrent_load_never_torn`、`models::tests::registry_injects_future_models_from_disk_cache`，两者单独运行均通过，为共享 `~/.future` 磁盘缓存的并行 flaky，与沙盒无关，待修）。
- `cargo clippy -p future-agent --all-targets -- -D warnings`：PASS（Linux host target）。
- `cargo fmt -p future-agent -- --check`：PASS。
- smoke（`--ignored`，真 bwrap）：`production_plan_with_real_default_rules_starts_a_shell`、`filesystem_no_new_privs_and_exit_status`、`unreadable_mount_and_fd_allowlist_are_enforced`、`command_signal_is_preserved`、`helper_parent_death_does_not_leave_command_running`、`production_request_fd_transport_reaches_both_helper_phases`、`omitted_missing_guard_created_by_command_is_reported_detection_only`：7/7 PASS。
- 仍未修复/未验证：可写域缺失 guard 的运行时强保护（隔离挂载点设计）、缺失目标并发创建动态拒读、发行版矩阵、信号与超时全阶段终止。
