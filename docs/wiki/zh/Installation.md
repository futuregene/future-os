# 安装 FutureOS

FutureOS 运行于 **macOS 和 Windows**。本页介绍下载、首次启动、数据位置、更新与卸载。

---

## 下载

到 Releases 页下载对应系统的最新版:

> **下载地址:** https://github.com/futuregene/future-os/releases

| 系统 | 下载什么 |
|---|---|
| **macOS** | `.dmg` 磁盘镜像 |
| **Windows** | 安装包(`.exe` / `.msi`),或便携版 `.zip` |

命令行工具 `future` **随每个下载包一起附带** —— 安装包和便携包里都有,就装在应用旁边,无需单独安装。想使用它请看 [[命令行工具(future)|CLI]] 页。

---

## 首次启动

当前版本**未签名 / 未公证**,首次打开时系统会告警,属于正常现象。

### macOS

1. 打开 `.dmg`,把 **FutureOS** 图标拖进「应用程序」文件夹。
2. 在「应用程序」里**右键**(或按住 Control 点击)FutureOS →「打开」,在弹窗里再点一次「打开」。之后即可正常双击启动。
3. 若提示**「已损坏」/「无法打开」**,打开「终端」应用执行下面这行,再打开应用:

   ```bash
   xattr -dr com.apple.quarantine /Applications/FutureOS.app
   ```

### Windows

- **安装版:** 运行 `.exe` / `.msi`,按提示安装。
- **便携版:** 把压缩包**整个文件夹**解压,再双击 `FutureOS.exe`。请把 `FutureOS.exe` 和 `future-agent.exe` 放在**同一文件夹**——后台服务会从这里被自动拉起,所以不要单独移动 `FutureOS.exe`。
- 首次运行时,**SmartScreen** 可能提示「Windows 已保护你的电脑」,点「更多信息 → 仍要运行」。
- FutureOS 需要 **Microsoft Edge WebView2 运行时**。较新的 Windows 10 和 Windows 11 一般已内置。若窗口打不开或提示缺组件,请到微软官网安装「Evergreen」版 WebView2 运行时后重试。

> **便携版(zip)小贴士(Windows):** 若窗口能开但提示后台服务未连接,是下载的 `.zip` 被打上了「来自 Internet」标记。解压前右键 `.zip` →「属性」→ 勾选「解除锁定」→ 确定,再解压。(或解压后在该文件夹里用 PowerShell 执行 `Get-ChildItem -Recurse | Unblock-File`。)

---

## 登录

首次使用需联网,并在**应用内**快速登录。步骤见 [[快速开始|Quick-Start]]。

---

## 数据位置

你的会话和设置保存在主目录下的 `.future` 文件夹中:

| 系统 | 位置 |
|---|---|
| **macOS** | `~/.future` |
| **Windows** | `C:\Users\<你>\.future` |

---

## 更新

下载最新版,覆盖安装到旧版之上(便携版则替换整个文件夹)。你的 `.future` 数据会保留。

你也可以在应用内打开**设置 → 检查更新**,检查新版本并下载。

---

## 卸载

- **macOS:** 从「应用程序」里删除 `FutureOS.app`。
- **Windows:** 从 Windows 设置里卸载(安装版),或删除便携文件夹。

如需一并清除数据,再删除 `.future` 文件夹。

---

## 另见

- [[快速开始|Quick-Start]] —— 登录并发出第一条消息。
- [[常见问题|FAQ]] —— 首次启动告警及其他常见问题。
