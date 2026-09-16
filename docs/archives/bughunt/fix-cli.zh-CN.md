# fix-cli 发现账本

> 本文是历史快照 [fix-cli finding ledger](./fix-cli.md)（2026-09-11，commit `914f27f6`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

会话 `20260911-123030-f93dca`；分支 `claude/bughunt-cli-channels`；基线 `fc81c016`。**已交付供 SUPERVISOR 评审——自有实现与完整范围内检查通过**。这不是全局目标/PR 完成。第一轮于 12:30 CST 开始；受控停止保留了所有文件。第二轮授权自 epoch 1789108361 起 120 分钟，截止 1789115561（2026-09-11 16:32:41 CST）。第三轮授权（start1789127056/deadline1789134256）取代该历史窗口。阅读 CONTRACT、ROUND2、ROUND3、RUN 与 supervisor-review。无 push/PR/merge/额外 worker。

只读证据来源：`/Users/geilige/future-os/.future/bughunt/{REPORT,CROSS-MODEL-VERIFICATION,w21*,w22*,w23*,w24*,w25*,w26*,w32*,verify/V3,verify/V4,verify/KIMI,verify/GLM-b5,verify/GLM-b6,verify/GLM-b7,verify/GLM-b8}.md`。源码实现与实际 fixture 优先于历史报告标签。下方 **Implemented** 表示代码加回归在 2026-09-11 19:56:14 CST 的最终范围内批次中验证；这不声称原生平台/实时服务认证。61 个原始行 + 3 个 V4 额外项 + 第二轮 w13-6。

## Finding coverage

下方路径相对于 workspace。回归名可用 `cargo test -p <crate> <name>` 运行；套件汇总如下。

| Finding | 处置与来源证据 | 实现 / 回归 / 剩余缺口 |
|---|---|---|
| w21 BUG-1 | 确认：`tools_call` 从不识别文档化的 --args | 在 `cli/src/commands/tools.rs` 实现顶层 JSON 参数解析与显式标志覆盖；`tools_call_input_mask_base64_and_file_type_lowercasing` 中记录的 MCP 体现在使用 --args 并断言无嵌套参数。 |
| w21 BUG-2 | 确认平台无关的错误种类 bug | 在字符串转换前实现 ErrorKind::NotFound；`load_api_key_resolution_order` 覆盖缺失 auth；未执行原生 Windows 诊断。 |
| w21 BUG-3 | 确认：格式化器在解码/写入失败后无条件声称已保存 | 经图像/工具格式化器与 CLI 退出实现 Result 传播；bad-base64、blocked-parent 测试。无剩余虚假成功路径。 |
| w21 BUG-4 | 确认：seconds*1000 未检查 | 实现检查后的 u64 转换；`timeout_overflow_and_paper_limit_are_rejected`。 |
| w21 BUG-5 | 确认：parent expect 对根/空 panic | 实现文件名验证；`format_image_result_root_output_path_has_no_parent` 现在使用真实 b64_json（旧测试误用 base64，从未到达保存）。 |
| w21 BUG-6 | 确认：全路径 rfind 点使用父级扩展名 | 实现 Path file_stem/extension；`image_output_parent_dots_and_write_errors`。 |
| w21 BUG-7 | 确认：含 1401/4403 的消息被误诊 | 实现完整数字 token 匹配；`translate_error_tool_specific_and_default` 覆盖四个反例。对真实独立数字错误文本仍为启发式，不是结构化 HTTP API 重设计。 |
| w21 BUG-8 | 确认：宣传的 1..20 限制缺失 | 实现 int_range；timeout/paper-limit 回归。 |
| w22 BUG-1 | 确认静默连接挂起；拒绝提议的总体 5 分钟截断，因为当前传输显式支持长 run | 实现 300 秒无活动看门狗，由每个事件重置；`silent_event_stream_times_out_instead_of_hanging` 使用静默 mock 与 50ms 测试超时。健康 ping 的长 run 有意保持无界。 |
| w22 BUG-2 | 确认流 Err 被当作正常成功 | 实现错误传播并修正误导性奇偶注释/测试；`stream_events_mid_stream_error_is_reported`。 |
| w22 BUG-3 | 确认接受空键存在 | 在 auth/account 中实现空键过滤并拒绝空 OAuth token；`get_future_auth_entry_edge_cases`；新增 CLI account 空键拒绝回归；无原生 Windows 执行。 |
| w22 BUG-4 | 确认缺失余额被虚假变成零 | 实现省略缺失的 profile 键并将未知信用置 null/NaN；`balance_missing_field_is_unknown`；完整检查通过。 |
| w22 BUG-5 | 确认 reqwest 没有默认 30s 超时 | 实现覆盖 auth 中正文的显式 30 秒请求超时；现有 mock 登录测试；专用 `auth_request_timeout_includes_response_body` 测试实际 30 秒请求超时。 |
| w22 BUG-6 | 确认未检查 u64 算术；原始报告错误地说巨大 elapsed 仍小于 expiry | 实现单调 Instant、检查后的 expiry、Duration 秒（无乘法）、按剩余授权生命限制 sleep。`login_rejects_extreme_expiry_and_empty_granted_key` 覆盖 u64::MAX expiry 与空授予键，无真实打开器/网络。 |
| w23 BUG-1 | 确认 var 闭包捕获最后一个控制台级别/方法 | 在实际嵌入脚本中实现 let 绑定；`node cli/tests/console-hook.mjs` 执行源码，14:47 通过。 |
| w23 BUG-2 | 确认 href 回退在 _blank/download/慢原生动作后错误导航当前标签 | 移除合成 href 导航而非猜测标签目标；mock 回归 `click_href_does_not_synthesize_navigation_when_native_action_is_unobserved`。本轮未重跑原生浏览器。 |
| w23 BUG-3 | 确认仅头部超时排除正文 | 在 endpoint、browser status、Safari status 与 WebDriver fetch 实现请求超时；新增 `response_body_is_covered_by_timeout`；修正两个旧诊断竞态预期。 |
| w23 BUG-4 | 确认显式导航与 ActionNavigationObserver 丢弃 unsubscribe 句柄而不执行 | 在完成/dispose/重新武装时实现 unsubscribe；观察器回归在 dispose 后派发并断言状态未变。 |
| w23 BUG-5 | 确认引号遗漏 tab | 在 windows_process 实现空白谓词与 tab/newline 测试；未执行原生 Windows 启动。 |
| w23 BUG-6 | 确认 float/端口收窄 | 在 cast 前实现有限整数/范围检查；`tabs_error_paths` 覆盖小数索引与非法端口。 |
| w23 BUG-7 | 确认负半舍入偏离承诺的 JS 语义 | 在 input 与 click 实现 floor(center+0.5)；负半中心测试。 |
| w23 BUG-8 | 确认小写 Safari 键与未转义文本 XPath | 实现大小写不敏感键映射，将 mock press 改为小写；添加引号安全 XPath 字面量构建器；引号字面量回归覆盖两种引号；未执行原生 Safari。 |
| w24 BUG-1 | 确认 UTF-8 审批预览 panic | 实现 floor_char_boundary；真实审批卡片测试现含 >500B 的 CJK+emoji；与 w32-2 重复。 |
| w24 BUG-2 | 确认钉钉在完整权限会话创建前无准入 | 实现 sender_allowlist，默认空/拒绝；显式 ["*"] 选择信任全部。在斜杠/提示词之前应用于 DM 与群发送方。`admission_denies_unlisted_senders_before_slash_or_prompt`。运营迁移必须填充发送方 ID。 |
| w24 BUG-3 | 确认远程文件名被拼接为路径；写入错误被吞 | 实现可移植单文件名验证、唯一 UUID 前缀 create_new 保存、传播失败。测试绝对/穿越/Windows 分隔符/ADS/根/空/重复上传/被阻止父级。与 w32-1、V4 N3 重复。 |
| w24 BUG-4 | 确认每次 ensure_session 重置 model/effort | 默认值只初始化新会话；`session_defaults_are_not_reapplied_after_model_change`。现有会话重激活保留。 |
| w24 BUG-5 | 确认真实卡片事件省略 chat_type，且当前会话查找丢失线程 | 在实际审批事件上实现服务端审批 ID→来源 chat/type/session 绑定。策略使用绑定群/DM 上下文，拒绝未绑定/跨聊天 ID；`group_thread_card_uses_origin_policy_and_session` 包含缺失 chat_type 与重置线程映射。重启后的桥安全拒绝旧未绑定卡片（非静默成功）。 |
| w24 BUG-6 | 确认卡片消息 ID 不是点击事件身份；原始提议的可变决策语义被拒绝 | 卡片去重使用头 event_id，回退 message/sender/action 正文。`card_action_approve_and_reject` 中同卡片不同请求回归；移除 delivered 路由，重复决策不能改变已接受的审批。 |
| w24 BUG-7 | 确认 Error 与 AgentEnd(error) 跳过卡片定稿 | 在两条错误路径上实现 settings(false) 然后完整内容更新；`error_finalizes_existing_stream_card`。同时修复突发 EOF/传输清理：停止流式、发布部分文本、移除打字状态、传播传输错误。`abrupt_stream_end_cleans_card_and_reaction` 覆盖干净 EOF 与中途失败。 |
| w24 BUG-8 | 确认 ToolEnd 变更错误依赖现有 CardKit 卡片 | 实现与卡片无关的本地标记替换；`tool_first_completion_and_nonstreaming_thinking`。 |
| w24 BUG-9 | 确认早期提及门禁忽略配置的 false | 将实际 mention 标志传给策略；静默拒绝非提及保留；`group_without_mention_respects_disabled_requirement`。 |
| w24 BUG-10 | 确认 setup/传输错误不可见；流语义错误已有回复 | 为 spawn 失败与会话 setup 失败实现 webhook 错误回复；修正 `prompt_failure_is_logged_not_raised` 要求 Error 回复。 |
| w24 BUG-11 | 确认忽略所有 HTTP 状态；未观察到真实 provider 失败 schema | 实现 HTTP error_for_status 与非零 errcode 拒绝；`webhook_rejects_http_and_application_failures` 测试 503 与合成 200/310000。不声称原生钉钉集成。 |
| w24 BUG-12 | 确认负 expiry cast 产生巨大 Instant 加 | 实现饱和减法、非负 expiry 与检查后的 Instant 加；`token_short_or_negative_expiry_does_not_panic` 覆盖 30/0/-1/i64::MIN。 |
| w24 BUG-13 | 确认被取代流留下打字状态 | 在取代时实现移除打字并停止旧卡片流式；现有取代测试更新；迟到到达测试单独断言反应删除；现有取代测试确认卡片 settings 关闭。 |
| w24 BUG-14 | 确认死图像限制/打字/名称配置 | 在 Vec 增长前实现 content-length + 分块字节限制（所有附件）、打字标志、授权消息日志的发送方名称解析。`download_resource_ok_and_http_error` 包含限制与图像 post 测试禁用打字。`chunked_resource_is_limited_without_content_length` 覆盖无长度头的流式计数器。 |
| w24 BUG-15 | 确认仅图像 post 经过 file_key 解析器 | 实现 post img 键提取 + 图像处理器复用；`image_only_post_is_downloaded_and_typing_flag_is_respected`。仅支持第一张 post 图像；多图像 post 语义未扩展。 |
| w24 BUG-16 | 确认缺失/失败会话上的无条件 Approved | 实现真实的 not-delivered 确认与来源路由；现有 no-session/RPC 失败测试修正。 |
| w24 BUG-17 | 确认 thinking 忽略 streaming false | 实现共享流式谓词；组合 tool-first/nonstreaming-thinking 回归。 |
| w25 BUG-2 | 已修复：两个 WS 回调现在同步按会话排队到达 | 弱持有队列消费者只串行化 setup；代际在流任务 spawn 前分配，过期代际在 start 锁下被拒绝。飞书 slow-old-ACK 队列测试与两个渠道 stale-generation 测试通过。响应流保持并发；独立聊天有独立队列。第三轮将每队列限制为 128 事件，溢出返回 false 并警告；两个 `ingress_queue_has_a_fixed_capacity` 测试验证第 129 个事件被拒。溢出策略已文档化，不静默表述为已执行的工作。 |
| w25 BUG-4 | 确认 HashSet 迭代不是最老优先 | 在**两个**渠道实现 VecDeque FIFO 有界窗口。飞书有界去重回归断言 1005 次到达后保留最老=500/最新=1004；套件覆盖重复投递。 |
| w25 BUG-5 | 潜在，未发现当前生产重入处理器 | 路由器派发持锁，回调今天只变更自身状态。chromium_page/chromium_navigation/execution_context 闭包中的调用方更新自身状态，从不触碰 router add/unsubscribe/clear。无生产重入路径：保留为潜在 API 约束，无代码变更或生产修复声明。 |
| w25 BUG-7 | 已修复：浏览器配置直接写入与过期读/改/写 | OS 文件锁现在覆盖全部八个生产读/改/写站点。原子临时文件持久化发布完整 JSON；写入与发布间无 await，避免取消释放锁的写入竞态。并发 20 次更新 fixture 保留所有引用；现有 load/save 测试通过。 |
| w25 BUG-8 | 确认发送预检查与断开排干竞态 | 在持有 pending mutex 时实现复检；新增并发发送/断开回归（压力调度，非确定性强制的旧窗口）。 |
| w26 BUG-1 | 修复协议奇偶；不声称先前 data-first 消费者数据丢失 | 增量可选 ToolStart phase4/index5、ToolDelta snapshot4；编解码保留 true/false/absent。实际 agent-wire fixture 经 prost 序列化；冻结旧解码器兼容测试通过。新增 CLI bridge-consumer 奇偶集成。 |
| w26 BUG-2 | 修复协议奇偶；双写契约保留 | 追加 AgentEnd truncation6、UsageEvent stop_reason2、UsageInfo reasoning7/provider_metadata8。生产者 wire fixture 与实际 future_agent::types::Usage 集成覆盖元数据与成本序列化。无字段编号复用。 |
| w26 BUG-3 | 实现有意义的最终页 nextOffset 保留 | `final_entries_page_retains_pagination_metadata`；当前非可选 proto 中未分页与分页空/offset0 仍不可区分。历史报告省略有效的未分页响应。该残余默认键歧义无值级损失；无破坏性 optional-scalar API 变更。 |
| w26 BUG-4 | 确认显式自定义 shell RPC 时长契约超过固定 125s；内置调用方默认 | 实现动态请求超时匹配服务器 clamp +5s；`shell_deadline_tracks_clamped_execution_timeout` 测试默认/短/10min/max。 |
| w26 BUG-5 | 确认所有权检查前的 chmod 可在特权下变更外部目录，并在非特权下掩盖拒绝 | 重排元数据所有权先于 chmod；`foreign_socket_directory_is_rejected_before_chmod`，安全跳过 root。 |
| w26 BUG-6 | 为畸形导入图像块实现不透明保留 | `final_entries_page_retains_pagination_metadata` 也断言原始图像元数据保留；完整 session-entry proto 编解码现在断言原始畸形图像元数据保留。 |
| w26 BUG-7 | 潜在/契约评审：畸形旧条目被整体拒绝；同版本生产者使用严格规范条目 | 反驳为当前支持路径缺陷：`agent/session/display.rs::project_entries` 发出规范 SessionEntryPayload 且解码器有意 deny_unknown_fields；现有同契约测试拒绝旧内容。无行丢弃修复：静默丢失历史比显式不兼容更糟。潜在旧/外部 agent 兼容性保留。 |
| w26 BUG-8 | 潜在/损坏输入行为；字节稳定数据契约使建议的 JSON 访问器回退在没有消费者证明时不安全 | 无变更：所有仓库内生产者在分配 data 前序列化有效 JSON；字符串保留权威 journal 字节。损坏/外部写入者回退是潜在恢复策略请求，不是可达的同版本生产者缺陷。更改原始访问器将违反显式字节稳定契约。 |
| w26 BUG-9 | 潜在前向兼容：当前枚举仅 existing/running/queued；未知→Running 存疑 | 不存在新生产状态（RunAcceptedState 仅 Existing/Running/Queued；调度器构造函数匹配）。保留潜在兼容性关注，不声称修复生产行为；无新枚举/API 或推测默认语义。 |
| w26 BUG-10 | 确认空错误在 typed/JSON 间不同 | 实现通用未知错误解释并在 typed 重建中保留错误键；parse-both-twins 与空错误奇偶通过。 |
| w26 BUG-11 | 反驳生产歧义：accepted_state=queued 显式区分哨兵 epoch0 与 running epoch0 | 当前无消费者以 queued epoch 设围栏；文档化 epoch 有效性而非破坏 proto scalar API。Proto 与 RunAck 文档现在显式要求在使用 epoch 作围栏前检查 accepted_state。 |
| w26 BUG-12 | 潜在：run_terminal 生产者总是写 usage | 源码搜索：`agent/src/rpc/mod.rs` 仅经 `future_rpc::message::run_terminal` 获取 requestedRun，其总是发出 usage。机制需要契约外生产者：潜在，无代码变更或双写恢复。 |
| w32 BUG-1 | 重复确认 w24-3 | 相同实现/测试；不计为不同 bug。 |
| w32 BUG-2 | 重复确认 w24-1 | 相同实现/测试。 |
| w32 BUG-3 | 潜在，休眠钉钉 AI 卡片模块 | `stream_ai_card` 无桥调用方；裸字节切片存在但今天无生产影响。无推测激活/重构。 |
| w32 BUG-4 | **非自有确认** `agent/src/llm/sse.rs::push` 每个换行排干/memmoves 后缀 | Supervisor/agent 交接：使用游标扫描完整行并每次 push 只排干前缀一次，保留拆分 UTF-8/CRLF 与事件边界；以许多短行测试实际解码器。本分支无 agent 编辑。 |
| w32 BUG-5 | **非自有确认，有限** loop 页 goal ID 由 CLI 控制并经 encodeURIComponent 注入内联 onclick（撇号保留） | Supervisor/loop 交接：用数据属性/委托处理器替换内联 JS，或正确的 JS 字符串+HTML 转义。位置 `orchestration/loop/src/webui/page.rs` 370/381；todo-ID 537/713 当前已生成且潜在。此处无 loop 编辑。 |
| V4 N1 | 与 w26-8 同损坏数据族；原始 JSON 访问器有意承诺逐字数据 | 反驳为独立 bug：原始访问器承诺逐字字节，解析访问器无法表示无效 JSON；相同有效生产者输入一致。损坏输入恢复策略仍为潜在 w26-8，非生产修复声明。 |
| V4 N2 | 重复空错误键问题 w26-10 | 实现序列化保留空错误键；空错误奇偶通过。 |
| V4 N3 | 重复保存失败 w24-3 | 实现错误传播；失败时无成功保存路径。 |
| w13 BUG-6 (round 2) | 修复文档不匹配，非第一方运行时缺陷 | 检查 agent grpc Base64→ImageContent 与 prompt_helpers image_url 构造及渠道 data-URI 生产者。Proto Base64 field11 与生成文档现在显式要求完整 MIME data URI，而非原始字节。无字段重编号或猜测 MIME 转换；apps/agent 运行时边界仍归其 worker。 |

## commits

- `dc642044`：保留初始 CLI/channel/RPC 修复与账本。
- `d9bff2ab`：有序渠道入站/启动交接，FIFO 去重，浏览器 OS 锁事务与原子配置写入。
- `7e621a5c`：增量事件线上元数据，重新生成 proto，新旧兼容与生产者-消费者 fixture。
- `c7bb4ea5`：最终 EOF/错误清理、auth/限制回归、configure/env 隔离修复，及中英文配置迁移文档。
- `fbda3cce`：有界 128 事件会话队列、过载拒绝回归与文档化的饱和行为。
- 代码提交后 worktree 干净；本 worker 生成的 `target/test-homes` 已移除（共享构建 target 未动）。最终纯报告提交在交接中列出。

## coverage

全部 **61 个分配原始项**、**V4 N1/N2/N3** 与跨负责人 **w13 BUG-6** 在上有各自的当前处置。重复 ID 不是不同的生产 bug。没有已确认的自有发现被静默省略；非自有 w32-4/w32-5 显式交给 supervisor/loop worker。潜在的不受支持生产者/重入用例不计为已修复的生产缺陷。

## tests

**通过：** `python3 /Users/geilige/future-os/.future/bughunt-fix/check-rust.py future-rpc future-channel future-cli` 于 2026-09-11 **19:56:14 CST** 完成，exit0，使用固定 1.97.0、序列化 target 批次、隔离 HOME、完整 `fmt --check`、`clippy --all-targets -- -D warnings`，与三个 crate 的完整 `cargo test -p`。

- future-rpc：**115 单元 + 1 增量兼容 + 5 线上集成**，全部通过；文档测试通过（0）。
- future-channel：**362 单元 + 10 二进制集成**，全部通过；**2 个既有 live-agent 测试忽略**；文档测试通过（0）。
- future-cli：**724 库 + 2 二进制单元 + 11 二进制集成 + 2 生产者/消费者集成**，全部通过；文档测试通过（0）。
- `node cli/tests/console-hook.mjs`：针对实际嵌入 JavaScript 通过。
- `git diff --check`：通过。第三轮保留最终第二轮通过，提交全部 14 个待处理文件，然后添加有界入站并成功重跑整个批次。
- 机械 INDEX.json 对账：**61/61 原始分配 ID 存在；无缺失行；全部 V4 额外项与 w13-6 存在**。

并行 configure 失败被诊断而非重跑消除：`mcp::tests::initialize_error_without_code_says_unknown` 调用 `point_platform_at`（只写 base_url 的 auth.json）而没有 env 锁或隔离 HOME，覆盖 configure 的 fixture。添加共享 env 锁与全新测试 home；configure 保留随后在**三次完整并行套件**中通过。第二个无守卫 run-argument 测试读到并发 grpc override；它现在持有相同锁并移除 override。非法浏览器事务测试对象引用 fixture 修正为规范字符串引用，保留严格生产验证。这些是测试工具修复，非声称的生产缺陷。流清理中后来出现的 clippy useless-conversion 失败在最终成功批次前修正。

## gaps

- Supervisor 必须集成并独立评审所有提交，然后对照追加的 proto 字段检查组合直接消费者（agent/TUI/apps）。CLI 集成已测试实际 agent Usage 类型与共享桥解析器；未发生运行时协议重写或事件数据双写移除。
- 原生 Windows 浏览器引号/文件锁行为、真实 Chrome/Safari 默认动作与真实钉钉 webhook 失败未在这台 Mac 上执行。确定性可移植测试覆盖实现边界；未作原生/实时声明。
- w32-4 SSE 修复归 supervisor；w32-5 WebUI 转义归 loop-worker。精确位置/所需行为在其行内；本分支不含 agent/ 或 loop/ 中的编辑。
- 保留的潜在用例：w25-5 重入路由器 API；w26-8 损坏数据恢复，w26-9 未知未来 accepted state，w26-12 不可能的当前生产者形态；w32-3 休眠 AI 卡片 Unicode 路径。w26-7 不支持的旧页拒绝与 w26-11 排队 epoch 歧义不是当前支持路径缺陷。V4 N1 在逐字数据契约下不是独立缺陷。
- w26-3 保留有意义的最终页偏移；分页空 offset0 与未分页默认键在线上仍不可区分，无值级损失。未发明破坏性 scalar-to-optional 迁移。
- 升级迁移在中英文渠道配置文档中已文档化：钉钉 sender_allowlist 默认拒绝全部；显式通配为信任全部。旧飞书审批卡片在桥重启后不能重绑，报告未送达而非到达不同会话。

## Executed validation / rejected evidence（历史检查点）

第一轮观测：
- `cargo test -p future-channel --lib`：初始 350 通过 / 2 旧预期失败。新增/修正后，13:02 **355 通过**（最终小改动前）。
- `cargo test -p future-rpc`：12:56 **114 单元 + 5 集成通过**。
- CLI 库首跑：713 通过 / 5 失败。修正两个超时诊断竞态与旧 click/balance 预期。未改动的 configure 测试并行失败一次，随后 13:04 **隔离通过**；这不是 flake 已修复的证明。历史待处理状态；所有新增随后在 16:13:41 完整批次通过。
- 对触碰 crate 应用格式化。`git diff --check` 在第二轮恢复时通过。
- 组合 clippy 尝试在共享 target 锁/重建期间 120 秒超时；**不是通过**。第二轮使用序列化 check-rust.py 与长超时。
- 未执行原生 Windows、真实 Chrome/Safari/钉钉或外部凭据测试。测试使用 mock HTTP/gRPC 与临时路径。未启动/杀死生产 agent。

## Round2 validation checkpoint (15:36 CST)

- 提交：`dc642044`（保留第一轮），`d9bff2ab`（到达顺序/浏览器事务）。
- 序列化 `check-rust.py future-rpc future-channel future-cli`：RPC fmt/clippy/完整测试通过（115 单元 + 1 增量兼容 + 5 线上集成）；channels fmt/clippy/完整测试通过（358 单元 + 10 二进制；2 个 live-agent 集成测试保持有意忽略）。CLI fmt/clippy 通过；完整测试停在库测试：719 通过 / 3 失败。
- 失败：新浏览器事务 fixture 使用无效对象引用而非规范字符串选择器（修正 fixture，不弱化验证）；既有 configure 保留再次并行失败（根因随后识别并修复；见最终结果）；无守卫 run-argument 测试读取并发变更的 grpc env（修正测试持有共享 env 锁）。CLI 集成/doc 测试在此失败检查点批次未运行，但随后在最终批次通过。
- 增量 proto 字段已实现并重新生成：ToolStart phase=4/tc_index=5，ToolDelta snapshot=4，AgentEnd truncation_json=6，UsageEvent stop_reason=2，UsageInfo reasoning_tokens=7/provider_metadata_json=8。无复用编号；原始事件数据保持字节稳定。Fixture 来自实际 agent 线上投影，经 protobuf 编解码，覆盖 snapshot true 与 false。冻结旧 ToolStart 解码器证明双向兼容。新 CLI 集成使用实际 agent Usage 类型加桥消费者；两个集成测试随后通过。

## Checkpoint / integration notes

第二轮第一个动作检查状态/差异：34 个受跟踪代码文件 + 账本 + console-hook 回归保留在检查点 **dc642044**。确认基础设施 HTTP500 中断；无重置或截止延长。14:47 时 channels **358 个库测试通过**（含新有序到达与过期代际测试），且 `node cli/tests/console-hook.mjs` 针对实际嵌入脚本通过。后续第二轮变更实现逐会话入站队列、spawn 前分配代际、钉钉 FIFO 去重、浏览器 OS 锁读/改/写站点与原子配置发布；浏览器事务回归随后通过。移除回调迁移暴露的未使用导入。Proto 新增为仅追加；未更改任何现有字段编号。公共渠道 API `download_resource` 现在接受显式字节限制；仅找到并更新 crate 内调用方。钉钉配置迁移：填充 `sender_allowlist`；空拒绝全部，通配显式信任全部。审批绑定仅内存且按来源限定：桥重启后过期卡片报告未送达，而非审批无关的当前会话。EOF/传输清理、到达排序、浏览器事务与 typed 事件奇偶此后已实现并通过完整批次；剩余边界列于下方。
