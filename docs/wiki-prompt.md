# FutureOS Wiki 写作契约

创建或更新 `docs/wiki/{en,zh}/` 时遵循本契约。Wiki 是用户指南，不是发布日志或内部设计
文档副本。写作前对照当前源码；旧核验笔记只代表其日期/提交，不能据此跳过检查。

## 读者与范围

用易懂语言描述已实现、界面可见的行为。桌面平台为 **macOS、Windows 和 Linux**；
Android/iOS 是运行中桌面的远程客户端。Research/Data 导航仍隐藏时不写入用户指南。
Skills 与 Remote 已面向用户，不要沿用旧的隐藏功能假设将其排除。

Wiki 不必单独创建 TUI 页，仓库已有 `docs/tui*.md`；CLI 页可以介绍 `future tui`、
`future channel` 和 `future loop`。普通 App 页面不展开协议内部实现；排障所需的准确
连接方式与默认值放到 CLI 和仓库指南。

## 输出与链接

- 中英文文件名一一对应，事实内容一致。
- 页内使用同语言 `[[Page]]` 或 `[[标签|Page]]` 链接；同步 `_Sidebar.md`、`_Footer.md`。
  不要把省略 `.md` 的 Markdown 链接当成仓库相对文件链接。
- 两套语言独立可用，不要求跨语言导航。
- `Models.md` 由 `scripts/generate_models.py`（`make generate-models`）生成，禁止手改；
  不要只为修改周边说明而重新生成模型数据。
- 共同事实改变时，同步 README、构建安装、目录和安全参考；新增用户页要进入文档索引。

## 页面清单与核验入口

| 页面 | 内容 | 核验来源 |
|---|---|---|
| `Home.md` | 定位、场景、导航、平台概览 | 根 README；`desktop/src/components/layout/ActivityRail.tsx` |
| `Installation.md` | 产物/架构、首次启动、运行库、更新与卸载 | `.github/workflows/release.yml`、`build-{macos-signed,windows-signed}.yml`、`build-linux.yaml`；`scripts/install.sh`、`install.ps1`；`docs/dist/` |
| `Quick-Start.md` | 登录或 BYOK、首个对话、审批选择、模型选择 | 桌面登录/新会话流程及设置文案 |
| `Using-FutureOS.md` | Chat/workspace、附件、工具、批准卡、Files/Runs/Review | `desktop/src/features/agent/`、`review/`、`filetree/`；`components/layout/` |
| `Settings.md` | 当前设置项与默认值 | `desktop/src/features/settings/`；`desktop/src-tauri/src/store/app_settings.rs` |
| `Sandbox.md` | 默认值、三档、平台边界、Linux 安装、提权与排障 | `agent/src/sandbox/`；`agent/src/rpc/session.rs`、`session_prompt.rs`；`desktop/DEV_MD/SANDBOX/`；`useSandboxAvailability.ts` |
| `Remote.md` | Android/iOS 配对、在线要求、撤销、隐私与排障 | `mobile/README.md`、`mobile/src/remote/`；`desktop/src/features/remote/`、`desktop/src-tauri/src/remote/`；remote 文案 |
| `Skills.md` | 浏览/安装/使用及精选能力 | `skills/builtin/*/SKILL.md`；`cli/src/commands/skills.rs`；桌面 Skills 页面 |
| `CLI.md` | 可选 CLI 位置、命令、默认值、Agent 要求 | `cli/src/main.rs`、`cli/src/lib.rs`、`cli/src/commands/{run,auth,configure,tools,skills,doctor}.rs`；`packages/rpc/src/transport.rs` |
| `Feishu.md` / `DingTalk.md` | 机器人配置、访问策略、启动、命令与排障 | `channels/src/` 配置和各渠道处理器；外部步骤另核验平台文档 |
| `FAQ.md` | 真实故障与安全处理方法 | 对应实现及产物/渠道信息 |
| `Models.md` | 自动生成模型目录 | 生成器与内置模型数据 |
| `_Sidebar.md` / `_Footer.md` | 导航 | 上述页面清单 |

## 不应再次写错的事实

1. **不是默认每次工具调用都审批。**桌面默认 `off`，新 Agent 会话权限 `all`；enum 默认值
   不等于应用默认值。手动模式可放行普通读取/workspace 写入，沙箱命令不必逐次预先询问。
   区分旧式 `--permission` 与 `off/manual/sandbox`；`none` 禁止所有工具，不仅是外部写入。
2. **三个 OS 后端，保护范围不同。**macOS Seatbelt、原生 Linux 系统 Bubblewrap ≥ 0.9.0、
   Windows 受限令牌写保护；网络开放。Linux 无 seccomp 且缺失/动态路径存在检测型限制；
   Windows 有读取/ACL/删除限制。披露 `auth.json` 例外，不能将可用 probe 或历史 smoke PASS
   写成所有环境已完成发布与安全认证。
3. **默认本地 IPC。**Unix 依次考虑 `FUTURE_AGENT_SOCKET`、Linux `XDG_RUNTIME_DIR` 和 HOME
   回退；Windows 使用每用户命名管道。TCP 显式启用。TUI/Desktop 可启动 sidecar，但只拥有
   自己启动的 Agent，不负责终止外部管理的进程。
4. **区分正式版和测试包。**正式 macOS/Windows release 工作流签名，macOS 还公证；未签名
   测试产物单独说明。`readme-macos.txt` 注入未签名 DMG，不能证明所有当前 release 都未签名。
   不建议绕过无法解释的签名错误。Linux 有 x86_64/aarch64 deb、portable、静态 CLI 发布包；
   本地构建的命名/链接方式可能不同，使用实际产物文件名。
5. **本地优先不等于仅本地。**模型请求、在线工具、Remote 中继和 IM 桥都会传输数据。
   手机强制 WSS，桌面 NATS 由部署配置控制；不承诺端到端加密或所有凭据均被隔离。
6. **命令与代码一致。**读取 Rust 源码，不引用退役的 `cli/src/*.ts`。优先使用 `future channel`；
   若运行 release 二进制，构建须加 `--release`。桥接层处理斜杠命令仍可能调用 Agent RPC，
   “不发给模型”不等于“不需要 Agent 连接”。
7. **数量不是长期常量。**技能/命令/模型数由当前注册表推导，或避免固定数量。历史验收报告
   保留原日期、候选提交与结果，并明确加上历史快照提示。

## 最终检查

检查本地链接、Wiki 目标、代码围栏、中英文页面对应、命令参数/默认值、平台/架构与渠道措辞。
搜索过时的“沙箱仅 macOS”、默认 TCP、旧 CLI TypeScript 路径、无条件审批承诺，以及禁止
Linux/隐藏 Remote 的指令。确认未来设计和历史测试结果没有被表述为当前事实。
