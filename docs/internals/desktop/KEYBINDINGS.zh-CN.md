# 快捷键

> （[English](KEYBINDINGS.md)）状态：**已实现（P0 + P1）**。一张命令表、一个分发器、
> 一个设置页，键位按机器保存在本地、用户可改。尚未做：命令面板与 `⌘/` 速查（P2）、
> macOS 菜单项与「终端内也生效」开关（P3）。

本文写的是理由；代码不长，可以对着读。

## 为什么是一张表

在此之前，应用从五个互不相干的地方响应按键：终端开关在
`features/terminal/shortcut.ts`，会话内查找在 `ThreadSearch` 内部（一个捕获阶段
`window` 监听并 `stopPropagation`），`Esc` 走 `components/ui/overlayStack`，审批提示
自带一对，`Enter`/`Shift+Enter` 在 `MentionEditor` 里。每处各自都说得通，但没有一处
知道别的存在——新增快捷键可能悄悄撞车，快捷键无法被发现，也没有任何一处可以改键。

一张命令表一次解决三件事：分发器、设置页、（将来）tooltip 读同一份清单，因此
「文档里写了」「设置里能改」「按下去有效」不可能各自跑偏。

## 布局

```
desktop/src/lib/keybinding.ts            解析 / 序列化 / 匹配 / 排版 / 判定（无 DOM、无 i18n）
desktop/src/features/keyboard/
  catalog.ts                             命令表：id、分类、默认键位、终端策略
  shortcuts.ts                           用户覆盖（localStorage）+ 解析；将来搬去 app_settings 只改这里
  commands.ts                            当前谁拥有哪条命令（注册表 + useCommand）
  dispatch.ts                            规则：这个事件是不是我们的、是哪条命令
  useDispatcher.ts                       唯一的 window 监听（捕获阶段）
  ShortcutHost.tsx                       把监听器做成组件，视图的测试才能装它
desktop/src/features/settings/
  ShortcutsPage.tsx                      设置页
  ShortcutRecorder.tsx                   「按下新的组合键」控件
  shortcutRecorderRules.ts               录制器接受什么、怎么拒绝
```

`lib/` 不含 React 也不含 i18n（`keybinding.ts` 是纯函数）；`features/keyboard` 是
应用级基础设施；设置页是两者的使用者。

## 模型

一个 `KeyBinding` 是若干修饰键加一个 `event.code`：

* `mod`——平台加速键：macOS 是 `⌘`，Windows/Linux 是 `Ctrl`。因此一个默认值
  （`mod+n`）在三个平台都成立，用户改键后也始终是「本平台的加速键」。
* `ctrl`、`alt`、`shift`——显式写出，因为 mac 用户也会想要真正的 `⌃`（和 `⌥`），
  也因为 Windows 布局上的 `Ctrl+Alt` 常常就是 `AltGr`。
* 键是**物理键**（`KeyN`、`Comma`、`BracketLeft`），不是字符。`event.key` 是布局
  打出来的东西：AZERTY 上 `Shift+,` 是 `?`，德语布局上 `z` 与 `y` 互换位置，Dvorak
  用户的 `Ctrl+Z` 会落在另一个物理键上。只有引擎不报 `code`（部分 WebKitGTK 版本、
  合成事件）时才回退到 `key`。

存储用的是规范文本（`mod+shift+bracketright`）：修饰键固定顺序，然后才是键。解析器
不认识的留存值在读取时直接丢弃，绝不猜测。

## 分发规则

`dispatch.ts` 只回答一个问题——*这个事件该不该跑命令，跑哪一条？*——从不执行任何
东西。判定顺序：

1. **已被前一个处理者消费的事件**不属于我们（`defaultPrevented`）。
2. **输入法组词期间键盘归输入法**：`isComposing`、Chromium 的 `keyCode 229`、以及
   WebKitGTK 的 `key: "Unidentified"`。对组词中的按键作出反应，等于在用户打中文时
   触发命令。（终端自己那段输入法事故——中文被发送两次——见 `embedded-terminal.md`。）
3. **AltGr 是打字**：Windows/Linux 把它报成 `Ctrl+Alt`，绑在这一对上的命令会在用户
   输入 `{` `[` `@` 时触发。
4. **没有处理者的绑定不算数。** 没有打开会话就没有 `ThreadSearch`，此时 `mod+f`
   属于平台自己，而不是被吞掉去做一件空事。
5. **弹层或浮动菜单持有键盘**（`overlayStack`、`keyboardOwners`）。两者都按**存在**
   而非焦点判断——WebKit 不会因为点击而聚焦按钮，焦点判定会在菜单明明在屏幕上时
   回答「没人持有键盘」。
6. **按住不放只重复声明允许的命令**（`repeat`）：列表里走动希望连续，
   开关面板不希望。
7. **在终端内，只有标了 `inTerminal` 的命令生效。** Windows/Linux 的加速键就是
   `Ctrl`，而终端是一个 shell：`Ctrl+B` 是「后退一个字符」、`Ctrl+F` 前进、
   `Ctrl+A` 行首、`Ctrl+K` 删到行尾、`Ctrl+[` 是 `ESC`。夺走其中任何一个，都会破坏用户唯一在敲命令行
   的地方的行编辑。

监听装在**捕获阶段**：xterm 把处理函数挂在自己的 textarea 上并取消它处理的按键，
放在冒泡阶段会被抢先吞掉。它真正处理的按键会在那里被终止：`preventDefault` 挡住
浏览器自身的含义，`stopPropagation` 挡住焦点所在的视图再动作一次。

## 导航分两层，因为列表本身就是两层

左栏列两类东西——**工作区**（每个装着若干对话）与**对话**——外加挂在对话下面的分支。
一个按键服务不了全部：扁平的「下一个对话」跨越一个有 20 个对话的工作区要按 20 次，
而只有「下一个工作区」又根本进不了对话。

所以走动是分层的，每层回答自己的问题：

| 层级 | 按键 | 回答的问题 |
| --- | --- | --- |
| 细调 | `mod+[` / `mod+]` | *下一个对话是哪个（不管它在哪）？* |
| 粗调 | `mod+shift+[` / `mod+shift+]` | *哪个工作区、或哪个对话？* |
| 对话内部 | `Page Up` / `Page Down` | *这段对话里的哪一条提问？* |

为什么是这两组键：`mod+[` / `mod+]` 是所有浏览器里「后退 / 前进」的那一对，是习惯而不是
需要背的组合；按住 `⇧` 迈更大一步，是同一个直觉再用力一点。早期版本把细调放在
`mod+alt+↑/↓` 上，为的是避开文本编辑键：键是准的，但最常做的动作要用三根手指。两组都
可以改，所以 `[` 不好按的布局（德语需要 `AltGr`）不用改代码就能挪走。

两层读的是**同一份**列表描述（`ActivityRail.tsx` 里的 `walkBlocks`）：一个块是一个
对话连同挂在它下面的分支，工作区的块就是整个工作区。细调把当前分区绘制的块压平，粗调
在全部块之间跳。各自推导一次，正是第一版出问题的地方——合并列表里细调跟的是树序，粗调
跟的是分组序，于是 `⇧]` 跳过了 `]` 会走进去的那个对话。

值得写下来的细节：

* **粗调跨越两个分区。** 对话与工作区在用户心里是同级的东西，尽管左栏把它们上下分区
  绘制；所以粗调按「置顶 → 工作区 → 对话」走，再反向走回来。打开对话本身就会切换
  分区——外壳按对话自己的类型推导，别处不需要知道这件事。
* **细调只在本分区内走。** 它走的是眼睛能看到的行：打开工作区分区时走各工作区的行，
  打开对话分区时走对话。这才是「屏幕上的下一个对话」；跨过去交给粗调。
* **工作区内部的对话属于该工作区的那个停靠点**，所以从一个嵌套对话按「上一个」，去的是
  这个工作区之前的停靠点，而不是它已经在里面的工作区。
* **没有对话的工作区仍是一个停靠点**（空工作区也会列出来，因此可以选中）；带分支的
  停靠点落在它的根对话上。
* **两者都不循环。** 走出两端就什么都不做，按键也不会被消费；列表就是列表，不是环。
* **无处可去时两条命令都不注册**（只有一个停靠点，或该分区没有列表），按键不会被
  空动作吞掉。

## 模式：只在模式打开时存在的键位表

全局键位表适合「永远成立」的事。**模式**需要相反的性质：它打开时，一个键应该按这个模式
的意思来，而且允许占用全局键位表已经用了的键。`scopes.ts` 就是这一层。

| 作用域 | 何时生效 | 承载什么 |
| --- | --- | --- |
| `global` | 始终 | 上面全部：新建对话、导航、面板、终端、设置 |
| `vibe` | 面板打开 | 布局、设备筛选、增删、待处理队列、应用内全屏 |
| `vibeGrid` | 网格持有键盘（没有格子被放大） | 方向键、`↩`、`1`–`9` |
| `vibeZoom` | 单个格子铺满屏幕 | `Esc` 退出、块光标、右栏两个键 |
| `vibeInput` | 输入层：光标在一个语音编辑器里 | `⏎` 发送、`Esc` 退出 —— 唯一会**越过**编辑器执行的一层 |

解析顺序是**最内层作用域优先**，由内向外，最后才是 `global`。因此：

* 模式内的键在模式打开时**覆盖**同键的全局命令，模式一关就还回去——按构造就不存在
  冲突；
* 两条命令可以**在不同作用域**共用同一个键，录制器只在**同一作用域内**报冲突
  （`conflictingCommand` 是作用域感知的）；
* 裸键（`←`、`↩`、`Esc`、数字）只能在非全局作用域里分配，且分发器只在**该作用域生效、
  且光标不在文本框里**时才执行它。`Shift` 单独不算修饰键：`Shift+Enter` 是换行。唯一的
  例外是输入层：那一层的命令标了 `overEditable` —— 它就是「光标本来就在文本框里」的那一层，
  它的 `⏎` 就是发送。其他任何一层都不许越过编辑器执行。

面板自己的键位（都可改，`vibe.cell` 是唯一例外，见下）。`mod` 在 macOS 是 `⌘`，
其他平台是 `Ctrl`：

| 动作 | 默认 | 作用域 |
| --- | --- | --- |
| 打开 / 关闭面板 | `mod+shift+v` | global |
| 移动选中 | `← ↑ → ↓` | vibeGrid |
| 打开选中格子 → 它的**输入层** | `↩` | vibeGrid |
| 关闭面板 | `Esc` / `返回` | vibeGrid |
| 退出放大 | `Esc` / `返回` | vibeZoom |
| 放大视图里的上一个 / 下一个对话 | `←` / `→` | vibeZoom |
| 上一段 / 下一段消息 | `↑` / `↓` | vibeZoom |
| 进入输入层 | `↩` | vibeZoom |
| 把这次输入发出去（空草稿：收起） | `↩` | vibeInput |
| 退出输入层 | `Esc` / `返回` | vibeInput |
| 当前层的菜单 | `mod+shift+k` / `菜单` | vibe |
| 跳到会话正在发生的地方 | `主页` | vibe |
| 跳到编号格子 | `1`–`9` | vibeGrid |
| 上一个 / 下一个格子 | `mod+[` / `mod+]` | （复用 `thread.previous`/`next`；面板注册，左栏此时不在） |
| 循环网格布局 | `mod+shift+g` | vibe |
| 增减列 / 恢复自动 | `mod+=` / `mod+-` / `mod+0` | vibe |
| 选中格前移 / 后移 | `mod+alt+←` / `mod+alt+→` | vibe |
| 右栏 显示隐藏 / 固定 | `mod+\` / `mod+shift+\` | vibeZoom |
| 切换机器筛选 | `mod+shift+d` | vibe |
| 跳到下一个需要你处理的格子 | `mod+shift+a` | vibe |
| 把会话加进面板 | `mod+shift+e` | vibe |
| 把选中会话移出面板 | `mod+shift+backspace` | vibe |
| 应用内全屏 | `mod+ctrl+f` | global |
| 查看全部快捷键 | `mod+/` | global |

关于这张表的三点：

* **`菜单`、`返回`、`主页` 是遥控器上的键，不是键盘上的。** 它们在命令表里是 `aliases`：
  用户改绑组合键之后、清空绑定之后照样生效；设置页只读显示；但在同一作用域里给别的命令
  录制它们仍会被拒。遥控器的按键是设备的属性，不是一项偏好。哪个键在哪一层做什么、
  缺键时什么会失效，写在面板自己的文档里（`VIBE.md` §9.2–9.3）。

* **`mod+[` / `mod+]` 与左栏是同一对命令**，不是第二套。命令的**含义**来自当前拥有它
  的视图：在面板里由面板把它注册成「下一个格子」，在面板外由左栏注册成「下一个对话」。
  注册表一条命令只保留一个处理者，所以两者不可能同时生效。
* **`1`–`9` 是「一条命令九个绑定」**（`vibe.cell`），因为格子是按印在上面的数字选的。
  设置页把它显示为只读键位列表：给一条九键命令录制单个键，等于问页面一个它答不出的
  问题。`vibe.cell` 的处理者会收到命中的绑定，从里面取数字（`runCommand(id, binding)`）。

面板本身的设计、网格/顺序模型与语音输入模型跟功能走
（`docs/internals/desktop/VIBE.md`）；本文只负责它的键位。有一处设计修订值得记在这里：
那份方案当时把「下一个格子」放在 `mod+alt+↓/↑`，因为左栏的细调那时还在那里。细调后来
挪到了 `mod+[` / `mod+]`，所以面板复用它们——面板的「下一个」和左栏的「下一个」是同一条
命令、同一组键。

## 每条命令注册在哪

外壳注册属于自己的命令（导航、面板、设置）。属于某个视图的，由该视图在挂载期间
注册（`useCommand`）：

| 命令 | 注册者 | 为什么是它 |
| --- | --- | --- |
| `chat.new`、`panel.*`、`section.*`、`settings.open` | `AppShell` | 外壳状态 |
| `thread.find` | `features/agent/search/ThreadSearch` | 只有打开会话时才存在 |
| `chat.stop` | `features/agent/AgentThread` | 中断在 `useAgentThreadState` 里，且只在回复流式输出期间 |
| `composer.focus` | `features/agent/Composer` | 只有输入框自己知道它能不能持有光标 |
| `group.previous` / `group.next` / `thread.previous` / `thread.next` / `rail.search` | `components/layout/ActivityRail` | 绘制顺序是左栏自己的 |
| `terminal.toggle` | `features/terminal/useTerminalPanel` | 需要面板的打开状态；也是终端唯一放行的命令 |

注册即「可用」：没人注册就没人认领。所以没有回复在流式输出时，`chat.stop` 既不做事，
也不占用按键。

## 存储

键位存在 `localStorage` 的 `future.shortcuts.v1` 下，且**只存差异**：
`{"version":1,"overrides":{"chat.new":"mod+shift+KeyK"}}`，`null` 表示「用户移除了
这个绑定」。没有条目的命令继续跟随默认值，因此后续版本改进默认值会到达所有没有
覆盖过它的用户。

为什么用 localStorage 而不是 `~/.future/app/app.db`：

* 键位是**按机器、也按平台**的——在 Windows 上改的 `Ctrl+K` 不该跟着账号跑到 Mac，
  对手机也没有意义；
* 它在按键路径上读取，不在数据路径上；
* 这个应用里同类偏好（主题、字号、面板宽度、终端面板高度）本来就都在那里，理由相同。

`features/keyboard/shortcuts.ts` 是唯一读写这个键的文件。将来要把这份文档搬到后端的
`app_settings`，只需改那里的 `read()`/`write()`。

## 默认键位（v1，全局键位表）

`mod` 在 macOS 是 `⌘`，在 Windows/Linux 是 `Ctrl`。Vibe 面板的键位属于另一张表，
见下面的「模式」。

| 命令 | 默认 | |
| --- | --- | --- |
| 新建对话 | `mod+n` | |
| 停止生成 | `mod+.` | 仅在回复流式输出期间注册 |
| 回到输入框 | `mod+l` | |
| 在当前对话中查找 | `mod+f` | 原本是 `ThreadSearch` 的私有监听 |
| 上一个 / 下一个对话 | `mod+[` / `mod+]` | 细调：逐个对话行；可连按 |
| 上一个 / 下一个工作区或对话 | `mod+shift+[` / `mod+shift+]` | 粗调：按住 ⇧ 迈更大一步；可连按 |
| 筛选对话列表 | `mod+shift+f` | 跳到输入框，不做跳转 |
| 切换到对话 / 工作区 / 技能 / 定时任务 / 远程桌面 | `mod+1` … `mod+5` | |
| 显示或隐藏左侧栏 | `mod+b` | |
| 显示或隐藏右侧面板 | `mod+shift+b` | |
| 显示或隐藏终端 | `mod+j` | 原本硬编码在 `features/terminal/shortcut.ts` |
| 打开设置 | `mod+,` | |

固定按键（设置页列出，不可修改）：`Enter` 发送、`Shift+Enter` 换行、`Esc` 关闭/拒绝、
`⌘/Ctrl+Enter` 批准、`Page Up`/`Page Down` 正文跳转（导航的第三层）、菜单内方向键 + `Enter`。

## 录制器拒绝什么

* **没有修饰键**——裸键是普通打字，`Shift` 单独按也只是打大写字母的方式。
* **保留组合**——平台的文本编辑与窗口组合（`⌘C/V/X/A/Z`、`⌘Q/H/M/W`、`⌥⌘Esc`、
  `Alt+F4`、`⌘Space`、`⌘⇧3/4/5`）以及 webview 的重载与开发者工具（`⌘R`、`⌥⌘I`）。
  被平台吃掉的绑定看起来有值、按下去却没有反应。
* **已被别的命令占用的组合**——拒绝文案里点名是哪条命令。
* **AltGr**——见分发规则。

录制期间 `Backspace`/`Delete` 移除绑定，`Esc` 取消。

## 平台差异

| | macOS | Windows | Linux |
| --- | --- | --- | --- |
| 加速键 | `⌘` | `Ctrl` | `Ctrl` |
| 排版 | `⌃⌥⇧⌘K` | `Ctrl+Shift+K` | `Ctrl+Shift+K` |
| 引擎 | WKWebView | WebView2（Chromium） | WebKitGTK |
| 特有 | `⌘` 组合不产生控制字符，理论上终端可以放行更多（P3）；v1 仍然只放行 `terminal.toggle` | `AltGr` 表现为 `Ctrl+Alt`，不能触发命令 | WebKitGTK 把被输入法消费的键报成 `Unidentified`；组词中按修饰键会取消预编辑（需要再按一次，见 `embedded-terminal.md`） |

系统级全局热键（在其他应用前台时也生效的快捷键）**不在**本次范围内：
`tauri-plugin-global-shortcut` 会从机器上所有其他应用手里抢键，那是另一个功能，
也是另一类风险。

## 测试

| 文件 | 钉住什么 |
| --- | --- |
| `lib/keybinding.test.ts` | 解析、规范文本、两个平台上的 `mod`、物理键与布局字符、排版、保留表 |
| `features/keyboard/catalog.test.ts` | id 唯一、默认值合法且互不冲突、终端白名单、每条命令与分类都有中英文案 |
| `features/keyboard/dispatch.test.ts` | 规则顺序（输入法、AltGr、连按、弹层、终端、无处理者），不依赖 DOM |
| `features/keyboard/shortcuts.test.ts` | 只存差异、重置、可移植的 `mod`、无法解析的留存值 |
| `features/keyboard/useDispatcher.test.tsx` | 真实 window 监听：处理的键被消费、改键下一次按键即生效、终端内不碰 shell 的键 |
| `features/keyboard/scopes.test.tsx` | 作用域按层级排序、两个持有者共享一个作用域、释放互不影响 |
| `features/keyboard/modeKeymap.test.tsx` | 模式键只在模式内触发、同键两个作用域各归各、裸键不抢文本框 |
| `features/settings/ShortcutsPage.test.tsx` | 录制、拒绝（缺修饰键 / 保留 / 冲突）、取消、解绑、恢复、搜索 |
| `ThreadSearch*`、`Composer.focus`、`AgentThread`、`ActivityRail.nav`、`AppShell` | 各视图自己的命令，走真实分发链路 |

除自动化测试外还人工验过：Linux 上中文输入法组词中按修饰键不触发命令；终端里
`Ctrl+B`、`Ctrl+F`、`Ctrl+A`、`Ctrl+K`、`Ctrl+[`（ESC）仍然交给 shell。

## 后续（P2 / P3，未实现）

* `mod+k` 命令面板：命令 + 会话 + 工作区一个入口。`mod+shift+f` 保持不变——它是列表
  过滤，两者不抢语义。
* `mod+/` 速查面板：列出**当前生效**的键位（含用户改动）。
* macOS 菜单项（设置、新建对话）。它们必须只发事件、**不带**加速键：菜单里的加速键
  会在改键后继续显示旧键。
* 每条命令的「终端内也生效」开关，默认关，给需要在终端聚焦时用 `⌘N` 的 mac 用户。
* `leader` 连击（opencode 风格）。数据模型留了位置；v1 没有超时、没有 HUD、没有连击。
