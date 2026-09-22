# 安全策略

> （[English](SECURITY.md)）

FutureOS 是一个本地优先（local-first）的 AI agent。本文档描述其信任模型、当前的控制手段与局限，以及漏洞报告方式。

## 信任模型与数据流

- **本地持久化，而非仅离线处理。** 会话与配置存储在本地 `~/.future/` 下；loop 状态默认随项目本地存放。纳入上下文的提示词、选定的附件与工具结果会发送给所配置的模型提供方。FutureOS 托管的模型与在线工具使用 FutureOS 服务；其他提供方/工具使用各自端点。
- **可选的远程通道。** 启用 Remote 后，命令、会话事件与所请求的文件会经由所配置的 NATS 中继传输。移动端要求 TLS WebSocket（`wss://`）；桌面端到 NATS 的传输遵循部署配置，并非无条件强制 TLS。飞书/钉钉的消息与回复同样经由这些平台。本地存储并不意味着端到端加密，也不意味着没有任何数据离开设备。
- **按用户的本地后端。** agent 默认使用 macOS/Linux 上的 Unix 域套接字（私有目录与对端 UID 校验）或仅当前用户可用的 Windows 命名管道。Unix 遵循 `FUTURE_AGENT_SOCKET`；拥有自己 FutureOS home 的实例（`FUTURE_HOME` / `future agent --home`）使用 `<home>/run/agent.sock`，而不是共享的 XDG 运行时目录；Linux 否则在设置时使用 `$XDG_RUNTIME_DIR/future/agent.sock`，以 `~/.future/run/agent.sock` 作为回退/macOS 默认。`--grpc-addr` 显式启用 TCP。切勿把 agent 的明文 TCP 服务暴露给不可信网络；如需远程访问，请使用经认证的安全隧道。
- **凭据。** 提供方密钥本地存储，通常位于 `~/.future/agent/auth.json`；旧版 `agent-app/auth.json` 位置同样会被读取。提供方配置中也可能含有密钥。请将这些文件、备份、日志以及 `future auth credential` 的输出视为敏感信息。本地存储并非加密凭据保险库。

## 工具执行安全

核心工具为 `read`、`write`、`edit` 与 `shell`。

**防护是可配置的，默认并非对每次调用启用。** 新建的 agent 会话默认权限级别为 `all`；桌面端默认不限制（`off`）。TUI/CLI/channel 客户端不会为新会话独立启用沙箱策略。继续既有会话可能保留其策略。枚举值的 `manual` 默认值并非应用的实际默认值。

桌面端「Settings → General」或输入框（composer）的盾牌图标可选择：

- `off` —— 无审批规则，亦无 OS 沙箱。
- `manual` —— 路径规则决定 Allow/Ask/Deny（允许/询问/拒绝）。普通读取与工作区/临时写入可不经询问放行；敏感路径与外部写入会被询问或拒绝。Shell 命令除非命中只读允许清单，否则都会询问。已批准的 shell 命令以当前用户权限运行。
- `sandbox` —— 路径规则保持生效；shell 命令使用可用的 OS 后端。它们并非全部在执行前询问。macOS 使用 Seatbelt，原生 Linux 使用系统 Bubblewrap，Windows 使用非提权的受限令牌写入保护。明确不可用的探测回退为手动审批；真实沙箱命令的初始化失败会使该次调用失败，而不会静默地以不受限方式运行。

另有相互独立的旧版权限级别：`all`（不受限）、`workspace`（经审批的访问）与 `none`（拒绝所有工具调用）。它们不是三个沙箱档位的别名。参见[沙箱指南](docs/wiki/en/Sandbox.md)。

### 已知边界

- **网络是开放的。** 这些后端不提供域名/网络过滤。Linux 目前没有 seccomp 过滤器。shell 允许清单或沙箱都不能证明某条命令安全、阻止一切数据外传，或撤销副作用。
- **Linux 快照限制。** 需要受信任的系统 Bubblewrap ≥ 0.9.0 与可用的用户命名空间。既有的受保护路径以受限方式挂载；命令执行期间缺失的受保护路径与新增的 glob 匹配只在事后检查，不会被动态阻止或回滚。复杂重叠的规则可能导致准备失败。实现可用性并非对所有发行版/架构的认证；参见 [Linux 边界与验证](docs/internals/desktop/SANDBOX/LINUX.md)。
- **Windows 仅写入保护。** Shell 读取/网络仍然开放。既有 ACL 与父目录删除权限可能削弱写入边界。额外访问仅为具体的文件/子树写入能力获批；Windows 不使用 macOS/Linux 的整条命令脱离沙箱机制。参见 [Windows 边界](docs/internals/desktop/SANDBOX/WINDOWS.md)。
- **凭据例外。** `auth.json` 目前被排除在沙箱内置的硬拒绝清单之外，以便官方基于 CLI 的技能能够认证。这也可能使其暴露于任意的 shell 读取；它不是按二进制划分的信任，也不是安全的凭据通道。其他写入规则仍然适用；`models.json` 仍被内置策略拒绝。切勿假定沙箱隔离了所有 agent 凭据。
- **审批有作用域。** macOS/Linux 提权一次性授权整条命令在沙箱外执行。请审查其中是否有无关动作与重复副作用。已保存的规则会在不再询问的情况下授权未来匹配的动作。
- **同一用户的信任边界。** 这些控制无法防御已经控制 agent 进程或用户主机账户的攻击者。

## 提示注入、技能与渠道

不可信的页面、文档、技能与工具结果可能试图诱导模型。启用时，审批与沙箱能够降低风险；它们不能保证注入的指令无法生效。请审查工具活动、收窄权限，并避免向不可信任务暴露机密。

技能来自 [future-skills](https://github.com/futuregene/future-skills)，并通过会话的工具权限执行；只安装你信任的技能。渠道会话可以按其配置的权限（默认 `all`）驱动本地 agent。请像对待 shell 访问一样谨慎地限制机器人成员/允许清单与凭据；切勿假定桌面端的审批设置适用于每个客户端或新会话。

## 支持的版本

安全修复应用于 `main` 上的最新版本。我们不为旧版本维护补丁分支；请停留在最新版本。

## 报告漏洞

**切勿为安全报告公开提交 GitHub issue。** 请使用 [GitHub Security Advisories](https://github.com/futuregene/future-os/security/advisories/new)。

我们承诺在 **72 小时**内确认收到报告，在调查与修复过程中随时向你通报进展，并在报告者不要求匿名的情况下致谢报告者。

请包含受影响的版本/commit、平台、问题与影响，以及复现步骤或（如有）概念验证。切勿包含真实凭据。
