# Linux Bubblewrap 沙箱真机验收与安全 Review 手册

状态：**待执行**（2026-09-03）。本手册是 Linux 沙箱 L5 发布门槛的唯一逐机记录模板；开发机、容器和 CI 结果不能替代目标真机结果。实现语义见 [`LINUX_SANDBOX_PLAN.md`](LINUX_SANDBOX_PLAN.md)，本地自动化结果见 [`LINUX_SANDBOX_IMPLEMENTATION.md`](LINUX_SANDBOX_IMPLEMENTATION.md)。

## 1. 状态与完成规则

每个单元格只能填写以下状态之一，并附日期、执行人、机器标识和日志/截图位置：

- `PASS`：命令已在指定原生主机或指定安装包上执行，实际结果完全符合预期。
- `FAIL`：已执行但至少一个安全或产品断言不符合预期；记录实际结果并停止该制品发布。
- `NOT RUN`：尚未执行。
- `ENVIRONMENT LIMIT`：当前主机无法产生目标结论，例如架构不符、没有该包格式、组织策略禁用 user namespace。环境限制不是通过。

L5 只有在目标发行版/架构、实际发布包和安全 review 的所有必需项均为 `PASS` 后才完成。任何 `FAIL` 都阻断发布；任何 `NOT RUN` 或 `ENVIRONMENT LIMIT` 都表示仍需换用满足条件的真机补测。

## 2. 测试制品与证据头

每台机器先复制并填写以下信息。使用同一候选 commit 和同一组发布制品；不要在测试期间从其他分支重建。

```text
Tester:
Date (UTC):
Host label:
Native host (yes/no; VM is allowed, container/WSL is not):
Distribution/version:
Architecture:
Kernel:
Desktop session:
Candidate commit:
FutureOS version:
Package kind and SHA-256:
bwrap path/version/package version:
Evidence directory or issue URL:
```

采集命令：

```bash
set -eu
cat /etc/os-release
uname -a
uname -m
getconf GNU_LIBC_VERSION || true
command -v bwrap || true
bwrap --version || true
# Debian/Ubuntu
(dpkg-query -W -f='${Package} ${Version} ${Architecture}\n' bubblewrap 2>/dev/null || true)
# Fedora
(rpm -q bubblewrap 2>/dev/null || true)
sha256sum ./FutureOS_* 2>/dev/null || true
future --version
future agent --probe-sandbox
future doctor
```

预期：`bwrap` 来自系统绝对路径；正常主机 probe JSON 为 `available:true`、`backend:"linux_bubblewrap"`、`code:"available"`，并含 path/version/capabilities。`future doctor` 的 backend/code 与 JSON 一致，且不输出规则内容或受保护文件路径。

## 3. 必需真机矩阵

发行版版本必须是原生安装；Podman/Docker/chroot 不能替代。每行至少保留完整命令输出和第 5 节 smoke 日志。

| ID | 原生系统 | 架构 | bwrap 来源 | 状态 | 证据/备注 |
|---|---|---|---|---|---|
| RH-01 | Ubuntu 22.04 LTS | x86_64 | 官方 apt | NOT RUN | |
| RH-02 | Ubuntu 24.04 LTS | x86_64 | 官方 apt | NOT RUN | |
| RH-03 | Debian stable | x86_64 | 官方 apt | NOT RUN | |
| RH-04 | Fedora（当前受支持稳定版） | x86_64 | 官方 dnf | NOT RUN | 先记录具体 Fedora 版本 |
| RH-05 | Ubuntu 24.04 LTS | aarch64 | 官方 apt | NOT RUN | 必需 aarch64 行 |
| RH-06 | Debian stable 或 Fedora | aarch64 | 官方包管理器 | NOT RUN | 至少再覆盖一个 aarch64 发行版 |

若发行范围在候选时改变，发布负责人必须在本表增加对应系统；不能删除失败行来收窄结论。

## 4. 安装包矩阵

本期发布范围仅包含 `.deb` 和 portable tarball，明确不发布 AppImage/rpm；后两者不属于本次验收项。

| ID | 制品/动作 | x86_64 | aarch64 | 状态 | 预期结果 |
|---|---|---|---|---|---|
| PKG-01 | `.deb` 全新安装 | Ubuntu/Debian | Ubuntu/Debian | NOT RUN | GUI 与 `future` sidecar 可启动；无 bundled bwrap |
| PKG-02 | `.deb` 原位升级 | Ubuntu/Debian | Ubuntu/Debian | NOT RUN | 设置/会话保留；probe 仍一致 |
| PKG-03 | `.deb` 卸载 | Ubuntu/Debian | Ubuntu/Debian | NOT RUN | 应用文件移除；系统 bwrap 不被移除 |
| PKG-04 | portable tarball | 任一目标发行版 | 任一 aarch64 目标 | NOT RUN | 解压后 CLI/GUI 可运行；helper 自重入可用 |

`.deb` 建议命令（替换文件名）：

```bash
sudo apt install ./FutureOS_<version>_<arch>.deb
command -v future
future --version
future agent --probe-sandbox
future doctor
sudo apt install ./FutureOS_<new-version>_<arch>.deb   # 升级候选
future --version
sudo apt remove futureos
command -v bwrap && bwrap --version                    # 必须仍存在
```

portable tarball 按制品内 README 解压和启动；记录 `find <extract-root> -iname '*bwrap*'`。预期不包含 bwrap 副本，`future agent --probe-sandbox` 仍固定到 system bwrap。

每种实际 GUI 包还需检查：

1. 未安装 bwrap 时，设置页不允许选择“沙箱保护”，展示 `binary_missing` 和发行版安装命令；不得自动下载后端。
2. 安装 bwrap 并完全重启应用后，完整 probe 通过才显示“沙箱保护”。
3. 保存 sandbox 后让 bwrap 不可用，再启动应用：设置明确回退并持久化为 manual，显示原因；命令不得裸跑。
4. 仅停止 Agent 制造瞬时 RPC 失败：UI 保留原设置等待重试，不把瞬时故障持久化成 manual。
5. Composer 和 Settings 的中英文文案均说明：网络开放、system bwrap only、WSL 不支持、glob 是启动时快照且新匹配仅 detection-only。

## 5. 每台正常主机的功能与安全 smoke

在候选 commit 的干净 checkout 中运行，确保测试二进制与待审 commit 一致：

```bash
git rev-parse HEAD
cargo test -p future-agent --test linux_sandbox_smoke \
  -- --ignored --test-threads=1 --nocapture
```

预期：5 个 ignored smoke 实际执行并 PASS，输出中**不得**出现 `skipping Linux sandbox smoke`。它们证明：workspace 写入、外部写拒绝、`NoNewPrivs: 1`、exit/signal 保真、secret 不可读写、command 仅见允许 FD、missing path 无 host 残留、运行中新 glob 产生 detection-only marker、helper 父进程死亡后无残留后代。

补充产品检查：

```bash
future agent --probe-sandbox
future doctor
```

在 GUI 中选择“沙箱保护”，以临时 workspace 执行以下等价命令并保存审批截图/事件日志：

```bash
printf ok > ./inside.txt
printf blocked > "$HOME/future-sandbox-outside.txt"
printf '%s\n' "network-open"   # 再访问测试者控制的本地 HTTP 服务
```

预期：workspace 写成功；未经允许的 workspace 外写触发 Linux policy violation 和整命令脱沙盒审批，不是 infrastructure error；批准文案明确“整条命令脱离 Bubblewrap 重跑一次”；拒绝时 host 无目标文件。网络访问成功，因为一期不隔离网络。

| ID | 检查 | 状态 | 证据/备注 |
|---|---|---|---|
| SM-01 | ignored smoke 5/5 实际执行、无 skip | NOT RUN | |
| SM-02 | probe/doctor 一致且无敏感路径泄漏 | NOT RUN | |
| SM-03 | workspace 写成功、外部写被拒绝 | NOT RUN | |
| SM-04 | violation 才触发 escalation；infra/exit 2/125/126/127 不触发 | NOT RUN | |
| SM-05 | 网络保持开放 | NOT RUN | |
| SM-06 | Settings/Composer 中英文与 manual fallback | NOT RUN | |

## 6. 负向环境矩阵

负向测试必须使用专用 VM/主机快照，不要修改日常开发机的全局安全策略。每个场景恢复后再次确认正常 probe。具体 sysctl/LSM 配置因发行版而异，必须由机器管理员按发行版文档实施；以下只规定观察命令和产品断言，不提供绕过组织安全策略的命令。

| ID | 场景 | 观察命令 | 状态 | 必须结果 |
|---|---|---|---|---|
| NEG-01 | bwrap 未安装/不在可信绝对 PATH | `PATH=/usr/local/bin /absolute/path/to/future agent --probe-sandbox` | NOT RUN | `binary_missing` 或 `path_rejected`；sandbox 不可选 |
| NEG-02 | PATH 首项为相对目录或 workspace 假 bwrap | `PATH=.:$PATH future agent --probe-sandbox` | NOT RUN | 假 binary 不执行；可信 system bwrap 可继续被选择，否则 fail closed |
| NEG-03 | 非特权 user namespace 被策略禁用 | `future agent --probe-sandbox` | NOT RUN | `user_namespace_disabled`；manual fallback；不 escalation |
| NEG-04 | fresh `/proc` mount 被限制 | `future agent --probe-sandbox` | NOT RUN | `proc_mount_restricted` 或稳定 `probe_failed`；不得运行用户命令 |
| NEG-05 | bwrap 缺必要参数/版本不可解析 | 受控 shim 仅用于 probe 测试，不能放在 workspace | NOT RUN | `required_feature_missing` / `version_unreadable`；fail closed |
| NEG-06 | probe 超时 | 受控测试 fixture | NOT RUN | `probe_timeout`；失败不永久缓存，修复后可重探测 |
| NEG-07 | probe 后替换 binary inode | 专用测试 fixture | NOT RUN | `binary_identity_changed` 或重新完整 probe；旧凭据不执行 |
| NEG-08 | WSL 1/2 | `uname -a; future agent --probe-sandbox` | NOT RUN | 记录为 unsupported；不作为原生 Linux PASS |

若环境不能安全构造某场景，填写 `ENVIRONMENT LIMIT` 并把该行派给具备隔离 VM 的测试者。

## 7. 安全 Review 清单

安全 reviewer 必须阅读实际 diff（建议 `git diff <base>...<candidate> -- agent/src/sandbox agent/src/tools/mod.rs agent/src/cli.rs`），逐项给出 `PASS`/`FAIL`，附代码位置、测试证据或 issue。仅运行测试不能替代代码 review。

| ID | Review 项 | 状态 | Reviewer 需要确认的证据 |
|---|---|---|---|
| SEC-01 | mount TOCTOU / symlink | NOT RUN | canonical lexical+target 双检查；source FD 与 dev/inode/type 复核；identity 变化 fail closed |
| SEC-02 | FD 泄漏 | NOT RUN | stdio/request/mount 白名单；Agent listener/db/log FD 不进入 inner command；smoke 只见 fd 0–2 |
| SEC-03 | setuid bwrap 与提权边界 | NOT RUN | 只接受可信 system bwrap；外层兼容发行版 bwrap；内层 `PR_SET_NO_NEW_PRIVS`；cap drop 全量 |
| SEC-04 | namespace 与网络语义 | NOT RUN | user/PID/IPC 隔离、fresh `/proc`、最小 `/dev`；明确没有 `--unshare-net` |
| SEC-05 | PID 1 / signal / parent death | NOT RUN | 转发信号、回收后代、exit/signal 保真；timeout/abort/父死无残留 |
| SEC-06 | missing path / 临时对象 cleanup | NOT RUN | host placeholder 使用 inode identity/CAS 清理；不得删除并发创建的用户对象；失败也清理 |
| SEC-07 | 规则编译 fail closed | NOT RUN | 坏规则、unsupported matcher、glob 数量/节点/深度/时限、危险 reopen 全部阻止执行 |
| SEC-08 | deny/reopen 顺序 | NOT RUN | 宽 writable → 保护覆盖 → 窄 reopen；hard deny 永不可 reopen；read deny 不被 write allow 绕过 |
| SEC-09 | probe 与执行一致性 | NOT RUN | path/version/features/真实能力/identity/expiry 属于同一 receipt；执行前重验 |
| SEC-10 | violation 与 escalation | NOT RUN | 可信 marker 优先；infra 和普通 exit 不误判；只有 policy violation 可请求整命令重跑 |
| SEC-11 | 日志与 RPC 脱敏 | NOT RUN | UI/普通日志不输出完整受保护路径集合、规则或 secret；只暴露必要 bwrap path/code/digest/count |
| SEC-12 | helper 攻击面 | NOT RUN | singleton 前分派但 request 有版本/大小/数量/FD/path/NUL 校验；非法调用 exit 125，不启动 Agent |

Reviewer sign-off：

```text
Reviewer:
Date (UTC):
Candidate commit:
Decision: PASS / FAIL
Blocking issues:
Non-blocking follow-ups:
Evidence/issue URLs:
```

## 8. 汇总与剩余 TODO

| 发布门槛 | 当前状态 | 完成条件 |
|---|---|---|
| L5-01 目标真机 | NOT RUN | RH-01～RH-06 与 SM-01～SM-06 在对应机器全部 PASS |
| L5-02 发布包 | NOT RUN | PKG-01～PKG-04 全部 PASS；本期不发布 AppImage/rpm |
| L5-03 安全 review | NOT RUN | SEC-01～SEC-12 全部 PASS，reviewer 完成 sign-off，阻断问题为零 |

交付给测试/发布负责人后的剩余 TODO：

1. 提供同一 commit 的 x86_64/arm64 候选 `.deb` 与 portable tarball 及 SHA-256。
2. 分配 RH-01～RH-06 的原生主机并上传逐机日志；所有 smoke 必须实际运行而不是 skip。
3. 在隔离 VM 完成 NEG-01～NEG-08；环境限制项重新分配，不视为豁免。
4. 由非实现者完成 SEC-01～SEC-12 和 sign-off。
5. 把本手册中的状态与证据链接回填后，才可把实施矩阵 L5-01～03 改为 PASS，并宣称满足主干发布门槛。
