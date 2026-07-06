FutureOS 使用说明（macOS）
==========================

【安装】
把本窗口里的 FutureOS 图标拖到「应用程序」文件夹。

【首次打开】
本版本未做 Apple 公证，直接双击会被拦下（提示“来自身份不明的开发者”
或“已损坏”），属正常。二选一打开：
  A（推荐）在「应用程序」里右键（或按住 Control 点击）FutureOS →「打开」
    →在弹窗里再点一次「打开」。之后即可正常双击启动。
  B（若提示“已损坏”）打开「终端」，执行下面这行，再双击打开：
    xattr -dr com.apple.quarantine /Applications/FutureOS.app

【说明】
· 首次使用需联网登录（在应用内完成）。
· 个人数据保存在 ~/.future ；退出应用时后台 future-agent 会一并关闭。
· 已附带命令行工具 future-cli（位于 FutureOS.app/Contents/MacOS/）。

如遇问题，请把报错弹窗截图反馈给我们。
