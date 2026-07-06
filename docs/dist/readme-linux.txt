FutureOS 免安装版使用说明（Linux）
==================================

【启动】
解压到任意目录，三个文件保持在同一文件夹：
    tar -xzf FutureOS-portable-linux.tar.gz
    ./futureos
注意：futureos、future-agent、future-cli 必须在同一文件夹，勿单独移动 ——
future-agent 是后台服务，启动时会被自动拉起。

【运行环境】
需要系统的 WebKitGTK 运行库。若启动时报缺少库，用包管理器安装：
    Debian/Ubuntu:  sudo apt install libwebkit2gtk-4.1-0
    Fedora:         sudo dnf install webkit2gtk4.1

【说明】
· 首次使用需联网登录（在应用内完成）。个人数据保存在 ~/.future 。
· 退出应用时后台 future-agent 会一并关闭。
· 已附带命令行工具 future-cli（同目录）。

如遇问题，请把报错信息截图反馈给我们。
