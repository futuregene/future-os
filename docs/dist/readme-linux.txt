FutureOS 免安装版使用说明（Linux）
==================================

【启动】
将下载的压缩包解压到任意目录，然后运行 ./futureos。
使用实际下载文件名：正式发布为
FutureOS_<version>_linux_<arch>-portable.tar.gz（x86_64 或 aarch64）；
本地开发构建可能叫 FutureOS-portable-linux.tar.gz。
futureos 与 future 必须同目录。没有可连接的兼容 Agent 时，应用通过 future agent 启动它。

【运行环境】
发布的 GUI 包需要 glibc >= 2.39（约 Ubuntu 24.04+）与 WebKitGTK：
    Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0
    Fedora:        sudo dnf install webkit2gtk4.1
官方 Linux CI 使用静态 musl 构建 future CLI，不需要 GUI 运行库。
本地源码构建使用所选 host target，不一定静态链接。
无桌面主机可下载对应架构的官方 CLI-only 包，运行 ./future config 和 ./future tui，
或运行 ./future agent 供其他 CLI 客户端使用。

【可选沙箱】
桌面默认不受限，请在设置中按需选择手动或沙箱模式。
Linux 沙箱需要可信系统 Bubblewrap >= 0.9.0（应用不捆绑）：
    Debian/Ubuntu: sudo apt install bubblewrap
    Fedora:        sudo dnf install bubblewrap
旧发行版包可能需要可信渠道升级。完全重启 FutureOS 后，运行
future agent --probe-sandbox 与 future doctor。主机 namespace 策略可能阻止使用；
明确不可用时回退手动模式。网络开放，缺失保护路径与新匹配存在仅检测的限制。
完整边界见仓库 Wiki 的审批与沙箱指南。

【说明】
· 配置 provider/key，或联网登录使用托管模型。
· 个人数据保存在 ~/.future。在线模型/工具和 Remote 会向对应服务发送请求，
  本地优先不代表数据不会离开本机。
· 应用仅关闭自己启动的 Agent；外部管理的 Agent 保持运行。
· 同目录已附带统一 future CLI。

【许可】
FutureOS 主体按 MIT 许可发布；内置的 future loop 组件派生自 LoopX，
按 Apache-2.0 许可发布。许可证全文与归属声明见 licenses/ 目录。

如遇问题，请反馈版本、架构和错误信息，不要包含凭据。
