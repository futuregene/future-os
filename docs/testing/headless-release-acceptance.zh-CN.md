# Headless 后置发布验收清单

> [English](headless-release-acceptance.md)
>
> 2026-10-20：实现已随 [PR #1259](https://github.com/futuregene/future-os/pull/1259) 合入，普通 CI 全部完成且成功。用户授权的 dev unsigned 构建已全部成功，公开下载与产物证据见[结果清单](headless-unsigned-results.zh-CN.md)；平台签名由用户稍后执行，真实安装尚未执行。

## 已确认与候选身份

- 自动化候选：`dac8832f725f7f2813cf2cc453d0f847b60f7cfa`。
- squash 合并：`a5f91752b4e470f90fbe5f4bc303ec0ca814944d`，合并时已核对两者源码树一致。
- [最终 CI 39408277224](https://github.com/futuregene/future-os/actions/runs/39408277224)：19 个成功、2 个可选性能检查跳过、0 个失败，均已结束。Windows Host 1693 通过、0 失败、10 个原忽略；Linux/macOS Host 各 1723 通过、0 失败、10 个原忽略。
- 已通过 Windows 安装器夹具、macOS 公证恢复逻辑、原生入口/信号、共享核心和依赖边界检查。这些不能替代实际签名产物、真机安装/升级或公证服务结果。
- 用户报告人工体验完成，关联旧体验提交 `a0d4d840`；不是后续候选的逐项/全平台签名验收。详见[人工记录](headless-manual-acceptance.zh-CN.md)与[最终 CI 证据](headless-ci-39408277224-2026-10-20.json)。

后置构建以已验证合并提交为起点；如采用更新源码，先固定新的完整 SHA 并完成受影响自动化，再以同一候选重新核对人工和产物。不得用当前移动的 main、旧 debug 二进制及不同 run 的产物拼接验收。


## 本轮 dev unsigned 构建（2026-10-20）

用户授权先做 unsigned，系统签名/公证测试稍后由用户执行。为避开总协调流程的 iOS 签名和 TestFlight 上传，本轮直接运行三个现有平台 workflow，固定远程引用 `test/headless-unsigned-20261020` 到已通过普通 CI 的 `a5f91752b4e470f90fbe5f4bc303ec0ca814944d`，不跟随移动 main。

| 平台 | 运行 | 显式参数 | 最终状态 |
|---|---|---|---|
| macOS Intel / Apple Silicon | [39418462362](https://github.com/futuregene/future-os/actions/runs/39418462362) | `signed=false`、`strict_verification=false` | 两架构成功 |
| Windows | [39418473647](https://github.com/futuregene/future-os/actions/runs/39418473647) | `signed=false` | 成功 |
| Linux x86_64 / aarch64 | [39418482527](https://github.com/futuregene/future-os/actions/runs/39418482527) | 现有 dev 默认参数 | 两架构成功 |

这些运行沿用既有 `dev/` 产物上传路径；没有启动 Build Test 总协调、nightly、正式 Release、Android 或 iOS。平台 unsigned 模式仍保留现有 Tauri updater 校验签名，不调用 Authenticode/Developer ID/公证服务。构建完成后还需记录实际版本、下载地址、摘要、包入口/二进制依赖检查和旧 Linux smoke；排队/运行中均不是通过。失败需按具体日志修复、重新标记候选并复验，不把不同源码的包拼成一轮。签名证书、时间戳、公证/Gatekeeper 验证不因 unsigned 成功而关闭。

初始执行记录见 [unsigned 构建清单](headless-unsigned-build-2026-10-20.json)。

[完整下载与验证结果](headless-unsigned-results.zh-CN.md)。

## 剩余工作与完成证据

| 项目 | 当前状态 | 执行入口/范围 | 完成时必须保留 |
|---|---|---|---|
| R01 固定版本与发行候选 | unsigned 候选已固定；签名/最终发行身份待确认 | 固定完整 SHA、版本/渠道、工具链、features、各 target 与 runner；记录 Mobile 构建身份 | 候选清单、源码树身份、版本 JSON、构建 run、产物 SHA-256 |
| R02 CLI-only 与 Desktop sidecar | unsigned 实际包通过；Windows/macOS CLI-only archive 未覆盖 | [Linux](../../.github/workflows/build-linux.yaml)、[Windows](../../.github/workflows/build-windows-signed.yml)、[macOS](../../.github/workflows/build-macos-signed.yml)；检查独立 CLI 包和 GUI 包内的同版本 CLI | 包内容清单；仅 CLI 安装即可运行完整 `future headless`；不包含或依赖 `futureos-headless` |
| R03 Linux musl 与旧系统 | 双架构 musl、Rocky 8 无网络 smoke 通过 | x86_64/aarch64 musl CLI、独立 GNU GUI、deb/portable；既有无网络旧 Rocky 8 smoke | 实际 ELF 静态/NEEDED 结果、旧 Linux 启动/首次非交互拒绝日志 |
| R04 Windows 签名 | unsigned 包/安装器回归通过，系统签名由用户后验 | 既有 Authenticode 流程：staged CLI/sidecar、GUI、NSIS/portable 及更新载荷 | 每个实际文件签名/时间戳/证书验证、版本与摘要；无证书时保留待验，不能用 unsigned 代替 |
| R05 macOS 签名与公证 | unsigned 包/恢复逻辑通过，签名公证由用户后验 | arm64/x86_64 的 app、内置 CLI、DMG、updater archive；Developer ID、entitlements、公证及 stapling | codesign、notarytool 受理结果/诊断、stapler/Gatekeeper 验证与包摘要 |
| R06 实际二进制无 GUI 依赖 | 普通依赖图及本轮实际包内二进制检查通过 | [产物检查器](../../scripts/ci/check-headless-artifact.py)：ELF、PE 普通/延迟导入、Mach-O；[包入口检查](../../scripts/ci/check-packaged-entrypoints.py) | 同一个包里实际 CLI 的链接依赖、help/版本、隔离 home 的首次非交互拒绝及无敏感状态副作用 |
| R07 首次安装、升级与卸载 | 待真机/虚机 | Windows/macOS/Linux 的现有安装渠道；由旧正式版本升级，保留原数据与配对；回滚和卸载 | 平台/旧新版本/操作日志；旧配对无需重配、旧入口无运行依赖、外部 Agent 不被误杀、卸载遵守原数据策略 |
| R08 同候选人工核对 | 用户体验已完成，最终身份待补 | Desktop、CLI/headless、Mobile；按[人工用例](headless-manual-acceptance.zh-CN.md)核对受源码变更影响的项目 | 平台、完整 SHA、真实 App 版本、通过/失败/未执行项及日志；不得把未反馈的平台推定通过 |
| R09 匹配性能记录 | 工具已接线，本轮跳过 | CI 的 `headless-measurement` scope；[观察工具](../../scripts/ci/headless-measurement.py)，同 runner/工具链/target/features/命令/缓存策略 | 编译/直接 linker/真实 Store 样本执行时间、原始大小、启动样本、Cargo timings；记录顺序/页缓存/样本限制，不预设改善 |
| R10 安装分发与渠道发布 | dev 下载/摘要已验证；正式渠道和安装升级待验 | 既有安装器下载入口、CLI 安装/升级、Desktop updater；仅使用经过前述检查的同一候选 | 下载与 checksum、平台架构/渠道匹配、升级路径和最终确认；不额外发布新协议或服务 |

## 顺序与需要真机的部分

先固定 R01，再构建并验证 R02/R03/R06；在既有签名条件下完成 R04/R05；随后用明确版本做 R07/R08，并补 R09，最后才按既有流程核对 R10。需要用户真机时，应提供具体平台、候选版本、下载包/摘要、操作步骤与预期结果；当前未假定用户已有某一签名包。

[Build Test](../../.github/workflows/build.yml) 也会发布 test/nightly 产物，并调用 Android/iOS 流程；签名 nightly、TestFlight 和正式 [Release](../../.github/workflows/release.yml) 都存在外部发布效果。整理本清单不等于已经触发这些流程。版本与发布参数应在执行前固定，不能把普通 CI 的通过当作发布工作已完成。

整体 Goal 保持未完成，直到上述适用项目的证据、同候选人工结果和用户确认齐备。
