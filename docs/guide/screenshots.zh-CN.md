# 截图工具：用真实界面，不需要显示器

[English](screenshots.md)

过去做产品截图，得有人在机器前跑 `make run-desktop`：桌面端是 Tauri webview，手机端是 React Native，两边的内容都来自一个浏览器够不着的本地后端。这套工具改成本地 Chrome 里渲染**真实前端**，用固定的演示数据顶替后端，于是截图可以在没有屏幕的机器上完成——CI、远程 shell，或者没有屏幕录制权限的会话。

被替换的只有数据来源；组件、hook、store、样式和 i18n 都是线上那份代码。点击和输入都以真实输入事件派发，所以截到的就是界面真实的行为。

## 它用来做什么

典型需求（都是：先用真实界面产出图，再组装成文档，见「生成文档」）：

- 「给 X 到 Y 版本之间的功能增减做一份 PDF」——截取受影响的界面，再生成「文字配一到两张图」的文档。
- 「给 X 到 Y 版本的变更做一份测试点清单」——用截图把每个变更点**实际执行一遍**，据此写出可复现的测试点（见「常见请求怎么做」第 2 条）。
- 「给桌面端某个功能出一张图」——一个场景，一张 PNG。
- 「写一份手机端某个功能的使用流程说明」——每个步骤一个场景（列表 → 对话框 → 结果），组装成文档。
- 「录一段演示视频」——`video-desktop` / `video-mobile` 把场景录成 mp4，桌面端和手机端都支持。
- 「展示某条命令的终端输出」——用 `terminal` 子命令把命令的真实 stdout 渲染进终端外框。

范围以「界面能不能表现出来」为准：如果某个改动没有界面（协议变更、只存在于 TUI 的提示），那就没有可截的东西——直接说明，比拿别的画面凑数更合适。

## 常见请求怎么做

下面七条覆盖了绝大多数请求。共同前提：先按「桌面端 / 手机端」把对应服务跑起来。

### 1. 两个版本之间的功能增减 → PDF

```bash
git log --oneline vX..vY                       # 先看有哪些变更
git log --oneline vX..vY --format="%h|%s" | grep -E "feat|refactor"   # 只挑功能增减
```

按提交逐个判断「这个变更对应哪个界面」，为它加/找一个场景并截图；性能优化、缺陷修复不收录。然后照 `scripts/screenshots/examples/release-notes-1.1.8.json` 写内容文件，用 `pdf` 子命令生成。截图里的文案要点出「该看哪里」，不要复述正文。

写给非技术读者时，「前因后果」比「做了什么」更重要：先说这个改动之前用户遇到什么麻烦，再说现在是什么体验，最后才落到具体在哪操作。截图贴在「现在是什么体验」那一段旁边。

### 2. 两个版本之间的变更 → 测试点清单

和上一条同样的取变更方式，但产出是清单而不是文档。做法：

1. 对每个功能变更，在 harness 里**真的走一遍**：找到对应场景（没有就新加一个并截图）。
2. 每个测试点写成「前置条件 → 操作 → 期望结果」，期望结果以界面上看得见的东西为准。
3. 每个测试点配一张该步骤的截图，作为期望结果的依据；截图路径写在测试点下面。
4. 无法用界面验证的变更（协议、内部重构）单独列一节「需要接口/日志验证」，不要硬编 UI 步骤。

harness 的价值在这里：测点不是照着代码猜的，而是照着真实界面点出来、截下来的，别人拿着截图就能复现。

### 3. 单个功能 → 一张图（桌面端 / 手机端）

在 `scenarios.json` 里加一个场景，只做「打开这个功能」所需的操作，最后一张 `shot`。`capture-desktop <场景名>` 或 `capture-mobile <场景名>` 即可。功能藏在菜单里就补一个 `tap`；需要悬停才出现的按钮，先 `hover` 它的容器。

### 4. 使用流程说明 → 文档（手机端尤其常用）

把流程拆成步骤，**每步一个场景**（不要一个长场景），这样每一步都能单独重拍、单独引用；再用 `pdf` 组装成文档。手机端步骤示例：打开列表 → 打开某个会话 → 输入 `/` 选技能 → 结果。视频则用 `video-mobile <场景名>`，一个场景就是一段连贯的操作。


### 5. 几个样式让我挑一个 → 变体对比图

场景里声明 `variants`，每个变体给一个注入脚本，harness 会把同一个界面渲染 N 遍，拼成一张带标签的对比图：

```json
"running-icon-variants": {
  "variantsTitle": "运行指示图标 · 三个备选样式",
  "variants": [
    { "label": "A · 实心圆点", "inject": "injects/running-dot.js" },
    { "label": "B · 同心圆环", "inject": "injects/running-ring.js" }
  ],
  "steps": [ { "wait": 900 }, { "inject": "{variantInject}" }, { "shot": "d-icon.png" } ]
}
```

```bash
python3 scripts/screenshots/capture.py variants-desktop running-icon-variants
```

要点：

- **`{variantInject}` 占位符**让同一个场景服务所有变体：注入发生在**步骤里**而不是启动时，因为 React 会重渲染，启动时改掉的 DOM 会被还原。如果这个改动会让布局跟着变，把注入放在最后一次交互之后。
- **这只是视觉提案**：注入改的是浏览器里的 DOM，产品代码一行没动。选定的方案仍要在应用里实现。
- 注入脚本就是一段 JS，放在 `scripts/screenshots/injects/`；仓库里的 `running-*.js` 是可用的例子。

### 6. 两个版本的样式对比 → 高亮差异

先各截一次，再对比：

```bash
# 在版本 Y 的 checkout 里
python3 scripts/screenshots/capture.py capture-desktop chat
cp .screenshots/d-chat.png /tmp/y.png
# 切到版本 X（或另开一个 worktree），同样截一次
python3 scripts/screenshots/capture.py capture-desktop chat
python3 scripts/screenshots/capture.py compare-desktop /tmp/y.png .screenshots/d-chat.png \
    --output /tmp/style-diff.png --label-left "1.1.8" --label-right "1.1.7"
```

产出一张三联图：左图（红框标出改动处）、右图（蓝框 + 编号）、以及一张差异面板（未变内容淡化、改动处标红）。

要点：

- **两次截图必须同尺寸**：同一个视口（`--w/--h`）和同一个场景即可；尺寸不同时右图会被缩放到左图大小。
- **`--threshold`** 决定多大的像素差算「改过」（默认 24，抗锯齿噪声不会触发）；**`--min-area`** 过滤小到不值得标注的区域。
- **老版本没有 harness**：`scripts/screenshots/`、`desktop/shot/`、`desktop/vite.shot.config.ts` 是 #703 之后才有的。要对更早的版本截图，把这三处拷进那个 checkout 再跑；老前端会用当前 mock 的数据渲染，遇到 mock 没实现的命令会在控制台打印 `[mock] UNHANDLED COMMAND`，按报错补上即可。
- 版本对比天然带噪声：文字渲染差异、时间戳、滚动位置都可能被标成「差异」。对比前先把视图滚到同一个位置——用 `eval` 设绝对 `scrollTop`，不要用相对滚轮。

### 7. 元素间的像素间距 / 位移 → 偏移图

场景里加 `offsets` 步骤，harness 会量出每个元素的框（带名字和尺寸），并在它们之间画虚线标出像素距离：

```json
{ "offsets": {
    "axis": "y",
    "mode": "gap",
    "targets": [ { "at": "新对话", "name": "新对话" },
                 { "at": "模型", "name": "模型" } ] } }
```

- **`mode: "gap"`**（默认）标相邻元素之间的距离，回答「这两行差多少」；**`mode: "edge"`** 标每个元素到视口边的距离，回答「对没对齐」。
- **`axis`** 选 `"y"`（纵向）或 `"x"`（横向）。
- **数值是 CSS 像素**（当前模拟视口下的布局单位），和设备像素无关，和设计稿口径一致。
- `targets` 里 `{ "at": "…" }` 的匹配是**按优先级排序**的（先精确 `aria-label`，再可交互元素自身文字精确匹配……），而且驱动会把每个目标实际命中的元素打印出来——**量错元素时一眼能看见**（例如 `技能` 曾错误命中输入框的 `选择技能` 按钮）。
- 想知道「两个版本之间位移了多少」，先量基线再对比：

```bash
# 版本 X
python3 scripts/screenshots/capture.py measure-desktop rail-offsets
cp .screenshots/desktop-rail-offsets-measure.json /tmp/base.json
# 换到版本 Y 后，带基线再截一次
python3 scripts/screenshots/capture.py capture-desktop rail-offsets
```

带基线时，每个元素旁会多一个绿色标签，写明自基线以来的位移（如 `位移 +8, 0px`）和尺寸变化。基线通过 `--baseline` 传给驱动（`measure-*` 只负责产出这个文件）。

## 录制视频

```bash
# 终端 1、2：先起服务（同上）
python3 scripts/screenshots/capture.py video-desktop            # 全部场景
python3 scripts/screenshots/capture.py video-desktop chat rename  # 或指定几个
python3 scripts/screenshots/capture.py video-mobile chat
```

输出 `<平台>-<场景名>.mp4` 到 `--out`（默认 `.screenshots/`），桌面端和手机端都支持。

几个要点：

- **需要 ffmpeg**（`brew install ffmpeg`）。
- **节奏是真实的。** Chrome 只在画面变化时给帧，所以视频按每帧实际间隔编码：停顿处会真的停顿，而不是被压成定帧率。编码时会跳过画面完全没变的片段。
- **想录什么就写进步骤里。** 视频录的就是场景的步骤，所以为了视频好看，把 `wait` 调到人能看清的长度（比如点开菜单后停 1.5 秒）。
- **指针指示会录进去**，这也是手机端录像看得懂的关键；见「指针指示与标注」。
- **分辨率**：截图是 2 倍像素（1280×860 的视口出 2560×1720 的图）；视频帧由 Chrome 按 **CSS 视口尺寸**发出（它不接受设备像素倍率），所以编码时会用 Lanczos 放大到和截图同样的像素尺寸——是插值放大，不是额外的光学细节。
- 视频和截图共用场景表；同一个场景既能截图也能录像，不需要维护两份。

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
- **录视频**另外需要 ffmpeg：macOS 用 `brew install ffmpeg`，Linux 用发行版包管理器。缺了会在开始录像前直接报错，不会录到一半失败。

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

步骤类型：`eval`、`wait`、`shot`、`tap`（按可访问名称）、`tapText`（按可见文字）、`hover`、`type`、`key`、`scroll`、`inject`，以及下面的标注 / 偏移 / 测量类步骤。三个平台级细节值得先知道：

- **`ready`** 是驱动会反复轮询直到为真的 JavaScript 表达式，它也是截图快的关键：热缓存下约 1 秒就绪，冷启动可能要 20 秒。`settle` 只是就绪后的一小段缓冲。如果截图抢在界面绘制之前，应该调大 `readyTimeout` 而不是 `settle`。
- **`tap` 需要可访问名称。** 先匹配 `aria-label`，再匹配可交互元素自身的文字。两者都没有就用 `tapText`；如果你发现自己在写坐标点击，那通常说明这个控件缺标签。
- **点不到的状态用 `eval`。** 只在悬停时出现的控件，需要先对它的容器加一个 `hover` 步骤（否则驱动会报告目标尺寸为 0，而不是去点页面角落）。例如对话内搜索是 ⌘F 打开的，驱动没法把它当按键发出去，所以场景改为访问 `?press=meta%2Bf`，由 `desktop/shot/main.tsx` 把快捷键派发给真实的 window 监听器。
- **`eval` 里断言就是测试。** 步骤里的 `eval` 抛错会让这次截图以非零退出码失败（和点不到目标一样计入 `step(s) could not reach their target`），所以「界面必须处于某状态」写成 `eval` 断言才有意义：`if (document.body.innerText.includes('正在生成')) throw new Error('…');`。驱动内部自己的探测（`ready`、可选的元素读取）不在此列，不会因为读不到就判失败。

`desktop/shot/main.tsx` 还接受 `?lang=en` 和 `?settings=key:value,...`，用来固定界面语言、在启动前预置应用设置；`mobile/shot/mock/shareIntent.ts` 读 `?share=1`，好让分享面板只出现在分享相关场景里。

### 指针指示与标注

驱动会在页面里画这些标注，截图和视频里都会带上。

- **指针指示**跟着每个 `tap` / `hover` 走：一个蓝色圆点，点击瞬间还有一圈扩散的环。**录像时默认开**（否则手机画面里看不出手指点在哪里），**截图时默认关**（产品图不该因为场景点了几下就多出个圆点）。截图里想要，就加 `--pointer true`，或用 `{"pointer": [x, y]}` 步骤指定位置。
- **标注**来自 `marks` 步骤：目标点上一个编号徽标，旁边跟一句文字。截图能当说明用，靠的就是它。
- **`--annotate false`** 把上面这些全部关掉，得到一张干净的图。

```json
{ "marks": [{ "at": "工作区", "label": "① 工作区列表", "dy": -34 },
            { "at": "对话",   "label": "② 对话列表，可左右滑动切换", "dy": -34 }] }
```

每条标注用 `at`（可访问名称，匹配规则同 `tap`）或直接给 `x`/`y`。`dx`/`dy` 平移徽标，`side`（`"left"`/`"right"`）把文字放到另一侧；两个目标挨得近时必须用其中之一，否则两条标注会叠在一起。编号在场景内累加，需要重新从 1 开始就加 `{"marksClear": true}`；`{"pointerHide": true}` 用来在录像中途把圆点收掉。

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
| `scripts/screenshots/capture.py` | 入口：起服务、截图、录视频、终端外框、PDF 组装 |
| `scripts/screenshots/cdp.mjs` | Chrome DevTools Protocol 驱动（视口、输入、截图、录屏取帧） |
| `scripts/screenshots/scenarios.json` | 两个平台的场景表 |
| `scripts/screenshots/document.css` | 生成文档用的打印样式 |
| `scripts/screenshots/terminal.html` | 命令行输出的终端外框 |
| `scripts/screenshots/injects/` | 变体样式脚本（见「常见请求怎么做」第 5 条） |
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
| `cdp: app not ready after … ms` 且界面停在启动态 | 新界面调用了 mock 还没有的命令。mock 会对未知命令返回 null，前端随后在渲染里抛错、整个界面卡住（桌面端表现为停在「请稍候」）。控制台里搜 `[mock] UNHANDLED COMMAND` —— 那就是缺的处理器，补上即可（`get_agent_status` 就是这么缺的）。 |
| `error: nothing is listening on port …` | dev server 没起：先跑 `capture.py serve-desktop` 或 `serve-mobile`。 |
