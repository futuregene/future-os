# 审批与沙箱

FutureOS 可以在文件访问前询问并限制 shell 命令，但**桌面默认是不受限（`off`）模式**。
在让不可信任务访问文件前，请通过**设置 → General → 批准模式**或输入框盾牌选择保护方式。
可用性在实际运行 Agent 的机器上检测。

## 模式与默认值

| 模式 | 行为 |
|---|---|
| **不受限**（`off`，桌面默认） | 无审批规则、无 OS 沙箱，命令以当前用户权限执行。 |
| **手动**（`manual`） | 路径规则决定 Allow/Ask/Deny；普通读取、workspace/temp 写入可不询问。敏感路径及外部写入可能询问或拒绝。shell 除只读白名单外先询问。 |
| **沙箱**（`sandbox`） | 文件工具继续遵循路径规则；shell 在可用的 OS 后端中执行，通常不逐次预先询问。 |

新 Agent 会话的旧式权限级别默认是 `all`。TUI、CLI、渠道客户端不会为新会话主动开启桌面
沙箱策略；继续已有会话时可能保留其策略。`future run --permission` 是独立设置：`all`
不受限，`workspace` 启用审批门控，`none` 拒绝**所有**工具调用；它不选择 OS 沙箱。

## 平台支持

| 平台 | 后端 | 重要限制 |
|---|---|---|
| macOS | Seatbelt（`sandbox-exec`） | 文件系统规则；网络仍开放。 |
| 原生 Linux | 系统 Bubblewrap ≥ 0.9.0 | 基于快照的文件系统挂载；网络开放、没有 seccomp filter。WSL 不在支持的验证范围内。 |
| Windows | 非提权受限令牌 + 文件系统 ACL | 仅写保护；shell 读取和网络开放。既有 ACL 和父目录删除权限可能削弱保护。 |

Linux 在命令启动时保护已有匹配路径。尚不存在的受保护名称，以及命令执行中产生的新 glob
匹配，可能仅在结束后检测。检测不阻止创建、不回滚修改，也不能证明是谁创建的文件。
复杂重叠规则可能无法安全准备。平台可用不代表所有发行版、架构都完成了发布与独立安全验证。

## Linux 安装与诊断

通过发行版可信软件源安装 Bubblewrap：

```bash
# Debian / Ubuntu
sudo apt update
sudo apt install bubblewrap
# Fedora: sudo dnf install bubblewrap
future agent --probe-sandbox
future doctor
```

FutureOS 不捆绑或下载 Bubblewrap。旧发行版软件包可能低于 0.9.0，应通过受支持的可信渠道
升级，不要换成来源不明的二进制。版本下限是兼容要求，不保证包含所有上游安全修复。
安装或修复后，请完全退出并重启 FutureOS。

Linux 沙箱选项在检测中或不可用时仍显示但禁用。设置页提供原因、建议操作和诊断 code：

| code | 检查方向 |
|---|---|
| `binary_missing` | 安装系统包，并确保所在系统目录在 PATH 中。 |
| `path_rejected` / `binary_invalid` | 使用可信、可执行、root-owned 的系统程序，而非工作区副本。 |
| `version_too_old` / `version_unreadable` / `required_feature_missing` | 升级或修复发行版软件包。 |
| `user_namespace_disabled` / `proc_mount_restricted` | 请管理员评估主机/容器策略是否支持所需 namespace/mount；不要绕过组织安全策略。 |
| `probe_timeout` / `probe_failed` | 查看 `future doctor` 和本机日志。 |
| `probe_transport_error` | 恢复应用到 Agent 的连接；不表示平台永久不兼容。 |

明确不可用时回退手动模式；瞬时连接错误保留设置。实际命令准备失败时，该次调用失败，
不会静默改为不受限执行。Agent 可对必要的替代操作显式申请审批。

## 批准卡片与保存规则

卡片可提供**允许一次**、**拒绝**，或保存当前 workspace/chat 的路径规则。请核对真实
命令、路径与范围。Cmd/Ctrl+Enter 批准；Esc 拒绝，若规则编辑器已打开则先关闭它。
待审批请求没有超时。

规则保存在 `~/.future/approval_rule.json` 和 workspace 的 `.future/approval_rule.json`。
内置保护优先于用户规则。并非所有普通读取都会被拦截；允许一条规则不代表其中内容可信。

macOS/Linux 提权审批是**整条命令脱离沙箱执行一次**，不只是放行一个路径；重试可能重复
之前的副作用。Windows 则申请已有文件或目录子树的具体写权限，命令仍受限；创建或替换
文件可能需要批准其父目录 subtree。

## 不保证什么

沙箱不是网络过滤器、加密凭据库、回滚系统或完整的提示注入防线。Agent 的 `auth.json`
当前为 CLI 技能保留了硬拒绝例外，任意 shell 命令也可能读到它；不能认为沙箱隔离了所有
凭据。已保存规则和不受限模式允许操作不再询问。应核对产物和文件改动，不要把秘密交给
不可信任务。

另见 [[使用 FutureOS|Using-FutureOS]]、[[设置|Settings]]、[[命令行工具|CLI]] 和
[[常见问题|FAQ]]。仓库 SECURITY.md 与各平台设计文档记录了进一步的边界和对应候选版本的
验证证据。
