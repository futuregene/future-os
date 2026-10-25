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
python3 scripts/screenshots/capture.py capture-desktop remote-conversation
cp .screenshots/d-remote-conversation.png /tmp/y.png
# 切到版本 X（或另开一个 worktree），同样截一次
python3 scripts/screenshots/capture.py capture-desktop remote-conversation
python3 scripts/screenshots/capture.py compare-desktop /tmp/y.png .screenshots/d-remote-conversation.png \
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

### 8. 终端界面 → 一张真实截图

终端界面（`future tui`）是全屏界面，所以它按桌面/手机端的方式截，而不是走 `terminal.html` 外框：

```bash
cargo build -p future-cli --bin future
cargo build -p future-tui --example demo_agent
python3 scripts/screenshots/capture.py tui tui-chat.png
```

图里的程序就是真实二进制：`capture.py tui` 先起 `demo_agent`（一个只回一套固定会话的 gRPC 替身），再在固定尺寸的 pty 里跑 `future tui --offline --grpc-addr …`，把它写进 pty 的字节原样留下。`tui_terminal.py` 把这股字节流按终端单元模型重放（绝对定位、SGR 颜色、双宽 CJK、256 色板、以及应用留下的光标），再由 headless Chrome 把字形画出来。

几个参数：

- `--cols` / `--rows` 决定 pty 尺寸（默认 130×40）。画面是它们的函数，所以布局变了会体现成"另一张图"，而不是被压扁的一张。
- `--keep-stream <file>` 把原始字节另存一份——截图是空白时，第一个要看的就是它。
- `--scale` 设置设备像素比（默认 2）。
- `--session <id>` 指定会话，而不是交给 agent 决定。
- `--scenario <name>` 把界面驱动到读者实际会到达的那一屏：`chat`（打开时的样子）、
  `help`（`?`）、`palette`（`/`）、`palette-filter`（按下 `/` 再输入查询、不回车）、
  侧栏面板 `todo` / `usage` / `skills` / `sessions`（靠点击页签切换，没有对应按键）、
  两条指面板的命令 `command-usage` / `command-skills`、`launchpad`（`/new`）、
  `full-panel`（窄终端上的 `ctrl+b`）、`scrolled`（点一下聊天再滚轮上滚三次）、
  `light`（`/theme` 后下移一行回车；它会持久化，所以放在最后）以及 `drag`
  （抓住分隔条往右拖六列）。所有坐标都在步骤执行时从当时屏幕上的帧算出来——点页签按
  画出来的标签，拖拽按画出来的分隔条——所以跟着布局走，而不是写死列号。
- `--addr <host:port>` 接到已在运行的 agent（也就是**真会话**），而不是启 `demo_agent`；
  并且不动调用方的 `FUTURE_HOME`。

仓库里拍这些帧的入口是 `tui/scripts/shot.sh <scenario> [WxH] [out.png]`，它是
`capture.py tui` 的一层包装：补上 `--scenario` 与尺寸，并按对应资产命名文件。

```bash
tui/scripts/shot.sh chat 120x38      # → docs/tui/assets/workbench-120x38.png
tui/scripts/shot.sh usage 120x38     # → docs/tui/assets/workbench-panel-usage-120x38.png
tui/scripts/shot.sh drag 120x38 $PWD/docs/tui/assets/workbench-drag-fixed.png
```

两次运行产出的 PNG 是逐字节相同的，这才让它能当评审证据用：有差异说明界面变了，而不是模型这次回答得不一样。两点限制照实说：字形是 Chrome 依据抓到的单元格画的，不是产生这些字节的那个终端画的；数据是脚本化的（和其他 mock 一样）——截图永远不是"真实用户会话"的证据。

```bash
python3 scripts/screenshots/tui_terminal.py --self-test   # 只验单元模型，不开浏览器
python3 scripts/screenshots/capture.py --self-test        # 只验场景步骤，不开浏览器
```

## 录制视频

```bash
# 终端 1、2：先起服务（同上）
python3 scripts/screenshots/capture.py video-desktop            # 全部场景
python3 scripts/screenshots/capture.py video-desktop steps-expanded rename  # 或指定几个
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
- **命令行输出是"样式化呈现"，不是截屏**：`terminal.html` 把命令的真实 stdout 渲染进终端外框。终端界面**不属于**这一类——`capture.py tui`（见第 8 条）在真实 pty 里跑真实二进制，再把它写出的字节栅格化，所以图里的画面就是应用自己画的。它的**数据**仍是脚本化的，和桌面/手机端 mock 一样。
- **有一处 web 独有的渲染偏差是被修正的，而不是被拍进去的。** web 上 react-native-web 会把 inverted 列表的 refresh control 包装也带上列表自己的翻转，于是转录上下颠倒——真机不会这样。镜像的手机截图比没有还糟，所以 `cdp.mjs` 会在手机端截图时中和这层多余翻转（DOM 里能看到修正：那个包装没有 `transform` 了），并由 `check-upright-mobile` 把结果量出来。

## 环境要求

- Node.js（驱动用到的 `ws` 已是仓库依赖）和 Python 3。
- Chrome、Edge 或 Chromium。不支持 Safari（驱动需要 CDP）。
- 截图只驱动 **headless** 浏览器，而且只驱动自己起的那个（或 `--cdp-port` 上已在跑的 headless 实例）。该端口上若是**可见**的浏览器，工具会提示并跳过、绝不去驱动——驱动它就会导航并截取你正看着的那个窗口——改在下一个空闲端口起自己的浏览器。被打断的运行（turn 超时、agent 重启）会留下浏览器和 `future-shot-` profile：每条命令启动时都会回收它们，SIGTERM 处理器则在运行被杀时收掉浏览器。该处理器只在 POSIX 上成立：Windows 无法把 SIGTERM 投递给 Python 进程，`os.kill` 在那里是直接终止进程、处理器不会执行，所以那边靠下一条命令启动时的回收——它用 `Get-CimInstance Win32_Process` 读进程表（Windows 没有 `ps`），并以「父进程已不在」判定孤儿（Windows 不会像 POSIX 那样把孤儿改挂到 init）。
- `pip install pillow` 是可选的：它只用来压小 PDF 里的图片。但 `compare-*` 与 `check-upright-mobile` 需要它，缺了会明确报错。
- 手机端工具另外需要 `react-native-web`、`@expo/metro-runtime` 和 `react-dom`；`capture.py serve-mobile` 会把它们按需装进 mobile workspace，且不动依赖清单（见"不入库的约定"）。
- **录视频**另外需要 ffmpeg：macOS 用 `brew install ffmpeg`，Linux 用发行版包管理器。缺了会在开始录像前直接报错，不会录到一半失败。

## 桌面端

两个终端，因为 dev server 一直占着前台：

```bash
python3 scripts/screenshots/capture.py serve-desktop     # 终端 1：提供 http://localhost:5299/shot/
python3 scripts/screenshots/capture.py capture-desktop       # 终端 2：跑全部场景
python3 scripts/screenshots/capture.py capture-desktop rail-tree steps-folded              # 或只跑几个
```

场景名就是 `scripts/screenshots/scenarios.json` 里的键（`d-` 前缀属于输出 PNG，不属于场景名）；名字写错时 `capture-desktop` 会列出可用的场景。

截图写到 `.screenshots/`（已 gitignore）。你也可以自己打开 <http://localhost:5299/shot/> 手动探索；工具还会在 7391 端口起一个替身 PTY 服务，让内嵌终端面板有东西可显示。

## 手机端

```bash
SHOT_WEB=1 python3 scripts/screenshots/capture.py serve-mobile     # 终端 1：Expo web，http://localhost:8099/
python3 scripts/screenshots/capture.py capture-mobile       # 终端 2
```

手机端以手机视口渲染（默认 390×844、2 倍像素密度），并通过 HTTP 提供演示图片，所以在聊天里内嵌的图片和可缩放预览都能正常显示。`serve-mobile` 会导出 `SHOT_WEB=1`；这是唯一的开关——`mobile/metro.config.js` **仅在 web 平台、仅在该变量为 1 时**把 remote context 和两个原生模块指向 `mobile/shot/mock/`，因此原生构建（`make run-mobile-android`）完全不受影响。

### 聊天转录是正的（inverted 列表那一层多余翻转）

`mobile/src/features/chat/components/ChatTimeline.tsx` 用 `inverted` 的 FlatList 渲染转录。react-native-web 靠**两层**翻转实现它——滚动视图翻一次、每个 cell 再翻回来——两层相消，所以最新的一行在底部。列表一旦同时带 `refreshControl`，就会多出**第三层**：`ScrollView.render` 用滚动视图自己的样式克隆 refresh control（`dist/exports/ScrollView/index.js` 里的 `React.cloneElement(refreshControl, { style: props.style })`），而这套样式里带着那个翻转。于是整份转录上下颠倒——字是镜像的、行序也是反的——真机不是这样，所以这种截图会和设备对不上。（在 react-native-web 0.21.4 上复现，也就是 `serve-mobile` 现在装的那版。）

`cdp.mjs` 在手机端截图的页面里中和掉这一层（`UNFLIP_INVERTED_LIST`）：找出"唯一子节点就是那个被翻转的滚动容器"的外层包装，去掉它的 `transform`，并在 React 重渲染后再补上。不改产品代码、不改依赖版本，而且这个修正就摆在 DOM 里。每次运行都会报告它做了什么：

```
cdp: neutralized 1 inverted-list wrapper(s) (react-native-web gives a refresh control the scroll view's inversion)
```

`chat` 场景断言了结果（`the transcript is not mirrored`），所以转录镜像的截图会直接失败、不会被发布出去，这段 workaround 也不会悄悄烂掉。要判定**一张截图**而不是一次运行：

```bash
python3 scripts/screenshots/capture.py check-upright-mobile chat
```

它带着 `upright-probe.js` 跑一次该场景：把转录里的文字原样复制一份到没有翻转祖先的 `document.body` 里，于是同一张截图同时有"原位"和"正立"两块同样文字、同样字号字宽的区域，再把两块做直接相关和上下镜像相关：

```
   in-place tile: 348x64 css px at 21,389
   control tile: 348x64 css px at 7,773
   upright  correlation 1.000   (the transcript against an unflipped copy of itself)
   mirrored correlation 0.205   (the same comparison, control flipped top-to-bottom)
   verdict: upright — the capture reads the right way up
```

镜像时两个数字对调、命令以非零退出；两个分数相差不到 0.05 时报"inconclusive"而不是放行。判定逻辑与该方向由 `scripts/tests/test-screenshot-upright-check.py` 离线钉住（`make test-screenshot-harness`）。

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

**分片文件 `scenarios.d/`：** 两个 session 同时给不同平台加场景时，不要都改 `scenarios.json`（后写的会覆盖先写的）。改成在 `scripts/screenshots/scenarios.d/` 各写一个片段，`capture.py` 启动时按文件名顺序合并到共享表上：

```json
{ "mobile": { "audit-settings-bottom": { "steps": [ { "wait": 900 }, { "shot": "m-audit-settings-bottom.png" } ] } } }
```

片段按平台名挂到对应表下，同名场景以片段为准（可用于临时覆盖）；平台名写错或结构不对会直接报错，不会静默丢掉整段场景。`make test-screenshot-harness` 钉住了这三条规则。

**场景可以有自己的 `ready`，这是让「就绪」不再是竞态的办法。** `capture.py` 传的是 `spec.get("ready", config["ready"])`，所以比「界面已绘制」要求更多的场景可以自己说：深色窗口的主题选择器要断言 `document.documentElement` 带 `dark`，于是它的就绪条件里就包含这个 class，而不是靠一个「够长」的 `wait` 去赌冷启动。条件写进 `ready`，断言仍然是断言。

**截图判断不了 `hitSlop`。** `react-native-web` 不实现它——用 `elementFromPoint` 探到的是绘制盒——所以只扩大触控区域的移动端修复，在截图和 `TOUCH` 探针里都看不见。这类改动由单测守（`mobile/src/features/chat/messages/touchTargets.ts`，断言在 `foldedRowTouchTargets.test.ts`），真实效果只能在设备上看。不要靠「把绘制盒撑大」让截图好看：那改的是设计，不是命中区。

`desktop/shot/prefs.ts`（在 `main.tsx` 之前加载）负责固定那些「在 import 时就被读走」的偏好：`?lang=en`、`?theme=dark|light|system`、`?palette=<主题 id>`（见 `lib/theme.ts` 的 `THEME_PALETTES`）；它同时给出默认值（zh / light / FutureOS），所以切换配色主题的场景不会渗到同一轮后面的截图里，深色系统的机器也仍然拍到多数用户看到的画面。`?settings=key:value,...` 用来在启动前预置应用设置；`?remoteSlowRead=1` 会把对端主机的回答压慢，这是截图唯一能拍到"读取还在途中"那一刻的办法（例如模型控件在对端回答之前的样子）；`mobile/shot/mock/shareIntent.ts` 读 `?share=1`，好让分享面板只出现在分享相关场景里。

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
  target/debug/futureos --headless
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
| `scripts/screenshots/tui_capture.py` | 在固定尺寸 pty 里跑终端界面并留下它的字节（第 8 条） |
| `scripts/screenshots/tui_terminal.py` | 把这些字节重放成 PNG 的终端单元模型 |
| `scripts/screenshots/injects/` | 变体样式脚本（见「常见请求怎么做」第 5 条） |
| `scripts/screenshots/upright-probe.js` | 把截图里的原文复制一份正立的副本（见手机端「聊天转录是正的」） |
| `scripts/screenshots/examples/` | 完整示例：1.1.8 更新说明的内容文件 |
| `scripts/screenshots/gen-demo-assets.py` | 重新生成演示图片（需要 matplotlib） |
| `desktop/shot/` | 桌面端 mock、演示数据、入口、替身 PTY 服务 |
| `mobile/shot/` | 手机端 mock 与演示数据 |

## 排查

| 现象 | 原因 |
|---|---|
| `cdp: no page target on port …` | 上一次运行留下的 **headless** 浏览器占着 CDP 端口且没有页面目标；关掉它或换 `--cdp-port`。该端口上若是**可见**浏览器，工具会提示并跳过，而不是去驱动它。 |
| `note: CDP port … 被占用 / 是可见浏览器` | 不是错误：那个浏览器不是本次运行可以驱动的，于是这次截图改在下一个空闲端口起自己的浏览器。 |
| `target not found: …` | `aria-label` 变了，或步骤跑在界面稳定之前——把前一个 `wait` 调大。 |
| 点击没有反应 | 在可滚动列表里，react-native-web 会优先响应滚动手势；驱动已经发送触摸序列，请确认没有关掉 `--touch`。 |
| 手机端界面空白或不全 | 首次请求时 Expo web 还在打包；等第一次截图完成后再跑一次。若页面全白且控制台报 "Incompatible React versions"，说明 mobile workspace 里 react / react-dom 版本不一致——`serve-mobile` 会检查并给出修复命令。 |
| `cdp: app not ready after … ms` 且界面停在启动态 | 新界面调用了 mock 还没有的命令。mock 会对未知命令返回 null，前端随后在渲染里抛错、整个界面卡住（桌面端表现为停在「请稍候」）。控制台里搜 `[mock] UNHANDLED COMMAND` —— 那就是缺的处理器，补上即可（`get_agent_status` 就是这么缺的）。 |
| 捏合截图拍出来是空白（或画面从手指下漂走） | 缩放锚定在「每次布局只测一次」的窗口原点上：滑入式面板是在**还在屏幕外时**完成布局的，于是那个原点永远差着大半屏，之后每次捏合都会把画面平移 `偏移 × (缩放 − 1)`，直接推出视野。改用**相对视图**的触点坐标（`locationX/locationY`），并在场景里断言「居中捏合的平移量接近 0」——这条断言在旧行为下会直接失败。 |
| `error: nothing is listening on port …` | dev server 没起：先跑 `capture.py serve-desktop` 或 `serve-mobile`。 |
