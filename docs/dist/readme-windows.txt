FutureOS 免安装版使用说明（Windows）
====================================

【启动】
把压缩包整个解压到任意文件夹（如 D:\FutureOS），双击 FutureOS.exe 运行，
无需安装。注意：FutureOS.exe 与 future-agent.exe 必须在同一文件夹，勿单独
移动 —— future-agent.exe 是后台服务，启动时会被自动拉起。

【首次运行】
· 若弹出「Windows 已保护你的电脑」（SmartScreen）：点「更多信息」→「仍要运行」。
· 若窗口能开、但提示“后台服务未连接”：是下载的压缩包被打上了“来自 Internet”
  标记。推荐解决：解压前右键 .zip →「属性」→勾选底部「解除锁定」→确定→再解压；
  或解压后在该文件夹里打开 PowerShell 执行：
    Get-ChildItem -Recurse | Unblock-File

【运行环境】
需要 Microsoft Edge WebView2 运行时（Win10 较新版本 / Win11 一般自带）。
若双击后无窗口或提示缺组件，请到微软官网安装「Microsoft Edge WebView2
Runtime」（Evergreen 版），再重新运行。

【说明】
· 首次使用需联网登录。个人数据保存在 C:\Users\<用户名>\.future 。
· 关闭窗口即退出，后台 future-agent 会一并关闭。
· 已附带命令行工具 future.exe（同目录）。

如遇问题，请把报错窗口截图反馈给我们。
