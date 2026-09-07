# 常见问题与排错

常见问题速查。若仍卡住,可以[反馈问题](https://github.com/futuregene/future-os/issues)。

---

### macOS 打不开(「身份不明的开发者」/「已损坏」)

官方 release 构建已签名并公证，未签名测试包属于独立渠道。先核对来源与产物渠道；正式签名版出现意外警告时，应重新下载并反馈，不要直接移除隔离标记。以下步骤仅用于可信的未签名测试包：

- 在「应用程序」里**右键**(或按住 Control 点击)**FutureOS** →「打开」→ 再点一次「打开」。首次之后即可正常启动。
- 若提示**「已损坏」**,在「终端」应用里执行下面这行一次,再打开:

  ```bash
  xattr -dr com.apple.quarantine /Applications/FutureOS.app
  ```

### Windows 提示「Windows 已保护你的电脑」

这是 **SmartScreen**，签名软件也可能遇到信誉警告。先核对官方来源和发布者，仅在信任产物时选择「更多信息 → 仍要运行」；发布者或签名不符时不要绕过。

### Windows:启动后没反应

- 到微软官网安装 **Microsoft Edge WebView2 运行时**(Evergreen 版)——应用需要它。较新的 Windows 10 和 Windows 11 一般已内置。
- **便携版**请确认 `FutureOS.exe` 与 `future.exe` 在**同一文件夹**。
- 若窗口能开但提示后台服务未连接,是 `.zip` 被打上了「来自 Internet」标记。右键 `.zip` →「属性」→ 勾选「解除锁定」→ 重新解压。

### 用不了任何模型 / 未登录

打开**设置 → Providers → FutureGene → Sign in** 登录,或添加自己的 provider。见 [[设置|Settings]]。

### 怎么切换模型?

用输入框里的**模型选择器**,或在**设置 → Models** 里管理哪些模型出现。

### agent 停下来问我东西

所选审批模式/规则要求你决定。选**允许一次**、**拒绝**或在可用时保存项目规则；没有超时。默认不受限模式不会询问，保护模式也并非每次工具调用都弹卡。见 [[审批与沙箱|Sandbox]] 和 [[使用 FutureOS|Using-FutureOS]]。

### 会话和设置存在哪?

在主目录下的 `.future` 文件夹里:

- **macOS/Linux:** `~/.future`
- **Windows:** `C:\Users\<你>\.future`

### 怎么更新?

下载最新版覆盖安装到旧版之上(便携版则替换整个文件夹)。你的 `.future` 数据会保留。也可以从**设置 → 检查更新**里查看。

### 怎么卸载 / 清除数据?

删除应用（macOS：删除 `FutureOS.app`；Windows：卸载或删除便携文件夹；Linux：deb 安装用 `sudo apt remove futureos`，便携版删除文件）。如需一并清除数据,再删除 `.future` 文件夹。在应用内,**设置 → 重置(Reset)**也能清除本地数据。

### 支持哪些平台?

**桌面支持 macOS、Windows 和 Linux；Android/iOS 通过 [[手机远程|Remote]] 使用。** Linux 发布包含 x86_64/aarch64 桌面与 CLI-only 版本，见 [[安装|Installation]]。

### Linux 沙箱不可用

安装可信系统 Bubblewrap ≥ 0.9.0，完全重启 FutureOS，再运行 `future agent --probe-sandbox` 和 `future doctor`。主机策略可能限制 user namespace 或 fresh `/proc` 挂载，请管理员评估，不要绕过策略。诊断 code 与手动回退见 [[审批与沙箱|Sandbox]]。

### 旧服务器上 Linux GUI 无法启动

发布的 GUI 需要 glibc ≥ 2.39 和 WebKitGTK 4.1。无桌面主机可改用对应架构的官方静态 CLI-only 包，见 [[安装|Installation]]。

---

## 另见

- [[安装 FutureOS|Installation]]
- [[快速开始|Quick-Start]]
- [[使用 FutureOS|Using-FutureOS]]
