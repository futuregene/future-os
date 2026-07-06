# 常见问题与排错

常见问题速查。若仍卡住,可以[反馈问题](https://github.com/futuregene/future-os/issues)。

---

### macOS 打不开(「身份不明的开发者」/「已损坏」)

当前版本未公证,这属于正常现象。

- 在「应用程序」里**右键**(或按住 Control 点击)**FutureOS** →「打开」→ 再点一次「打开」。首次之后即可正常启动。
- 若提示**「已损坏」**,在「终端」应用里执行下面这行一次,再打开:

  ```bash
  xattr -dr com.apple.quarantine /Applications/FutureOS.app
  ```

### Windows 提示「Windows 已保护你的电脑」

这是 **SmartScreen**。点「更多信息 → 仍要运行」。

### Windows:启动后没反应

- 到微软官网安装 **Microsoft Edge WebView2 运行时**(Evergreen 版)——应用需要它。较新的 Windows 10 和 Windows 11 一般已内置。
- **便携版**请确认 `FutureOS.exe` 与 `future-agent.exe` 在**同一文件夹**。
- 若窗口能开但提示后台服务未连接,是 `.zip` 被打上了「来自 Internet」标记。右键 `.zip` →「属性」→ 勾选「解除锁定」→ 重新解压。

### 用不了任何模型 / 未登录

打开**设置 → Providers → FutureGene → Connect** 登录,或添加自己的 provider。见 [[设置|Settings]]。

### 怎么切换模型?

用输入框里的**模型选择器**,或在**设置 → Models** 里管理哪些模型出现。

### agent 停下来问我东西

那是**批准机制**——agent 在有风险的操作前会暂停等你(不设超时)。选 **Allow once(允许一次)**、**Deny(拒绝)**,或(在可用时)为本项目允许。见 [[使用 FutureOS|Using-FutureOS]]。

### 会话和设置存在哪?

在主目录下的 `.future` 文件夹里:

- **macOS:** `~/.future`
- **Windows:** `C:\Users\<你>\.future`

### 怎么更新?

下载最新版覆盖安装到旧版之上(便携版则替换整个文件夹)。你的 `.future` 数据会保留。也可以从**设置 → 检查更新**里查看。

### 怎么卸载 / 清除数据?

删除应用(macOS:删除 `FutureOS.app`;Windows:卸载或删除便携文件夹)。如需一并清除数据,再删除 `.future` 文件夹。在应用内,**设置 → 重置(Reset)**也能清除本地数据。

### 支持哪些平台?

**macOS 和 Windows。**

---

## 另见

- [[安装 FutureOS|Installation]]
- [[快速开始|Quick-Start]]
- [[使用 FutureOS|Using-FutureOS]]
