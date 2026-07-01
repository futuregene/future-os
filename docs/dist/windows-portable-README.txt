FutureOS 免安装版（Windows Portable）使用说明
================================================

一、如何启动
------------
1. 把压缩包整个解压到任意文件夹（例如 D:\FutureOS）。
2. 双击文件夹里的 FutureOS.exe 即可运行，无需安装。

重要：FutureOS.exe 和 future-agent.exe 必须放在同一个文件夹里，
不要单独移动其中一个。future-agent.exe 是后台服务，FutureOS 启动时
会自动把它拉起来。

二、首次运行的安全提示（SmartScreen）
------------------------------------
本版本未做代码签名，首次运行时 Windows 可能弹出
“Windows 已保护你的电脑”。这是正常的：
  点击“更多信息” → 点击“仍要运行”。

二·五、如果窗口能打开、但提示“后台服务未连接 / agent 不可用”
------------------------------------------------------------
从网上下载的压缩包，Windows 会给里面每个文件打上“来自 Internet”的
标记（Mark-of-the-Web）。这会导致：你手动放行了 FutureOS.exe，但它
去启动 future-agent.exe 时，后者仍被系统拦下，于是应用能开、后台却连
不上。解决办法二选一：

  方法 A（推荐，解压前处理一次即可）：
    右键点“下载下来的 .zip 压缩包” → 属性 →
    勾选底部的“解除锁定（Unblock）” → 确定 → 再解压。
    这样解压出来的所有文件都不带标记。

  方法 B（已经解压了，用 PowerShell 批量解除）：
    在解压出的文件夹里，按住 Shift 右键 → “在此处打开 PowerShell
    窗口”，执行：
      Get-ChildItem -Recurse | Unblock-File
    然后重新双击 FutureOS.exe。

  方法 C（只解锁两个 exe）：
    分别右键 FutureOS.exe 和 future-agent.exe → 属性 →
    勾选“解除锁定” → 确定。

三、运行环境要求（WebView2）
---------------------------
FutureOS 需要 Microsoft Edge WebView2 运行时。
Windows 10（较新版本）和 Windows 11 一般已自带。
如果双击后没有出现窗口、或提示缺少组件，请到微软官网搜索并安装：
  “Microsoft Edge WebView2 Runtime”（选 Evergreen 版本），
安装后再次双击 FutureOS.exe。

四、登录与数据
-------------
- 首次使用需联网登录（在应用内完成）。
- 个人数据保存在：C:\Users\<你的用户名>\.future
  卸载就是直接删掉解压出来的文件夹；如需彻底清除数据，
  再删除上面的 .future 目录。

五、退出
-------
关闭窗口即退出，后台的 future-agent 会一并关闭。

如遇问题，请把上面提到的窗口/报错截图反馈给我们。
