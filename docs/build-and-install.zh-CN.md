# 构建与安装

在 macOS、Linux 和 Windows 上构建与安装 FutureOS——包括 agent 后端、TUI/CLI 前端、桌面 GUI、渠道桥接，以及 loop 控制面（`future-loop`）。

> 关于*使用* FutureOS（模型、技能、启动 agent），请见 [README](../README.zh-CN.md)。

## 环境要求

完整构建（agent + TUI + CLI + GUI）在所有平台都需要的：

- **Rust** 1.97+（由 `rust-toolchain.toml` 固定版本）
- **Node.js** 24+（见 `.nvmrc`）—— 用于 TUI
- **Bun** —— TUI 构建需要（`bun build`）；CLI 已是 Rust（`cargo build`），不再需要 Bun 或 Node
- 可选：**Python 3** —— 仅用于 `make generate-models` 与 CLI golden 差分测试（`make test-cli-diff`）
- 可选：**protoc**（Protocol Buffers 编译器）—— 仅用于 `make generate-proto`；生成代码已入库，正常构建不需要

## 克隆

```bash
git clone https://github.com/futuregene/future-os.git
cd future-os
```

## macOS

安装依赖：

```bash
xcode-select --install                                            # 系统工具链（Tauri）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
brew install node oven-sh/bun/bun                                 # Node.js 24+ / Bun（也可用 nvm——见 .nvmrc）
brew install protobuf                                             # 可选 —— 仅用于 make generate-proto
```

构建：

```bash
make install        # 构建全部并安装到 /opt/homebrew/bin
make install-nogui  # 仅终端栈（跳过 Tauri GUI）
make package-gui    # 桌面打包 → .app + .dmg 位于 gui/src-tauri/target/release/bundle/
scripts/build-macos-dmg.sh  # 本地 DMG；有 Developer ID 证书时自动签名
```

`scripts/build-macos-dmg.sh` 将 agent 与 CLI sidecar 连同 GUI 一起构建。它会自动使用 Keychain 中唯一的 `Developer ID Application` 身份并生成 `*-sign.dmg`；若身份不唯一，则回退到普通 DMG。用 `--help` 查看证书选择、输出目录与 Apple 公证选项。

## Linux（Debian/Ubuntu）

安装依赖：

```bash
sudo apt update
sudo apt install -y build-essential mold libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # Rust
curl -fsSL https://bun.sh/install | bash                          # Bun
# Node.js 24+ —— 用 nvm install 自动读取仓库的 .nvmrc
sudo apt install -y protobuf-compiler                             # 可选 —— 仅用于 make generate-proto
```

> x86_64 上必须装 `mold` —— `.cargo/config.toml` 会向链接器传 `-fuse-ld=mold`。ARM Linux 不需要。

构建：

```bash
make install        # 构建全部并安装到 /usr/local/bin（sudo）
make install-nogui  # 仅终端栈（跳过 Tauri GUI）
make package-gui    # 桌面打包 → .deb 位于 gui/src-tauri/target/release/bundle/
```

## Windows

安装工具链：

1. **Visual Studio Build Tools**，勾选 *Desktop development with C++* 工作负载（MSVC + Windows SDK）—— Rust MSVC 工具链与 Tauri 必需。`winget install Microsoft.VisualStudio.2022.BuildTools`，然后在安装器中选择 C++ 工作负载（或从 [visualstudio.com](https://visualstudio.microsoft.com/downloads/) 安装）。
2. **Rust**：`winget install Rustlang.Rustup`（host triple `x86_64-pc-windows-msvc`）
3. **Node.js 24+**：`winget install OpenJS.NodeJS` 或 [nodejs.org](https://nodejs.org)
4. **Bun**：`winget install Oven-sh.Bun`（或 `powershell -c "irm bun.sh/install.ps1 | iex"`）
5. **WebView2 Runtime**：随 Windows 10/11 自带 —— GUI 的*运行时*依赖，现代系统无需安装

无需 `make` —— 下面的 PowerShell 命令与 make 目标一一对应。在仓库根目录执行。

**终端栈** —— 等价于 `make install-nogui`：

```powershell
# Rust 组件：agent + channel bridge               （对应 make build-agent / build-channels）
cargo build --release --manifest-path agent/Cargo.toml
cargo build --release --manifest-path channels/Cargo.toml

# Rust 组件：TUI + CLI                        （对应 make build-tui / build-cli）
cargo build --release --manifest-path tui/Cargo.toml
cargo build --release --manifest-path cli/Cargo.toml

# 安装到 %USERPROFILE%\.future\bin                （对应 install-* 中的复制步骤）
$bin = "$env:USERPROFILE\.future\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item target\release\future-agent.exe, target\release\future-channel.exe, target\release\future-tui.exe, target\release\future.exe $bin

# 内置技能 —— make install-skills 使用符号链接；Windows 上改用 CLI 安装
& "$bin\future.exe" skills install
```

**桌面应用** —— `make install` 的 GUI 部分（先执行上面终端栈以生成 sidecar）：

```powershell
# 将 agent + CLI 以 host triple 命名暂存为 Tauri sidecar
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
New-Item -ItemType Directory -Force -Path gui\src-tauri\binaries | Out-Null
Copy-Item target\release\future-agent.exe "gui\src-tauri\binaries\future-agent-$triple.exe"
Copy-Item cli\dist\future.exe "gui\src-tauri\binaries\future-$triple.exe"

# 构建应用并安装为 future-gui.exe                 （对应 make install-gui）
Push-Location gui; npm install; npx tauri build --no-bundle; Pop-Location
Copy-Item gui\src-tauri\target\release\futureos.exe "$env:USERPROFILE\.future\bin\future-gui.exe"
```

**安装包** —— 等价于 `make package-gui`（sidecar 就绪后）：

```powershell
node scripts\version.mjs --set-bundle
Push-Location gui; npm run tauri:build; Pop-Location   # → NSIS 安装 .exe 位于 gui\src-tauri\target\release\bundle\nsis\
```

说明：

- `scripts\start-gui-test.bat` 以开发模式针对本地构建的 agent 运行 GUI。
- `scripts/` 下的脚本（`build-macos-dmg.sh`、`build-windows-portable.ps1`、`build-windows-installer.ps1`）把上述步骤封装成单条命令，复刻 CI 打包流水线（DMG / 便携 zip / NSIS 安装器）。它们会预先检查工具链，且需要 `protoc`（`brew install protobuf` / `choco install protoc`）。产物包含 GUI、agent 与 CLI——不含 TUI。

## Loop 控制面（`future-loop`）

loop 控制面位于 `orchestration/loop`，是普通 workspace 成员：

```bash
cargo build -p future-loop                 # 调试构建 → target/debug/future-loop
cargo build -p future-loop --release       # 发布构建 → target/release/future-loop
```

本地安装 CLI 与 `/future-loop` agent 技能：

```bash
bash scripts/install-future-loop.sh        # CLI → ~/.local/bin/future-loop，技能 → ~/.future/agent/skills/
bash scripts/install-future-loop.sh --release
```

若 `~/.local/bin` 不在 `PATH` 中请手动加入，然后验证：

```bash
future-loop status
```

功能与用法见 [loop 控制面指南](loop-control-plane.zh-CN.md)。

## 安装技能（可选）

FutureOS 内置一组精选技能——面向常见任务（深度研究、浏览器自动化、文档处理等）的专用指令。它们维护在 [future-skills](https://github.com/futuregene/future-skills) 仓库：

```bash
make install-skills                          # 从内置 skills/ 子模块符号链接
# 或从平台目录安装：
future skills install                        # 安装全部 future-* 技能（14 个）
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
make build          # 构建全部组件（不安装到系统）
make lint           # 全量 lint：agent、channels、TUI、CLI、GUI（含 stylelint）、mobile
make fmt            # cargo fmt（agent + channels）+ mobile 格式化
make test           # 全部 7 个套件：agent、channels、CLI、TUI、GUI、GUI Rust、mobile
make clean          # 清理构建产物与已安装二进制
```

### Proto

规范 API 是 `future-rpc/proto/future.proto`。生成的 Rust/TS 代码已入库——正常构建不会改动它。编辑 `.proto` 文件后重新生成：

```bash
make generate-proto          # future-rpc/rust + channels + future-rpc/ts
```

## 开发（源码方式）

源码构建使用仓库根目录的 Makefile：

```bash
make build          # 构建全部组件（不安装到系统）
make lint           # 全量 lint（agent + channels + TUI + CLI + GUI）
make fmt            # cargo fmt（agent + channels）
make test           # cargo test（agent）
make clean          # 清理构建产物与已安装二进制
```

### Proto

规范 API 是 `future-rpc/proto/future.proto`。生成的 Rust/TS 代码已入库——正常构建不会改动它。编辑 `.proto` 文件后重新生成：

```bash
make generate-proto          # future-rpc/rust + channels + future-rpc/ts
```
