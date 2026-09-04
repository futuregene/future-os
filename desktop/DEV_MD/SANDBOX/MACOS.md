# macOS：Seatbelt 沙箱

更新：2026-09-04。v2 已实现；共享规则、审批 UI 与 Codex 参考见 [COMMON.md](COMMON.md)。本页以源码为准，历史 PASS 不代替当前版本实测。

## 1. 技术方案与实现

`agent/src/sandbox/seatbelt.rs` 将 RuleSet 编译为 SBPL，`PreparedShell` 启动 `/usr/bin/sandbox-exec -p <profile> bash -c <command>`。没有 Linux 的 mount helper，也不修改宿主 ACL。

- profile 从 `(deny default)` 开始，允许 fork/exec/process-info、same-sandbox signal、pseudo-tty、sysctl-read、mach-lookup、ipc-posix、file-ioctl 等开发工具所需操作；这些系统接口当前较宽，不宣称最小权限 syscall 沙盒。
- 默认 `(allow file-read*)`；写开放 workspace、`temp_roots()` 与 `/dev/null`、`/dev/zero`、stdout/stderr、`/dev/fd/*`、tty、dtracehelper 等伪设备。
- 网络使用 `(allow network*) (allow system-socket)`，没有断网或域名过滤。
- RuleSet 层与层内顺序均反向发射：低优先级到高优先级，SBPL last-match 对应引擎 first-match；overrides 最后生效。read/write 分别发射，ask 和 deny 均为 OS deny。
- 无通配符 matcher 统一为 `subpath`（**不是先判断文件存在再选择 literal**）；glob 用规则引擎生成的 regex。只保证实现支持的 `*`/`**`/`?` 语义，不把 SBPL regex 能力等同全部 shell glob。
- 路径规范化处理 `/tmp`、`/var` 的 `/private` 目标；SBPL 字符串转义双引号和反斜线，避免路径注入。

规则作用于后续文件操作，而非启动时枚举文件。因此允许父目录内，缺失 `.env`、未来 `.pem` 以及 rename 到保护目标仍能命中 profile；这是与 Linux mount 快照和 Windows ACL 的核心差异。

Agent 的 Unix 进程组终止逻辑处理 timeout/abort。native read/write/edit 在 Agent 进程内先求规则。旧稿的独立 `run_grep` / `run_ls` 待办已不对应当前 tools 实现；经 shell 调用 grep/ls 随该命令包装，不能据此宣称任意新增 subprocess/MCP 自动获得覆盖。

## 2. 审批与不可用行为

主动 `escalated: true` + justification 或失败文本匹配后，经 `EscalationRequester` 回到 RPC 审批。批准后当前整命令不再包 Seatbelt，只有一次，不持久改档、不生成精确路径授权。完整标题、路径展示限制见 [COMMON §4](COMMON.md#4-审批协议与界面)。

目前拒绝判断包含 `Operation not permitted` / `sandbox-exec` 文本启发式，网络错误不作为拒绝依据。无法可靠证明内核拒绝的确切路径或命令此前未产生副作用，因此不得承诺无误报/安全自动重放。规则文件 hard deny 也不覆盖用户批准后的裸命令。

源码 `sandbox/mod.rs` 当前 macOS availability 是 `/usr/bin/sandbox-exec` 的存在检查，**不是 Linux/Windows 那样的真实生产能力 probe**。不可用走公共 manual 行为。`sandbox-exec` deprecated 风险已接受；未来系统兼容性需持续原生复验。

## 3. 开发进度与验收

R1（2026-07-04）规则/Seatbelt/native read 审批、R2 GUI 文件持久化、R3 守卫与当轮注入已完成。原有 profile 转义、路径处理、进程组终止和 smoke 框架复用；旧网络拒绝测试改为开放预期。

历史 R3：Agent 58 lib、Seatbelt 9 smoke、GUI 72、前端 39 通过，lint/check-desktop 通过；不是本次整理重跑结果。`auth.json` 后来暂移出 override，因此旧“auth 拒读 PASS”不再代表当前能力。

在候选提交的 macOS 真机执行（测试需显式运行 ignored，不能把跳过当通过）：

```bash
cargo test -p future-agent --test sandbox_smoke -- --ignored --test-threads=1 --nocapture
```

保留完整 commit、OS/架构、日志。必查：workspace/temp 写允许、外部写失败、已有及新建 secret 读写拒绝、规则文件直接写及 rename 拒绝、symlink 指向 workspace 外不能借路径别名绕过、Unicode/转义路径、cargo/git/python、网络开放、timeout/abort 后代清理。额外检查主动/被动标题、拒绝不重跑、允许一次不改变后续保护、原生敏感读取无持久允许。

本次文档整理未运行 macOS smoke；当前新增 Linux 私有报告不等于 macOS 也有可信错误通道。

## 4. 差异、缺口与后续计划

| 优先级 | 工作 / 边界 |
|---|---|
| P1 | 二期 execution_grants：已批准 ask 路径临时放开、仍在 Seatbelt；不能用低于 guards 的 session allow 冒充。公共设计见 COMMON §5.1 |
| P1 | auth.json 可信凭据通道完成前，暂移除 override 的读取风险仍在 |
| P1 | 整个规则文件读取/解析失败在 macOS 不同于 Linux 的 compile fail-closed；评估统一诊断与处理，个别无效条目也需独立设计 |
| P2 | 将可用性由存在检查提高为有界真实 probe，扩充 denial 负向语料和系统升级 smoke |
| P2 | 按实际工具集审计新子进程接入，持续收窄 mach/IPC 等宽许可时做开发工具兼容矩阵 |
| 已接受 | 网络开放、清单外敏感内容可外传、`.git` 可写、temp 按实际根开放、sandbox-exec deprecated；不属于已解决安全问题 |

Seatbelt 本身能够按命令临时加路径 allow；当前精确授权缺口主要在 FutureOS 协议与层级，不是必须整命令脱沙箱的 OS 限制。Windows 独立用户候选方案与本平台无关，不应混入 macOS 改造。
