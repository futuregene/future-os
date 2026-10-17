# 测试编写与平台差异（本仓库）

## 新写的乐观测试会让 jest 不退出 → CI 的 mobile job 卡 30+ 分钟（2026-10-13 实测）

改 `mobile/src/remote/syncEngine` 那类**带 lane 重试定时器**的模块时：一个**新写/改写的测试**如果踩到"reconcile 失败"路径，lane 会 `setTimeout` 挂一个**真**重试定时器；该测试若既不用 `jest.useFakeTimers()`、也没在 `finally` 里 `engine.clear()`，进程就永不退出。

- 本地症状**很轻**：套件全绿 + `Jest did not exit one second after the test run has completed` 一句警告，`npx jest` 一直挂着（我把它当背景任务跑，没看退出码，白等两轮）。
- CI 症状**很重**：`Mobile typecheck + lint + test` 的 jest 步骤 `in_progress` 35 分钟（正常 2–3 分钟），step 内部测试全过，job 卡在选择等进程退出 —— 而同一 run 的 Desktop/Rust 全绿，容易误判成"排队卡死"去 cancel/推空提交。
- 判据（比读日志快）：把改动过的每个测试文件**单独**跑并看退出码 —— `timeout 90 npx jest --runInBand <file>` 后 `echo $?`，`124` 就是这个坑。`--detectOpenHandles` 会**同样挂住**（它要等 handle 关闭才打印），别指望它。
- 修法：该测试里 `try { … } finally { h.engine.clear(); }`（引擎的 `clear()` 会取消 pending retry），或全程 fake timers。
- 顺带：`npx jest --runInBand` 用 `nohup … &` 跑时**一定要把 `echo $?` 写进日志**再 poll 日志，不然"绿了"的假象能骗过整轮验证。

## Linux CI vs macOS 差异（各踩过一次）
1. `wait()` poll fd 0：CI 的 stdin=/dev/null 永远 EOF 就绪 → 超时断言变 Input；用 idle-pipe Fd0Guard 修。
2. struct-update `..guard` 对 Copy 字段不消耗 base → guard 提前 Drop 把 fd 0 恢复了；直接构造。
3. WT_SESSION env 污染：两把锁（KITTY_STATE_LOCK vs ENV_LOCK）不互斥，两个都要拿。
4. broadcast channel 无历史 → subscribe 前的 Close 事件丢失；server 侧延迟 200ms 发关闭。
5. Safari webdriver 测试必须 cfg(macos) 门控（Linux 直接报错）。
6. `localStorage` 只在 Linux CI 由 jsdom 提供真 `Storage`；本机 macOS 上 jsdom 没有它，`src/test/i18nTestSetup.ts` 才补了一个普通对象。所以"让存储抛错"的测试必须 `vi.stubGlobal("localStorage", …)`；`Storage.prototype` / 实例 spy 只会在一侧生效（#542 本地绿、CI 挂）。
7. **本机 macOS 跑整套 lib 测试前必须提 fd 上限**：默认软限 256（硬限 unlimited），`cargo test -p future-agent --lib` 会因 "Too many open files (os error 24)" 报**几十个互不相关**的失败（tools/session/skills 混着挂，单跑各自全绿），容易误判成代码回归。跑前 `ulimit -n 10240`（同一 shell 内有效）即 1711 全绿；CI runner 无此限制。
8. **tui 里"本机全绿、Linux CI 挂一片"的固定成因：测试断言了某一平台的行为，而被测代码读了当前 host**（#776 一次 12 个：粘贴断言 `osascript`/`pbpaste` 而代码读 `std::env::consts::OS`；sandbox 面板断言 macOS 默认 tier；`recording_clipboard` 继承 host 解析出的候选命令，Linux 无 X11/Wayland 时为空走了 OSC52；`skills_cli_for_host` 在 CI 上没有可解析的 `future` 而返回 None）。排查=在被测路径里 grep `std::env::consts::OS` / `SandboxPlatform::current()` / `Clipboard::new()`；修法是**构造上**与 host 无关（`#[cfg(test)]` 钉住平台、OS 作为参数传入、fixture 用合成候选），而不是在测试里再读一次 host 分支。

## agent 测试基建陷阱
- `tokio::time::pause`（start_paused）让 45s idle 超时瞬间到，但与真实网络不兼容（reqwest 自己的 timeout 会触发）→ 用 `FUTURE_TEST_STREAM_IDLE_SECS`/`FUTURE_TEST_TOOL_IDLE_SECS` cfg(test) 缝。
- 真 `~/.future/agent/auth.json` 会漏进测试（如 resolve_future_base_url_returns_default，base_url 为 http:// 时挂）→ 套件跑前 `HOME=$PWD/target/test-home`；测试内用 `crate::test_support::TestHome`（锚定 target/test-homes，编译期 CARGO_MANIFEST_DIR）。
- `HOME_ENV_LOCK` 是 std Mutex，panic 中毒会级联挂所有 `.lock().unwrap()` → 用 poison-tolerant helper 获取。
- tracing verbose arm 仅在有 subscriber 时求值 → `tracing::subscriber::set_default(fmt().with_writer(std::io::sink).finish())`。
- Mock HTTP server 必须循环 accept（一次性 server 负载下 ~50% flake）。
- ApprovalGate：decider 线程轮询 pending_for_session + decide；ask_user 用 block_in_place → 需 multi_thread runtime。
- 固定窗口 pump 在 instrumented（~4x 慢）下竞态 → 用 bounded pump_until_*（1200×25ms）。
- flake 排查：先抓 `^failures:` 段拿测试名；standalone 同 env 重跑复现。
- **并行 flake 簇的根因与诊断法**（2026-08-12 排查，三根因均已修于 PR #183）：
  ① 共享缓存文件撕裂读（truncate+write 窗口）：症状=load 返 None 但紧接其后 exists=true 且内容完好 → 写必须 tmp+rename 原子化（future.rs 已修，带回归测试 cache_save_and_concurrent_load_never_torn）；
  ② std Mutex 中毒级联：首个持锁 panic → 后续 `.lock().unwrap()` 全部 PoisonError。HOME_ENV_LOCK 一律用 poison-tolerant helper（approval/tools/session_prompt 已改）；
  ③ **macOS 时钟粒度 ~μs**（连续 time_ns 89% 同 tick）→ 测试 temp 路径用裸纳秒戳会跨测试碰撞（一个创建目录、另一个假设它不存在）。共享命名空间的生成器必须加原子序号（test_support::unique_temp_path）。
  诊断法：失败集合每轮不同且含 PoisonError → 找出**首个** panic（primary），其余是受害者；primary 的机制可以从"断言时刻与事后状态不一致"反推。
- **/tmp 下 checkout 跑套件会翻转 sandbox 断言**（环境伪影，不改代码）：TestHome 锚定 workspace 下 target/test-homes，workspace 若在系统 temp 里，HOME 就落进 sandbox 的 temp_roots 放行区 → outside-workspace 断言 Allow/Ask 反转。CI 与正常路径不触发；在 /tmp 临时 worktree 验 flake 时别用这个假象当根因。
- **`packages/rpc` 的 local endpoint 租约测试是 CI-only flake（2026-10-16 量化，未修）**：`cargo test -p future-rpc --lib` 偶发挂 `transport::tests::unpolled_local_listener_releases_its_endpoint_on_drop`（transport.rs:694）或 `transport::unix_endpoint::tests::lease_excludes_other_binders_and_allows_clean_restart`（unix_endpoint.rs:251）或 `..._stale_socket_is_reclaimed`；失败信息统一是 `Custom { kind: AddrInUse, error: "local endpoint ownership is occupied: <tempdir>/agent.sock" }`。机制：`LocalEndpointReservation` 用 `flock(LOCK_EX|LOCK_NB)` 占租约文件（unix_endpoint.rs:63），失败的那次 acquire 说明前一个租约的 FD 还没释放——每个用例各有 tempdir，所以不是路径碰撞而是**租约 FD 的释放/持有竞态**。判据：连跑 30 次 `cargo test -p future-rpc --lib`，本分支 9/30 挂、干净 `origin/main`（另一个 worktree，非 /tmp）**16/30** 挂 ⇒ 与改动无关、甚至 base 更糟。症状会出现在 `Rust tests (Linux) (channels, rpc, goal)` job，`gh run rerun --failed` 即转绿。
- **断言「两次构建的 system prompt 逐字节相同」的测试必须自己钉住 HOME**（2026-10-16 定位）：prompt 里嵌 <available_skills>，其内容由 `discover_skills_cached(global_skill_dirs())` 决定——读 `$HOME` 展开 `~`，而缓存是**进程级、60s TTL**。同二进制里 `skills_tests` 等用例用 `TestHome` 把 HOME 指到装了假技能（`demo`）的临时目录并持有 `HOME_ENV_LOCK`，于是被夹在两次 run 之间的那次缓存刷新会让 run1/run2 拿到不同技能集。实测：`session_prompt/tests.rs::renaming_does_not_change_conversation_system_prompt` 在加了几条会构建 prompt 的新测试后，全量 lib 跑到 ~50% 挂（改动的 src 本身不新增任何 discovery 调用），修法是该测试自己 `let _home = TestHome::new();`——锁是 Mutex，持有期间别人改不了 HOME——之后 7/7 绿。判据：diff 两次 prompt，差异只有 `# Available Skills` 段、且 location 指向另一个 worktree 的 target/test-homes ⇒ 就是这个坑，不要去怀疑被测逻辑。反面：给**只是**构建 prompt 的测试加 TestHome 反而有害（把「异地 HOME 窗口」拉长到整个用例），实测让上面那条从 ~50% 变成 4/4 必挂。

- **别在 lib 测试里用 thread-local subscriber 抓日志断言**（2026-10-16 实测，自己写出来的 flake）：`tracing::subscriber::with_default(fmt().with_writer(capture))` 抓「某段代码记了什么日志」时，**并行就漏行**，`--test-threads=1` 全绿 —— 因为 `with_default` **不重建 tracing 的进程级 callsite interest 缓存**：同一批 callsite 若先被同二进制的其它并行测试（或全局 subscriber）判成 `never` 并缓存，你这条线程上的事件会被直接跳过，漏掉的正是你断言的那几行。
  - 判别：失败信息里「日志只截到某一行就没了」；逐个 `--skip <兄弟测试>` 都还是挂（说明不是某个特定邻居）；`--test-threads=1` 必过 ⇒ 就是这个坑，别去怀疑被测的日志调用。
  - 修法：把断言搬到**独立进程**里 —— `agent/tests/service_startup.rs` 的 fixture 形态（`run_case("logging")` 让子进程自己 `logfile::init_embedded_tracing()`，父进程 grep 子进程 stderr）。子进程没有「别人的 subscriber」，天然确定性；顺带真的覆盖了 host 装层这件事。
  - 连带：子进程 stderr 的 fmt 层**带 ANSI**（字段渲染成 `session\x1b[0m\x1b[2m=\x1b[0mlegacy`），断言前先 `future_agent::logfile::strip_ansi(...)`（它同时就是日志文件里的文本），否则 `contains("session=legacy")` 永远假。

## 2026-08-26 第二轮 flake 猎杀（PR #377，flaky-stress workflow 已入仓）
- **猎杀工具**：`.github/workflows/flaky-stress.yml`（workflow_dispatch；一次 run = agent 套件 2× + tui/cli 套件 2×，无 concurrency 组，连续推 N 个 commit 即 N 个并行 run）。`gh workflow run flaky-stress.yml --ref <branch>`；注意 workflow 文件必须在 default branch 上才能 dispatch——临时分支上的探索版用 `on: push` 触发。
- **三个已修 flake 的机制**（都是"并行测试共享一个进程级单槽"变体）：
  ① `SCHEDULED_DEQUEUE_HOOK` 是单个 `Option` 槽：两个 hook 测试并行时后 arm 的覆盖前一个 → 另一个测试的 hook 不触发、`result.is_err()` 偶发 false（本地 2/5 复现）。修：改成按 run_id 的 `HashMap`（每测试用自己的 key，消费时 remove）。
  ② future models cache 原子写用固定 `models.tmp`：两个并发写者（background refresh 线程在别的测试的 $HOME 重定向下解析路径）写同一 tmp 文件 → write/rename 交错可在最终路径留下撕裂文件（症状：sync 成功后 `load_future_models_cache()` 返 None，future.rs:1115 panic）。修：tmp 名加 pid+原子序号。
  ③ 上述 panic 时测试持有 TestHome（= HOME_ENV_LOCK）→ Mutex 中毒 → 后续 `.lock().unwrap()` 全 PoisonError 级联（approval.rs×2、tools/mod.rs×1 是漏网未用 tolerant helper 的）。修：统一 `test_support::home_env_lock()`。
- **诊断法**：同一 job 里多个失败含 PoisonError 时，PoisonError 都是受害者；找首个 panic（primary），其余是级联。primary 若持 std::sync::Mutex，持锁 panic 即中毒源。

## desktop agent_bridge 测试基建（cov100 分区）
- `TestHome` 持有进程级 `TEST_HOME_LOCK`、`MockAgentGuard` 持有 `MOCK_LOCK`，**同一测试内不得同时构造两个 fixture**（同线程自死锁，症状=全套件级联挂）→ 第二个 fixture 前先 `drop(fixture)`。
- `observer::spawn_observer` 用 `tauri::async_runtime::spawn` → 观察者在测试 runtime 销毁后仍存活（zombie），持续轮询共享 mock server；脚本队列被其偷答会导致后续测试挂 → get_state 带 run_id 的回复用 `push_run_state` 独立队列。
- mock 未脚本化的 stream = `stream::pending()` 永久挂；collector 对"提前关闭的流"会重连最多 6 次（STREAM_RECONNECT_ATTEMPTS）→ 测试脚本的流若不含 terminal agent_end，必须为重连续推脚本，否则挂死在重连的 pending 流上。
- 事件 run_id 不匹配 canonical 会直接 Err（stream.rs ~229）→ pipeline 生成 run id 的测试用 `@attach` 占位符（mock 绑定到 attach 请求的 run_id）。
- `rename_thread` 拒绝空 title，但 `create_thread` 不校验 → 空 title 行用 create_thread(Some("")) 造。
- chat 线程的临时 workspace 只有 `create_thread(mode:"chat")` 才建；`update_chat_workspace_path` 只更新 kind='temporary' 且 path=chat_workspace_path(thread_id) 的行。
- 诊断挂死：libtest 的 `tail` 会吞输出 → 直接跑 binary 落日志；`sample <pid>` 抓栈（lldb attach 会挂）；先 --test-threads=1 找首个挂的测试。

## mobile jest 的“永不退出”：泄漏的定时器把 CI 挂满而不是报错（2026-09-26，实测一小时的坑）
- 症状：`Mobile typecheck + lint + test` 长期 `in_progress`（实测 >100 分钟；该 job 正常 **2–2.5 分钟**）。**测试本身全绿**，没有失败可看。
- 机制：`mobile/package.json` 的 `test` 是 **`jest --runInBand`**。单进程下，任何测试遗留的句柄都会让 jest 在跑完后打印 `Jest did not exit one second after the test run has completed` 然后**一直等**（它不会强制退出）。典型来源是 `SyncEngine` 的 lane 重试定时器：一次 gap 判定就 `scheduleRetry` 挂一个 `setTimeout`。
- 判据（本地即可复现，别去翻 CI 日志——进行中的 job 取不到日志，`gh run view --log` 会报 not available）：在 worktree 里跑 `npx jest --runInBand`，**看进程是否自己退出**：`timeout 240 npx jest --runInBand; echo $?` —— `124` = 挂住。
- 修法：新写的 sync 测试在 `finally { h.engine.clear(); }`（`SyncEngine.clear()` 会 clear `lane.retryTimer` 并 bump generation，使 `isCurrent` 变假）；同文件已有的 timer 类测试都这么做。
- **两个连带坑**：① 只跑**单个**测试文件时常常看不出来（该文件自己不泄漏就退出），必须跑**整套**才复现；② `git checkout HEAD -- <dir>` 会**一并丢掉未提交的修复**（我因此把已写好的 `clear()` 修复冲掉，白查一小时），改动先提交或先备份再回退。
- 反面：基线（`origin/main`）在本机跑整套同样打印该警告，但**会**退出；所以“打印警告”不是判据，**“进程是否退出”**才是。

## mobile jest 不执行 TDZ：use-before-define 只在真 bundle 里炸（2026-09-24）
- 症状：代码写在它引用的 `const` 声明之前（典型：`useEffect(..., [sessionId, sync])` 被插到 `const sync = useCallback(...)` 之前）→ **jest 全绿**；真 Metro bundle（截图 harness、真机 dev）打开那个界面时抛 `Cannot access 'sync' before initialization`，整屏白屏。别先怀疑 mock 数据或 Metro 缓存。
- 机制：babel-jest 把块级声明降级成 `var`（实测：在 jest 里读未初始化的 `const` 得到 `[null]` 而不是 ReferenceError），TDZ 语义丢失；Metro 产物保留 `const`，于是真的 ReferenceError。
- 拦截点只有两个，都要跑：`npx tsc --noEmit`（TS2448 "used before its declaration" / TS2454，最便宜）；真 bundle（`capture-mobile <scenario>` 或真机）。jest 抓不到这一类。
- 判据：白屏截图 + `serve-mobile` 日志里的 `An error occurred in the <ChatScreen> component` 及紧跟的 ReferenceError 行；cdp 侧只报 `eval error` / 目标找不到，看不出根因。

## desktop backend 套件的 download/transfer 家族在 CI 上也会偶发（2026-10-15 #1094 实证）
- 症状：`Rust tests (Linux) (desktop backend)` 只挂一个 `commands::remote_peer::tests::a_download_reaches_the_chosen_destination`，panic 是 `download: Message("peer_not_connected")`，同测试 stdout 里先有一行 `remote: network failure recovered [NW001] after 1 attempts and 2410ms`。
- 机制：chunk pull 在测试下只有 **500ms** 预算（`remote_peer/runtime.rs::chunk_pull_timeout`，真值是 15s），runner 负载高时被当成 transport failure → 丢连接 → 重试就报 `peer_not_connected`。**与本 PR 无关的判据**：失败点在 `remote_peer`/transfer 家族，而 diff 不碰这些文件；本机同族失败同样是"单跑绿、整套偶发"（见上面 src-tauri 那条）。
- 应对：`gh run rerun <run-id> --failed`，或推一个 merge main 的新提交让整套重跑，即绿（#1094 第二次 run 绿，desktop backend 15m38s）。别为了它改断言或动 500ms 预算。

## desktop backend 套件的 `database is locked`（CI-only flake，未修）
- 症状：`Rust tests (Linux) (desktop backend)` 里 1 个测试挂（如 `agent_bridge::headless::tests::run_prepared_prompt_propagates_an_agent_error`），panic 在 `agent_bridge/test_support.rs` 的 `initialize_app_store().expect(...)`，消息 `SqliteFailure(DatabaseBusy, "database is locked")`；同 job 其余 1279 个测试全绿。
- 判别：失败点是 **TestHome 构造**（共享 HOME/SQLite 迁移），不是被测逻辑；与本 PR 改的文件无关（改 agent_providers 也会挂 headless 的测试）。
- 应对：`gh run rerun <run-id> --failed` 必转绿（2026-09-19 #726 实测）。

## `cargo test --workspace` 的 safari 就绪窗口 flake（全量负载下才现，未修）
- 症状：`cli::browser::safari::safari_manager::start_launch_success_against_fake_driver` 全量跑时挂，单跑 0.52s 秒过。
- 根因：测试给 python 假 driver 留 10s 就绪窗口，全量负载下解释器 page-in 被挤爆 → 窗口不够。
- 判据与应对：先 `grep '^failures:'` 拿名字 → **单跑复现** → 与本波改动无关则**重跑全量确认**，别当回归去改代码。

## agent/tests/cli_smoke.rs 的 profile 计时器竞态（已知 flake，未修）
- 症状：CI `Rust tests (Linux) (agent)` 挂 `agent_default_ipc_shuts_down_gracefully_and_releases_its_endpoint`（`cli_smoke.rs:258`），断言是 `the default transport must be local IPC`；同 job 其余 24 个测试全绿，`Rust tests (Linux)` 聚合门也连带 fail（它只汇总各分组结果）。
- 根因：该测试用 `--profile-seconds 0` 让计时器立刻触发，而它要断言日志里有 `gRPC server listening on local IPC`；CI runner 负载高时**计时器先于 bind 的日志行胜出**（失败日志里能看到完整的优雅关闭序列，唯独缺 bind 那一行），于是断言失败。
- 判据：本地与改动无关且不复现 —— 本次 20/20 单独重跑全绿、整文件 5/5 全绿（改的只是 `agent/src/prompt/mod.rs` 的一句 guideline，碰不到 listen 顺序）。
- 应对：`gh run rerun <run-id> --failed` 一次即转绿（2026-10-07 #911 实测）。注意本地跑 agent 测试别只跑 `--lib`：这条在 `--test cli_smoke`（集成测试）里，`cargo test -p future-agent --lib` 覆盖不到，而 CI 的 agent 分组跑的是完整 crate。
- 根治方向：测试不要依赖「bind 日志先于计时器」，例如等到日志出现再让计时器到期，或把 `--profile-seconds` 调成足以完成 bind 的值。
- 同族（2026-10-15 #1095 又撞一次）：同文件同机制的 `agent_starts_serves_and_shuts_down_via_profile_timer`（`cli_smoke.rs:345`，失败输出就一行 `agent exited with Some(1)`）也会偶发挂，同样 `--failed` 重跑即绿。

## `a_transport_error_is_classified_by_the_kind_that_caused_it` 的 loopback 握手竞态（CI-only flake，未修）
- 症状：CI `Rust tests (Linux) (agent)` 挂 `llm::tests::a_transport_error_is_classified_by_the_kind_that_caused_it`（`agent/src/llm/mod.rs:1057`），断言是 `the catch-all arm needs an error outside every named kind`，失败输出里的错误为 `reqwest::Error { kind: Request, url: "http://127.0.0.1:<port>/", source: … hyper::Error(Io, Os { code: 104, kind: ConnectionReset }) }`；同 job 其余测试全绿。
- 根因：该测试为覆盖分类器的 catch-all 分支，在本机起一个回 `302 Location` 的 socket，靠 reqwest 的重定向策略造一个「不属于任何具名 kind」的错误；CI runner 负载高时那次握手被 reset → 错误落进 `is_request()` → 期望的 `unknown` 变成 `request`。属环境相关，分类逻辑没错。
- 判据：`git diff origin/main...HEAD -- agent/src/llm/mod.rs` 为空；干净 base 上同命令同样会偶发挂；直接覆盖本改动的分组（如 `Rust tests (Linux) (tui, cli)`）已 pass。
- 应对：`gh run rerun <run-id> --failed` 即转绿（2026-10-15 #1095 实测）。**不要**为了让门禁变绿去改断言。
- 根治方向：别依赖「恰好能拿到落在所有具名 kind 之外的那个错误」——用构造出的错误值喂分类器，或把 socket 侧做成确定性的（读完请求再按序应答、必要时重试握手）。

## cli/tests/bin.rs 的 agent 单例锁竞态（已知 flake，未修）
- 症状：CI `Rust tests (Linux) (tui, cli)` 挂在 `embedded_agent_run_failure_is_exit_1`，断言消息只有 `stdout: `（空），退出码却是期望的 1；同 job 里 tui 全绿，main 与其他 PR 同一 job 又是绿的（曾让人误判为"本 PR 引入"，PR #348 连挂 2 次后确认无关）。
- 根因：`run_from_args` 在 `run(cli)`（tracing init 所在）**之前** `acquire_agent_instance_lock()?` —— 用户级锁 `~/.future/agent/agent-instance.lock`。bin.rs 的 11 个测试并行跑，`embedded_agent_logfile_failure_returns_err` 已持锁（它在 run() 内部的 logfile::init 才失败）→ run_failure 那个进程拿不到锁，早于 tracing init 返回 Err → stderr 只有 `Error: Future Agent is already running for this user`，stdout 空。
- 判别法：看 job log 里两个测试的完成顺序 —— run_failure **先**完成（<0.1s）= 输了锁竞态；正常应是它最后完成（~0.4s，真的走完启动再 bind 失败）。
- 应对：`gh run rerun <id> --failed` 重跑通常就绿（错开时序即可）。根治方向：给 bin.rs 里每个 spawn 的 agent 单独 HOME（temp dir，Windows 还要 USERPROFILE），顺带避免污染真实 `~/.future`。
- 本地不复现：`HOME=$(mktemp -d) target/debug/future agent --grpc-addr 127.0.0.1:1` 单跑一定绿（stdout 有 `Agent exited with error: transport error`）。

## 改 proto 结构体：`cargo check -p <crate>` 不够，要让 test code 也编（2026-10-02，两次踩）

往 `packages/rpc/proto/future.proto` 加字段后，prost 会生成**字段齐全的结构体**，而代码里
有几处是**穷举式字面量构造**（不能用 `..Default::default()`）：

- `agent/src/grpc/mod.rs`（proto → 内部命令的映射）
- `desktop/src-tauri/src/agent_bridge/client.rs` 的 `base_command()`（agent 之外唯一
  逐字段枚举 `proto::RpcCommand` 的地方；desktop 不在 workspace 里，要
  `make desktop-sidecar-placeholder` 后在 `desktop/src-tauri` 里单独 check）
- `tui/src/rpc/grpc_client.rs` 的**测试**里（`AgentInfo { .. }`）

第三处是坑：`cargo check -p future-tui` **不编测试代码**，本地全绿而 CI 的
`Rust tests (Linux) (tui, cli)` 挂在 E0063。`clippy --all-targets` 能抓到（CLAUDE.md
已写），但**用 `check` 图快时同样要加 `--all-targets`**；换成 proto 结构体后建议直接
`cargo check -p <crate> --all-targets`。枚举全部站点：
`rg -n '<StructName> \{' --glob '*.rs'`（含 test 模块），或 `rg -n '<field_name>:' | grep -v target`。

**同一类坑不限 proto（2026-10-07 实测，tasks UI 那次）**：给 `orchestration/tasks` 的
`Task`/`TaskRun` 加字段后，我按 CLAUDE.md「只测改动的模块」跑了 crate 与 desktop backend
两套检查，**漏了 `cli/src/commands/task.rs`**（它也是 `future_tasks::Task` 的直接消费者，
有 struct 字面量），CI 里 `Rust build (Linux)/(macOS)` 直接 E0063 红。
`cargo check -p <改动的 crate>` 看不到别的 crate；跨 crate 公共类型变形时，
**`cargo check --workspace --all-targets` 才是能抓到的那条**（本机约 1 分钟，
比等一轮 CI 便宜）。新增字段还要顺带想一遍：CLI 有没有表达这个新概念的口子——
这次没有，于是"加个默认值让它编过"会把功能留成半接线，正确做法是补 flag+help+测试。

## cli lib 的 CI-only flake：tools_call_input_mask_base64_and_file_type_lowercasing（2026-09-16，未修）
- 症状：CI 的 `Rust tests (Linux) (tui, cli)`（命令是 `cargo test -p future-tui -p future-cli`）挂在 `cli/src/commands/tools.rs:2900` 的 `.expect("call")`，panic 内容为 `call: "\0handled-exit"` —— 即 CLI 返回了 `HANDLED_EXIT` 哨兵（`cli/src/lib.rs:31`，命令已自行处理错误并退出），而不是把记录到的请求 JSON 交给断言。
- 判定证据（避免误判成自己引入的回归）：同一 cli 代码在别处能过（#650 分支该 job pass），本地用与 CI 完全相同的命令全绿（cli `724 passed`），`gh run rerun --failed` 一次即转绿。触发条件疑似与并行压力/机器相关（`mcp_env` 写共享 auth 文件、`lock_env()` 串行），未定位到确定性根因。

## mobile `useSkillRecommendation.test.ts` 的"本地午夜"flake（未修）
- 症状：本地 `cd mobile && npx jest` 报 1 failed，`skips a skill already shown today instead of offering another` 挂在 `expect(await evaluate(api, DRAFT)).toBe(false)`（收到 true），其余 127 suites 全绿；CI（UTC）同代码必绿。
- 根因：**本地时间在 00:00–08:00（东八区）之间**时，测试用 `new Date().toISOString().slice(0,10)` 造"今天"的存储键（UTC 日期），而 hook 按**本地日期**读写同一个键 → 对不上 → 没被跳过 → 断言挂。属于测试自己依赖 host 时区，不是被测逻辑。
- 判别：`git stash` 后单跑该 suite 仍挂 = 与本次改动无关；过了本地 08:00（本地与 UTC 同日）自动消失。
- 正确修法（未做）：测试改用与 hook 相同的"本地日"函数，或注入时钟。参考同一文件里的 CI-only flake 条目（判据永远是"干净树上是否同样挂"）。

## loop / CLI 测试
- Mock FutureAgent gRPC server 在 orchestration/loop/tests/common/（脚本化 fail/hang/mid-stream-error）；经 FUTURE_LOOP_AGENT_ADDR 指向。
- 子进程驱动的覆盖率只在干净退出时落地，kill 会丢（serve-status 用 in-process leaked thread 代替）。
- `todo update --help` 调 process::exit —— 不要在 in-process 测试里调。
- WS 测试用 `#[tokio::test(flavor="current_thread")]`；close 握手测试需 server 延迟回 close（subscribe 竞态）。
- **src-tauri 双 mock 冲突**：commands 侧自建 server 与既有的 `remote/test_support.rs::MockAgent`
  都 `set_var` 同一个 `FUTURE_AGENT_GRPC_ADDR`，并抢 `agent_bridge/client.rs` 的 `AGENT_CHANNEL`
  OnceCell 缓存 → 谁先 init 谁赢，另一家测试全挂（单跑各自全绿，很难归因）。
  **解法：只留一个 mock server**，另一侧改为它的薄适配层，不要造第二个。

## CLI 测试碰桌面端 store 必须隔离 HOME（2026-10-09）
- `future workspace …`、`future desktop settings …`、以及 **`future session set <id> --cwd`** 读写的是**真实** `~/.future/app/app.db`（桌面端自己的库，**不看 FUTURE_HOME**）；`session set --cwd` 还会按 cwd 建/复用工作区行。
- 所以这些命令的 CLI 测试若不设 HOME，就会读——并在本机桌面端导入过该会话时**写**——开发者真实的工作区列表。实测：`session set --cwd` 的测试在没隔离 HOME 时读到真库里的 thread，输出多出一行 `workspace: …` 而挂。
- 修法：测试里 `std::env::set_var("HOME", tempdir)`（session.rs 的 `mock_env` 里已加 `isolate_home()`，用 thread_local 持有 TempDir）。判据：`future_app_settings::app_db_path()` 必须落在临时目录。
- 相关：库由 CLI 自己创建时只有 `workspaces` 表、没有 `threads` 表（app 还没跑过）——那是"还没有会话"不是坏库，`future_app_workspaces::find_thread_by_agent_session` 用 `threads_table_exists` 先探一下。

## desktop(src-tauri) 整套 lib 测试在本机 macOS 会死锁 + 2 个 host 相关的确定失败（2026-10-07 确认，均与改动无关）
- **默认并行会挂**：`cd desktop/src-tauri && cargo test --lib` 在本机跑到约 75–80 个用例后**停住不动**（0% CPU），`remote_host::pairing::http_tests` / `remote_host::files::flow_tests` / `remote::commands::bridge_tests` 报 "has been running for over 60 seconds"，之后不再前进。**单跑这些模块或整个模块都秒过**（`remote_host` 单模块 138 全绿 3.4s），所以不是代码 bug 而是并行下的跨模块互锁。
- **判据（别误判成自己引入的回归）**：在干净 base（`git worktree add --detach /tmp/x <base-sha>`）上跑同一条命令，卡在同一个位置（75 passed）；提 `ulimit -n 10240` 无效。
- **本机能用的跑法**：`cargo test --lib -- --test-threads=1`（约 165s，1532 passed / 9 ignored / 2 failed）。
- **那 2 个确定失败也是 host 相关、且在干净 base 上同样失败**：`remote::commands::bridge_tests::the_approval_tier_reflects_what_the_desktop_can_enforce`（期望 downgrade 成 `manual`，实得 `sandbox`）与 `desktop_settings_refuse_to_guess_sandbox_availability`（期望 false，实得 true）——被测路径打了脚本化的 `probe_sandbox` mock，但本机真实探测结果漏了进来。CI（Linux）不触发。
- **补充（2026-10-13 实测）：失败集不是固定那 2 个，会随 run 变，且常落在 transfer/download 家族**。同一提交连跑两次：一次是 `sandbox_availability` + 4 个 `remote_peer::transfer_tests::*`；另一次是 `sandbox_availability` + 3 个 `commands::remote_peer::tests::*download*`；两次都没出现 `the_approval_tier_reflects_what_the_desktop_can_enforce`。**判据（照上面那条的写法）**：在干净 base（`git worktree add --detach /tmp/x 9e0b0ae1` 之类）上跑**同一条** `cargo test -- --test-threads=1`，它也会挂 5 个、且是同族的姐妹测试 ⇒ 与本 PR 无关。再单跑：单个测试、或把整个 transfer/download 家族（8 个）一起跑，都是全绿。
  ⇒ **别拿“固定 2 个”当清单**：看到 transfer/download 家族的失败先去干净 base 上复现，不要当成自己引入的回归。（上面 `src-tauri 双 mock 冲突`那条的“全量 12 挂 / commands:: 过滤 7 挂 / 单跑各自全绿”是同一机制的不同实例。）
- 结论：desktop Rust 侧的"本地全绿"不是可达状态；**要拿有效信号就用 `--test-threads=1`，并把上述 2 个失败按已知失败排除**，别去改它们。
- **根因（2026-10-14 定位，上面"跨模块互锁"就是它）**：两把进程级测试锁**加锁顺序相反**，并行时互相等死。
  `remote_host/business/settings.rs:381-382` 先 `HomeGuard::new` 再 `mock_agent_lock`（HOME→MOCK）；
  `remote::commands::bridge_tests` 的 `active_bridge`（`commands.rs:2193-2194`）是**已被测试持有的** `mock_agent_lock` 里再拿 `HomeGuard`（MOCK→HOME）。
  证据=`sample <pid>` 抓栈：一侧停在 `mock_agent_lock` 等锁，另一侧停在 `HomeGuard::new` 等锁；全部测试线程 0% CPU。
  **最短复现（不含任何新代码，2 分钟内可判）**：`cargo test --lib -- remote_host::business::settings::tests remote::commands::bridge_tests` —— 必挂。
  修法=把 `settings.rs` 那处改成与 bridge_tests 同序（先 MOCK 后 HOME；`TestHome`/`MockAgentGuard` 同一测试内不得嵌套的规则见下）。**尚未修**：属独立问题，别混进别的 PR。

## worktree 里跑 mobile jest：`jest.spyOn` 假失败与 node_modules 布局（2026-10-07 再确认）
- `mobile/src/components/__tests__/MarkdownText.test.ts`（"large file previews virtualize blocks"）在 worktree 里必挂，报 `expect(jest.fn()).toHaveBeenCalledWith(...)` + `Number of calls: 0`：**spy 根本没拦住**（模块命名空间绑定没被替换），不是被测代码错。
- **判据**：把 `mobile/src` 整份 checkout 成 `origin/main`（即本次改动全部消失）后仍报同样的 `Number of calls: 0` ⇒ 与本 PR 无关。主树（真实路径）跑同一文件 36/36 绿。
- 逐项软链 node_modules（而不是整目录软链）**也修不掉**——所以这属于"worktree 路径 + 软链依赖"的组合，别在这上面继续花时间；需要 mobile 真信号时在主树跑该 suite，或只跑本次改动的 suite（`jest src/features/settings src/remote`）。

## 平台契约的真相在另一个仓库：`~/future-server`（2026-10-12）
- 桌面/移动端的「远端控制」（pairing、NATS）**没有**服务端代码在本仓；权威实现是 `/Users/geilige/future-server`，远程控制的路由与响应结构在
  `platform/service/src/routes/remote.rs`。判断「服务端到底会不会返回某字段」去那里读，别猜、也别只信本仓的类型声明。
- 实测字段**不对称**（这是真 bug 的来源）：`/pair/code`（host 建码）与 `/auth/token`（刷新）返回 `nats_url` + `nats_ws_url`；
  但 `/pair/claim`（客户端领取）的 `ClaimPairResponse` **只有 `nats_ws_url`，没有 `nats_url`**。客户端角色要 TCP 端点，只能 claim 后立刻刷新一次
  （refresh 的前置条件 claim 已满足：`client_device_id`/`client_public_key`/`status='active'`）。
- **通用教训（比这条字段更值得记）**：**夹具比真实服务端宽容 ⇒ e2e 全绿、生产全挂**。桌面端远程客户端曾因「claim 必须有 nats_url」而**配不上任何机器**，
  而所有 e2e 测试都过，因为共享夹具 `remote_peer::testing::claim` 自己手写了服务端从不发送的 `nats_url`。
  **判据**：凡是断言 wire 形状的测试，要问「这个字段真实服务端会不会发」——去服务端仓库或抓一次真实请求核对；只对着自己的 mock/夹具断言等于自证。
  （同款问题见 `tui-agent-version-skew.md`：fixture 别手写 key。）

## 别让 GUI 二进制在测试里跑到 Tauri 插件阶段（会真的动到用户在跑的 App）（2026-10-15）

- 单实例插件的 socket 路径是 **`/tmp/<identifier>_si.sock`**（macOS 实现 `socket_path()`：identifier 取自 tauri.conf.json，这里是 `com_futureos_desktop_si.sock`），
  **与 `HOME` / `FUTURE_HOME` 无关**，Windows 则是按 identifier 命名的窗口类 + 互斥体。所以"隔离 HOME 的测试"并不能隔离它。
- 后果：任何**子进程测试**只要让 `futureos` 走到 `run_with()`（即 Tauri 的 `.build()`），它就会去通知那台机器上**真实在跑的** App；
  如果 argv 里带 `--re-pair`（含内部 `--re-pair-reply <path>`），那个真实 App 会**执行重配**：撤销用户真实配对、切掉已连上的设备。
- 判据/做法：给 `--re-pair` 写测试时，只测**到达 `run_with` 之前**的行为（参数解析、终端门卫、等待循环、reply 文件——都在 `launch.rs` 里纯函数化）；
  要确认 `run_with` 是否被走到，看有没有真的通知发生，而不是看退出码（通知成功时进程在插件 setup 里 `exit(0)`）。
- 排查现场：`ls -la /tmp/*_si.sock` + 连一下（`python3 -c "s=socket.socket(socket.AF_UNIX); s.connect('/tmp/com_futureos_desktop_si.sock')"`）
  就能知道本机有没有 App 在跑；有监听者时**不要**跑任何会进入 Tauri 的 GUI 子进程，也别指望在本机演练两进程交接。
- 现有 `desktop/src-tauri/tests/headless_cli.rs` 是这么写的：只用 `--help`/非法旗标（到不了 `run_with`），或者用 `--no-default-features --features headless` 那个二进制。

## 改 i18n 文案值 = 同时改一批测试（2026-10-15 实测，顺手抓到一个假绿）

改 locale 的 **value**（不动 key）时，桌面/手机有一批测试是**按渲染文本或 aria-label 查询**的，必须同步改，否则它们在 CI 里才红：

- 桌面：`buttonByText(container, "New task")`、`container.querySelector("button[aria-label='Delete task']")`、`expect(text).toContain("No tasks yet")`；`layout.json` 的 `activityRail.*` 既是可见 label **又是** `IconButton` 的 aria-label，所以它同时是断言和选择器。
- 手机：`TasksSettingsPage.test.ts` 里成片的 `t("tasks.…")` 断言与 `getByText`。
- 截图 harness：`scenarios.json` 的 `tap`/`tapText` 是**可见文本匹配**（`__shotMatch` 与 `tapText` 都要求精确或 startsWith），文案一改就必须跟着改，否则捕获直接失败（好在是 fail-loud）。

**机械替换的坑（本次真实漏掉一处）**：按出现次数断言的 `str.replace` **看不到转义引号形式** —— `button[aria-label=\"Tasks\"]`（JS 字符串里的 `\"`）里那个 `"Tasks"` 与普通 `"Tasks"` 计数不同也不匹配，于是"计数全对"却漏了它，直到 `vitest` 才暴露。收尾判据：写一个把 `"X"`、`'X'`、`\"X\"`、`\'X\'` **四种引用形式都扫一遍**的脚本（并排除 `useTasks`/`TaskView` 这类标识符），命中必须为 0；`grep "Tasks"` 会漏，不能当判据。

**顺带一个好用的比对法**（改名/改文案时替代 sed 的机械证明）：把 locale 的**键集合**改前改后各打一份（en/zh 都要），`diff` 必须为空 —— 只有 value 变、key 一个不多不少；再配合 `--help` 输出的改前改后 diff，diff 里只允许出现预期的那几行文案，**命令拼写与 flag 集合**单独再抽一次 token 集合 diff。

## fork 会复制描述符表 —— `close()` 不是 `flock` 的释放（2026-10-16，由 F-RPC 单元实测）

`future-rpc` 的本地端点租约竞态（干净基线上 3/12 = 25% 失败，`AddrInUse`）根因不是 flock 本身，
而是**所有权被 clone 的 fd 延长**：

- `fork()` 复制**描述符表**，父子共享同一个 open file description ⇒ **子进程持有它时，父进程
  `close(fd)` 并不会释放 `flock` 锁**，锁要到**最后一个**指向该 description 的描述符关闭/解锁才释放。
- 所以"我 close 了 fd" 不等于"我释放了租约"。正确做法是**显式 `flock(fd, LOCK_UN)`**（F-RPC 的修法：
  `LeaseFile` newtype，`Drop` 里先 `LOCK_UN` 再 close，由 reservation 与 cleanup 共同持有）。
- 复现要点：一个**会 fork** 的测试（spawn 子进程/命令）与一个断言"已释放的 listener 可被重新取用"的
  测试必须互相串行 —— 去掉那把 `fork_guard()` 就是 **30/100 失败**，它自己测试的 fork 窗口则是 **67/200**；
  带上则 0/1130。
- 症状长这样：`AddrInUse`，消息是 "local endpoint is active or cannot be classified as stale"，
  每个测试有**自己的** `tempfile::tempdir()`（所以**不是**路径碰撞），失败率随机器负载升高。

**量速率时另外两个坑**（同一单元报告，我实测踩过其一）：

1. 速率循环里看到 `timeout` **不要立刻当成 flake**：冷 registry 会让 `cargo test` 卡在
   `Updating crates.io index`（本机并发构建多时尤其），短超时下每次都是"超时"。判据：
   日志里有没有 `Compiling <crate>` / 有没有真的跑测试。我自己的 30 次里有 4 次是这个，
   不是那个竞态（真正判据是 `AddrInUse`/`panicked at` 计数为 0，且 201 个测试每次都绿）。
2. 用 `mv` 恢复过的文件会留下**陈旧二进制** ⇒ 速率要以日志里的 `Compiling <crate>` 行为准，
   否则你测的是旧构建。

## 有计数器也有"根本没有计数器"的行：两类实测的归因伪影（2026-10-16，AG-A 单元实测）

按行核对覆盖率时，下列两类行会读成 `DA:0`，但它们**不是"没测到的工作"**，而是宏展开/语法结构
造成的归因缺口。把它们当工作项去"关"，是白花力气，也是本 campaign 反复出现的错分类来源：

1. **`tracing` 的字段行**：同一条 event 里，字段行可以永远没有计数器 —— 实测
   `line 149 = 782`（事件真的触发了几百次）而同组的 `line 150 = 0`。判据：**同一 event 的兄弟行有
   大计数而它恒 0**，就记为 `attribution-artifact`，并要求在台账里写出兄弟行的计数。
2. **`if` 头部行与块尾 `}`**：块体执行上千次（实测 3354 次与 472 次）而头部/尾括号行恒 0。
   判据同上：**体有计数、结构行没有** ⇒ `attribution-artifact`，不是"不可达"也不是"缺夹具"。

两类都写进台账时必须附**同一次运行的邻居计数**（这是本仓豁免台账的既有要求），否则读者无法区分
"结构伪影"与"悄悄跳过的行"。

## `assert!` 的失败消息参数永远不可达（2026-10-16，AG-A 实测）

Rust 只在断言**失败**时求值 `assert!` 的 message 参数 ⇒ **绿套件永远到不了那些行**。它们看起来像
"缺夹具的可覆盖行"（台账里被记为 `coverable` 六行，配了不可能满足的配方），正确类别是
`unreachable-by-construction`。

判据（可直接 grep）：该行是 `assert!(<条件>, <这里>)` 中**第一个逗号之后**的表达式 ——
字面量、格式化参数或它们的续行。实测样本：`agent/src/rpc/session.rs` 3497 / 3614 / 4875 / 4912 /
5048 / 5327，其中 3614 与 4875 是 `"{}", value` 形式的格式化实参。

形态族里这是第五种（前四种：`tracing` 字段行、`if` 头部与块尾括号、宏展开区域、测试夹具体的
`panic!`）；它是**最容易被当成真实工作量**的一种，因为行本身是普通表达式、不是括号也不是宏。

## 一个"名字对、断言也过、但什么都没覆盖"的测试：夹具比应用契约更弱（2026-10-16，TAU-B 单元发现，supervisor 复核）

**形状**：测试名写着"import 的 panic 臂""import 的错误臂"，套件全绿，但目标行读 `DA:0`。看起来像"行不可达"，**实际是测试根本没走到那里**。

**机制（可复用的判据）**：该测试的夹具推的是一行**最小** `list_sessions` 行，而消费端 `list_agent_sessions` 走**类型化 payload**（`SessionSummaryPayload` 要求 `updatedAtMs`/`queryCount`/`isStreaming`），最小行**解码失败** ⇒ 会话列表为空 ⇒ 被测函数在**空列表守卫处提前返回**，import 循环一次都没跑。夹具"太弱"与"太宽松"是同一枚硬币的两面：本仓此前记录的陷阱是**夹具比应用更宽松**（允许应用产不出的形状，造出假绿）；这一例是**夹具比应用要求更少**（应用解不出的输入，让测试静默早退）。

**怎么发现（三步，便宜）**：
1. 一条**指名某臂**的测试存在且通过，而该臂的行读 `DA:0` —— 先怀疑测试，不要先记"不可达"。
2. **直接问被测的读取函数**：写一个探针断言 `list_agent_sessions()` 对这份夹具返回**非空**。它返回 0 就说明早退了（本次就是这样定案的：**断言而非阅读**）。
3. 对照：把夹具补成应用真正要求的形状（本例是加那三个字段），看目标行是否从 0 变成 >0。本例实测 `import.rs` 448/727/730/731/732：修前 **0/0/0/0/0**，修后 **2/4/2/2/2**。

**为什么值得单列**：这类测试**计入"已覆盖"却什么也没证明**，是"测试通过 ≠ 目标臂被走到"的最隐蔽形态——比 `attribution-artifact` 更危险，因为它看起来是有意为之的行为测试。凡是"某臂的行恒为 0，而它的测试一直是绿的"，都要按上面三步过一遍。

**相关**：`assert!` 失败消息参数永远不可达（另一类恒 0）；"被调用方 `disabled` 复制掉的守卫"（注释当豁免依据的假前提）。

## 一个测试泄漏进程级全局状态，会让 27 个别的不相干的测试失败（2026-10-16，TAU-B 单元发现，supervisor 复核）

**形状**：测试开头 `set_var("PATH", <夹具目录>)`，**在函数体最后一行**还原。任何更早的断言失败/panic 都会跳过还原 ⇒ **该进程内之后的所有测试**都继承被污染的 `PATH`，凡是 `spawn` 外部程序的都会以 `NotFound` 失败。

**实测影响**（不是推断）：一次插桩 gui 运行红了 31 个测试，第一个是这个测试本身，其余 **27 个**都死于 spawn `git` 的 `NotFound`；同一份代码在负载 3.12 下 **0 失败**。后果是**整套变红、那轮 lcov 作废**（段 6 的合并数字因此有两个 in-scope 文件需要附注）。

**判据（三步，便宜）**：
1. **一簇失败先怀疑"一个泄漏的全局"**：若失败集中在 `NotFound`/环境不对，那通常**是一个 bug 而不是三十个**。
2. 定位那个测试：搜 `set_var`/`remove_var`，看还原是不是**函数体最后一行**、有没有 RAII guard（`Drop`）或 `catch_unwind`。
3. **注意 `test_env::lock()` 不是还原**：`tui/src/lib.rs:82` 的 `ENV_LOCK` 是**互斥锁**（且 poison-safe，所以 panic 不会让别的测试因锁失败），但它**不还原环境变量的值**——锁定只是把竞态去掉了，泄漏仍在。

**规模（本轮实测，`.future/cov100/global-leak-scan.py`）**：**218 处**候选（desktop 70 / orchestration 38 / tui 27 / packages 6 / cli 4 / agent 2 / channels 1）。工具答的是**形状**问题（"改了全局且可能在最后还原前 panic"），**不是判决**——别的模块里的 helper 可能让它安全。已知正例：`desktop/src-tauri/src/cli_tool.rs:441`（泄漏 PATH，实测 27 级联）。

**正确修法**：把"设值 + 还原"收进一个 **RAII guard**（`Drop` 里还原），而不是散落在测试体里；这是本仓多数测试该用的形态。

## "没有调度能到达这里" 是一个**定量**断言，必须实测；而且测量要**偏向命中**（2026-10-17，AG-A 段 20 实测）

**形状**：从源码推理出一个窗口"在两次检查之间是同步的，所以只有另一个 OS 线程能撞到"，于是把行登记为不可达 / `unreachable-in-this-environment`。

**为什么这个推理会错**：那句话其实是**关于相对时长**的定量断言——"我的线程来不及，而它来得及"。而"来得及/来不及"取决于两侧各要做多少工作，以及线程什么时候开始盯着看。

**实测反例（本仓）**：`agent/src/rpc/service/mod.rs` 的 `server_running.store(true)`（:324）与 server 的首次 poll（:338 起）之间，`serve` 还要**订阅两个 watch 通道、构造 guard、创建并 pin 那个 future**（数百纳秒）；而一个**已经在自旋等待** `server_running` 的线程**数十纳秒**就能看到。结果：窗口 **40/40 命中**（再测 20/20），而此前的记录（F-21）说它不可调度。

**判据（两条，都要）**：
1. **别用源码推理代替测量**：写一个真的去撞那个窗口的夹具；若夹具无法构造，登记时说"我没有可构造的夹具"，**不要**说"没有调度能到达"。
2. **测量必须偏向命中**（AG-A 原话：`the measurement must be BIASED TOWARDS A HIT`）：给撞窗口的一方**先手**（自旋等待、提前就位）。否则**零次命中什么都证明不了**——它同样符合"窗口不存在"和"我的夹具太慢"两种解释。

**本仓对称的另一半**：反过来，一个"能命中"的测试也要能**排除**另一条解释。本例的做法值得抄：把 `StartupCancelled` 的**来源**限定住——入口检查（:301）在 `server_running` 被置位**之前**就已跑过，所以那里看到的 `StartupCancelled` **不可能**来自入口检查；再加"20 次里至少命中 1 次"的断言，使它不是一个抛硬币。

## 进程级全局：两个不同的坑，两种不同的修法（2026-10-17，tui 单元提出，supervisor 采纳并推广）

**不要把两件事混为一谈。它们形状相似、修法不同，混起来会修错：**

| | 破坏性泄漏（destructive leak） | 合法窗口（legitimate window） |
|---|---|---|
| 触发 | 测试改了进程级全局（`PATH`/`HOME`/`FUTURE_*`），**只在成功路径还原**；任何更早的断言失败/panic 让改动留给之后的每一个测试 | 测试**允许**在它运行期间持有那个改动（它自己在跑），但**别的测试**在这期间 spawn 外部程序/读环境 |
| 实测代价 | FLAKES 第 27 项：**一次泄漏导致 27 个不相干测试失败**，并使一次插桩运行作废 | FLAKES 第 24 项：并发 12 次里 **3 次红**，每次 137–147 s（绿时 25–30 s） |
| 修法 | **RAII**（`Drop` 里还原；`EnvVarGuard` 形状）。**互斥锁不是还原**——锁只挡住竞态，不还原值 | **钉绝对路径**（`absolute_tool(name)`），或让消费者在**整个窗口**持 `test_env::lock()` |

**tui 单元的原话值得背下来**：*"Pinning cannot fix a leak; RAII cannot fix a window."*

**判断某个站点要不要改，看的是爆炸半径，不是形状**：工具（`.future/cov100/global-leak-scan.py`）只能回答"它是否在最后一次还原之前改过全局"；**只有人能回答"一个不持锁的测试会不会被它打到"**。一个只在自己进程里跑的集成测试（自带一个二进制）不在此列——记下理由即可，不必为统一而转换。

**另附**：改完后要有**那一条**断言——"guard 在 panic 之后仍然还原了值"（本仓 `env_var_guard_restores_the_value_after_a_panic`）。没有这条，RAII 只是写法更漂亮。

## 一个"失败了"的夹具可能测的根本不是它命名的那条分支（2026-10-17，TAU-D3 单元发现，supervisor 复核并推广）

**现象**：TAU-D3 的"已撤销配对"臂第一次插桩运行**没有失败，而是挂了 300 s**。表面看像一个超时 flake；实际是**这个测试从未走进它命名的分支**。

**机制（macOS，实测）**：从一个**非阻塞** listener `accept()` 出来的 socket **继承 `O_NONBLOCK`**。夹具 `read()` 于是立刻拿到 **`EAGAIN`**，它把这当成连接失败丢掉、去等下一个；而**被测入口点正确地走了它自己的 "deferred"（稍后重试）路径**。于是断言等的是"撤销"路径的效应，而进程走的是"deferred"路径 —— **测试测的是另一条臂**。

**修法（两层，缺一不可）**：
1. **让传输按夹具的意图工作**：对 accept 出来的流显式 `set_nonblocking(false)`（本例 `tests/headless_cli.rs:398`），并**每连接一个线程** + 一个**记录并重试**的 accept 循环（否则一条停滞的连接会把后续连接一起饿死）。
2. **断言"实际发生了哪个可观测"**，而不是断言"我想让它发生的那件事"。本例的 listener 记录它**服务过哪些 endpoint**（`served: Arc<Mutex<Vec<String>>>`），于是"走进了错误的臂"会**响亮地失败**，而不是伪装成超时。

**为什么这类比空洞测试更危险**：空洞测试（夹具弱于应用契约）是**绿着什么都没覆盖**；这一类的形状是**红着/挂着，看起来像 flake 或超时**，于是它最可能的下场是被当成"负载问题"重新登记（甚至被"修"成更快失败，从而永远测不到目标臂）。**判据**：当一个夹具的失败表现为**传输层错误或纯超时**时，先问"它到底走进了哪条分支"，而不是先问"它为什么慢"。

**同族**：这与本仓已记的"夹具比应用契约更弱"（TAU-B）和"没有调度能到达这里必须实测"（AG-A 段 20）是同一条纪律的三个面：**一个关于"哪条分支在跑"的断言，必须由该分支独有的可观测来证明，不能由"测试没有报错"来证明。**

## `O_EXCL` + 悬空符号链接：**一个任何文件系统都能造出的状态**（2026-10-17，REV-GOAL 实测 + supervisor 在源码中复验）

**用途**：当一条"获取锁/创建文件"的循环里的**退出语句**被标成 `unreachable-in-this-environment` 时，先检查它的创建调用是不是 **`create_new(true)`（= `O_CREAT|O_EXCL`）**，以及读取侧是不是**跟随符号链接**。若两者都成立，**一个悬空符号链接**就是那个状态：

- `O_EXCL` 遇到**悬空符号链接**返回 **`EEXIST`**（POSIX：符号链接**自身**存在，而 `O_EXCL` 拒绝跟随它）→ 被"竞争"分支吞掉，循环继续；
- 而 `fs::read_to_string` **会跟随**那条链接 → 目标不存在 → **`NotFound`** → 被分类为"已释放" → `continue`；
- 于是循环**每一轮都空转**，直到撞上它自己的上界（本例 `MAX_LOCK_ATTEMPTS = 512`）**掉出循环**，执行那条"拿不到锁"的 `bail!`。

**实测（module-goal 的 `compat.rs:270`）**：夹具就是**一个悬空符号链接** ⇒ `DA:270` **0 → 1**，而 `DA:235`（`LockState::Released => continue`）**3 → 512**——**512 正好等于上界**，这个数字**佐证了机制**而不只是陪着它。该行原标 `unreachable-in-this-environment`。

**一般化**：`O_EXCL` 与"跟随链接的读取"**对同一个路径给出相反结论**（"存在"vs"不存在"），这是**一个真实可造的状态**，不是推演。所以**凡"创建/打开"与"读取/分类"用不同语义访问同一路径的循环**，都先问：**这两个语义会不会对同一个路径给出矛盾答案？** 若是，那条"循环耗尽"的退出语句通常**可达**，且夹具常常只有一行。

**同族**：这与"没有调度能到达这里必须实测"（AG-A 段 20）同一条纪律——**一句"本机造不出这个状态"是关于本机的定量断言，必须实测**；而本例的价值在于：**状态不但可造，还便宜得荒谬**（一个 `ln -s`）。

## 让 `commit()` 失败：**延迟外键（DEFERRED FK）**是唯一没有注入点时的夹具（2026-10-17，TAU-B 段 9 发现，supervisor 在源码中复验并推广）

**问题**：一条"**批量提交失败**"的臂（本例 `store/threads.rs:669-675`）看起来无法测——`tx.commit()` **没有任何注入点**：事务在函数**内部**创建、savepoint 早已提交，而 SQLite **在语句执行时就拒绝**大多数破坏（于是那条"语句级失败"的**另一条已经覆盖**的臂先接住了它，被测臂永远不执行）。

**夹具**：**schema 级的延迟外键**。

1. 建一张探针表，外键指向目标表并声明 **`DEFERRABLE INITIALLY DEFERRED`**；
2. 在目标表上建一个 **`AFTER DELETE` 触发器**，把被删的 id 记进探针表；
3. 于是 SQLite **在 `COMMIT` 时才检查**这条约束，而不是在语句执行时 ⇒ **每个 savepoint 都干净提交**，而违例**恰好**在 `COMMIT` 那一刻暴露 ⇒ **正好落在被测的那条臂上**。
4. 断言**批量契约**本身（本例：`deleted_count == 0`、两个 id 都在 `failed` 里、两行**仍在**——因为回滚把它们恢复了）。

**关键的反面**（原记录写明，值得照抄）：**`PRAGMA defer_foreign_keys` 在这里无效**——它是**每连接**的，而那个连接属于**另一个函数**（`delete_thread_trees`），你拿不到它。所以必须走 **schema** 而不是 pragma。

**推广**：**凡"创建、提交、回滚"在函数内部完成、因而没有注入点的臂**（commit / post-commit / 回滚后的补偿），都先问：**这个后端有没有"延迟到提交时才求值"的约束机制？** 若有（SQLite 的 deferred FK、PostgreSQL 的 `DEFERRABLE` 约束/触发器、`AFTER` 触发器等），夹具就能**在业务语句全部成功之后**才制造失败——这正是这类臂需要的时序。

## `ls -t /tmp/<pattern>*.log | head -1` **不是**"找我自己那份日志"的办法（2026-10-17，TAU-B 段 10 实测，supervisor 采纳）

**现象**：本 campaign 的多个单元共用同一套日志命名（`<prefix>-measure-<pid>.log`、`<prefix>-gate-<pid>.log`）。于是 **`ls -t` 的"最新"会在不同单元之间来回跳**——你刚写的那份**不是**最新的那份，因为**别的单元恰好在你之后写了一行**。

**后果**：TAU-B 段 10 第一次读测量日志时**读到了同业的那份**。它是怎么发现的——**那份日志里出现了它的脚本根本不会打印的步骤**（`GUI_RUN_EXIT`、`CLEAN_EXIT`）。**这是发现的关键**：两份日志都"像是"你的，只有**内容里的步骤名**能区分。

**规则**：
1. **把你写日志的确切路径钉住**（写成变量、或直接记在记录里），**永远不要 `ls -t` 去找它**。
2. 读日志时**先确认它是不是你的**：找一个"只有你的脚本会打印"的标记（本例的 `GUI_RUN_EXIT`）。这与本仓已记的"点名你检查的那个 token，并确保它能够失败"是同一条纪律的**另一个面**：**token 不但要能失败，还要能把你的产物和别人的产物区分开**。
3. **同理适用于 target 目录**：段 10 期间有同业在往 `target/cov-tauri-appdata` 跑第三次测量——这是"flavor 检查是强制的"的又一个理由（热的共享 target 目录会让合并报告静默丢掉一个 flavor，FLAKES 第 30 项）。

## 代码围栏里的 `#` 是注释，不是标题——但工具可能不知道（2026-10-17，由 agent-svc 单元发现，supervisor 复核并修好）

**陷阱**：文档里粘贴的示例输出常常以 `#` 开头（shell 注释），且**顶格**：

```
# -> files with uncovered lines=202
#    MISSING (uncovered, not mentioned): 0
```

如果某个工具用 `^#{1,N} ` 去找"下一个标题"来**切分章节**，那么这条**注释**会被当成标题，章节**在此处被截断**——它下面的台账行全部**看不见**，于是工具报出**幻影 MISSING**（实测：某个配对 13 条，某份模块记录 7 条；全campaign 30 份文档里有 283 处这样的行）。

**修法（工具侧）**：切分时必须**跟踪围栏**，围栏内的标题一律当作不透明——**找开头标题和找结束标题两头都要**（只修一头仍会从围栏里的示例标题开始切）。

**给写文档的人**：不必为了绕开这个坑去缩进你的示例输出（**顶格 `#` 是自然的写法**）；该修的是工具。但要**知道它存在**——并在发现工具报了"你从没写过的行"时，先怀疑是**围栏**而不是自己漏写。
