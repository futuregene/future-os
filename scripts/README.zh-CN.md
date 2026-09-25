# scripts/

仓库自动化脚本，按用途分目录组织。跨切面的公共工具保留在本目录根部，
因为 Makefile 与 CI 工作流都从这里调用它们：

- `version.mjs` — 构建版本的唯一事实来源。`node scripts/version.mjs` 打印版本号，
  `--set-bundle` 把版本注入 bundle 元数据，Rust 侧的 `build.rs` 也镜像它的解析逻辑。
- `npm-install-if-needed.mjs` — 检查 workspace 安装戳，仅在依赖树变化时执行 `npm install`。

## build/

复刻 CI 流水线的桌面打包脚本。

- `build-desktop-linux.sh` — 构建 Linux `.deb` 与便携 tarball，并复制到 `--out-dir`。
- `build-desktop-macos.sh` — 构建 macOS DMG；存在唯一 Developer ID Application 身份时自动签名，
  支持 Apple 公证选项。
- `build-desktop-windows-installer.ps1` — 通过 `tauri build` 构建 NSIS 安装包，
  可选经由 `../release/sign-file.ps1` 做 Authenticode 签名。
- `build-desktop-windows-portable.ps1` — 构建含 sidecar 二进制的便携 Windows zip。

## dev/

本地开发会话：构建并启动 agent 与一个前端。

- `start-desktop-linux.sh` — Linux Tauri 桌面开发会话，对接本地构建的 agent。
- `start-desktop-macos.sh` — 上者的 macOS 版本。
- `start-desktop-windows.bat` — 上者的 Windows 版本。
- `start-mobile-android.sh` — Expo/Android 开发会话（SDK 检查、prebuild、Gradle 或 Metro）。
- `start-mobile-ios.sh` — Expo/iOS 开发会话（CocoaPods、prebuild、Xcode 构建或 Metro）。

## release/

发布安装器、签名与上传辅助脚本。

- `install.sh` / `install.ps1` — 由下载站托管的一行安装器；自动识别操作系统、
  校验 SHA-256 并安装发布产物。
- `install-future-loop.sh` — 把 `future-loop` CLI 安装到 `~/.local/bin`，
  并把 loop 技能装进 agent 技能目录。
- `sign-file.ps1` — 作为 Tauri `signCommand` 使用的 Authenticode 签名回调。
- `setup-ossutil.sh` — 安装阿里云 ossutil v2，并为 CI 上传推导 `OSS_REGION`。
- `lib/windows-signing.ps1` — 上述脚本共同点源（dot-source）的签名辅助函数。

## measure/

覆盖率、性能剖析与测量工具。

- `coverage.sh` — 基于 `cargo llvm-cov` 的 workspace 覆盖率测量；`--check` 强制执行门槛。
- `coverage_ratchet.py` — 把 llvm-cov JSON 导出聚合为逐 crate 门槛（`emit`）并强制执行（`check`）。
- `coverage-baseline.json` — 逐 crate 行覆盖率下限；只能通过已批准的流程修改。
- `chan-cov.sh` — `future-channel` crate 的逐行覆盖率与未覆盖行报告。
- `chan-missed.py` — 按文件分组打印 channels 未覆盖行。
- `show-lines.py` — 按行号打印带上下文的源码行。
- `agent-profile-bench.sh` / `agent-profile-bench.ps1` — 在负载下采样 agent 并生成火焰图 SVG。
- `profile-isolated.py` — 在一次性用户 HOME 下运行剖析命令，绝不打扰正在运行的 agent。
- `profile-quick.ps1` — Windows 快速 agent 剖析入口（`make profile-quick` 调用）。
- `measure-live-lane.py` — 针对桌面发布器的真实流量通道测量。
- `measure-lean-history.py` — 针对真实会话测量精简历史页（推理正文、工具输出、
  未使用的调用参数），回放经过线上 Rust 裁剪逻辑。
- `measure-mobile-performance.mjs` — 构建离线浏览器 A/B 探针（基线 ref 对比工作树）。
- `measure-mobile-performance.ts` — 导入生产移动端同步代码的探针入口。
- `serve-mobile-performance.py` — 供探针使用的只读 loopback 回放服务器。
- `measure-sync-browser.py` — 隔离启动器：SQLite 快照副本、隔离 agent、loopback 探针。
- `measure-sync-browser.html` — 仅显示指标的同步探针浏览器外壳。
- `measure-sync-browser.ts` — 导入生产 Mobile 同步/投影代码的浏览器入口。
- `measure-sync-snapshot.ts` — A/B 入口：传统原始回放对比快照引导。
- `measure-sync-warm.ts` — 历史轨迹回放入口，用于热同步测量。

## docs/

生成或校验文档本身的脚本。

- `check-docs.py` — 文档结构检查：目录归属、双语配对、本地链接、wiki 目标与代码围栏。
- `check-channel-matrix.py` — 校验渠道矩阵文档与 provider 注册表的一致性。
- `generate_models.py` — 拉取模型目录，重新生成内置模型清单与 wiki 模型页。

## tests/

离线回归测试与原生验收脚本（默认不进入 CI）。

- `test-android-config.py` — 用伪工具测试 `../dev/start-mobile-android.sh` 的主机/架构选择。
- `test-check-docs.py` — `../docs/check-docs.py` 的离线回归测试。
- `test-generate-models.py` — 模型目录生成器的离线回归测试。
- `test-install.ps1` — mock `../release/install.ps1` 的下载边界；绝不真正安装。
- `test-install.py` — `../release/install.sh` 版本/资产解析的离线测试。
- `test-linux-sandbox-real-machine.sh` — 真实机器上的 Linux bubblewrap 沙箱验收。
- `test-mobile-ios-project.cjs` — 验证 prebuild 后生成的 Xcode 工程/权限接线。
- `test-mobile-ios-share.py` — 运行真实 Swift 分享收件箱测试。
- `test-mobile-share-io.py` — 无需 SDK 或模拟器的 Android 分享暂存 IO 测试。
- `test-profile-isolated.py` — `../measure/profile-isolated.py` 的子进程测试。
- `test-windows-installer-preflight.ps1` — 使用下述 fixture 的 NSIS 安装器生命周期验收。
- `test-windows-sandbox.ps1` — Windows 原生沙箱验收（手动调用，不进 CI）。
- `test-windows-sandbox-lifecycle.ps1` — 沙箱生命周期验收动作
  （Snapshot / ExpectBundled / SeedCleanupFixture / ExpectStopped）。
- `test_s2_compaction.py` — 针对已构建 `future` 二进制的本地 S2 压缩冒烟测试。
- `validate-ios-share-profiles.py` — 主/扩展 provisioning profile 缺少共享权限时在归档前失败。
- `test_ios_share_profiles.py` — `validate-ios-share-profiles.py` 的离线单元测试。
- `windows-installer-fixture.cs` / `windows-installer-preflight.nsi` — 供
  `test-windows-installer-preflight.ps1` 使用的 fixture。

## ci/

CI 辅助脚本（由工作流文件调用）。

- `check-headless-linux.sh` — 校验发布二进制为静态链接，然后在无网络、旧 glibc 的
  干净 Linux 镜像中启动它（便携包基线）。
- `install-apt-packages.sh` — 安装仅 CI 使用的 Ubuntu 软件包；Azure runner 镜像
  不可用时快速切换到 Ubuntu 公共归档源。

## screenshots/

用于文档的无头 Chrome 截图/视频工具（见 `docs/guide/screenshots.md`）。

## skill_reco/

技能推荐基准测试装置（bench 脚本、数据集与本地 UI）。

## compaction_experiment/

闭卷/开卷压缩实验（见 `docs/internals/compaction/`）。
