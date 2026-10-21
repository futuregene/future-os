FutureOS 免安装版使用说明（Windows）
====================================

【启动】
把压缩包整个解压到任意文件夹（如 D:\FutureOS），双击 FutureOS.exe 运行，
无需安装。注意：FutureOS.exe 与 future.exe 必须在同一文件夹，勿单独移动 ——
Desktop 默认在自身进程中运行完整 Agent；future.exe 用于技能、维护和独立 CLI。

【首次运行】
· SmartScreen 信誉提示也可能出现在签名包上。先核对官方来源和发布者，仅在信任
  产物时选择「更多信息」→「仍要运行」；不要绕过意外的签名或发布者不符。
· 若提示后台服务不兼容或实例锁占用，先确认是否已有 GUI、主机模式或独立 Agent
  使用同一数据目录，正常退出或升级该服务后重开；不要删除锁文件绕过检查。
  Desktop 不会在外部服务不兼容或持锁不可用时启动第二份 Agent。

【运行环境】
需要 Microsoft Edge WebView2 运行时（Win10 较新版本 / Win11 一般自带）。
若双击后无窗口或提示缺组件，请到微软官网安装「Microsoft Edge WebView2
Runtime」（Evergreen 版），再重新运行。

【说明】
· 首次使用需联网登录。个人数据保存在 C:\Users\<用户名>\.future 。
· 退出内置宿主会取消所有客户端的活动、排队和审批任务，包括 CLI/TUI 对话；
  有任务时 GUI 会提示确认。连接外部 Agent 时只取消本 Desktop 的运行，外部服务保持运行。
· 桌面默认是完全访问模式；需要审批/写保护时请在设置中选择。在线功能仍向对应服务发送请求。
· 已附带命令行工具 future.exe（同目录）。

【许可】
FutureOS 主体按 MIT 许可发布；内置的 future goal 组件派生自 LoopX，
按 Apache-2.0 许可发布。许可证全文与归属声明见 licenses/ 目录。

如遇问题，请把报错窗口截图反馈给我们。
