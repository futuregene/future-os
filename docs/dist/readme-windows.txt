FutureOS 免安装版使用说明（Windows）
====================================

【启动】
把压缩包整个解压到任意文件夹（如 D:\FutureOS），双击 FutureOS.exe 运行，
无需安装。注意：FutureOS.exe 与 future.exe 必须在同一文件夹，勿单独移动 ——
后台 agent 由 future.exe（`future agent`）自动拉起。

【首次运行】
· SmartScreen 信誉提示也可能出现在签名包上。先核对官方来源和发布者，仅在信任
  产物时选择「更多信息」→「仍要运行」；不要绕过意外的签名或发布者不符。
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
· 应用退出时仅关闭自己启动的 Agent，外部管理的 Agent 保持运行。
· 桌面默认不受限；需要审批/写保护时请在设置中选择。在线功能仍向对应服务发送请求。
· 已附带命令行工具 future.exe（同目录）。

【许可】
FutureOS 主体按 MIT 许可发布；内置的 future loop 组件派生自 LoopX，
按 Apache-2.0 许可发布。许可证全文与归属声明见 licenses/ 目录。

如遇问题，请把报错窗口截图反馈给我们。
