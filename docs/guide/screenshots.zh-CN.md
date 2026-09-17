# 截图工具：用真实界面，不需要显示器

[English](screenshots.md)

过去做产品截图，得有人在机器前跑 `make run-desktop`：桌面端是 Tauri webview，手机端是 React Native，两边的内容都来自一个浏览器够不着的本地后端。这套工具改成本地 Chrome 里渲染**真实前端**，用固定的演示数据顶替后端，于是截图可以在没有屏幕的机器上完成——CI、远程 shell，或者没有屏幕录制权限的会话。

被替换的只有数据来源；组件、hook、store、样式和 i18n 都是线上那份代码。点击和输入都以真实输入事件派发，所以截到的就是界面真实的行为。

## 它用来做什么

典型需求（三种都是：先用真实界面产出图，再组装成文档，见「生成文档」）：

- 「给 X 到 Y 版本之间的功能增减做一份 PDF」——截取受影响的界面，再生成「文字配一到两张图」的文档。
- 「给桌面端某个功能出一张图」——一个场景，一张 PNG。
- 「写一份手机端某个功能的使用流程说明」——每个步骤一个场景（列表 → 对话框 → 结果），组装成文档。
- 「展示某条命令的终端输出」——用 `terminal` 子命令把命令的真实 stdout 渲染进终端外框。

范围以「界面能不能表现出来」为准：如果某个改动没有界面（协议变更、只存在于 TUI 的提示），那就没有可截的东西——直接说明，比拿别的画面凑数更合适。

## 它是什么，不是什么

| 真实 | 被替换 |
|---|---|
| `desktop/src/**`——每个组件、hook、store、worker、样式 | `@tauri-apps/*` IPC，由页内 mock 应答（`desktop/shot/mock/`） |
| `mobile/src/**`、`mobile/App.tsx` | `mobile/src/remote/RemoteContext`（NATS 客户端）和两个原生模块（`mobile/shot/mock/`） |
| 真实 Chrome 渲染真实布局 | 真实数据、真实网络、真实 agent |

用截图当证据之前，需要知道这些边界：

- **内容是编的。** 演示对话、文件名、项目名来自 `desktop/shot/mock/data.ts` / `mobile/shot/mock/data.ts`。任何情况下都不要把它当作真实用户数据的证据。
- **没有验证任何后端行为。** 这里不证明某个 RPC 能通、run 能流式输出、沙箱边界成立——那些必须跑真实应用。
- **有两处是"样式化呈现"而非截屏**：命令行输出是用 `terminal.html` 把命令的真实 stdout 渲染出来的；只存在于终端界面（`future-tui`）的内容不在覆盖范围内。

## 环境要求

- Node.js（驱动用到的 `ws` 已是仓库依赖）和 Python 3。
- Chrome、Edge 或 Chromium。不支持 Safari（驱动需要 CDP）。
- `pip install pillow` 是可选的：它只用来压小 PDF 里的图片。
- 手机端工具另外需要 `react-native-web`、`@expo/metro-runtime` 和 `react-dom`；`capture.py serve-mobile` 会把它们按需装进 mobile workspace，且不动依赖清单（见"不入库的约定"）。

## 桌面端

两个终端，因为 dev server 一直占着前台：

```bash
python3 scripts/screenshots/capture.py serve-desktop     # 终端 1：提供 http://localhost:5299/shot/
python3 scripts/screenshots/capture.py capture-desktop       # 终端 2：跑全部场景
python3 scripts/screenshots/capture.py capture-desktop d-rail-tree d-steps-folded            # 或只跑几个
```

截图写到 `.screenshots/`（已 gitignore）。你也可以自己打开 <http://localhost:5299/shot/> 手动探索；工具还会在 7391 端口起一个替身 PTY 服务，让内嵌终端面板有东西可显示。

## 手机端

```bash
SHOT_WEB=1 python3 scripts/screenshots/capture.py serve-mobile     # 终端 1：Expo web，http://localhost:8099/
python3 scripts/screenshots/capture.py capture-mobile       # 终端 2
```

手机端以手机视口渲染（默认 390×844、2 倍像素密度），并通过 HTTP 提供演示图片，所以在聊天里内嵌的图片和可缩放预览都能正常显示。`serve-mobile` 会导出 `SHOT_WEB=1`；这是唯一的开关——`mobile/metro.config.js` **仅在 web 平台、仅在该变量为 1 时**把 remote context 和两个原生模块指向 `mobile/shot/mock/`，因此原生构建（`make run-mobile-android`）完全不受影响。

## 场景

场景是数据不是代码：`scripts/screenshots/scenarios.json`。每个场景就是一串人也能照着复现的步骤。

```json
"rail-pin-menu": {
  "steps": [
    { "wait": 900 },
    { "tap": "单细胞转录组 的操作" },
    { "wait": 1500 },
    { "shot": "d-rail-pin-menu.png" }
  ]
}
```

步骤类型：`eval`、`wait`、`shot`、`tap`（按可访问名称）、`tapText`（按可见文字）、`hover`、`type`、`key`、`scroll`。三个平台级细节值得先知道：

- **`ready`** 是驱动会反复轮询直到为真的 JavaScript 表达式，它也是截图快的关键：热缓存下约 1 秒就绪，冷启动可能要 20 秒。`settle` 只是就绪后的一小段缓冲。如果截图抢在界面绘制之前，应该调大 `readyTimeout` 而不是 `settle`。
- **`tap` 需要可访问名称。** 先匹配 `aria-label`，再匹配可交互元素自身的文字。两者都没有就用 `tapText`；如果你发现自己在写坐标点击，那通常说明这个控件缺标签。
- **点不到的状态用 `eval`。** 只在悬停时出现的控件，需要先对它的容器加一个 `hover` 步骤（否则驱动会报告目标尺寸为 0，而不是去点页面角落）。例如对话内搜索是 ⌘F 打开的，驱动没法把它当按键发出去，所以场景改为访问 `?press=meta%2Bf`，由 `desktop/shot/main.tsx` 把快捷键派发给真实的 window 监听器。

`desktop/shot/main.tsx` 还接受 `?lang=en` 和 `?settings=key:value,...`，用来固定界面语言、在启动前预置应用设置；`mobile/shot/mock/shareIntent.ts` 读 `?share=1`，好让分享面板只出现在分享相关场景里。

## 不入库的约定

两条刻意的规则，让它保持是工具而不是一堆二进制文件：

- **截图永不入库。** 截图落在 `.screenshots/`（已 gitignore）；`desktop/shot/assets/` 同样忽略——那里是 `gen-demo-assets.py` 为对话生成的演示图。缺少时 `capture.py` 会自动重新生成（没有 matplotlib 就告警并继续截图）。
- **仅为工具服务的 web 依赖永不写进依赖清单。** `serve-mobile` 用 `--no-save` 把 `react-native-web`、`@expo/metro-runtime` 和 `react-dom` 装进 mobile workspace，因此 `mobile/package.json` 和 `package-lock.json` 保持原样。之后跑一次 `npm install`，依赖树就回到应用自己的样子。

`mobile/shot/assets/` 是刻意不存在的：手机端改为通过 HTTP 读取演示图（`scenarios.json` 里的 `assetsPort`），两个平台共用同一份。

## 生成文档

`capture.py pdf` 把截图和内容文件组装成 PDF（`FutureOS 1.1.8 功能更新说明.pdf` 就是这么来的）：

```bash
python3 scripts/screenshots/capture.py \
  --out .screenshots \
  pdf scripts/screenshots/examples/release-notes-1.1.8.json "$HOME/Documents/notes.pdf"
```

内容文件的字段看那个示例即可：封面、前言、目录，以及若干章节；每一条包含正文和一张图或一对图。说明文字应该写"该看哪里"，而不是复述正文。

命令行输出则先取真实 stdout，再套终端外壳渲染：

```bash
python3 scripts/screenshots/capture.py terminal t-headless.png -- \
  desktop/src-tauri/target/debug/futureos --headless
```

## 应用改动之后

mock 是唯一需要跟着产品走的部分，而漏掉的地方会主动报出来，不会静默出错：

- 桌面端 mock 对没实现的命令会在浏览器控制台打印 `[mock] UNHANDLED COMMAND <name>`。看截图命令的输出或浏览器控制台，是最快知道新界面需要什么的方式。
- 手机端 mock 对未知的 context 成员返回空操作并警告 `[shot] mock RemoteContext has no "<name>"`。请补上真实值，让界面渲染出有意义的内容。
- 界面需要新的演示内容时，扩展数据模块，不要往 mock 里塞特例。

## 目录结构

| 路径 | 作用 |
|---|---|
| `scripts/screenshots/capture.py` | 入口：起服务、截图、终端外框、PDF 组装 |
| `scripts/screenshots/cdp.mjs` | Chrome DevTools Protocol 驱动（视口、输入、截图） |
| `scripts/screenshots/scenarios.json` | 两个平台的场景表 |
| `scripts/screenshots/document.css` | 生成文档用的打印样式 |
| `scripts/screenshots/terminal.html` | 命令行输出的终端外框 |
| `scripts/screenshots/examples/` | 完整示例：1.1.8 更新说明的内容文件 |
| `scripts/screenshots/gen-demo-assets.py` | 重新生成演示图片（需要 matplotlib） |
| `desktop/shot/` | 桌面端 mock、演示数据、入口、替身 PTY 服务 |
| `mobile/shot/` | 手机端 mock 与演示数据 |

## 排查

| 现象 | 原因 |
|---|---|
| `cdp: no page target on port …` | 上一次运行留下的浏览器占着 CDP 端口；关掉它或换 `--cdp-port`。 |
| `target not found: …` | `aria-label` 变了，或步骤跑在界面稳定之前——把前一个 `wait` 调大。 |
| 点击没有反应 | 在可滚动列表里，react-native-web 会优先响应滚动手势；驱动已经发送触摸序列，请确认没有关掉 `--touch`。 |
| 手机端界面空白或不全 | 首次请求时 Expo web 还在打包；等第一次截图完成后再跑一次。若页面全白且控制台报 "Incompatible React versions"，说明 mobile workspace 里 react / react-dom 版本不一致——`serve-mobile` 会检查并给出修复命令。 |
| `error: nothing is listening on port …` | dev server 没起：先跑 `capture.py serve-desktop` 或 `serve-mobile`。 |
