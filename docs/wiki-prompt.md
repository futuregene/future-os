# Wiki 生成提示词

> 这是给 AI 使用的**生成提示词**(prompt),不是给用户看的文档。
> 把本文件的全部内容交给 AI,它就应当能(重新)生成 `docs/wiki/` 下的整套 wiki 页面。
> 本文件只负责**内容生成**;如何发布到 GitHub Wiki 不在此讨论。

---

## 1. 角色与目标

你是 FutureOS 的技术文档撰写者。目标:为 **FutureOS 桌面应用**编写一套面向**普通用户**的 wiki,帮助用户从下载、安装、登录到日常使用全程顺畅上手。

FutureOS 是一个**桌面 AI Agent 工作台**:用户不只是"聊天拿答案",而是能**看到并核对** agent 的工作——它读了什么、跑了什么命令、改了哪些文件、在等你批准什么、以及之后如何接着做。面向真正推进多步骤任务的人:软件开发、研究、数据分析、写作、报告生成、调试,全在一个地方完成。

## 2. 读者与语气

- **读者是普通用户**,不是开发者。默认他们不懂命令行、不懂 gRPC/架构这类内部细节。
- 语气:**清楚、简洁、友好、以操作为导向**。多用"你",少用被动语态和术语。
- 讲**怎么用**,不讲**怎么实现**。不要暴露内部模块名、端口号(除非某条命令确实需要)、代码架构。
- 反复强调 FutureOS 的核心卖点:**你始终掌控** —— agent 在做有风险的事之前会停下来征求批准;所有工作都可查、可核对。

## 3. 写作前:先读代码,只写已实现的功能

**在写每个页面之前,先阅读该页面相关的代码和文档,以真实行为为准**,不要凭想象或旧文档描述功能。第 7 节为每个页面列出了 3–5 个**代码入口**,请以它们为起点向外探索(顺着 import、组件引用、i18n 文案 key 追查),确认:

- 功能是否真的存在、UI 里是否真的能看到;
- 具体的按钮名、菜单名、页面名、流程步骤;
- 平台差异(macOS / Windows)是否与代码/打包配置一致。

**只写已经实现、用户在界面上真的能用到的功能。** 任何"计划中/开发中/已隐藏"的功能一律不写。已知当前**未上线、不要写**的:

- **Research(研究)** 入口 —— 已从导航隐藏。
- **Data(数据源)** 入口 —— 已从导航隐藏。
- **Remote / 手机远程** —— 仍在开发中。

> 判断依据看代码:例如 `gui/src/components/layout/ActivityRail.tsx` 里 `featureItems` 若为空数组,即表示这些侧边栏入口当前不对用户显示。写作时若发现某功能在代码里被隐藏或未接线,就不要写进 wiki。

## 4. 输出要求:中英双语

每个页面都要有**中文**和**英文**两个版本,内容对应一致。按**语言子目录**组织:

```
docs/wiki/
  en/            # 英文版全部页面
    Home.md
    Installation.md
    Quick-Start.md
    ...
    _Sidebar.md
    _Footer.md
  zh/            # 中文版全部页面(文件名与 en/ 一一对应)
    Home.md
    Installation.md
    Quick-Start.md
    ...
    _Sidebar.md
    _Footer.md
```

- 两个目录下**文件名完全相同**,只是内容语言不同(`en/Home.md` 对应 `zh/Home.md`)。
- **中英文文档之间不需要互链**,不放语言切换链接,两套页面各自独立。
- `_Sidebar.md` 和 `_Footer.md` 每个目录各出一份。
- **页面互链只在本语言目录内**(英文版链英文版,中文版链中文版),走本目录内的相对路径。

## 5. 平台范围

**只支持 macOS 和 Windows。不要写 Linux 的任何内容**(不写 `.deb` / `.tar.gz`、不写 apt、不写 Linux 首次启动步骤等)。凡是"支持哪些平台"的表述,一律写 **macOS 和 Windows**。

## 6. 页面清单

生成以下页面(**不要生成 TUI / 终端界面页面**)。下表列的是文件名,`en/` 和 `zh/` 目录下各出一份。

| 页面文件 | 页面标题 | 作用 |
|---|---|---|
| `Home.md` | FutureOS | 落地页:一句话讲清是什么、能做什么,导航到各页 |
| `Installation.md` | 安装 FutureOS | 下载、首次启动(macOS/Windows)、数据位置、更新、卸载 |
| `Quick-Start.md` | 快速开始 | 从全新安装到第一个回答:登录 → 开始对话 → 发消息 → 选模型 → 查看工作 |
| `Using-FutureOS.md` | 使用 FutureOS | 应用导览:三栏布局、Chat 与 Workspace、如何跟进和引导 agent、批准机制、右侧面板 |
| `Settings.md` | 设置 | 设置各页(重点 General / Providers / Models);内置 FutureGene 登录、自定义 provider、模型可见性 |
| `Skills.md` | 技能 | 内置技能包一览及使用方式 |
| `CLI.md` | 命令行工具(`future`) | 可选的高级命令行工具:位置、运行、命令组 |
| `FAQ.md` | 常见问题与排错 | 常见问题速查 |
| `_Sidebar.md` | — | 左侧导航 |
| `_Footer.md` | — | 页脚(下载、反馈问题链接) |

> 这是**最小页面集**。若在代码里发现本清单未列、但确已发布给用户的小功能,就近补进最相关的现有页面(一般不新开页面),并在生成后的偏差报告里说明(见第 9 节)。

### 侧边栏结构(去掉 TUI)

```
### FutureOS
- Home

**开始使用 / Getting started**
- 安装 / Install → Installation
- 快速开始 / Quick Start → Quick-Start

**使用应用 / Using the app**
- 使用 FutureOS → Using-FutureOS
- 设置 / Settings → Settings
- 技能 / Skills → Skills

**命令行(进阶)/ Command line (advanced)**
- CLI (future) → CLI

**帮助 / Help**
- FAQ
```

## 7. 各页面内容要点

> **下面每条 bullet 都是"参考内容,以代码为准"** —— 它给的是覆盖范围和大致事实,但按钮名、页面名、命令、选项、技能清单等**具体细节一律以你读到的代码为准**;两者冲突时,按代码写,并把冲突记入偏差报告(第 9 节)。本文件的参考内容可能滞后于代码。
> 写作时保持面向用户、可操作。每页的「代码入口(先读再写)」只是给你的探索起点,**不要**把这些行写进真正的 wiki 页面。

### Home
**代码入口(先读再写):** `gui/src/app/App.tsx`、`gui/src/components/layout/AppShell.tsx`、`gui/src/components/layout/ActivityRail.tsx`、`gui/src/i18n/locales/`(功能命名与文案)、`CLAUDE.md`(整体定位)。
- 一句话定义:**桌面 AI Agent 工作台**,不只是聊天,而是能看到并核对 agent 的工作。
- "开始使用"三步:安装 → 快速开始 → 使用 FutureOS。
- "你能做什么"要点:与会流式展示思考、调用工具、展示过程的 agent 对话;快速 Chat 或绑定文件夹的 Workspace;你始终掌控(风险操作前会征求批准);可核对工作(后台任务、文件改动、产出);使用技能包。
- 底部注明:运行于 **macOS 和 Windows**。

### Installation
**代码入口(先读再写):** `gui/src-tauri/tauri.conf.json`(打包产物:dmg / nsis|msi / zip,确认真实产物类型)、`gui/src-tauri/build.rs`(随包附带的 sidecar 二进制)、`scripts/build-windows-portable.ps1`(Windows 便携包内容)、`Makefile`(`package-gui` 等打包目标)、`CLAUDE.md`(`~/.future` 数据/配置位置)。
- **下载**:去 Releases 页下载对应系统最新版。
  - macOS:`.dmg` 磁盘镜像
  - Windows:安装包(`.exe` / `.msi`),或便携版 `.zip`
- 说明命令行工具 `future` **随每个下载包附带**(安装包与便携包都有,装在应用旁边),详见 CLI 页。
- **首次启动**(当前构建未签名/公证,系统会告警,属正常):
  - **macOS**:把 FutureOS 拖进"应用程序";首次右键 →"打开"→ 再点"打开";若提示"已损坏",在终端运行 `xattr -dr com.apple.quarantine /Applications/FutureOS.app` 再打开。
  - **Windows**:安装版跑 `.exe`/`.msi`;便携版解压整个文件夹后双击 `FutureOS.exe`(便携版需把 `FutureOS.exe` 和 `future-agent.exe` 放在同一文件夹)。首次 SmartScreen 提示时点"更多信息 → 仍要运行"。需要 **Microsoft Edge WebView2 Runtime**(Win10 近期版与 Win11 一般已内置,缺失则从微软官网装 Evergreen 版)。
- **登录**:首次使用需联网并在应用内登录,详见快速开始。
- **数据位置**:主目录下的 `.future` 文件夹(macOS `~/.future`,Windows `C:\Users\<你>\.future`)。
- **更新**:下载最新版覆盖安装(便携版替换文件夹),`.future` 数据保留。
- **卸载**:macOS 删除 `FutureOS.app`;Windows 从设置卸载或删除便携文件夹。要清数据再删 `.future`。

### Quick-Start
**代码入口(先读再写):** `gui/src/features/settings/FutureLoginDialog.tsx`(设备码登录流程)、`gui/src/features/settings/ProvidersPage.tsx`、`gui/src/features/agent/NewConversation.tsx`、`gui/src/features/agent/Composer.tsx`(发送、模型选择器、附件)、`gui/src/components/layout/ActivityRail.tsx`(New Chat / Workspace 入口)。
- **打开并登录**:Settings(左下齿轮)→ Providers → 内置 FutureGene → Connect → 浏览器授权(不自动打开时用应用给出的验证码 + 可复制链接)。提一句也可改用自己的 provider(见 Settings)。
- **开始对话**:两种方式 —— **New Chat**(最快,适合提问和一次性任务)、**Workspace**(绑定电脑上的文件夹,适合真实项目)。
- **发第一条消息**:底部输入框发送;会看到流式回复、工具调用展示、风险操作时**暂停等你批准**;支持任意本地文件，每轮最多 4 张图片（单张 25 MiB），非图片不限数量。
- **选模型(可选)**:模型选择器就在输入框里;也可在 Settings → Models 管理。
- **查看工作**:右侧面板 —— Runs(后台任务)、Review(Workspace 的文件改动)、Artifacts(Chat 的产出)。

### Using-FutureOS
**代码入口(先读再写):** `gui/src/components/layout/AppShell.tsx`(三栏布局)、`gui/src/components/layout/ActivityRail.tsx`(左侧导航,以此为准确认到底有哪些入口)、`gui/src/components/layout/ContextPanel.tsx`(右侧面板)、`gui/src/features/agent/ApprovalPrompt.tsx`(批准机制)、`gui/src/features/runs/RunsPanel.tsx` + `gui/src/features/review/ReviewPanel.tsx` + `gui/src/features/artifacts/ArtifactsPanel.tsx`(右侧三种视图)。
- **三栏布局**:左=导航(以 `ActivityRail.tsx` 实际渲染的入口为准:New Chat、你的 Workspaces 及其会话、Chats、Settings);中=对话(消息、流式回复、计划、工具活动、命令预览、错误、批准卡片,输入框固定底部);右=上下文(查看 agent 在做什么,可折叠)。
- **Chat vs Workspace**:用表格对比(建立方式、适用场景、右侧面板显示的内容)。强调每个会话是独立 agent session,互不干扰。
- **和 agent 对话**:输入框发送;模型选择器可逐会话切换;每轮最多 4 张图片，非图片附件不限数量。
- **批准机制 —— 你掌控**:风险操作会停下来在输入框上方弹批准卡片并等待(不超时);Allow 继续、Reject 取消并告知 agent 以便调整。
- **右侧面板核对工作**:Runs(运行中/已完成,可停止/清理,每张卡显示真实命令)、Review(Workspace 文件改动:文件列表、统计、diff;版本控制下还有"上一轮改动"视图)、Artifacts(Chat 产出)。
- (不要写 Research / Data 入口——当前已从导航隐藏。)

### Settings
**代码入口(先读再写):** `gui/src/features/settings/SettingsDialog.tsx`(页面构成)、`gui/src/features/settings/GeneralPage.tsx`、`gui/src/features/settings/ProvidersPage.tsx` + `CustomProviderDialog.tsx`、`gui/src/features/settings/ModelsPage.tsx`、`gui/src/features/settings/FutureLoginDialog.tsx`。**以此确认实际有哪几个设置页、每页真实字段**。
- 从左下齿轮进入;New Chat 下还有 Models 快捷入口。**页面数量与名称以 `SettingsDialog.tsx` 为准**(除 General / Providers / Models 外通常还有"检查更新""重置"等用户可见页;开发版专用页不写)。重点讲下面三页。
- **General**:桌面级选项。以代码里的真实标签为准,通常含:**界面语言(Language)**、**批准模式(Approval mode:手动 / 沙盒[仅 macOS] / 无限制)**、**是否显示思考过程(Show thinking)**。
- **Providers**:
  - **FutureGene(内置)**:Connect 登录流程(浏览器授权 / 验证码 + 链接);连接后可重新登录或登出。
  - **自定义 provider**:添加 OpenAI 兼容或 Anthropic 兼容的 provider;需填 id、名称、API 类型、Base URL、API key、模型列表;应用会校验并检查 id 唯一;可编辑/删除。
- **Models**:按 provider 分组列出所有可用模型;可切换每个模型的可见性、可搜索;输入框里的选择器同源,并显示模型来自哪个 provider。

### Skills
**代码入口(先读再写):** `gui/src/features/skills/SkillsView.tsx`、`gui/src/integrations/skills/skillsClient.ts`、`cli/src/commands/skills.ts`、`agent/src/skills/mod.rs`(技能发现)。**先用这些确认当前真实存在的技能清单和用途**,再据实写下方表格。
- 定义:内置能力包,agent 在相关时**自动使用**;同时也是一个**可浏览/安装/卸载的目录**(Installed / All 标签,清单来自在线目录)。**Skills 侧边栏入口是可见的**(与 Research/Data 不同,后者才是隐藏的)——以 `ActivityRail.tsx` 为准。
- 常见内置技能表(技能名 + 用途,**参考,以应用 All 标签实际清单为准**):Account(账户资料/额度/充值)、Web(搜公网并读全文)、Paper(检索 PubMed/ArXiv/DOI 并取全文)、Deep research(多源交叉核对、带引用的报告)、Document(PDF/Word 转结构化文本)、Image(生成/编辑/分析图像,含读图中文字)、Browser(驱动浏览器:开页、点击、输入、截图)、Hand-drawn posters(手绘竖版信息图海报)、Hand-drawn slides(手绘草图幻灯并合成 PDF)、Subagent(并行跑多任务)、Skill creator(帮忙做新技能)。
- **使用方式**:无需手动开启,直接描述需求即可;也可在 Skills 页浏览与安装/卸载。(不要写 Research / Data 入口——它们已隐藏。)

### CLI
**代码入口(先读再写):** `cli/src/index.ts`(子命令分发,以此确认真实存在的命令组)、`cli/src/commands/run.ts`、`cli/src/commands/auth.ts` + `agent.ts`、`cli/src/commands/tools.ts` + `skills.ts`、`cli/src/help.ts`。**命令、子命令、选项一律以代码为准**。
- 定位:可选的命令行工具 `future`,随下载包附带;桌面应用已能满足大多数需求,想脚本化/自动化/纯终端操作时再用。开头提示不熟悉终端可跳过本页。
  - > ⚠️ 命令名统一为 **`future`**:发布产物的二进制名(见 `tauri.conf.json` 的 sidecar、`docs/dist/readme-*.txt`、应用内文案)与开发期 npm link 装的命令一致,都是 `future`。全文一律用 `future`,不要写成 `future-cli`。
- **位置**:
  - macOS(`.dmg`):应用内 `/Applications/FutureOS.app/Contents/MacOS/future`
  - Windows(**便携** `.zip`):解压文件夹里的 `future.exe`
  - 注明:Windows 上命令行工具在**便携包**里,普通安装版只含应用和 agent。
- **运行**:在含二进制的文件夹开终端;`--help` 查看;可加入 PATH 或做别名(给 macOS 别名示例)。
- **agent 必须在运行**:每条命令都要连 FutureOS agent;开着桌面应用则已在运行,否则 `future agent start`。
- **命令组**(以 `cli/src/index.ts` 实际分发为准;去掉 tui 组):
  - `auth`:登录/登出/状态(`login` / `status` / `logout`)
  - `agent`:启停后台 agent(`start` / `stop` / `restart` / `status`)
  - `run`:发一次性 prompt 并打印回答(给示例:直接问、`--model`、`@文件`、管道输入;说明 `@<path>` 包含文件、常用选项 `--model`(支持 `model:thinking`)、`--thinking`、`--continue`/`-c`、`--cwd`、`--mode json`、`--no-session`)
  - `tools`:列出与调用工具(`tools list`、`tools call <name> --args '<json>'`、`--output`、`--stdin`;文件路径参数自动转换)
  - `skills`:管理技能包(`list` / `install` / `uninstall`;**没有 `update`**,子命令以代码为准)
  - `channel`:聊天渠道桥接(进阶,一句带过)
- **小贴士**:macOS 首次被拦 → 先右键打开应用清除拦截;"Connection refused" → agent 没运行,`future agent start` 或打开桌面应用。

### FAQ
**代码入口(先读再写):** `gui/src-tauri/tauri.conf.json`(安装/签名相关)、`gui/src/features/settings/FutureLoginDialog.tsx` + `ProvidersPage.tsx`(登录问题)、`gui/src/features/agent/ApprovalPrompt.tsx`(批准)、`cli/src/commands/agent.ts`("连接被拒"/agent 未运行)、`CLAUDE.md`(数据位置)。
覆盖这些问题(去掉一切 Linux 与 TUI 相关项):
- macOS 打不开("身份不明的开发者"/"已损坏"):右键打开;"已损坏"用 `xattr -dr com.apple.quarantine /Applications/FutureOS.app`。
- Windows 提示"Windows 保护了你的电脑":SmartScreen,点"更多信息 → 仍要运行"。
- Windows 启动没反应:装 Microsoft Edge WebView2 Runtime;便携版确认 `FutureOS.exe` 与 `future-agent.exe` 同文件夹。
- 用不了任何模型/未登录:Settings → Providers → FutureGene → Connect,或加自己的 provider。
- 怎么切换模型:输入框选择器,或 Settings → Models。
- agent 停下来问我东西:那是批准机制,Allow/Reject,不超时。
- 会话和设置存哪:主目录 `.future`(macOS `~/.future`,Windows `C:\Users\<你>\.future`)。
- 怎么更新:下载最新版覆盖安装,数据保留。
- 怎么卸载/删数据:删应用;要清数据再删 `.future`。
- 支持哪些平台:**macOS 和 Windows**。

## 8. 格式与交叉链接规范

- 页面名来自文件名:`Quick-Start.md` → 页面 **Quick-Start**。
- 跨页链接走**本语言目录内的相对路径**,例如英文版用 `[快速开始的英文标题](Quick-Start)`、中文版用 `[快速开始](Quick-Start)`,均指向同目录同名文件。
- **不要跨目录互链、不放中英文语言切换链接**(见第 4 节)。
- 外部链接:Releases 页 `https://github.com/futuregene/future-os/releases`;反馈问题 `https://github.com/futuregene/future-os/issues`。
- 每页底部适当放"另见 / See also"互链。
- 保持 Markdown 表格、代码块、引用块的清爽排版。

## 9. 生成后自检与偏差回报

写完所有页面后,必须做以下两步。

**A. 自检(不通过就修到通过):**

1. **链接完整性**:每个 `[[显示文字|Slug]]` 的 Slug 都能对应到**同一语言目录内真实存在**的 `.md` 文件;没有跨语言目录的链接、没有语言切换链接。
2. **泄漏扫描**:全量搜索,确认**没有**出现——Linux / `.deb` / `.tar.gz` / apt、TUI / 终端界面页面或其链接、gRPC / 端口号(如 50051)、Research 入口 / Data 数据源入口 / Remote 手机远程。(注意:技能名 **Deep research**、以及首页用例里的 "research/数据分析" 等描述性词属正常内容,不算泄漏。)
3. **中英对齐**:`en/` 与 `zh/` 文件名一一对应、数量相同;同名页面的章节结构与覆盖点一致(只是语言不同)。
4. **CLI 名称**:全文用 `future`(命令、路径、示例都要检查),不要写成 `future-cli`。

**B. 偏差回报**:生成结束后,单独输出一份「代码 vs 本提示词参考内容」的差异清单——凡是你按代码写、而与本文件第 7 节参考内容不一致的地方(如技能清单、设置页数量与名称、CLI 命令/子命令、按钮名等),逐条列出。目的是让人把这些修正**反哺回本提示词**,形成闭环。

## 10. 禁止事项

- ❌ 不要生成 TUI / 终端界面页面,也不要在其它页面链接或提及它(CLI 的命令组里去掉 `tui`)。
- ❌ 不要写未上线/已隐藏/开发中的功能:**Research(研究)入口、Data(数据源)入口、Remote(手机远程)**,以及任何在代码里被隐藏或未接线的功能。
- ❌ 不要写 Linux 任何内容。
- ❌ 不要写 wiki 的发布/同步/CI/GitHub Action 等维护流程 —— 本提示词只管**内容**。
- ❌ 不要暴露内部实现细节(架构、模块名、gRPC 等),除非某条 CLI 命令确实需要。
