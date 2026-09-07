# 安装 FutureOS

桌面应用支持 **macOS、Windows 和 Linux**。Android/iOS 客户端通过 [[手机远程|Remote]]
连接运行中的桌面应用。

## 下载

请使用官方下载渠道或 [GitHub Releases](https://github.com/futuregene/future-os/releases)，
选择与机器相符的版本和架构：

| 系统 | 发布产物 |
|---|---|
| macOS arm64 / Intel | `.dmg` 磁盘映像 |
| Windows x64 | `.exe` 安装程序或 `.zip` 便携包 |
| Linux x86_64 / aarch64 | `.deb`（`amd64` / `arm64`）、桌面 portable 压缩包或无界面 CLI 压缩包 |

Linux 发布文件名包含版本和架构：`FutureOS_<version>_amd64.deb`、
`FutureOS_<version>_arm64.deb`、`FutureOS_<version>_linux_<arch>-portable.tar.gz`、
`FutureOS_<version>_linux_<arch>-cli.tar.gz`（`arch` 为 `x86_64` 或 `aarch64`）。
操作时使用实际下载文件名，不要照抄占位符。本地开发构建可能使用不同名称。

每份桌面下载都包含统一 `future` CLI，它同时运行 Agent、TUI 和渠道桥。
Linux CLI-only 下载不含 GUI。用法见 [[命令行工具|CLI]]。

### 一行安装

macOS/Linux：

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

Windows PowerShell：

```powershell
iex (irm https://dl.future-os.cn/install.ps1)
```

Debian/Ubuntu 使用匹配的 `.deb`；其他 Linux 使用 portable 桌面包。脚本最后执行
`future init`。无桌面服务器请改用 CLI-only 压缩包，不必安装桌面运行库。

## 首次启动与签名

官方 release 工作流对 macOS/Windows 安装包签名，并对 macOS 产物进行公证。
未签名测试包属于另一个渠道，可能触发首次启动警告。请核对下载的产物和渠道，不能认为
所有构建都未签名，也不要把意外签名警告一概当成正常现象。见 [[常见问题|FAQ]]。

### macOS

打开 `.dmg`，将 **FutureOS** 拖到**应用程序**并启动。可信未签名测试包若被拦截，
按随包首次启动说明操作；正式签名版若出现意外警告，应先核对来源并重新下载，
不要直接绕过系统保护。

### Windows

- **安装版：**运行 `.exe` 并按提示安装。
- **便携版：**解压整个目录，运行 `FutureOS.exe`，保持它与 `future.exe` 在同一目录；
  应用通过该程序启动后台 Agent。
- GUI 依赖 **Microsoft Edge WebView2 Runtime**。较新的 Windows 10/11 通常已有，
  缺失时安装微软 Evergreen 运行时。
- 签名包也可能遇到 SmartScreen 信誉提示，请先核对发布者和来源。
  可信 portable zip 若后台服务被拦截，可在解压前通过属性 → **解除锁定**处理。

### Linux

- **Debian/Ubuntu：**使用 `sudo apt install ./<实际下载文件>.deb` 安装，然后从应用菜单启动。
- **桌面便携版：**解压实际下载的 tarball，保持 `futureos` 与 `future` 同目录，运行 `./futureos`。
- **无界面 CLI：**解压 CLI-only 包，运行 `./future config`，再运行 `./future tui`（需要时自动
  启动 Agent）；也可手动运行 `./future agent` 供需要 Agent 的 CLI 命令使用。按需将目录加入 PATH。

发布的 GUI 包需要较新的 glibc 发行版（glibc ≥ 2.39，约 Ubuntu 24.04+）和 WebKitGTK 4.1。
portable 用户可能需要 `sudo apt install libwebkit2gtk-4.1-0` 或
`sudo dnf install webkit2gtk4.1`。官方 Linux CI 使用静态 musl 构建 CLI，CLI 不需要这些
GUI 运行库；本地源码构建的依赖取决于所选 target。

沙箱模式需要安装**系统 Bubblewrap ≥ 0.9.0**，重启应用后运行
`future agent --probe-sandbox` / `future doctor`。它不随应用捆绑，旧发行版包可能需要可信
渠道升级。要求、namespace 限制和回退行为见 [[审批与沙箱|Sandbox]]。

## 配置与使用

使用 FutureOS 托管模型可在应用内登录，也可以配置自己的 provider/key，见
[[快速开始|Quick-Start]]。桌面默认**不受限**；执行任务前按需选择手动或沙箱模式。

## 数据、更新与卸载

持久数据位于 macOS/Linux 的 `~/.future` 或 Windows 的 `C:\Users\<你>\.future`。
模型和在线功能仍会向对应服务发送请求；本地存储不代表仅离线处理。

安装版可通过**设置 → 检查更新**使用签名校验后的更新。Linux `.deb` 更新在校验后调用
系统包管理器。便携版可替换解压文件更新，用户数据单独保存。

卸载时，macOS 删除应用；Windows 用系统设置卸载或删除便携目录；Linux deb 使用
`sudo apt remove futureos`，手动安装则删除 portable/CLI 文件。不需要卸载系统 Bubblewrap。
仅当确实要删除会话、凭据和设置时再删除 `.future`；先备份需要保留的内容。

另见 [[常见问题|FAQ]]、[[命令行工具|CLI]]、[[审批与沙箱|Sandbox]] 和 [[手机远程|Remote]]。
