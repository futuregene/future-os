FutureOS 免安装版使用说明（Linux）
==================================

【启动】
解压到任意目录，三个文件保持在同一文件夹：
    tar -xzf FutureOS-portable-linux.tar.gz
    ./futureos
注意：futureos、future 必须在同一文件夹，勿单独移动 —— 后台 agent 由
future（`future agent`）自动拉起。

【运行环境】
GUI 需要系统的 WebKitGTK 运行库。若启动时报缺少库，用包管理器安装：
    Debian/Ubuntu:  sudo apt install libwebkit2gtk-4.1-0
    Fedora:         sudo dnf install webkit2gtk4.1
注意：GUI 对系统版本有要求（glibc ≥ 2.39，约 Ubuntu 24.04+）。但附带的
命令行工具 future 是静态编译的，不依赖系统库——即使 GUI 在旧系统上无法
启动，仍可直接使用 ./future（CLI / `future tui`）。

【说明】
· 首次使用需联网登录（在应用内完成）。个人数据保存在 ~/.future 。
· 退出应用时后台 agent 会一并关闭。
· 已附带命令行工具 future（同目录）。

【许可】
FutureOS 主体按 MIT 许可发布；内置的 future loop 组件派生自 LoopX，
按 Apache-2.0 许可发布。许可证全文与归属声明见 licenses/ 目录。

如遇问题，请把报错信息截图反馈给我们。
