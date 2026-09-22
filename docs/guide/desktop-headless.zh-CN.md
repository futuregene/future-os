# Desktop 无头模式使用指南

[English](desktop-headless.md)

无头模式让 FutureOS Desktop 在终端中提供手机远程入口，不打开窗口或 WebView。
适合通过 SSH 使用服务器，也适合不需要桌面 UI 的本机运行。它复用 Desktop 的会话、
工作区、审批和文件处理，不是另一套 Remote 服务或 SDK。

**默认前台运行、持续占用终端；按 Ctrl+C 关闭远程入口并退出。**
不会自动转入后台，也不会安装系统服务。

## 1. 启动前准备

- Linux `.deb` 和 portable 发布包已包含 `futureos-headless` 及匹配版本的 `future` CLI；
  也可以按第 5 节从源码构建。独立入口不依赖 GTK/WebKit，默认无头启动。
  CLI-only 压缩包仍只提供 `future`，不提供 Desktop 后端入口。
- 将 CLI 放在 `futureos-headless` 旁边或 PATH 中，程序会按需启动本机 Agent。
  也可以提前独立运行 `future agent`，无头后端将连接它而不重复启动。
- 手机安装匹配生产/测试环境的 FutureOS App。编译渠道与手机环境必须一致。
- 主机能够访问 Future OS 平台和配置的 NATS 中继；手机也需要联网。
- 使用普通系统用户运行。平台登录、主机配对和工具执行均属于该运行用户；不要为方便而使用 root。
- 同一数据目录下只能运行一个 Desktop 后端：启动无头模式前，先退出使用该目录的 GUI。

平台登录需要有效的 Future OS 账号凭证，但**不需要在服务器上打开浏览器**。
首次登录和配对需要交互终端，不要将输出重定向到文件。

## 2. 前台启动

`.deb` 安装后可以直接运行 `futureos-headless`；portable 或源码构建请进入该可执行程序所在目录。

Linux / macOS：

```bash
./futureos-headless
```

Windows PowerShell：

```powershell
.\futureos-headless.exe
```

不需要也不接受 `--headless` 参数。`futureos` 只启动图形 Desktop，原来的 `--headless` 选项已移除。
旧命令如 `futureos --headless --no-qr` 应改成 `futureos-headless --no-qr`。
两个程序共享后端、数据和手机协议，不是另一套实现。

无需额外指定 `--pair --qr`。程序会按当前登录与配对状态引导下一步；配对成功后也不会
结束命令或返回 shell 提示符，而是继续处理手机请求。

### 第一步：平台登录授权

没有有效登录时，终端显示 **PLATFORM LOGIN**、二维码、授权网址和用户码。

1. 用手机系统相机或浏览器扫描二维码，或在自己电脑/手机的浏览器中打开授权网址。
2. 登录 Future OS，按页面提示授权当前服务器；若页面要求用户码，输入终端显示的码。
3. 服务器等待授权并通过 Agent 保存凭证，随后继续手机配对。

这一步授权的是“服务器访问你的平台账号”，不是手机对服务器的远程控制。
不需要服务器浏览器、SSH 图形转发或入站浏览器回调端口。

### 第二步：手机 App 配对

尚未配对时，远程入口与 Agent 就绪后，终端显示 **PHONE PAIRING**、第二个二维码和
完整的 `futureos://remote/pair?...` 链接。

- 用 **FutureOS App 内的添加设备/扫码入口**扫描这个二维码。
- 不方便扫码时，在 App 的“粘贴配对码”入口粘贴完整链接。

这里的“配对码”是完整邀请链接，不是几位数字的短码。邀请五分钟内有效且只能使用一次。
不要分享邀请、公开截图或将其保存到共享日志中。

### 已经登录或配对

有效登录和配对会复用，对应步骤自动跳过。已配对的手机直接连接，不需要每次扫码。
“入口就绪/已有配对”不等于手机当前在线；手机需要打开 App 并连接该主机。
普通断网不会主动替换有效配对，网络恢复由现有 Remote 连接逻辑处理。

## 3. 启动选项

| 选项 | 行为 |
|---|---|
| `--no-qr` | 只输出授权/配对链接，不绘制二维码 |
| `--re-pair` | 明确撤销保存的手机配对并创建新邀请 |
| `--help` | 显示帮助，不启动 Desktop 或 Agent |

例如：

```bash
./futureos-headless --no-qr
./futureos-headless --re-pair
```

`--re-pair` 会影响原手机绑定，只在换手机或确需替换配对时使用，不是普通网络故障的修复开关。
终端过窄时，二维码会自动回退为文字链接；可直接复制链接，或扩大终端后重新启动。

## 4. Ctrl+C、SSH 断开与再次启动

Ctrl+C 在登录等待、配对、连接启动和正常运行阶段都可用。退出会先停止新的远程操作、
取消连接恢复工作，再有界清理资源。

- **由本次 Desktop 启动的 Agent**：取消活动对话并停止 Agent；进行中的任务会中断。
- **原本独立运行的 Agent**：不终止它。无头 Desktop 的远程入口仍会关闭。
- **已保存的登录与完成的配对**：保留；下次 `futureos-headless` 启动时可复用。退出不等于解绑。

Unix 也处理 SIGTERM 和 SIGHUP。SSH 断开后不承诺继续运行；若确实需要跨 SSH 会话保活，
可以由用户显式选择 tmux 或系统服务托管，但这不是无头模式的默认行为。

## 5. 无图形依赖的服务器构建

图形程序 `futureos` 链接 GUI 系统库，Linux 加载器在**解析参数前**就需要加载这些库。
独立的 `futureos-headless` 构建排除了 Tauri/GTK/WebKit，不需要 X11/Wayland 会话。
官方 Linux 发布将 `futureos-headless` 构建为完全静态的 musl 二进制，因此也没有
glibc 版本要求，可以在老的企业/HPC 系统（CentOS 7 / Rocky 8 年代）上运行。
本地源码构建使用 host 工具链，不一定完全静态链接。
这条源码构建路径不需要修改发布工作流。

从仓库根目录构建两个程序，无需 npm/Tauri 打包：

```bash
make build-desktop-headless
```

也可以显式构建：

```bash
cargo build --release --no-default-features --features headless \
  --bin futureos-headless --manifest-path desktop/src-tauri/Cargo.toml
cargo build --release -p future-cli
```

默认产物为 `desktop/src-tauri/target/release/futureos-headless`，Windows 后缀为 `.exe`。
将 `target/release/future`（Windows 为 `future.exe`）放在它旁边或 PATH 中；Make 目标会自动复制到同目录。
使用自定义 Cargo 输出目录时，请显式构建并复制相应产物。
`headless` feature 选择独立入口；同时开启 `gui` 会被拒绝，避免生成名字像服务器版、实际仍依赖 GUI 的程序。
通用构建要求见
[构建与安装](build-and-install.zh-CN.md)。

`--release` 是编译优化等级，不决定生产/测试平台渠道；渠道仍遵循项目现有版本与环境策略。
不要修改配对链接来绕过手机的环境校验。

本地验证独立 CLI 与进程生命周期：

```bash
cargo test --no-default-features --features headless \
  --manifest-path desktop/src-tauri/Cargo.toml --test headless_cli
```

在已启动 Docker 的原生 Linux 上，还可以手动检查 ELF 依赖，并在没有 GUI 库的
干净 Ubuntu 24.04 容器中验证启动：

```bash
bash scripts/ci/check-headless-linux.sh desktop/src-tauri/target/release/futureos-headless
```

该检查需要与 Ubuntu 24.04 兼容的构建产物；它是手动验证工具，未接入当前 CI 或发布工作流。

## 6. 数据、权限与排障

无头模式沿用运行用户的现有数据目录，包括 `~/.future/agent/auth.json`、
`~/.future/remote_pairing.json` 和 `~/.future/app/`。本机已经登录，不代表 SSH 服务器已登录；
更换系统用户或 HOME 也不会自动继承原凭证。后台托管时应使用与首次交互设置相同的用户和目录。

| 现象 | 处理方式 |
|---|---|
| 缺少 `libwebkit2gtk-4.1.so.0` | 当前运行的是 GUI 二进制；请构建并运行 `futureos-headless` |
| 提示 `--headless` 已移除或未知参数 | 使用不带该参数的 `futureos-headless`；`--no-qr`、`--re-pair` 属于独立入口 |
| 找不到 Agent | 放置匹配的 `future` CLI，或先独立启动 Agent；显式配置的 Agent 地址不可达时不会被自动接管 |
| 提示需要交互终端 | 首次登录/配对请在真实终端执行，不要使用管道、重定向或非交互服务启动 |
| 平台授权拒绝或过期 | 按提示重新启动并登录，不要把登录码当作 App 配对码 |
| 配对邀请过期 | 重新启动申请邀请；已完成配对无需因此重配 |
| 已有 Desktop 占用目录 | 退出使用该数据目录的 GUI 或另一无头进程；不要删除锁文件来绕过互斥 |
| 手机无法领取邀请 | 检查手机与主机的生产/测试环境是否匹配，确认邀请未过期且未被使用 |
| 平台账号退出、切换或续期授权被拒 | 无头入口会关闭或进入需处理的错误状态；恢复正确登录后重新启动 |

手机操作的工具在主机上执行，沿用原有审批与文件访问规则。无头模式不会自动授予额外权限，
也不等于额外的沙箱隔离。传输与中继信任边界见[手机远程](../wiki/zh/Remote.md)，
权限说明见[审批与沙箱](../wiki/zh/Sandbox.md)。
