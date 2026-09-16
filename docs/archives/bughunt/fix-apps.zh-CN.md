# fix-apps 修复证据

> 本文是历史快照 [fix-apps remediation evidence](./fix-apps.md)（2026-09-11，commit `914f27f6`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

## 结果与身份

- 会话：`20260911-123030-c18ec4`；worker：`fix-apps`；goal：`goal_bughunt_fix_20260911`；第一轮 todo：`todo_a3b806e59841`；第二轮 todo：`todo_807893558e98`。
- 独占 worktree：`/Users/geilige/future-os/.worktrees/bughunt-apps`；分支：`claude/bughunt-apps`。
- **覆盖：29/29 原始 apps 发现加上第二轮跨负责人 w10 BUG-1、w10 BUG-6 与 w13 BUG-6 在下方均有处置。** 第二轮另外关闭了 w31 BUG-3/离线脚本的 CI 执行路径工作。未分配 SF/NF 额外项。范围排除 `packages/rpc`；第二轮明确允许最小的 agent 内置模态变更。
- 本地实现/检查完成，**外部/原生验证缺口仍在**。不要把变更发现数解读为 26 个已确认生产 bug：它包括文档/纯净性修正与一个端到端发生尚未验证的重放守卫。
- 先读契约；已评审 REPORT、CROSS-MODEL-VERIFICATION、w27–w31、相关 V4、GLM-b7/b8 与 KIMI 章节。报告是 `/Users/geilige/future-os/.future/bughunt` 下的只读输入。
- 实现提交：`6ae8f550467ccd067ca080d2f74ceb26285c786c`（55 个文件；包含本覆盖账本）。随后的纯证据提交记录该哈希。无 push、PR、分支合并、额外 worker 或对 main 的更改。
- 检查点：实现于本地 13:17 提交，启动后约 47 分钟，在要求的 50 分钟部分提交边界之前。临时验证配置、依赖符号链接、生成的移动端版本与 Python 字节码已移除；实现提交后 `git status --short` 立即干净。下方外部/原生缺口被保留，未表述为已完成的验证。

## 第二轮——授权的跨负责人延续

Todo `todo_807893558e98`，授权开始 epoch `1789108361`，截止 `1789115561`（额外 120 分钟；取代第一轮检查点）。原始 29 行切片保持本地接受且未更改，除任何显式列出的跟进。当前状态：**本地实现已交付；原生 CI 与旧数据限制仍在下方显式列出。** 第一轮第二轮验证：`python3 scripts/test-generate-models.py` 通过（3 个测试，包括来自全部三个源 schema 的确定性元数据与旧/空输出区分）。生成器/目录读取器修改不改变 `agent/src/models/builtin/models.json`。检查发现 `get_default_model_with` 可能在 replacement_model 自身的过滤器之前选择非文本输出；因此最小 w10-6 补丁同样在全局默认选择中过滤文本输出。与 fix-agent 的精确重叠：`agent/src/models/mod.rs` 转换/默认选择/测试与 `agent/src/models/builtin/mod.rs` 输出 schema。未更改其他 agent 路径。

第二轮里程碑（本地 15:12）：序列化 `check-rust.py future-agent` **通过** fmt、clippy all-targets/-D warnings 与完整 cargo test：1715 个库测试通过（1 忽略），42 个集成测试通过（23+1+1+3+14；9 个原生沙箱 smoke 测试忽略），0 个 doctest。新的真实内置转换/替换回归通过。桌面端针对性 identity/hook 测试 4/4 + tsc/eslint 通过；移动端 identity/controller/outbox 测试 61/61 + tsc/eslint 通过。这是本 worker 分支针对原始 agent 基线的结果，不是对 fix-agent 独立分支的验收。

| 跨负责人 / 跟进 ID | 最终本地处置与证据 | 实现/回归 | 缺口 / 集成注记 |
|---|---|---|---|
| w10 BUG-1 | **第一方调用方缺陷已修复。** 目录 id 是 provider 局部的，未预限定；移动端错误地折叠了如 provider=openrouter/id=openrouter/auto 的 id。桌面端 modelKey 本已正确限定，但旧 modelOption 剥离斜杠后缀/选择首个裸匹配并返回未限定的旧默认值。 | `bbc5efe1`：移动端总是构造完整 provider/id；实际 controller set_model 与首个提示测试区分两个具有相同含斜杠 id 的 provider。桌面端 modelOption 只接受唯一的精确旧 id，默认值与活动钩子发出完整身份。Tauri 检查实际 provider 前缀，而不是把每个斜杠都当作限定。新辅助函数/钩子与真实 Rust bridge 测试覆盖裸斜杠与已限定路径。 | Agent 限定优先解析器仍是 fix-agent 的职责；从其独立 worktree 读取源码。已经以 provider 开头的旧冗余 provider+model 字符串在没有协议判别符时无法区分裸 id 与限定 id；当前客户端现在总是发送完整裸 id 限定，因此不要猜测/重写旧的歧义意图。 |
| w10 BUG-6 | **生成器/读取器流水线已修复；随后经授权的发布数据 pass 新增 158 个已验证输出字段（见下方延续）。** 显式输出元数据曾被丢弃且映射硬编码为 text；默认模型选择也绕过 replacement 的非文本过滤器。 | `6d96e393`：保留 models.dev 模态、architecture 模态/arrow 与显式网关非语言类型；序列化 output；serde 读取器以旧文本默认保留非文本/空输出；实际转换/默认/替换测试拒绝图像/嵌入/空候选。Python fixture 覆盖所有源适配器。 | 初始流水线交付保留全部目录字节。之后经授权的源码支撑 pass 处理 3394 个匹配中全部 158 个差异输出，并显式保留 432 个未匹配身份；见发布数据延续/溯源文件。无模型名/上下文大小猜测或其他目录字段刷新。精确重叠的 agent 函数：builtin_models/model_from_builtin/get_default_model_with 加新测试，以及 builtin::Model.output。 |
| w13 BUG-6 | **未发现桌面/移动调用方不匹配；修正过期注释并验证边界。** 桌面端 prompt_command 发送 Attachment 路径与空 images 向量。移动端上传图像字节、发送 uploadId 引用；remote claim_uploads 物化 AttachmentInput 路径；agent 加载/编码图像为 data URI。 | `bbc5efe1`：桌面测试显式断言 images 为空、图像 kind/path 完好；移动端实际 prompt 测试断言仅 uploadId 引用且无 images 字段。现有 agent 图像准备/data-URI 测试在完整 agent 套件中通过。移除过时的 encode_attachments/base64 调用方注释。 | 第三方 ImageContent::Base64 线上语义/proto 文档仍是 fix-cli 的职责。未添加推测性 image/* MIME 前缀，无 provider 调用或真实图像/秘密读取。 |
| w31 BUG-3 / 原生脚本 | **原生 CI 执行路径已实现。** 仅脚本变更此前不会触发原生回归。独立可选 job 不一定是分支保护所必需，因此该方案被拒绝。 | `.github/workflows/ci.yml` 在现有 `Rust build (Windows)` / 各平台状态内添加 scripts/Makefile/builtin 过滤器与离线 Python + Windows PowerShell 测试，无 job 级跳过或必需检查重命名。仅脚本变更 checkout/运行测试而不编译 Rust。YAML 结构、触发器包含与未更改的 job/check 名称已本地验证；三个 Python 回归脚本通过。 | 在 supervisor 打开最终 PR 且 CI 实际运行前，不声称原生 Windows 执行。这台 Mac 上未安装 Windows/PowerShell。 |
| w30 BUG-3 | **外部 TTL 未解决，保持不变。** | 无秘密/token 捕获，无推测性计时器变更。 | 平台过期策略缺失。 |

### 第二轮完整检查与提交

- 实现提交：`6d96e393`（生成器/内置输出元数据）与 `bbc5efe1`（限定调用方身份/图像边界）。最终 CI/额外回归/证据提交在交接中命名。
- 桌面端最终完整 `tsc --noEmit`、`eslint 'src/**/*.{ts,tsx}'`、Vitest：**100 个文件 / 866 个测试通过**。
- 移动端最终完整 `tsc --noEmit`、`eslint .`、Jest：**44 个套件 / 651 个测试通过**。
- Agent 完整批次：`python3 /Users/geilige/future-os/.future/bughunt-fix/check-rust.py future-agent` 通过（fmt/clippy/完整测试；计数如上）。无窄化 lib-only/串行替代。
- Tauri 完整 fmt/clippy all-targets/-D warnings/test 批次通过：**1138 个库测试 + 1 个二进制测试**，1 个 doctest 忽略。所提供的 check-rust.py 将 Tauri 排除在接受的 crate 名之外，因此临时验证器使用**相同的共享批次锁、固定工具链、Cargo target 与隔离 check-home** 来序列化这个被排除的 crate；随后移除。无冗余独立构建。
- 离线 Python：生成器 3 个测试通过，剖析 2 个测试通过，Android 配置 4 个主机/架构子用例通过。YAML 可解析；所有既有 job 名保留；原生离线安装器是现有各平台门禁的一部分，而非仅可选报告。
- 在所需构建前置条件就绪后，未观察到第二轮测试失败或 flake。下方第一轮 Tauri 失败历史是历史记录，不是另一个第二轮失败的声明。
- Supervisor 的剩余工作：与 fix-agent 对账重叠的 agent 函数，在最终 PR 上运行原生 Windows CI 步骤，保留旧目录元数据与 JWT TTL 限制，而非把它们计为已修复的生产事故。本 worker 无分支合并/push/PR/新 todo。

## 发布数据延续（todo_64d9f49b8ea7）

在未更改的 16:32:41 截止时间内，supervisor 明确授权对现有三个元数据端点做无凭据读取，并对精确匹配的现有 (provider,id) 做仅输出字段编辑。07:44 UTC 的检查发现 **3394/3826 精确匹配并带 output/type 元数据，432 未匹配，3236 已实际仅文本，158 个输出差异**。所有记录原位保留；不允许价格/上下文/URL/输入编辑或基于名称的推断。新来源证据还暴露了一个生成器 bug：Vercel 嵌入条目同时声明 `type: embedding` 与 `modalities.output: [text]`；显式嵌入任务类型必须对聊天资格优先。**最终结果：应用 158 个仅输出新增；110 个经源码确认的非聊天条目现被排除，48 个混合文本输出保留。** 全部 3826 条记录保留其精确顺序与所有既有的非输出值。432 个未匹配 (provider,id) 对被冻结在 `docs/bughunt/model-output-unmatched-20260911.json`，未做猜测。`docs/bughunt/model-output-provenance-20260911.json` 记录每个端点的获取时间戳、SHA256/字节数、匹配规则、覆盖范围与七个已检查的源 fixture。

相对上次交付的新证据：
- 这改变的是**实际发布数据**，不只是生成器 fixture。报告点名的 `vercel-ai/bfl/flux-2-flex`、`vercel-ai/google/veo-3.1-generate-001`、`vercel-ai/voyage/voyage-3-large`、`vercel-ai/openai/text-embedding-3-large` 与 `poe/openai/sora-2` 经真实内置加载器解析出图像/视频/嵌入输出。
- `handle_cycle_model` 此前完全没有输出过滤器。添加了与列表/默认/替换相同的文本输出资格谓词。真实 RPC 回归从带凭据目录中的每个点名非聊天条目开始，证明循环会跳过它；同时验证实际 `list_models`、全局默认与替换结果。该回归不插入合成目录条目；凭据是隔离的 fixture 字符串。
- 网关嵌入实际具有 `type=embedding` 且 `modalities.output=[text]`；生成器任务类型优先级现在处理该已观察形态。合成与已检查源 fixture 均覆盖它。

验证：序列化 `check-rust.py future-agent` 于 **16:20:49 通过**，包括 fmt、clippy all-targets/-D warnings、**1716 个库测试 + 42 个集成测试**，1 个库压力/9 个原生沙箱测试忽略，0 个 doctest。点名的发布数据回归通过。`PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-generate-models.py`：**4 个测试通过**，包括对照已检查源字段的真实发布目录。结构审计通过：3826 条记录顺序/非输出内容未变，158 个输出新增，110 个非聊天（6 音频/45 图像/35 视频/24 嵌入），432 个唯一未匹配键全部仍不变。

限制：432 个缺失身份无法由这些快照认证，包括已退役的 Vercel Imagen 与 xAI 图像/视频条目；它们保持显式未分类。上游元数据是证据，不是带凭据的推断/API 能力测试。下方先前的笼统“目录未更改”限制是历史记录，已被本次授权的仅输出补丁取代。本延续中未更改或重测前端。额外 agent 重叠仅限于 `rpc/commands/settings.rs::handle_cycle_model` 及其在 settings_tests.rs 中的发布数据回归；先前的 models.rs 变更已交付。无 push/merge/PR 或截止时间延长。

## Finding coverage

下方路径相对于本 worktree。测试文件名位于相应源码目录下，除非完全限定。“Fixed”描述本地实现，不是原生端到端认证。

| ID | 处置与来源/可达性证据 | 变更与回归 | 剩余缺口 |
|---|---|---|---|
| w27 BUG-1 | **已修复。** `ApprovalPrompt.confirmRule/confirmCapabilityRules` 只在 catch 中重置 deciding。当审批重新获取失败时，成功决策可能留下旧卡片挂载。抛出的父级刷新**不是**卡住路径（它本就进入 catch），纠正了原始报告。 | 两个处理器使用 finally。`ApprovalPrompt.test.tsx` 在每条成功规则保存路径后保持实际组件挂载，并断言控件恢复。 | 无真实 WebView 失败注入；覆盖实际 React 交互。 |
| w27 BUG-2 | **已修复。** 可编辑目标提前返回先于 Escape，而规则输入自动聚焦。 | 在可编辑守卫之前处理编辑器 Escape；保留普通输入快捷键隔离。同一测试聚焦输入、关闭编辑器，然后仅在第二次 Escape 时拒绝。 | 组件层面无缺口。 |
| w27 BUG-3 | **已修复，收窄为过度扩展。** `prepareSearch` 即使在首个候选更新时也使用 `all[0]`。 | 窗口从消息外最早的可能匹配开始。`useMessagePagingHook.test.ts` 搜索 u3 而 u1/u2 在外，断言 u3 而非 u1 成为首个渲染项。 | 有意保留用户在搜索关闭时的可见阅读锚点；重置到最新会隐藏已导航结果。搜索最老候选必然渲染连续后缀。无虚拟化重写。 |
| w27 BUG-4 | **对未变化的定稿输出已修复。** ContextPanel 重建工具数组；资源依赖使用数组身份。`commands/runs.rs::list_tool_outputs` 读取规范工具输出。 | `RunInspectPanel` 中使用稳定 run/id/status/end 键；仍允许活动工具轮询刷新。`RunInspectPanel.test.tsx`：三个等价分配只拉取一次；变更的 end 时间戳再次拉取。 | 活动工具有意继续刷新；这不是对实时输出轮询的禁令。 |
| w27 BUG-5 | **已修复。** `Composer.attachImageFiles` 吞掉读/存失败，并可能用后续成功覆盖大小错误状态。 | 累积图像拒绝并在附件处理完成后展示，同时继续处理其他项目。`Composer.paste.test.tsx` 用被拒图像读取驱动真实编辑器粘贴事件并观察文件名/错误。 | 超出分配的单一图像失败之外的混合文件/图像错误聚合不变。 |
| w27 BUG-6 | **修复资格边界。** 只有空白检测使用 trim；原始空白计入两码点非汉阈值的计数。 | 按修剪后的码点计数；更新 `threadSearchQuery.test.ts` 拒绝加号两侧空白，但保留汉字与多字符查询。 | 字面查询匹配不变；仅资格被规范化。 |
| w27 BUG-7 | **文档/死状态修正；非已复现的实时覆盖。** 所有 messagesGenRef 引用是写入/参数，而 `reconcileThreadHistory` 实际比较请求时基线。 | 从 useThreadMessages/useRunReattach/useAgentThreadState 移除未使用的 ref 管道/递增；注释标识基线保护。现有对账、分页与实时投影测试在完整桌面套件中通过。 | 未引入新的代际围栏行为；原始声称的生产覆盖仍被基线保护反驳。 |
| w28 BUG-1 | **已修复，开发 StrictMode 范围。** Cleanup 将 mountedRef 置 false，setup 从不恢复它。 | Setup 恢复 ref；订阅也在 install try/finally 内，使监听拒绝不会搁置 busy 状态。`UpdatePage.test.tsx` 渲染 StrictMode 并测试成功 invoke/unlisten 与失败订阅恢复。 | 不声称生产 StrictMode 双效应影响；无真实更新器下载。 |
| w28 BUG-2 | **已修复。** 线程标题与截断在非 BMP 边界使用 UTF-16 切片。 | 共享截断现在按码点切片；deriveThreadTitle 委托给它。`objects.test.ts` 覆盖标题边界的非 BMP 字符与精确上限输入。 | 未在 WebView 中重演 IPC 拒绝 vs 替换；绝不声称原始报告所假设的损坏 SQLite 写入。 |
| w28 BUG-3 | **部分确认并按站点修复。** 登出可拒绝（Rust logout-down 测试确认）；上传/打开与导出/保存对话框在 catch 之外；community save 只有 finally。**反驳 agent-down refresh_skills 子声明：** commands/skills.rs 在尽力 bridge 刷新后返回 Ok。 | AccountPage 与 CommunityEditionSection 展示捕获的错误；工件对话框进入现有错误处理。`actionErrors.test.tsx` 与 `artifacts/dialogErrors.test.tsx` 驱动全部四个被拒 Promise。未给 refreshSkills 添加无意义 catch。 | 无原生插件失败注入；测试了 mock 的实际组件边界。 |
| w28 BUG-4 | **纯净性修正，无已演示的用户可见生产缺陷。** AppShell 的 updater 调用另一个 setter；当前幂等。 | 将 overlay 关闭移出 updater；updater 只切换 expanded。完整桌面检查通过。 | 不为当前幂等操作虚构行为回归。 |
| w29 BUG-1 | **已修复。** 结构化上游/provider 终止字符串包含 interrupted/cancelled，并在源码之前被归类为用户中止。 | run_error.rs 优先两个结构化前缀。`structured_termination_is_not_a_user_abort` 测试两个实际字符串；现有用户中止/超时断言保留。 | 分类器层面无缺口。 |
| w29 BUG-2 | **已修复。** 补丁正文文本 `++ b/fake.rs` 变为 `+++ b/fake.rs`，覆盖 git_review 与 shadow_review 使用的 map 键。 | 跟踪 hunk 条目/重置并仅接受 hunk 外的 +++ 路径。`hunk_content_cannot_override_file_path` 也检查下一文件头的重置/重命名。 | 解析器测试使用有效统一补丁文本；无需破坏性真实仓库复现。 |
| w29 BUG-3 | **修复错位写入。** 桌面端接受空/相对 HOME，而下游 app/auth/skills 路径直接拼接它。 | home_dir_from 只接受非空绝对 HOME，然后是 USERPROFILE。`home_tests::rejects_empty_and_relative_home_overrides` 在不改变全局 env 的情况下测试回退与拒绝。 | 保留现有 no-home => None/error 行为；与 agent 不同，未新增 OS 账户数据库回退。仍建议原生 Windows 测试。 |
| w29 BUG-4 | **输入验证机制已修复；Windows 暴露仅为测试平台构建。** 原始目标在仅检查 '..' 后传给 Path::join；Windows 根/前缀替换基础。 | 在所有主机上 join 前拒绝反斜杠与冒号。实际本地 TCP `web_server_serves_files_and_rejects_bad_requests` 覆盖驱动器/根/UNC/ADS 目标返回 403，以及普通服务。 | macOS 验证文件系统 IO 前的拒绝；未执行原生 Windows 利用。未触碰真实文件/SMB 目标。 |
| w29 BUG-5 | **已修复。** macOS 回退 `open -t path` 缺少选项终止符（不同于 open crate）。 | 构造 `open -t -- path`；命令参数回归检查类选项操作数而不启动应用。 | 无 Calculator/应用启动复现。现有完整套件的不存在路径打开测试也通过。 |
| w29 BUG-6 | **独立反驳。** config_io.rs 每次调用递增 TEMP_COUNTER；AlreadyExists 经闭包传播到 remove_file(tmp)，因此残余碰撞至多是暂时的，而非永久的。 | 无代码变更。现有原子写入/错误清理测试通过。 | 拒绝建议的 truncate/create 替代：它削弱 create_new 安全性且不必要。 |
| w30 BUG-1 | **机制已修复并带显式可达性限定。** applyOps 在成功但为空的重放后清除 lane.ops 再丢弃缺口后缀。此处未端到端证明 agent-journal 与镜像事件排序。 | 保留所有后缀 ops；同 run 未解决重放使用现有有界指数重试机制与诊断失败回调，而非热循环或丢弃 agent_end/变更。`syncEngine.test.ts` 使用假时间与真实引擎：空成功重放、无热循环、journal-only 追赶、终态 streaming=false、保留的变更。 | 需要实际 journal/发布排序下发生率的集成证据。不计为已演示的生产事故。永久不可用缺口保持挂起并带上限重试延迟。 |
| w30 BUG-2 | **已修复。** ChatScreen 请求 common.retry；两个语言 common map 均缺它。 | 添加 en/zh 键；ComposerDock 测试文件验证两个资源。 | 资源层面无缺口。 |
| w30 BUG-3 | **外部证据未解决，保持不变。** client.scheduleRefresh 没有 int32 上限；GLM 将溢出收窄到 Android，而非 iOS。平台 JWT TTL 不在本仓库中；fixture 使用短 TTL。 | 无推测性生产 bug 修复。 | Supervisor 需要平台签发的 NATS JWT 过期策略（是否 >24.85 天），且不暴露真实 token。随后如相关可测试原生计时器行为。 |
| w30 BUG-4 | **已修复。** 提交清空后，一个字符串 approvalError 被传给每张待处理卡片。 | 错误携带请求 id；ComposerDock 仅将其传给匹配卡片。`ComposerDock.test.ts` 渲染两张卡片并断言只有 A 收到 A 的错误。 | 原生样式未变；用 mock 的展示叶子检查实际组件 props。 |
| w30 BUG-5 | **修复可达上传 EOF 停滞；收窄其他声明。** 本地文件可在 transferSize 捕获后缩小；readBytes 无偏移进展返回零。真实桌面 chunkBytes 为固定正值，下载越界错误终止请求。 | 拒绝空上传读取；验证上传 chunkBytes，取消传输，关闭句柄。`files.test.ts` 覆盖短源加 0/-1/NaN/Infinity 分块大小。 | 非法协议下载无限循环声明不是已确认的当前桌面路径；无推测性下载重写。 |
| w31 BUG-1 | **已修复。** 三种拉取失败均留下 unique_models=[] 且仍以成功退出写入目录/wiki。 | 当没有模型存活时在任何输出写入前抛出。`scripts/test-generate-models.py` 以 None/空源调用实际 main，使用既有目录哨兵，并验证无 wiki 生成。 | 部分上游可用性语义与崩溃原子输出不变；不是分配的 all-failed bug。 |
| w31 BUG-2 | **修复 profile 隔离。** Agent 锁在 profile 服务启动前是按用户的；仅新端口不能隔离它。 | `scripts/profile-isolated.py` 包裹所有 Makefile profile 目标与独立 bench/quick 脚本；子 HOME/USERPROFILE 一次性，发现覆盖清除，子状态保留。离线子进程测试确认环境/清理/非零状态并拒绝真实 home 覆盖。 | 默认 profile 无用户凭据/会话；模型加载剖析需使用单独配置的绝对 FUTURE_PROFILE_HOME。未运行实时 agent 剖析。Windows wrapper 调用需原生 smoke 检查；需要 Python。 |
| w31 BUG-3 | **按源码契约修复，Windows 执行待定。** install.ps1 固定路径保留 v 而发布资产命名剥离它；固定校验和绕过缺警告。 | 规范化前导 v/V 并警告。`scripts/test-install.ps1` mock 下载边界以测试三种版本拼写；绝不安装任何东西。 | 此处无 PowerShell，因此该新离线测试**未运行**。原生命令：`powershell -ExecutionPolicy Bypass -File scripts/test-install.ps1`。 |
| w31 BUG-4 | **已修复。** 实际递归转换器在约 2000 个块引用时崩溃；解析树本身可迭代检查。 | 递归转换/渲染前的迭代 AST 嵌套守卫（64）；超限文档变为无损纯文本。`bughuntParsing.test.ts` 以 2000 层与普通嵌套引用调用实际包解析器。 | 不是对 remark 本身的无限制输入资源保证；无推测性大小上限。 |
| w31 BUG-5 | **运行时通用声明被反驳；文档已修正。** 正确 CommonMark 转义的 UNC 完好到达解析器（新的通过反例）。单个前导反斜杠也是合法的 Windows 驱动器根路径，因此提升为 UNC 会改变目标/权威。 | 修正 localPath 文档以解释解码 href / 转义 / 文件 URI。`bughuntParsing.test.ts` 经真实解析器往返一个正确转义的 UNC。 | 原始未转义源必然遵循 CommonMark 转义。拒绝自动单反斜杠转网络共享“修复”，因其不安全/歧义。 |
| w31 BUG-6 | **已修复。** 数字扫描器缺少字面扫描器的词边界与彩色 error_404/user2 后缀。 | numberEnd 中的对称词边界检查。实际分词器回归以无损文本覆盖标识符与有效整数/分数/指数 token。 | 不旨在验证任意畸形 JSON；仅修复 token 边界缺陷。 |
| w31 BUG-7 | **修复新模拟器配置。** ABI 与 SDK 默认值硬编码为 Apple Silicon/macOS。 | 检测 x86_64/arm64、Darwin/Linux SDK 路径、尊重 ANDROID_SDK_ROOT、Linux javac home 发现。`test-android-config.py` 以假工具运行四个主机/架构组合的实际 shell 脚本，有意停在 SDK 边界。 | 无 SDK 下载/模拟器启动。现有错误配置的 AVD 不会自动删除/迁移。macOS Homebrew 用户可能仍需显式 JAVA_HOME。 |

## 第一轮验证结果（历史）

最终前端通过约为本地 13:15：

- 桌面端：`tsc --noEmit` **通过**；`eslint 'src/**/*.{ts,tsx}'` **通过**；Vitest **98 个文件 / 862 个测试通过**。
- 移动端：`tsc --noEmit` **通过**；`eslint .` **通过**；Jest `--runInBand` **44 个套件 / 647 个测试通过**。
- Tauri：固定 Rust **1.97.0**，`make desktop-sidecar-placeholder`，`ulimit -n 10240`，共享 `CARGO_TARGET_DIR=/Users/geilige/future-os/target`；`cargo fmt --check` **通过**，`cargo clippy --all-targets -- -D warnings` **通过**。最终完整 `cargo test --quiet`，隔离 HOME+USERPROFILE：**1137 个库测试 + 1 个二进制测试通过；1 个 doctest 忽略**（93 秒）。
- 保留失败历史：首次 Tauri 运行超过 120 秒工具超时。下一次完整运行有 **1136 通过 / 1 失败**，`agent_bridge::pipeline_tests::reanimate_still_streaming_run_attaches_an_observer`：test_support.rs:462 处 fixture 初始化期间 SQLite DatabaseBusy。未改 SQLite 代码；随后未更改的完整重跑通过 1137/1137。按观测到的测试 flake 处理，不静默“修复”。
- Python：`python3 scripts/test-generate-models.py` 通过（两个源子用例）；`python3 scripts/test-profile-isolated.py` 通过（2 个测试）；`python3 scripts/test-android-config.py` 通过（四个主机/架构子用例）。`bash -n scripts/start-mobile-android.sh scripts/agent-profile-bench.sh` 通过。
- PowerShell 安装器测试：**未运行**（未安装 pwsh/Windows PowerShell）。未安装任何工具来强迫平台证据。

### Worktree 依赖注意事项与可复现性

现有依赖从 main 检出只读复用。临时 Vite/Jest 配置将 `@future-os/{markdown,thread-projection,json-preview}` 映射到本 worktree 的源码，因此测试**不会**意外执行未更改的祖先工作区包。这些临时配置/依赖链接在交接前移除。在正常安装的集成 worktree 中，标准 `cd desktop && npx vitest run` 与 `cd mobile && npm test` 即足够。此 worktree 最初缺少生成的移动端 version.ts 与 future-share-intent 依赖链接；为检查生成常规版本工件并链接本地模块（无受跟踪的包/依赖变更）。临时生成/符号链接/缓存工件在交接前移除。

## New evidence versus source audits / rejected approaches

- 添加实际实现/组件测试而非复制的伪代码：StrictMode 安装、两条规则保存路径、聚焦 Escape、真实粘贴、四个 UI 错误边界、有界重放恢复、上传 EOF 取消、解析器深度、分词器、原生命令参数构造与 HTTP 处理器。
- 新 UNC 反例反驳“UNC 永远无法通过 markdown 存活”；文档不准确，但静默把根路径升级为 SMB 不是安全修复。
- 确认 w29 BUG-6 反驳并保留 create_new。未在尽力 refresh_skills 周围添加无意义 catch、未在关闭时把搜索重置到最新、未移除实时输出轮询、未把真实凭据复制进剖析 home、未推断生产 JWT TTL。
- 第一轮没有需要另一 worker 源码编辑的协议/schema/API 变更。第二轮添加向后兼容的内置目录 output 字段并文档化上述精确 models 模块重叠；无 RPC schema 变更。共享 markdown 行为仍只在超过嵌套限制时回退为纯文本；桌面/移动套件均覆盖消费者。Supervisor 应在集成 fix-cli 的共享 RPC 变更后重跑范围内检查。

## Remaining acceptance gaps / next useful checks

1. **外部未解决发现：** w30 BUG-3 平台 TTL。向平台负责人询问过期策略；无需 token 内容。在那之前不声称这是可达的生产缺陷。
2. **原生验证：** 第二轮将离线 PowerShell 安装器与 Python 剖析回归接入现有 Windows CI 门禁；等待最终 PR 实际执行。Tauri 静态服务器原生路径验证与真实 x86 Android 模拟器仍是平台检查，不由 macOS 单元测试声称。
3. **旧目录数据：** 经授权的延续现在补丁 158 个输出字段并排除 110 个经源码确认的非聊天条目。精确剩余的 432 个未匹配身份保留在 model-output-unmatched-20260911.json 中；无虚构注释。它们不可用/已退役的源元数据仍是证据缺口，不是断言的产生缺陷。
4. **集成限定：** 若存在确定性后端集成接缝，测试延迟的 journal 可用性 vs 镜像移动端事件；本地重放算法已测试，但原始发生率/可达性未证明。
5. Supervisor 拥有最终合并 PR、origin/main 同步、CI/auto-merge 与分支/worktree 清理。本 worker 不创建后继 todo 且不关闭整体 goal。交接使用 `--no-follow-up`；上述缺口明确留给 supervisor。
