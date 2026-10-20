# Headless dev unsigned 构建结果

> [English](headless-unsigned-results.md)

本轮三个 workflow、五个架构任务全部完成且成功，没有通过反复重跑或源码修复拼接产物。

- 完整候选：`a5f91752b4e470f90fbe5f4bc303ec0ca814944d`；固定引用：`test/headless-unsigned-20261020`。
- 程序版本：`0.0.2-a5f9175+dev`；文件版本：`0.0.2-a5f9175`。
- macOS [39418462362](https://github.com/futuregene/future-os/actions/runs/39418462362)：Apple Silicon 与 Intel 均成功。
- Windows [39418473647](https://github.com/futuregene/future-os/actions/runs/39418473647)：x64 unsigned 成功。
- Linux [39418482527](https://github.com/futuregene/future-os/actions/runs/39418482527)：x86_64 与 aarch64 均成功。
- 所有公开下载地址均来自实际成功 job 日志。15 个文件均得到 HTTP 200，并完整流式计算 SHA-256；Windows/Linux 的 8 个包与 CI 解包前记录的包摘要一致。
- [完整结构化证据](headless-unsigned-results-2026-10-20.json)、[SHA-256 清单](headless-unsigned-2026-10-20.sha256)。下载后可用 `shasum -a 256 -c` 或平台等价工具对照清单；只校验已下载的对应文件。

## 公开下载

下面是无需 GitHub 登录的公开 dev 地址。单独的 GitHub artifact 下载入口可能需要登录，不能替代本表的公开可访问性验证。

| 文件 | MiB | 公开下载 | SHA-256 前缀 |
|---|---:|---|---|
| FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz | 46.62 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz) | `8887d0d5173183f8…` |
| FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz.sig | 0.00 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_aarch64-unsigned.app.tar.gz.sig) | `094d39214644a8b5…` |
| FutureOS_0.0.2-a5f9175_aarch64-unsigned.dmg | 50.41 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_aarch64-unsigned.dmg) | `fc3519be124bb339…` |
| FutureOS_0.0.2-a5f9175_amd64.deb | 49.14 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_amd64.deb) | `f42b49991cf20541…` |
| FutureOS_0.0.2-a5f9175_arm64.deb | 46.95 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_arm64.deb) | `1b35d505fcbf41ff…` |
| FutureOS_0.0.2-a5f9175_linux_aarch64-cli.tar.gz | 22.12 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_aarch64-cli.tar.gz) | `5c6255ce30f0d22e…` |
| FutureOS_0.0.2-a5f9175_linux_aarch64-portable.tar.gz | 45.26 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_aarch64-portable.tar.gz) | `4eb7ce2ea92c7cc7…` |
| FutureOS_0.0.2-a5f9175_linux_x86_64-cli.tar.gz | 23.75 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_x86_64-cli.tar.gz) | `2c4eef8b826ae282…` |
| FutureOS_0.0.2-a5f9175_linux_x86_64-portable.tar.gz | 47.90 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_linux_x86_64-portable.tar.gz) | `fc2893039fb7d9d6…` |
| FutureOS_0.0.2-a5f9175_windows_x64-portable-unsigned.zip | 46.13 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_windows_x64-portable-unsigned.zip) | `2f1a28960b14ac24…` |
| FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe | 32.13 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe) | `db41a858ee04f240…` |
| FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe.sig | 0.00 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-setup-unsigned.exe.sig) | `c22a9cf60c91f84d…` |
| FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz | 49.63 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz) | `8339f6213584ff9d…` |
| FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz.sig | 0.00 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-unsigned.app.tar.gz.sig) | `ff28692700c6ee43…` |
| FutureOS_0.0.2-a5f9175_x64-unsigned.dmg | 54.73 | [下载](https://dl.future-os.cn/dev/0.0.2-a5f9175/FutureOS_0.0.2-a5f9175_x64-unsigned.dmg) | `89a4d52143fb2968…` |

## 已验证的产物行为

| 范围 | 实际结果 |
|---|---|
| Windows NSIS / portable | 解包后执行实际 CLI 的版本、headless/Agent help 和 Desktop help；无初始化副作用；CLI 的 PE 普通/延迟导入没有被禁止的 GUI 框架；两个包的 CLI 摘要一致 |
| Linux deb / portable / CLI-only，双架构 | 实际包内入口与版本匹配；CLI 无 ELF NEEDED；三种包内的 CLI 各自在本架构摘要一致；可执行位保留，无遗留独立 headless 程序 |
| Linux 旧系统 | deb、portable、CLI-only 的同一 CLI 在 Rocky Linux 8、无网络、无 GUI、隔离 home 中通过帮助和首次非交互拒绝；不输出授权材料、不产生数据库/配对或有效凭据 |
| macOS app 内置 CLI，双架构 | 原生 CI 执行 headless help 和首次非交互拒绝，Mach-O 链接无 Tauri/WebView 等 GUI 框架依赖；DMG 结构/checksum 验证成功 |
| macOS 公开 updater archive | 再次读取公开下载的 tar.gz，仅只读检查、不安装；其中实际 CLI 摘要与原生 CI 已验证二进制一致，可执行位存在，旧独立入口缺失 |
| 公开发布完整性 | 15 个文件完整读取并计算摘要；HTTP 长度与实际字节数一致；Windows/Linux 8 个包的摘要还与 CI 记录一致 |

Windows 实际 CLI 允许使用 USER32/shell32 等系统 API；“无 GUI 依赖”在这里指没有 Tauri/WebView/GTK 等产品 GUI 框架，不表示没有任何平台系统库。Linux CLI-only 是独立 tar.gz；本轮 Windows/macOS workflow 提供 GUI/portable 包中的 CLI，没有另外生成其 CLI-only archive，不能声称已验收这两个平台独立 CLI 安装渠道。

## 留给下一步

- 用户稍后做 Windows Authenticode、macOS Developer ID/公证/时间戳/Gatekeeper 测试。本轮没有这些证书签名或公证；构建所需 ad-hoc 和既有 updater .sig 不代替平台签名验收。已对 .sig 文件计算摘要，没有在本地独立做 updater 公钥验签。
- 真实安装、旧正式版升级/回滚/卸载，以及最终同候选 Desktop/CLI/Mobile 人工功能结果仍待记录。NSIS 解包 smoke 不等于安装器已在真机执行。
- 用户于 2026-10-20 取消性能对比，不再列为待办或验收条件；未进行受控对照，本轮构建耗时不能作为拆包性能改善结论。
- 本轮固定的是 `a5f91752`，不含随后 main 中的启动等待、自建中继等改动。若要测试较新源码，应另定候选并复验，不替换本轮身份。
- 完整后置事项见[发布验收清单](headless-release-acceptance.zh-CN.md)。unsigned 通过不代表整体 Goal 完成。

为后续签名测试保留固定引用；需要真机操作时仍应提供明确候选/下载包/摘要和逐项预期，不直接覆盖日常数据或停止外部 Agent。
