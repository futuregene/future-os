# macOS / Linux 沙箱安全审计——2026-09-14

> 本文是历史快照 [macOS / Linux sandbox security audit — 2026-09-14](./unix-sandbox-audit-2026-09-14.md)（2026-09-14，commit `a6885813`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

范围：agent 的共享路径规则、macOS Seatbelt 编译器与 Linux Bubblewrap 辅助程序。这是针对性实现评审与回归练习，不是对所有沙箱逃逸类别或发布平台的认证。

## 发现与修复

| 发现 | 影响 | 修复 / 回归 |
|---|---|---|
| macOS 正则字面量被转义为普通 SBPL 字符串 | `*.pem`、`*.key`、`.env.*` 等可能连普通小写文件名都匹配失败。现有 smoke 测试大多只检查字面 `.env`，因此漏掉这点。 | 将转义字符串传给 `(regex "...")`，而不是在 `#"..."` 内放第二层转义。内核测试检查拒绝、空 stdout 与新创建文件的缺失。 |
| RegexBuilder 的大小写不敏感标志未出现在发出的 SBPL 源中 | ASCII 大小写变体如 `secret.PEM` 可绕过 glob 守卫。 | 发出显式 ASCII 大小写类。测试大写扩展名与 `.ENV.PRODUCTION`。这不声称 Rust 与 Seatbelt 之间完全 Unicode 大小写折叠等价。 |
| 共享 `**` 使用的点排除换行 | 原生文件工具与 Seatbelt 可能漏掉换行目录下的秘密。Linux 独立扫描器本已匹配换行。 | 以两引擎都理解的语法编码含换行的重复；共享规则与真实 Seatbelt 回归。 |
| macOS 忽略规则文件解析错误 | 损坏/不可读策略或非法规则条目可能在仍启动命令的同时移除预期限制。 | 生产准备返回基础设施错误；公共 profile 构建器返回全拒 profile。显式批准的无沙箱执行保持独立。 |
| Linux FD 清理停在 `min(_SC_OPEN_MAX, 65536)` | 高编号 mount/source 描述符可幸存于内部清理。外层旧内核回退有同样问题，包括 RLIMIT_NOFILE 调低后。打开的描述符可绕过路径名/mount 限制。 | 枚举实际 `/proc/self/fd` 条目，与限制无关。fork 前为 CLOEXEC 回退做快照；传播清理错误。隔离子测试在允许处打开 FD 70000，把限制调到 1024，验证回退 CLOEXEC、关闭与保留的允许列表。 |

FD 发现是隔离演示的边界清理缺陷，不是默认配置端到端利用的声明。现有生产 smoke 测试继续覆盖 capability 丢弃、`no_new_privs`、私有报告、mount 保护与进程清理。

## 本地证据

- macOS 26.6.2 (25G83)、arm64、Rust 1.97.0：真实 Seatbelt 安全回归 **3/3 通过**，现有忽略 Seatbelt smoke **9/9 通过**。
- Docker Desktop Linux VM 内核 `6.12.76-linuxkit`、aarch64；Debian trixie、Bubblewrap **0.12.0**、Rust **1.97.0**：忽略 Bubblewrap smoke **11/11 通过**，无后端不可用跳过。测试包含实际 outer → bwrap → inner → shell 流程。
- 完整 `cargo test -p future-agent`：macOS **1768 个库测试通过**，Linux **1762 个库测试通过**，各自集成目标全部通过。两者都将显式忽略的大工作区测试排除在默认运行外；平台 smoke 测试如上单独运行。`cargo fmt -p future-agent --check` 与 `cargo clippy -p future-agent --all-targets -- -D warnings` 在两平台通过。
- 原始 macOS smoke 调用暴露一个继承的 `CARGO_TARGET_DIR` fixture 问题：cargo 试图写入沙箱外父运行器的构建目录。辅助程序现在为其一次性工作区移除该变量，重跑通过。这不是沙箱绕过。
- 全新 Docker 默认正确拒绝用户命名空间。仅测试容器以 `--cap-add SYS_ADMIN --security-opt seccomp=unconfined --security-opt systempaths=unconfined` 重建，以允许嵌套 Bubblewrap 命名空间与全新 proc 挂载。宿主源码只读挂载；构建/测试输出使用专用一次性 Docker 卷。作为容器 root 在只读检出上的首次完整套件尝试有五个环境失败（三个测试在检出内写入，两个需要普通用户 DAC 拒绝）。以非特权 `audit` 用户在仅容器的可写源码副本上重复完整套件与全部 11 个 Bubblewrap smoke 测试通过。未更改宿主策略、现有 agent 或凭据。

容器结果验证本环境中的 Linux 内核/辅助程序行为；它们**不**替代原生发行版、x86-64、桌面或安装包认证。小型离线 `seatbelt_security` 套件在 macOS 上随 `cargo test -p future-agent` 自动运行。把它接入当前 macOS CI 构建 job 被推迟：可用的 GitHub 凭据缺 `workflow` 权限，工作流变更被移除而非扩大凭据。现有 CI Linux 测试无法验证 SBPL 内核行为；使用下方 macOS 命令。

## 复现

```sh
cargo test -p future-agent --test seatbelt_security
cargo test -p future-agent --test sandbox_smoke -- --ignored --test-threads=1 --nocapture
cargo test -p future-agent --lib sandbox::
cargo test -p future-agent --test linux_sandbox_smoke -- --ignored --test-threads=1 --nocapture
cargo fmt -p future-agent --check
cargo clippy -p future-agent --all-targets -- -D warnings
cargo test -p future-agent
```

前两条仅 macOS；Linux smoke 必须显示真实执行，而非 `skipping Linux sandbox smoke`。在 macOS 上于完整 agent 套件前提高运行器的文件描述符上限；完整套件凭据隔离使用一次性 HOME。

## 剩余边界（未修复或新承诺）

先前文档化的产品决策保留：网络不受限；宽泛的 macOS Mach/IPC 权限；临时可读的 agent `auth.json`；显式审批解开整个命令；Linux 无 seccomp 过滤器，且缺失受保护路径/新 glob 匹配只获得检测性重扫，而非预防或回滚。Linux 复杂 deny/reopen 组合与缺失 allow 根可能准备失败。root 拥有的 Bubblewrap 路径链加固、完全 Unicode 大小写等价、并发别名/inode 变更与更广 IPC 攻击面未被本 pass 认证。这些已接受限制见 `desktop/DEV_MD/SANDBOX/` 下的现有平台设计文档；本报告只取代它们历史上的 macOS 正则/畸形策略行为与 Linux FD 清理描述。
