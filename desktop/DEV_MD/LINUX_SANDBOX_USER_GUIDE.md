# Linux Bubblewrap 沙箱安装与排障

状态：开发分支用户手册（2026-09-03）。本页适用于**原生 Linux**；WSL 1/2 不在支持范围内。

## 安装

FutureOS 只使用系统提供的 `bwrap`，不会下载或捆绑另一份 Bubblewrap。最低支持版本为 **0.9.0**；发行版命令安装到更旧版本时，仍需要通过该发行版的受信任更新渠道升级。

```bash
# Ubuntu / Debian
sudo apt update && sudo apt install bubblewrap

# Fedora
sudo dnf install bubblewrap
```

安装后完全退出并重新启动 FutureOS。设置页只有在完整探测通过后才显示“沙箱保护”。探测包含安全 PATH 查找、版本输出、必需参数和真实 user/PID/IPC namespace + `/proc` 挂载测试，而不只是检查命令是否存在。

## 诊断

机器可读诊断（不启动 Agent，也不受 Agent 单例锁影响）：

```bash
future agent --probe-sandbox
```

输出包含稳定的 `available`、`code`、`backend`，成功时还包含固定的系统 `path`、`version` 和 capability。普通综合诊断也显示同一结果：

```bash
future doctor
```

常见 code：

| code | 含义与处理 |
|---|---|
| `available` | 探测通过，可以选择沙箱保护 |
| `binary_missing` | 安装 `bubblewrap`，并确认绝对系统目录在 PATH 中 |
| `path_rejected` | PATH 命中了相对目录、当前项目/workspace 内二进制，或候选文件不是 root 所有；移除该项，使用发行版系统包 |
| `binary_invalid` | 候选不是可执行普通文件，或无法安全读取 identity |
| `version_unreadable` | 系统包版本输出不可识别；升级发行版提供的 Bubblewrap |
| `version_too_old` | Bubblewrap 低于最低支持版本 0.9.0；升级系统包后重试 |
| `required_feature_missing` | `bwrap --help` 缺少 FutureOS 所需参数（包括用于避免 `ARG_MAX` 的 `--args`）；升级系统包 |
| `user_namespace_disabled` | 主机、容器或安全策略禁止非特权 user namespace；启用后重试 |
| `proc_mount_restricted` | 当前环境不允许沙箱建立 fresh `/proc`；调整容器/主机策略 |
| `probe_timeout` / `probe_failed` | 真实运行探测超时或失败；查看 `future doctor`，修复环境后重启 |
| `binary_identity_changed` | 探测后的 bwrap 文件被替换；FutureOS 会拒绝执行并重新探测 |

明确的探测失败会把已保存的“沙箱保护”回退并持久化为“手动审批”；瞬时 Agent 连接失败不会改写设置。无论哪一种失败，FutureOS 都不会把请求沙箱执行的命令静默裸跑。

## 模型主动申请在沙盒外运行

命令在沙盒中失败后，模型可以说明原因，主动申请在沙盒外执行。FutureOS 会先显示审批卡片，只有你批准后才会执行；模型不能自行批准。

请检查完整命令和“原因说明”。**批准的是当前整条命令在沙盒外运行一次，不只是访问卡片上列出的文件。** 主动申请时可能没有可解析的失败输出，因此不一定显示具体文件路径。拒绝则不执行该次脱沙盒命令；单次批准不会切换全局审批模式，后续普通命令仍使用原来的保护方式。

这是模型可主动使用的能力，不保证每次故障都能自动恢复。若之前的命令可能已产生修改，重新运行前应核对结果，避免重复操作。产品定义见 [PRODUCT.md §4.6](PRODUCT.md#46-approval)。

## 能力边界

- 根文件系统默认只读；workspace、会话临时目录和明确允许的路径按规则开放写入。
- 网络保持开放，不提供网络隔离。
- 一期不安装 seccomp syscall filter；当前强制边界是 user/PID/IPC namespace、只读/受控 mount、capability drop 的内层复核和 `PR_SET_NO_NEW_PRIVS`。seccomp 作为后续纵深防御，不应被理解成当前能力。
- 精确路径和命令启动前已经存在的 glob 匹配由 OS 沙箱硬保护。
- 命令运行中新产生的 glob 匹配无法由固定 mount view 动态拦截，只在命令结束后报告 `detection_only` violation；这不是动态硬保护。
- 第一版越界批准会把**整条命令**脱离 Bubblewrap 重跑一次。批准界面会明确提示这一点；路径级临时能力属于后续版本。
- 仅支持 system bwrap；没有 bundled fallback，也不会自动改用较弱的 Linux 后端。
- `.deb`、portable tarball、目标发行版及 aarch64 的发布结论必须按真机验收手册验证；本地开发 smoke 不能替代这些结果。本期不发布 AppImage/rpm。
