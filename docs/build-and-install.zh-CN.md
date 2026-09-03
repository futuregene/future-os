# 构建与安装

在 macOS、Linux 和 Windows 上构建与安装 FutureOS——包括 agent 后端、TUI/CLI 前端、桌面 GUI、渠道桥接，以及 loop 控制面（`future-loop`）。

> 关于*使用* FutureOS（模型、技能、启动 agent），请见 [README](../README.zh-CN.md)。

## 环境要求

完整构建（agent + TUI + CLI + GUI）在所有平台都需要的：

- **Rust** 1.97+（由 `rust-toolchain.toml` 固定版本）
- **Node.js** 24+（见 `.nvmrc`）—— 用于 GUI 前端
- 可选：**Python 3** —— 仅用于 `make generate-models` 与 CLI golden 差分测试（`make test-cli-diff`）
- 可选：**protoc**（Protocol Buffers 编译器）—— 仅用于 `make generate-proto`；生成代码已入库，正常构建不需要

TUI 与 CLI 均为 Rust（`cargo build`），不再需要 Bun 或 Node。

## 克隆

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
make setup    # 安装 JS 依赖（desktop/mobile）+ 初始化 skills 子模块 + 创建 sidecar 占位文件
```

`make setup` 只准备仓库级依赖：安装共享的 `thread-projection`、desktop/mobile
JavaScript 依赖、初始化 `skills` 子模块，并创建空的 Tauri sidecar 占位文件，
使 `desktop/src-tauri` 的 `cargo check` / `clippy` / `test` 可在首次 `cargo build`
前运行。应用启动目标仍需要相应平台的 SDK 与模拟器或真机。

## macOS

安装依赖：

```bash
xcode-select --install                                            # 系统工具链（Tauri）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
source "$HOME/.cargo/env"                                         # 使 cargo/rustc 在当前 shell 中可用
brew install node                                                 # Node.js 24+（也可用 nvm——见 .nvmrc）
brew install protobuf                                             # 可选 —— 仅用于 make generate-proto
```

构建：

```bash
make install        # GUI + 统一 `future` CLI + 技能（agent/tui/channel/loop 已内嵌）→ /opt/homebrew/bin
make install-cli    # 仅统一 `future` CLI
make install-desktop    # 仅桌面应用（自带 agent/CLI sidecar）
make install-skills # 内置技能 + /future-loop 技能
make package-desktop    # 桌面打包 → .app + .dmg 位于 desktop/src-tauri/target/release/bundle/
scripts/build-desktop-macos.sh  # 本地 DMG；有 Developer ID 证书时自动签名
```

`scripts/build-desktop-macos.sh` 将统一 `future` CLI sidecar 连同 GUI 一起构建。它会自动使用 Keychain 中唯一的 `Developer ID Application` 身份并生成 `*-sign.dmg`；若身份不唯一，则回退到普通 DMG。用 `--help` 查看证书选择、输出目录与 Apple 公证选项。

## Linux（Debian/Ubuntu）

### 终端用户 —— 预编译包

一行安装器下载预编译版本（无需本地构建）：

```bash
curl -fsSL https://dl.future-os.cn/install.sh | bash
```

脚本自动识别平台并从发布清单安装匹配的包，校验 SHA-256，然后执行
`future init`：

- **Debian/Ubuntu** —— `FutureOS_<version>_amd64.deb`，通过 `apt` 安装（自动解析依赖）。
- **其他 Linux** —— `FutureOS-portable-linux.tar.gz`（`futureos` 桌面应用 + 统一 `future` CLI），解压到 `/usr/local/bin`（不可写时使用 `~/.local/bin`）。

用 `FUTUREOS_VERSION` 锁定特定版本（如 `FUTUREOS_VERSION=1.2.0`），或用 `FUTUREOS_BASE` 指向镜像。

### 从源码构建（开发者）

安装工具链：

```bash
sudo apt update
sudo apt install -y build-essential mold libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust（rust-toolchain.toml 锁定 1.97.0）
source "$HOME/.cargo/env"                                         # 使 cargo/rustc 在当前 shell 中可用
sudo apt install -y protobuf-compiler                             # 可选 —— 仅用于 make generate-proto
```

> x86_64 上必须装 `mold` —— `.cargo/config.toml` 会向链接器传 `-fuse-ld=mold`。ARM Linux 不需要。

构建与安装：

```bash
scripts/build-desktop-linux.sh --out-dir ./dist   # → ./dist/FutureOS_<version>_amd64.deb + FutureOS-portable-linux.tar.gz
scripts/start-desktop-linux.sh                    # 本地 Linux Desktop + agent 开发会话
make install        # 或直接从源码安装：GUI + 统一 `future` CLI + 技能（agent/tui/channel/loop 已内嵌）→ /usr/local/bin（sudo）
make install-cli    # 仅统一 `future` CLI
make install-desktop    # 仅桌面应用（自带 agent/CLI sidecar）
make install-skills # 内置技能 + /future-loop 技能
make package-desktop    # 桌面打包 → .deb 位于 desktop/src-tauri/target/release/bundle/
```

`scripts/start-desktop-linux.sh` 会以开发模式针对本地构建的 agent 运行 GUI，
并在 GUI 退出后停止由脚本启动的 agent。Bubblewrap 检查只作提示，以便同时测试
沙盒不可用时的界面。

## Windows

安装工具链：

1. **Visual Studio Build Tools**，勾选 *Desktop development with C++* 工作负载（MSVC + Windows SDK）—— Rust MSVC 工具链与 Tauri 必需。`winget install Microsoft.VisualStudio.2022.BuildTools`，然后在安装器中选择 C++ 工作负载（或从 [visualstudio.com](https://visualstudio.microsoft.com/downloads/) 安装）。
2. **Rust**：`winget install Rustlang.Rustup`（host triple `x86_64-pc-windows-msvc`）
3. **Node.js 24+**：`winget install OpenJS.NodeJS` 或 [nodejs.org](https://nodejs.org)
4. **WebView2 Runtime**：随 Windows 10/11 自带 —— GUI 的*运行时*依赖，现代系统无需安装

无需 `make` —— 下面的 PowerShell 命令与 make 目标一一对应。在仓库根目录执行。

**终端栈** —— 等价于 `make install-cli install-skills`：只需统一 `future` CLI（agent/tui/channel/loop
已内嵌其中），技能由 CLI 自行安装：

```powershell
# Rust CLI —— 统一二进制（对应 make build-cli）
cargo build --release --manifest-path cli/Cargo.toml

# 安装到 %USERPROFILE%\.future\bin                （对应 install-cli 的复制步骤）
$bin = "$env:USERPROFILE\.future\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item target\release\future.exe $bin

# 内置技能 —— make install-skills 使用符号链接；Windows 上改用 CLI 安装
& "$bin\future.exe" skills install
```

**桌面应用** —— `make install` 的 desktop 部分（自身暂存 sidecar，对应 `make desktop-sidecars`；只暂存统一 `future` CLI，GUI 通过 `future agent` 启动 agent）：

```powershell
# 将统一 CLI 以 host triple 命名暂存为 Tauri sidecar
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
New-Item -ItemType Directory -Force -Path desktop\src-tauri\binaries | Out-Null
Copy-Item target\release\future.exe "desktop\src-tauri\binaries\future-$triple.exe"

# 构建应用并安装为 future-desktop.exe                 （对应 make install-desktop）
Push-Location desktop; npm install; npx tauri build --no-bundle; Pop-Location
Copy-Item desktop\src-tauri\target\release\futureos.exe "$env:USERPROFILE\.future\bin\future-desktop.exe"
```

**安装包** —— 等价于 `make package-desktop`（sidecar 就绪后）：

```powershell
node scripts\version.mjs --set-bundle
Push-Location desktop; npm run tauri:build; Pop-Location   # → NSIS 安装 .exe 位于 desktop\src-tauri\target\release\bundle\nsis\
```

说明：

- `scripts\start-desktop-windows.bat` 以开发模式针对本地构建的 agent 运行 GUI。
- `scripts/` 下的脚本（`build-desktop-macos.sh`、`build-desktop-windows-portable.ps1`、`build-desktop-windows-installer.ps1`）把上述步骤封装成单条命令，复刻 CI 打包流水线（DMG / 便携 zip / NSIS 安装器）。它们会预先检查工具链，且需要 `protoc`（`brew install protobuf` / `choco install protoc`）。产物包含 GUI 与统一 `future` CLI（agent/tui/channel/loop 已内嵌）——不含单独的 TUI。

## Loop 控制面（`future-loop`）

loop 控制面位于 `orchestration/loop`，是普通 workspace 成员：

```bash
cargo build -p future-loop                 # 调试构建 → target/debug/future-loop
cargo build -p future-loop --release       # 发布构建 → target/release/future-loop
```

若要让 agent 使用 `/future-loop` 技能（无需构建——控制面已内嵌在统一 `future` CLI）：

```bash
make install-skills                    # 内置技能 + /future-loop 技能 → ~/.future/agent/skills/
```

可选：另行安装独立 `future-loop` 二进制（开发用途）：

```bash
bash scripts/install-future-loop.sh        # CLI → ~/.local/bin/future-loop，技能 → ~/.future/agent/skills/
bash scripts/install-future-loop.sh --release
```

验证：

```bash
future loop status        # 主要入口（与 `future-loop status` 同一套代码）
```

> 所有 Rust 组件都可通过统一的 `future` CLI 调用：`future agent`、`future tui`、
> `future channel`、`future loop`——每个都运行与独立二进制（`future-agent`、
> `future-tui`、`future-channel`、`future-loop`）完全相同的代码。独立二进制仍可通过
> `cargo build -p <crate>` 构建、`make run-*` 运行（开发用途）；独立 `future-loop`
> 二进制可用 scripts/install-future-loop.sh 安装。

功能与用法见 [loop 控制面指南](loop-control-plane.zh-CN.md)。

## 安装技能（可选）

FutureOS 内置一组精选技能——面向常见任务（深度研究、浏览器自动化、文档处理、
长程目标编排 `/future-loop` 等）的专用指令。它们维护在
[future-skills](https://github.com/futuregene/future-skills) 仓库：

```bash
make install-skills                          # 从内置 skills/ 子模块符号链接
# 或从平台目录安装：
future skills install                        # 安装全部 future-* 技能（15 个）
future init                                  # 安装技能并在 macOS/Linux 上链接本地命令
```

> 技能以符号链接放入 `~/.future/agent/skills/`，agent 会自动发现。用 `future skills list` 查看可用技能，`future skills update` 升级。

## 验证

```bash
make test        # 全部 7 个套件：agent、channels、CLI、TUI、GUI、GUI Rust、mobile
make lint        # 全量 lint：agent、channels、TUI、CLI、GUI（含 stylelint）、mobile
```

## 开发（源码方式）

源码构建使用仓库根目录的 Makefile：

```bash
make build          # 构建 GUI + 统一 CLI（不安装到系统；agent/tui/channel/loop 内嵌于 future，GUI 自带 sidecar）
make lint           # 全量 lint：agent、channels、TUI、CLI、GUI（含 stylelint）、mobile
make fmt            # cargo fmt --all（workspace）+ desktop/src-tauri + mobile 格式化
make test           # 全部 7 个套件：agent、channels、CLI、TUI、GUI、GUI Rust、mobile
make clean          # 清理构建产物（已安装二进制请用 make uninstall 删除）
```

### 从源码启动桌面应用

完成上面的环境要求与克隆后，运行：

```bash
make setup
make run-desktop
```

`make setup` 安装 JavaScript workspace、初始化内置技能并创建 Tauri sidecar
占位文件；`make run-desktop` 构建本地 CLI sidecar 并以开发模式启动桌面应用——
应用启动时会自动拉起内置的 agent sidecar，无需单独运行 `future agent`
（若 `127.0.0.1:50051` 上已有 agent 在跑，应用会直接复用而不重复拉起）。
Android 与 iOS 还需要各自的 SDK 和模拟器或真机，详见[移动端指南](../mobile/README.md)。

### Proto

规范 API 是 `packages/rpc/proto/future.proto`。生成的 Rust 代码已入库——正常构建不会改动它。编辑 `.proto` 文件后重新生成：

```bash
make generate-proto          # future-rpc + channels
```
