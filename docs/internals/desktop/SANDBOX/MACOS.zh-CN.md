# macOS：Seatbelt 沙箱

更新：2026-10-12。v2 已实现；共享规则、审批 UI 与 Codex 参考见 [COMMON.zh-CN.md](COMMON.zh-CN.md)。本页以源码为准，历史 PASS 不代替当前版本实测。

## 1. 技术方案与实现

`agent/src/sandbox/seatbelt.rs` 将 RuleSet 编译为 SBPL，`PreparedShell` 以与未加沙盒时相同的选定 Unix shell 启动 `/usr/bin/sandbox-exec -p <profile> <shell> -c <command>`（`$SHELL` 为 bash/zsh 时用它，否则取 PATH 上第一个 bash/zsh，再退到 `sh`）。没有 Linux 的 mount helper，也不修改宿主 ACL。

- profile 从 `(deny default)` 开始，允许 fork/exec/process-info、same-sandbox signal、pseudo-tty、sysctl-read、mach-lookup、ipc-posix、file-ioctl 等开发工具所需操作；这些系统接口当前较宽，不宣称最小权限 syscall 沙盒。
- 默认 `(allow file-read*)`；写开放 workspace、`temp_roots()` 与 `/dev/null`、`/dev/zero`、stdout/stderr、`/dev/fd/*`、tty、dtracehelper 等伪设备。
- 网络使用 `(allow network*) (allow system-socket)`，没有断网或域名过滤。
- RuleSet 层与层内顺序均反向发射：低优先级到高优先级，SBPL last-match 对应引擎 first-match；overrides 最后生效。read/write 分别发射，ask 和 deny 均为 OS deny。
- 普通规则的无通配符 matcher 为 `subpath`，单次 file 授权为 `literal`（**不是先判断文件存在再选择 literal**）；glob 用规则引擎生成的 regex。只保证实现支持的 `*`/`**`/`?` 语义，不把 SBPL regex 能力等同全部 shell glob。
- 路径规范化处理 `/tmp`、`/var` 的 `/private` 目标；SBPL 字符串转义双引号和反斜线，避免路径注入。

规则作用于后续文件操作，而非启动时枚举文件。因此允许父目录内，缺失 `.env`、未来 `.pem` 以及 rename 到保护目标仍能命中 profile；这是与 Linux mount 快照和 Windows ACL 的核心差异。

Agent 的 Unix 进程组终止逻辑处理 timeout/abort。native read/write/edit 在 Agent 进程内先求规则。旧稿的独立 `run_grep` / `run_ls` 待办已不对应当前 tools 实现；经 shell 调用 grep/ls 随该命令包装，不能据此宣称任意新增 subprocess/MCP 自动获得覆盖。

## 2. 审批与不可用行为

模型通过 `additional_permissions.read/write` 在执行前声明具体路径。批准后创建本次命令的 Seatbelt profile：file 使用 literal，subtree 使用 subpath；显式 deny 不可覆盖，会话规则不改变。完整请求格式与 UI 见 [COMMON §5.1](COMMON.zh-CN.md#51-macoslinux-单次-execution_grants)。

失败诊断只返回主模型，不请求被动审批、不重跑。模型检查部分完成效果后再提交剩余操作。显式 `escalated: true` + justification 保留兼容，一次批准后当前命令裸跑，不与精确权限混用；不能因此声称 hard deny 仍覆盖裸命令。

源码 `sandbox/mod.rs` 当前 macOS availability 是 `/usr/bin/sandbox-exec` 的存在检查，**不是 Linux/Windows 那样的真实生产能力 probe**。不可用走公共 manual 行为。`sandbox-exec` deprecated 风险已接受；未来系统兼容性需持续原生复验。

## 3. 开发进度与验收

规则/Seatbelt/native read 审批、GUI 文件持久化、守卫与当轮注入均已实现。原有 profile 转义、路径处理、进程组终止和 smoke 框架复用；旧网络拒绝测试改为开放预期。

`auth.json` 已暂移出 override（见 COMMON §3.1），因此旧的“auth 拒读 PASS”记录不再代表当前能力。

在候选提交的 macOS 真机执行（测试需显式运行 ignored，不能把跳过当通过）：

```bash
cargo test -p future-agent --test sandbox_smoke -- --ignored --test-threads=1 --nocapture
```

保留完整 commit、OS/架构、日志。必查：workspace/temp 写允许、外部写失败、已有及新建 secret 读写拒绝、规则文件直接写及 rename 拒绝、symlink 指向 workspace 外不能借路径别名绕过、Unicode/转义路径、cargo/git/python、网络开放、timeout/abort 后代清理。额外检查主动/被动标题、拒绝不重跑、允许一次不改变后续保护、原生敏感读取无持久允许。

macOS 没有与 Linux 对等的私有完成报告通道；不要把 Linux 的报告语义套用到 macOS。

## 4. 差异、缺口与后续计划

| 优先级 | 工作 / 边界 |
|---|---|
| P1 | execution_grants 已接入；原生验证需覆盖读写范围、deny carveout、一次消费和审批后路径变更。见 COMMON §5.1 |
| P1 | auth.json 可信凭据通道完成前，暂移除 override 的读取风险仍在 |
| P1 | 整个规则文件读取/解析失败在 macOS 不同于 Linux 的 compile fail-closed；评估统一诊断与处理，个别无效条目也需独立设计 |
| P2 | 将可用性由存在检查提高为有界真实 probe，扩充 denial 负向语料和系统升级 smoke |
| P2 | 按实际工具集审计新子进程接入，持续收窄 mach/IPC 等宽许可时做开发工具兼容矩阵 |
| 已接受 | 网络开放、清单外敏感内容可外传、`.git` 可写、temp 按实际根开放、sandbox-exec deprecated；不属于已解决安全问题 |

Seatbelt 本身能够按命令临时加路径 allow；当前已用独立 execution_grants 接入，不再依赖失败后整命令重跑。Windows 独立用户候选方案与本平台无关，不应混入 macOS 改造。
