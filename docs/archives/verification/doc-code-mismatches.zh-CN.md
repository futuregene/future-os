# 文档 ↔ 代码不符清单

[English](doc-code-mismatches.md)

文档整理工作（todo `todo_9b5f2e830753`，会话
`20260916-113427-73c5a9`）对迁移后文档与**当前代码**逐篇核对后产出的清单。
本文件只产出清单，**不修改**任何被审计文档。

- **基线：** worktree `/Users/geilige/future-os/.worktrees/docs-reorg`，
  分支 `claude/docs-reorg`，HEAD `7c2611b5`（docs: bilingualize
  internals/architecture part 1）。审计期间有并行会话持续合入双语化提交，
  部分 `docs/internals/desktop/` 文件在审计中途被重命名
  （`CONNECTION.md` → `CONNECTION.zh-CN.md` 等）。修复时请重新解析
  `文件:行` 引用。
- **方法：** 以下每条均与源码核对，同时给出文档位置（`文件:行`）与代码
  证据（`文件:行`），不凭印象。无法确认的标【待核实】并说明缺少什么证据。
- **范围：** 命令与 flags、文件/目录路径、环境变量、配置键与默认值、
  行为与状态机描述、已删除或改名的功能、架构与数据模型断言、跨文档矛盾。
  历史档案（`docs/archives/`、`long-run-evidence-ledger.md`、带日期的验证
  记录）按整理规则保留其时间与 commit 边界，未当作"当前事实"重新审计。
- **分类：**【错误】文档与代码矛盾 · 【过时】文档描述过去状态 ·
  【缺失】文档遗漏对用户/运维可见的内容。

## 已核对无误（高风险区，无发现）

以下文档逐行与代码核对后**一致**：

- `docs/guide/tui.md` — 全部 19 个斜杠命令与 `tui/src/app.rs` 的
  `handle_submit` 一致（model/sessions/help/reload/compact/export/import/
  clone/fork/tree/new/name/scoped-models/cwd/approve/reject/stop/cancel/
  status）；帮助屏子集与 `tui/src/help_screen.rs` 一致；`future tui` 参数
  与 `tui/src/index.rs` 一致；设置键（`defaultModel`、
  `defaultThinkingLevel`、`defaultPermissionLevel`、`enabledModelIds`）
  与 `tui/src/app.rs:294-332` 一致；`PI_DEBUG_REDRAW=1` / `PI_TUI_WRITE_LOG=1`
  路径与 `app.rs:4048-4056`、`terminal.rs:460-464` 一致。
- `docs/guide/channels-config.md` — 全部 15+ 配置键与默认值与
  `channels/src/config.rs` 一致（agent/feishu/dingtalk 三个块）；9 个斜杠
  命令与两个桥一致；30s/20s keepalive 与 `feishu_ws.rs:78` /
  `dingtalk_ws.rs:32` 一致；每会话 128 事件缓冲与
  `feishu/bridge.rs:134` / `dingtalk/bridge.rs:64` 一致；`max_image_mb`
  无 Content-Length 时仍生效与 `feishu_rest.rs:769` 一致；首启写模板并
  退出与 `config.rs` 的 `load()` 一致。
- `docs/architecture/loop-control-plane.md` — 7 组 / 41 命令及各组计数与
  `orchestration/loop/src/console.rs` 的 `build_cli_registry`
  （5/6/6/18/3/2/1）一致；todo 类别（advancement/monitor/blocker/
  coordination/user_gate/user_action）与 `state.rs:54-79` 一致；`--parent`
  三层上限、`--gate-question`、`--role`、`--max-validation-attempts`、
  `--parent-session` 与 `console.rs:705-727, 1364` 一致；验证器 120s 默认 +
  `FUTURE_LOOP_VALIDATOR_TIMEOUT_SECS` 与 `validator.rs:7` /
  `executor.rs:209` 一致；面板端口 7717/127.0.0.1/仅 GET 与
  `console.rs:3175-3177`、`webui/server.rs:54` 一致；32 条批量上报与
  `agents/supervision.rs:13` 一致；5 分钟待验证标记与
  `agents/supervision.rs:214` 一致；3 turn 交付跟进与
  `work_items/delivery_outcome.rs:215-225` 一致；语义历史 N=50 与
  `decision/goal_frontier/semantic_history.rs:21` 一致。
- `docs/internals/desktop/` 沙箱与连接 — `SANDBOX/MACOS.*` 的断言
  （`deny default`、`(allow file-read*)`、`(allow network*) (allow
  system-socket)`、`/usr/bin/sandbox-exec` 存在性检查、"Operation not
  permitted"/"sandbox-exec" 文本启发式、auth.json 临时豁免）与
  `agent/src/sandbox/seatbelt.rs:89-102`、`sandbox/mod.rs:924, 1220` 一致；
  `CONNECTION.*` 的模块引用（`remote/services.rs`、`remote_host/`、
  `agent_events.rs`、`headless/`、`--no-qr`/`--re-pair`、
  `remote_pending_revokes.json`、支持码 PA001/LC003）与桌面源码一致；
  `REMOTE_E2EE.md` 的 Noise 模式与 `packages/remote-crypto/src/lib.rs:34-35`
  一致；`embedded-terminal.md` 的路由/TTL 与
  `desktop/src-tauri/src/terminal/server.rs:188`、`ticket.rs:20` 一致；
  `desktop-windows.md` 与 `windows/installer-hooks.nsh`（exit 32、`/UPDATE`、
  旧版 exe 名）一致。
- `docs/wiki/**` — CLI/Settings/Installation/Quick-Start/Sandbox/Skills/
  Feishu/Home/FAQ/Remote 各页：命令、默认值（审批模式 `off` =
  `desktop/src/integrations/storage/appSettings.ts:44`）、4 图/25 MiB 上限
  （`attachments.ts:11,19`）、Bubblewrap ≥ 0.9.0（`linux/probe.rs:351`）、
  诊断码（`sandbox/mod.rs:979`、`linuxSandboxStatus.ts`）、技能表（15 个
  内置技能）、`future init` 行为（`commands/init.rs`）、安装脚本
  （`scripts/install.sh`、`install.ps1`）。
- `docs/architecture/rpc.md`、`docs/internals/provider-protocol.md`、
  `docs/architecture/sqlite-migration.*`（flags、100ms/128条/64KiB 微批 =
  `agent/src/rpc/protocol.rs:227-229`、WAL/FULL =
  `session/database.rs:122-123`）、`docs/guide/build-and-install.md`（make
  目标、Rust 1.97.0、Node 24、mold）、`docs/guide/directory-layout.md`
  （所列路径全部存在；loop 目录结构全部出现在 `orchestration/loop/src/`）、
  `docs/guide/desktop-headless.md`、`docs/internals/desktop/ER.*`（表清单与
  `desktop/src-tauri/src/store/schema.rs` 一致）。

---

## 【错误】 文档与代码矛盾

### E1. "显式 TCP 先试 TCP 再回退 IPC"——代码中不存在该回退

- **文档：** `docs/guide/channels-config.md:62` — "An explicit
  `http://host:port` tries TCP first, then local IPC"；
  `docs/guide/channels-config.zh-CN.md:61`（"先尝试 TCP 再回退 IPC"）；
  `docs/guide/directory-layout.md:83` — "An explicit client TCP address is
  tried before local IPC"；`docs/guide/directory-layout.zh-CN.md:75`。
- **代码：** `packages/rpc/src/transport.rs:59-61` — "An explicit TCP
  endpoint is authoritative: a failed remote/development target must not
  silently redirect commands to an unrelated local Agent"；
  `packages/rpc/src/transport.rs:62-75` — `connection_plan()` 把任何非
  `auto` 值映射为仅 `vec![AgentEndpoint::Tcp(..)]`，计划中没有 `Local`
  条目，回退 IPC 不可能发生。
- **跨文档矛盾：** `docs/guide/tui.md:18` 的表述才是正确的："an explicit
  TCP target is authoritative and never falls back to local IPC"。
- **建议修法：** 两份 guide 文件改为"显式 `http://host:port` 具有权威性
  ——连接失败直接报错，不会重定向到本地 IPC"，中英文同步删除"先试 TCP"
  措辞。

### E2. `~/.future/tui/keybindings.json` 文档说有，代码从不读取

- **文档：** `docs/guide/tui.md:81-82` — "Optional user keybinding overrides
  can be placed at `~/.future/tui/keybindings.json`"；
  `docs/guide/tui.zh-CN.md:75`；`docs/guide/directory-layout.md:29` 与
  `:115`；`docs/guide/directory-layout.zh-CN.md:28,104`。
- **代码：** 机制存在但从未接线：`tui/src/keybindings.rs:102-103` 注释说
  `apply_overrides` 覆盖来自 `~/.future/tui/keybindings.json`，但**没有任何
  非测试调用方**——`KeybindingManager::new()` 以空覆盖表启动
  （`keybindings.rs:44-49`），应用在 `tui/src/app.rs:855-960` 注册按键时
  不加载任何文件，TUI 实际读取的唯一文件是 `~/.future/tui/settings.json`
  （`tui/src/index.rs:756-760`、`tui/src/app.rs:3360`）。全仓库 grep 显示
  `keybindings.json` 只在 `keybindings.rs` 自身出现。
- **影响：** 按文档创建该文件的用户不会看到任何效果。
- **建议修法：** 二选一——在启动时真正加载该文件并调用
  `apply_overrides`，或把四处文档改为"保留字段/暂未支持"。不要继续
  文档化一个不存在的能力。

### E3. 飞书"运行时单群覆盖"——API 存在但无人调用

- **文档：** `docs/guide/channels-config.md:85-86` — "Per-group overrides
  are possible at runtime (e.g. disable a specific chat); the config file
  above only sets the defaults"；`docs/guide/channels-config.zh-CN.md:84`。
- **代码：** `PolicyEngine::set_override` 存在
  （`channels/src/feishu/policy.rs:105-108`），`check_group` 也会查询覆盖表
  （`policy.rs:54-60, 66-95`），但桥从不调用：`channels/src/feishu/bridge.rs:84`
  仅 `PolicyEngine::new(feishu_cfg.policy.clone())`，全仓库 grep 显示
  `set_override` 只出现在 `policy.rs`（自身单测）。没有任何运行时路径
  （CLI、热加载、API）能让运维禁用某个群。
- **建议修法：** 删除该声明，或明确写出"单群覆盖仅作为内部 API 存在，
  当前无面向运维的入口"（若该功能是有意为之，请另立缺口条目）。

---

## 【缺失】 文档遗漏

### M1. 钉钉 wiki 完全遗漏 `sender_allowlist`（用户可见、代价最高）

- **文档：** `docs/wiki/en/DingTalk.md:53-72`（配置 JSON 示例）与
  `:75-82`（字段表）——没有 `sender_allowlist`；
  `docs/wiki/zh/DingTalk.md:53-72, 75-82`——同样遗漏。
- **代码：** `channels/src/config.rs:56-60` — `DingtalkChannelConfig.
  sender_allowlist`，注释写明 "Empty denies all"；强制点位于
  `channels/src/dingtalk/bridge.rs:99-101`（授权检查，斜杠命令同样受其
  约束——`dingtalk/bridge.rs:195` 只有通过 allowlist 检查后才分发）。
- **影响：** 完全照 wiki 配置的用户会得到一台**拒绝所有发送者（含
  `/help`）**的机器人，且文档没有给出解决办法。guide 页
  （channels-config.md）对此描述正确并带有升级提示（"已配置的钉钉桥
  需要填写 `sender_allowlist` 才会接收 prompt"）——但用户真正会看的 wiki
  页两者皆无。
- **建议修法：** 中英文页面在 JSON 示例中加入
  `"sender_allowlist": ["..."]`，字段表加一行（"空列表拒绝所有发送者；
  `["*"]` 信任所有人"），并补一行升级提示。

### M2. TUI 快捷键表漏了已注册的按键

- **文档：** `docs/guide/tui.md:63-76` — 共 9 行（ctrl+p/t/o/r/c、tab、
  enter、escape、方向键）。
- **代码：** `tui/src/app.rs:855-960` 还注册了 `ctrl+l`（Clear screen /
  redraw）、`shift+tab`（Cycle thinking）、`page up` / `page down`
  （滚动对话）、`ctrl+↑` / `ctrl+↓`（逐行滚动）。帮助浮层
  （`help_screen.rs:16-37`）同样未列，这本身自洽，但文档表格以完整清单
  姿态出现且未声明"子集"。
- **建议修法：** 补上缺失行，或像斜杠命令一节那样加"应用内帮助仅显示
  子集"的说明。

### M3. `packages.md` 遗漏 `remote-crypto` 包

- **文档：** `docs/architecture/packages.md:7-10` — 只列了 `rpc`、
  `markdown`、`thread-projection`、`json-preview`。
- **代码：** `packages/` 实际有五个目录；`packages/remote-crypto` 是 Rust
  crate，被桌面后端消费（`desktop/src-tauri/Cargo.toml:47` —
  `future-remote-crypto = { path = "../../packages/remote-crypto" }`），
  实现 `docs/internals/desktop/REMOTE_E2EE.md` 中记载的 Noise 端到端加密
  （`packages/remote-crypto/src/lib.rs:34-35`）。
- **建议修法：** 增加一条："`remote-crypto`：桌面/移动远程通道共享的
  Rust Noise 协议端到端加密实现。"

### M4. `loop-control-plane.md` 漏列 `lease expire` 与两个 scheduler 子命令

- **文档：** `docs/architecture/loop-control-plane.md:54` — "Lease |
  `lease claim/renew/release/status`"；`:60` — "Scheduler |
  `scheduler tick/show/liveness`"（`loop-control-plane.zh-CN.md:39,44`
  同）。
- **代码：** registry 用法串包含 `expire` 与额外的 scheduler 子命令：
  `orchestration/loop/src/console.rs:348` — "lease
  claim|renew|release|**expire**|status"（处理入口 `:5931`）；
  `console.rs:521` — "scheduler tick|show|**record-host-failure**|**ack**|
  liveness"（处理入口 `:3535-3536`）。
- **建议修法：** lease 行补 `expire`，scheduler 行补
  `record-host-failure`/`ack`，中英文同步。

### M5. `channels-config.md` 未记录飞书 `domain` 的三种取值形式

- **文档：** `docs/guide/channels-config.md:74` — "`domain` | `feishu` |
  API domain."（zh：`channels-config.zh-CN.md:73`）。
- **代码：** `channels/src/feishu/config.rs:56-84` — 实际支持三种形式：
  `"feishu"` → `open.feishu.cn`；`"lark"` → `open.larksuite.com`
  （`api_base`/`api_domain`/`ws_base`）；完整 `http(s)://` URL 原样使用
  （自建网关/测试 mock）。wiki 已记载 `"lark"`（`docs/wiki/en/Feishu.md:92`）。
- **建议修法：** 字段参考补充三种取值，避免 guide 反而不如 wiki 完整。

---

## 【过时】 描述过去状态

### O1. `CONTEXT_COMPACTION.zh-CN.md` 状态行早于语义压缩实现

- **文档：** `docs/internals/desktop/CONTEXT_COMPACTION.zh-CN.md:3` —
  "状态：**v2 数据底座已落地；语义压缩阶段设计已确认，待开发**
  （2026-08-24）"；S1–S4 计划把语义管线当作未来工作描述。
- **代码：** 语义压缩机制当前已存在于 agent：
  `agent/src/compaction/semantic.rs`（2325 行）及公开入口
  `prepare_semantic` / `prepare_semantic_with_phase` /
  `prepare_semantic_with_phase_and_fallback` /
  `prepare_semantic_with_lifecycle`（`agent/src/compaction/mod.rs:183-239`）。
- **【待核实】：** S1–S4 是否已完整接入运行时（`prepare_semantic*` 的
  调用方、provider 链状态）。缺的证据：对 `compaction/mod.rs` 使用点的
  调用图走查。
- **建议修法：** 走查完成后刷新状态行，写明哪些阶段已落地、哪些仍待做，
  而不是笼统写"待开发"。

---

## 【待核实】 证据不足

### U1. 已发布 Linux GUI 的 "glibc ≥ 2.39" 下限

- **文档：** `docs/wiki/en/Installation.md`（Linux 段）、
  `docs/wiki/en/FAQ.md`（"Linux GUI won't start on an older server"）、
  `docs/guide/build-and-install.md`。
- **现有证据：** 与 `.github/workflows/build-linux.yaml:40-48` 一致
  （aarch64 在 `ubuntu-24.04-arm` 构建；x86_64 在 `ubuntu-latest`，当前即
  Ubuntu 24.04 → glibc 2.39），但仓库内没有任何位置把该下限写死。
- **需要的证据：** 发布构建日志，或对已发布二进制的 `ldd`/`objdump`
  检查，才能确认该表述完全成立。

### U2. 跨仓库 / 历史基线

- `docs/internals/desktop/CONNECTION.*` 引用 FutureOS 基线 `2774e4a9` 与
  future-server `bbd6c23`；`docs/architecture/loop/UPSTREAM.md` 引用 LoopX
  上游基线。这些仓库状态无法在本检出中验证。文档已在合适处自行标注
  现状/目标/待验证，故不记入发现；待相关仓库可获取时复核基线。

---

## 修复 PR 注意事项

1. **E1/E2 涉及每种语言多个文件**——`channels-config`、
   `directory-layout`、`tui` 的 `.md` 与 `.zh-CN.md` 应同一 PR 修复，保证
  配对校验不破。
2. **M1 用户可见代价最高**——钉钉 wiki 现状会导致机器人完全拒收，建议
  优先于装饰性修正落地。
3. **路径不稳定：** 并行双语化会话仍在重命名 `docs/internals/desktop/*`
   文件（`CONNECTION.md` → `CONNECTION.zh-CN.md` 等），修复时请重新解析
   上文所有 `文件:行`。
4. 本审计**未修改**任何被审计文档；修复属于后续事实修正 PR
   （目标中的 ③）。
