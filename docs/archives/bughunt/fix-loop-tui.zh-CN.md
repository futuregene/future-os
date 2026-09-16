# fix-loop-tui——最终切片交接

> 本文是历史快照 [fix-loop-tui — final slice handoff](./fix-loop-tui.md)（2026-09-11，commit `914f27f6`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

会话 `20260911-123030-76ccff`；任务 `todo_79f44876e5c3`；goal `goal_bughunt_fix_20260911`。
独占 worktree `/Users/geilige/future-os/.worktrees/bughunt-loop-tui`，分支 `claude/bughunt-loop-tui`，基线 `fc81c016`。

**结果：分配的切片已实现并本地验证，等待 supervisor 独立评审/集成。这不是全局目标/PR 完成。** 全部 55 个原始发现加上 SF/NF/V5/V6 新增与自有 WebUI w32-5 边界在下方均有处置。潜在机制与被反驳声明不计为已修复的生产缺陷。

已检查输入：只读 `/Users/geilige/future-os/.future/bughunt/{REPORT,CROSS-MODEL-VERIFICATION,w14..w20,w33}` 与适用的 V2/V3/V5/V6、GLM-b3/b4/b5/b8、KIMI 与 w32 证据。当前代码/调用路径与实际回归（而非历史 confirmed 标签）决定处置。

## commits

- `d2905a9d`——第一轮生命周期、调度、隐私与 Unicode 修复、回归文件与初始全范围账本。
- `736830dd`——保留受控停止编辑：手动 CLI 租约 TTL 预期与 clippy 借用修复。
- `05ca1f04`——持久 run 归属、时钟/延迟语义、列表宽度与专用回归。
- `3b60af69`——owner/bridge/help/mirror 回归、最终旧测试修正、WebUI 属性编码、已评审奇偶行。
- 最终纯报告提交随后。未执行 push、PR、分支合并、重置、额外 worker、main/集成源码编辑或 live-agent 干预。

第三轮从干净的 `3b60af69` 树恢复。最终第二轮检查在计时器打断报告写入前已通过；被取消的写入未执行。没有已定稿的实现被重做。

## tests

所需命令**实际通过**，不是手工完成覆盖：

```text
python3 /Users/geilige/future-os/.future/bughunt-fix/check-rust.py future-loop future-tui
PASS full scoped batch ['future-loop', 'future-tui'] at 16:31:13
```

日期：2026-09-11。两个 crate 通过 `cargo fmt -p … --check`、`cargo clippy -p … --all-targets -- -D warnings` 与**完整 `cargo test -p …` 包括集成与 doc-test 目标**。Loop：402 个库测试加每个集成二进制；TUI：850 个库测试、7 个新集成回归、5 个 CLI smoke 测试。Rust 1.97.0；隔离测试 HOME；fd 上限 10240；序列化共享 target 批次。这些检查覆盖 `3b60af69` 中所有源码变更。

额外实际执行：
- `node orchestration/loop/tests/graph_layout.mjs`：4 个实际源码几何 fixture 通过。
- `node orchestration/loop/tests/webui_attribute_context.mjs`：16 个实际模板 HTML 解析器/JavaScript 解析器往返通过（jsdom，无浏览器或实时服务）。
- 实际 `render_parity` 可执行文件针对语料与已评审 golden：**97/97 行字节一致**。编辑参考前恰有 95 行匹配；两处已评审修正在 `tui/tests/README.md` 中文档化。无笼统快照接受。
- `git diff --check`：通过；`3b60af69` 后源码树干净。

有意义的失败/被拒尝试在此保留而非抹除：
- 第一轮较短的 120/180/300 秒命令在共享 Cargo 锁/重建附近超时。它们不是通过；序列化长超时批次取代之。
- 朴素 `notify_one` 保留初始许可并虚假取消第一个订阅（六个 TUI 测试暴露之）。拒绝/回退；最终实现在状态读取前注册通知并使用现有版本计数器。所有流测试与确定性竞态测试通过。
- 完整 loop 测试暴露编码 bug 的旧预期：latest=oldest、手动 CLI 声明随 CLI 死亡、反转的 inbox 范围、声明忽略畸形账本。这些按显式源码语义修正。原子 claim 现在像规范重放一样 fail-closed，不在损坏输入后追加。
- 新 bridge fixture 最初预期在失败的单回合预算后成功。修正 fixture 要求现有 `max-turns reached` 非零退出，不弱化预算策略。

## coverage

下方所有路径相对于本 worktree。`L-reg` 表示 `orchestration/loop/tests/bughunt_regressions.rs`；`T-reg` 表示 `tui/tests/bughunt_regressions.rs`。所有点名 Rust 测试均在成功完整批次中运行。

| Finding | 最终处置、证据、变更路径与回归 |
|---|---|
| w14 BUG-1 | **已修复。** 手工完成可能缺少通过的验证器回执；`decision/goal_frontier/terminal.rs` 现在枚举 `unvalidated_deliveries`。L-reg `deferred_and_unvalidated_work_never_closes` 与 terminal-validator 回执契约通过。 |
| w14 BUG-2 | **已修复。** `state::is_terminal` 只接受 Done/Superseded，不接受到期 Deferred。同一回归与 schema/decision 契约。与 w17-1 重复。 |
| w14 BUG-3 | **已修复。** 到期 Deferred 经 `state::*_at` 与 decision 代码重新进入自身类别的开放通道，包括 monitor/gate/blocker。L-reg `every_due_deferred_class_reenters_its_open_lane_with_one_clock` 覆盖全部六个类别。Coordination/user-action 保留其 Open 同类的 operator/manual 语义；它们不会被静默提升为 worker 推进。 |
| w14 BUG-4 | **修复调度契约。** 提供的时钟到达 packet、identity、lane/frontier、终态判断、依赖检查与摘要。L-reg 固定 epoch 早/晚矩阵与注入时钟测试通过。Rollout UUID 仍是非确定性元数据，不是调度时钟。 |
| w14 BUG-5 | **已修复/收窄。** 停滞 monitor 现在发出 `monitor_stalled`。旧/咨询变体保留：简化有意移除了强制策略退出。L-reg 停滞分支与配额 wire-code 测试通过。 |
| w14 BUG-6 | **反驳为生产缺陷 / 潜在 API 广度。** 有效 packet 构造函数满足一致性不变量；防御性修复不在有效 packet 上触发是正确的。无消费者需要全部九个历史处置。`arbitration_contract` 通过全部 20 个用例，包括不一致契约的实际 fail-closed 处理。无调度器重写。 |
| w14 BUG-7 | **已修复。** 字符数守卫现在与 `decision::truncate` 中的字符截断匹配。L-reg Unicode 截断/元数据回归通过。 |
| w14 BUG-8 | **反驳为已演示生产缺陷。** 身份跳过已具有 `ok=false`、`should_run=false` 与显式注册说明。goal 级 keep-active 无需授权此未注册 worker；未确立 keep_active 消费者。身份测试通过。不要通过推测性更改 goal 活跃度来搁置其他已注册 worker。 |
| w14 BUG-9 | **修复咨询排序，不声称生产事故。** 时间戳在追加锁前捕获，因此 `decision/oscillation.rs` 按时间戳稳定排序观测。`append_order_cannot_fabricate_timestamp_order_oscillation` 通过。 |
| w15 BUG-1 | **已修复。** `runtime/run_history.rs` 选择第一/最新行。库与 `run_lifecycle_contract` fixture 现在断言最新 `run_recorded`，而非最老 `quota_monitor_poll`；均通过。 |
| w15 BUG-2 | **已修复。** `runtime/run_compaction.rs` 报告活动保留行，排除先前归档行，且不声称目标碰撞被归档。重复/碰撞测试通过。 |
| w15 BUG-3 | **已修复。** `scheduler/state.rs` 中检查节奏乘法；L-reg 溢出与正常小时用例通过。无任意新时长策略。 |
| w15 BUG-4 | **已修复。** `store.rs` 验证可写 goal ID 并将不安全读取 ID 映射到安全组件；运行时与回填读取路径共享该映射。L-reg 斜杠/反斜杠/绝对/点穿越拒绝通过。不安全历史 ID 不会自动迁移或在根外跟随。 |
| w15 BUG-5 | **已修复。** 在合并与加载/规范化中执行最新不同失败上限。L-reg 使用九个不同目标，而非仅重复的空洞上限测试；调度器契约通过。 |
| w16 BUG-1 | **已修复。** 原子 claim 在其独占锁下折叠规范账本事件，包括续期/归属/完成，而非部分租约解析器。L-reg 过期原始 claim + 实时续期拒绝对等窃取；lease/store 套件通过。 |
| w16 BUG-2 | **已修复。** 手工 claim 不记录短命 CLI PID；长命 run 使用 `try_claim_todo_with_pid`。L-reg、CLI 不相交 frontier/manual-lease 契约与归属投影子进程测试通过。 |
| w16 BUG-3 | **已修复。** 共享私有跨度检测器尊重 sk-/ak- token 边界；task-/disk-/risk-/peak- 保留。`projection/privacy.rs`、L-reg 分类/脱敏矩阵通过。 |
| w16 BUG-4 | **已修复。** 脱敏消费路径/token 后缀，不只是 `/Users/`；测试断言用户名与秘密后缀缺失，包括 Windows 风格文本。未读取真实秘密。 |
| w16 BUG-5 | **已修复。** 仅已寻址守卫修正。显式 scope/question/mention 矩阵与更新的 `work_items_drive::operator_inbox_kinds` 通过。捕获聊天不使普通陈述变为需要关注。 |
| w16 BUG-6 | **潜在，当前无生产影响。** 空标记 `contains("")` 机制成立，但 marker/hint API 无生产消费者；executor 使用实际工具/证据。无推测性接线或新增功能。 |
| w16 BUG-7 | **已修复。** 添加 Linux /home 与 /root 标记；隐私投影不再咨询当前 HOME。L-reg 路径分类通过。单独的状态层边界诊断保留其既有目的。 |
| w16 BUG-8 | **已修复。** `WorkLeasedToOthers` 包含在全部 17 个原因码线上往返/唯一性断言中。`quota/error_codes.rs` 测试通过。 |
| w16 BUG-9 | **已修复。** 桥回执与记录使用现有 goal 级基础偏移；桥 run ID 也携带 UUID。真实双进程 stdio 桥回归验证归属、不同 turns/run ID 与心跳 ID。现有预算失败仍非零。 |
| w17 BUG-1 | **重复已修复：** w14-2。拒绝仅否定到期检查的原始建议：所有 Deferred 工作保持非终态。 |
| w17 BUG-2 | **已修复。** 可选持久 `RunRecord.agent_id` 由 executor 与 stdio 桥盖章；webui 与通道归属使用它，而非 run 名前缀/当前 claims。实际 executor 测试加持久重放/完成/重新分配/UUID 风格记录测试通过。旧记录保持诚实未归属。 |
| w17 BUG-3 | **已修复。** 图使用实际层组高度/累积偏移。四个实际源码 Node fixture 确保包含/不重叠。原始声称节点即使手动平移也物理不可达过强；缺陷是布局/适配。 |
| w17 BUG-4 | **已修复。** `compat::write_run` 添加安全 UUID 文件名后缀。L-reg 16 个快速 JSON/MD 对保持不同并携带归属。权威花费账本不变。 |
| w17 BUG-5 | **潜在 / 当前调用方不可达。** 微小预算 truncate_evidence panic 存在，但生产调用方使用 1600/4096 或强制 >=12。无假设 API 重写。 |
| w17 BUG-6 | **已修复。** Compat 锚点对 note/resume/evidence 编解码百分号/换行/CR/tab/尖括号字符。L-reg 往返保留嵌入换行、`-->` 与字面 `%20`；回填套件通过。 |
| w18 BUG-1 | **已修复。** List Heading 与 blockquote 原始文本重建在 `markdown.rs` 中保留标题文本。T-reg 实际渲染通过。 |
| w18 BUG-2 | **已修复。** 源与解码文本在样式化前剥离 C0/C1 控制；OSC URL 目标拒绝控制。T-reg 数字实体文本与实际 cmark 链接目标测试通过；受信任的内部 Kitty 协议行仍通过。不是对生成终端协议的全面禁止。 |
| w18 BUG-3 | **已修复。** 选择器保存确定性排序的启用 ID；Ctrl+P 保留保存的 Vec 顺序。重复独立 HashSet 实例保存回归通过。 |
| w18 BUG-4 | **已修复。** 列表代码边框宽度计入内容缩进/项目符号宽度。T-reg 宽度 8/24/40/60/80 各产生两条未分裂边框且无幽灵行。一处显式 list-code golden 修正在文档中记录；97 行奇偶通过。 |
| w18 BUG-5 | **已修复。** SelectList 描述规范化 CR/LF。直接多行描述测试断言每个渲染字符串是一行终端行；通过。 |
| w18 BUG-6 | **已修复。** 使用 dirs::home_dir 与原生分隔符做组件感知 home 剥离。现有页脚测试加前缀表亲回归在 macOS 通过；原生 Windows 执行仍未声明。 |
| w18 BUG-7 | **已修复。** 顶层段落前导空白恢复，排除嵌套容器缩进。T-reg 测试空格/制表符/嵌套段落与奇偶检查通过。 |
| w18 BUG-8 | **已修复。** 非空纯空白文本产生一个空行；空字符串保持为空。T-reg 通过。 |
| w18 BUG-9 | **反驳为正确性缺陷 / 文档化 Unicode 分歧。** 单个 Unicode 标量 emoji 是有效过滤输入。重新引入 JS UTF-16 长度的偶然星面拒绝不可取；`tui/tests/README.md` 中显式接受的分歧。 |
| w19 BUG-1 | **已修复。** Input::cursor_byte 将 UTF-16 转 UTF-8，provider 守卫字节边界。实际 Stdin/Input/provider Unicode 回归通过。与 w20-3 重复。 |
| w19 BUG-2 | **已修复。** 视觉行保留源 UTF-16 偏移，包括丢弃的换行空格与硬换行。T-reg 导航通过；旧测试 column-1='w' 声明修正为源位置 7（'o'）。 |
| w19 BUG-3 | **已修复。** 缩进围栏检测保守保护流前缀缓存。实际逐帧 eager-vs-streaming 嵌套围栏回归通过。 |
| w19 BUG-4 | **已修复。** 每个消息分支饱和内部宽度，包括不安全的最大减后系统分支。宽度 0..2 用户/助手/思考回归通过。 |
| w19 BUG-5 | **已修复。** Provider 状态在查询前从当前应用 cwd 同步；文件搜索使用该 cwd，附件子进程获得 current_dir，从不进程全局 chdir。T-reg cwd-switch fixture 与现有 fd/find 桩测试通过。 |
| w19 BUG-6 | **已修复。** 防抖与 Tab 查询使用实际当前游标；斜杠前缀匹配与替换保留后缀。实际 App 中间选择与 tick 测试通过。 |
| w20 BUG-1 | **已修复。** 转义序列扫描仅在有效字符边界推进。实际 ESC+中文/重音/emoji StdinBuffer 回归通过。 |
| w20 BUG-2 | **已修复。** 重叠切片检查字符边界。实际 App Unicode 中间选择回归通过。 |
| w20 BUG-3 | **重复已修复：** w19-1。 |
| w20 BUG-4 | **已修复。** 加载的模型/会话结果刷新注册 provider；缺失斜杠参数的缓存异步获取并查询当前输入而不打开不想要的覆盖层。App 缓存接线与完整 RPC/UI 测试通过。非空缓存经现有列表操作刷新；未承诺新的实时目录轮询功能。 |
| w20 BUG-5 | **已修复。** 在空会话读取前预注册通知；跨订阅使用 poke 版本。确定性屏障测试恰在读取与等待之间 poke；与所有流测试通过。拒绝朴素 notify_one 方案（见下方历史）。 |
| w20 BUG-6 | **修复排序；循环子声明潜在。** BTreeMap 稳定 cwd 组顺序；完整 App 测试通过。正常 fork 创建将新会话指向现有父级；任意损坏/导入的循环元数据未被确立为正常可达生产者。无推测性循环恢复算法；不声称畸形外部循环被修复。 |
| w20 BUG-7 | **已修复/收窄。** 选择游标使用 UTF-16。原始无后缀示例不是可见缺陷，因为 clamp 落在末尾；实际 App 回归现在包含后缀并验证中文补全后游标位置 7。 |
| w33 BUG-1 | **已修复。** Help 与 console 共享基于 PathBuf 的项目根解析器，而非 HOME。L-reg 子进程测试不同 HOME/当前目录与 env 覆盖。 |
| w33 BUG-2 | **已修复。** Help 以换行结尾；同一子进程断言通过。 |
| w33 BUG-3 | **潜在 / 当前生产可达性被反驳。** 静态注册名不同；无生产跨组重复注册。避免不必要的公共 API 重设计。 |
| w33 BUG-4 | **潜在 / 当前影响被反驳。** 当前 builder 中无实验性子 setter/真子。缺失假设门控不是现有用户可见失败。 |
| SF-1 | **已修复。** Complete 清除 holder/expiry/PID；守卫与 agent list 忽略终态持有者。规范原子 claim 拒绝已完成工作。重放回归、终态/死工作区测试与真实 CLI completion→idle 投影全部通过。 |
| SF-2 | **修复诊断 / 收窄语义声明。** 大小写敏感身份与 onboarding 前分配是有意的。Add/update 以精确注册/重新分配动作与大小写建议警告；frontier 显示未注册 owner。子进程回归通过。与另一 owner 的安静等待本身不是命令失败；无强制大小写规范化或虚构退出错误策略。 |
| NF-1 | **已修复。** Owner 在原子 claim 的规范折叠内在锁下强制。非 owner 拒绝回归无论免费/过期租约均通过。 |
| NF-2 | **已修复。** Status 暴露 owner；frontier 暴露待处理 todo 分配、注册状态与租约，加文本 owner 提示。真实 CLI JSON/text 侧构造与子进程测试通过。 |
| V5-PID | **修复一致性。** 工作区守卫/agent list 使用与 claim 相同的 pid_alive 语义。Unix 死 PID fixture 通过；Windows 保持保守，因为其现有探测返回 alive。有效手工 None-PID 租约保留 TTL 保护。 |
| V5-lease-projection | **已修复。** Status 包含 claimed_by/lease_expires_at/holder_pid。CLI 回归观察活动手工租约与完成释放。 |
| V5-regression-gap | **关闭。** Done-with-live-lease 与 complete→idle/nonblocking 回归显式测试先前缺失的状态。 |
| V6-group-index | **潜在公共 API panic，无当前生产调用方。** Builder 使用 group() 返回的索引；无不受信任组索引进入调用图。单独跟踪，不计为已修复生产 bug。 |
| V6-line-reference | **文档修正。** 原始 w33 registry 行 1267 不存在；引用的消费者是 console::render_command_help。 |
| w32 BUG-5（自有跨切片） | **已修复/收窄。** WebUI 在全部四个动态 goal/todo 处理器站点先编码 JS 字符串再 HTML 属性；实际模板经 jsdom HTML 解码与 JavaScript 编译测试（16 用例）。新 goal ID 验证单独拒绝恶意 ID。现有/导入数据按数据处理，不声称第三方生产利用。 |

## gaps and integration notes

- 没有任何分配的**已确认、可达**修复被有意留为未实现；完整所需本地检查通过。Supervisor 必须独立评审上述收窄/反驳/潜在处置，并在全局验收前与其他 worker 集成。
- 不声称原生 Windows/Linux 执行与最终 CI。尤其 Windows pid_alive 仍保守为真；本补丁对齐消费者而非发明原生进程检测。复现未使用真实凭据、实时 LLM/浏览器服务或用户 agent。
- 无 agent_id 的历史 run 记录在所有权变更后无法可靠归属；它们保持未知。无虚构迁移猜测。`RunRecord.agent_id` 是增量 JSON（缺失旧值 -> None）；显式 Rust 结构 fixture 已更新。未更改 protobuf 字段。集成期间应检查直接根 CLI 消费者。
- `AttachmentProvider` 现在以 `Default` 构造；它携带 cwd 状态。其现有 Windows 回退限制（fd 不可用与 POSIX find 回退）未被重新定性为新修复的平台功能。
- 不安全的历史 goal ID 不会在配置根外自动移动或读取。新可写 ID 使用文档化的安全组件限制。
- 并发独立 run 进程仍可能预留相同数字基础回合；该既有控制台级机制在 w16-9 的顺序桥偏移 bug 之外。UUID run 身份唯一。不声称新的原子全局回合分配器。
- 损坏/导入的会话父循环与上述显式潜在 API 仍是潜在加固领域，非已演示生产缺陷。未为其创建额外 worker/todo。
- 额外 JS 回归需要现有仓库 Node 依赖（jsdom）；它们是显式命令，不静默表述为 cargo 测试。

## new evidence, rejected approaches, and next check

超出上次尝试：持久执行归属在完成/重新分配/重放后存活；每个到期 Deferred 类别在注入时钟下匹配其 Open 通道；真实 CLI owner/lease 视图一致；快速 run 文件保持不同；列表几何与终端/HTML 编码有实际实现回归；完整范围内 Rust 套件现在通过。

被拒方案：信任历史 confirmed 标签；仅否定一个 Deferred 谓词；从当前 owner 或 run 名前缀猜测历史 worker；折叠 worker 身份大小写；将短命 CLI PID 分配给手工租约；朴素 notify_one（保留初始许可虚假取消订阅）；保留断言 latest=oldest/反转范围/垃圾账本接受的旧测试；笼统 golden 再生成。

观测到的验证历史：较短的命令在共享构建附近超时，不计为通过。完整检查随后暴露并修正旧 latest-row、手工 claim、通道归属、反转 inbox 与损坏账本预期。新的不成功单回合桥 fixture 正确预期非零 max-turns 退出。这些是确定性契约修正，不是 rerun-until-green 的 flake 处理。最终序列化批次在 16:31:13 通过。稍后的报告写入在执行前被检查点打断；本最终报告取代过期的待处理段落。

Next useful check: supervisor cherry-pick/评审四个提交，对账共享 RPC/model 消费者变更，运行直接消费者/集成检查与两个 Node 回归，然后执行唯一授权的最终 PR/CI 工作流。Worker 交接声明 `--no-follow-up`；supervisor 拥有任何后继任务与全局关闭。
