# 嵌入式终端

> （[English](embedded-terminal.md)）状态：**Linux** 已实现（自动化测试验证）；
> macOS/Windows 路径存在但**未在本机验证**——见 §平台。

终端是一个由桌面主进程持有的 PTY 注册表，通过**仅回环的 HTTP/WebSocket 监听器**
服务给应用自己的 webview。它刻意不是 Tauri-IPC 消息泵。

## 为什么用 socket

这个设计来自 opencode，照搬它的理由不是赶时髦：

* **恢复只需一个数字。** 输出是带绝对游标的字节流。重连的视图（面板重开、webview
  重载、tab 重新挂载）只索取它已应用游标之后的字节，因此不会重渲染也不会丢内容。
* **渲染器保留自己的屏幕。** xterm 的序列化缓冲按 tab 持久化，即使服务端的有界
  tail 已越过该客户端，屏幕仍然存活。
* **不靠 GUI 也能测试。** `cargo test terminal::server` 走真实 TCP 驱动真实监听器，
  经真实 WebSocket 流式跑一个真实 shell。早先的 IPC 设计只能靠人点着应用检查。

## 布局

```
desktop/src-tauri/src/terminal/
  session.rs    PTY 子进程 + 有界输出 tail + 绝对游标 + viewers
  manager.rs    注册表、容量上限、退出保留、会话拆除
  pty.rs        portable-pty 边界；会话级进程树拆除
  protocol.rs   线上辅助：控制帧、replay 分块、输入解码
  server.rs     回环监听器：控制路由、票据签发、WebSocket 泵
  ticket.rs     一次性、会话级、60 秒连接票据
  cwd.rs        工作目录解析（线程 → 工作区 → 家目录）
  shell.rs      默认 shell + shell 列表
desktop/src/features/terminal/
  client.ts         控制路由；带服务端代码的类型化错误
  tabs.ts           每会话 tab 状态（持久化视图 + 标签）
  useTerminalTabs.ts 与服务端对账，创建/关闭/重启
  useTerminalPanel.ts 打开/高度偏好 + 全局快捷键
  TerminalView.tsx  xterm + WebSocket（懒加载 chunk）
  TerminalPanel.tsx tab 条、提示、退出
  TerminalToggleButton.tsx 头部入口
```

## 线上协议

控制路由是 HTTP 上的 JSON；输出走 WebSocket。

| 路由 | 用途 |
| --- | --- |
| `GET /terminal/shells` | 客户端可提供的 shell |
| `GET /terminal?threadId=` | 某会话的会话列表（运行中 + 保留的退出） |
| `POST /terminal` | 创建 `{threadId, title?, cols?, rows?, cwdPolicy?}` |
| `GET /terminal/:id` | 单个会话 |
| `PATCH /terminal/:id` | `{title?, cols?, rows?}` |
| `DELETE /terminal/:id` | 终止进程树并遗忘该会话 |
| `POST /terminal/:id/connect-token` | 签发一次性连接票据 |
| `GET /terminal/:id/connect?cursor=&ticket=` | WebSocket 升级 |

socket 上：

* **server → client**：PTY 原始字节的二进制帧，外加一个控制帧——`0x00` 后跟
  UTF-8 JSON `{cursor, start, exitCode?}`；
* **client → server**：二进制或文本帧，按 UTF-8 解码（非法 UTF-8 丢弃、绝不替换
  ——损坏的字节不能被敲进 shell）。

`cursor` 是要恢复的绝对末尾偏移。`start` 是首个重放字节的偏移：`start > requested`
表示保留的 tail 已盖不住客户端所要的内容，视图会明说这一点而不是显示一个空洞。
关闭码 `1000` 表示 shell 已退出；`4408` 表示 viewer 落后了、必须重新挂接。

缓冲：会话保留 2 MiB 输出；尚未激活的 viewer 最多累积 1 MiB，之后以 `4408` 切断
（它重挂接并重放）。脱离的 viewer 的内存增长永远不会无界。

## 安全模型

* 绑定 `127.0.0.1` 的**临时端口**；LAN 永远不可达。
* 每进程一个 32 字节随机秘密，经唯一的 Tauri 命令 `terminal_server_info` 交给
  webview；每条控制路由都要求携带。
* WebSocket 无法携带 header，所以它兑换一张经认证路由签发的**一次性、会话级、
  60 秒票据**。
* 来源不是应用自身（或回环 dev server）的请求即使持票也拒绝。来源检查是纵深防御：
  秘密与票据才是真正的门。
* 监听器每连接只服务一个请求并限制并发连接数；不接受 keep-alive，也不接受流水线。
* 终端输出永远不进 agent、RPC 桥接、远程控制面、日志或 SQLite。子进程环境是用户
  自身环境减去应用的私有管道（`FUTURE_AGENT_GRPC_ADDR`）。

## 生命周期

* 会话是**会话级**的：创建时必须带 `threadId`，删除会话（或其工作区）会关闭它的
  终端。shell 不可能活得比它打开时所处的上下文更久。
* 应用退出时有界宽限期拆除所有会话。
* 已退出的 shell 仍可寻址（最终屏幕、退出码），直到 tab 关闭或此后有 25 个会话
  退出。
* 面板只在打开时没有任何 tab 的情况下自动创建一个。其余 shell 都由用户的显式操作
  创建。
* 收起面板只是**隐藏**，从不关闭任何东西。shell 继续跑，tab 保留其序列化屏幕，
  重新打开会重挂到同一会话。只有关闭 tab（或删除会话）才终结 shell。收起时还把
  光标交还给输入框：面板控制器发出 `futureos:focus-composer`，Composer 响应它
  （在无法持有光标时拒绝）。否则焦点会停在一个已卸载的终端上，接下来的按键将
  无处可去。
* 快捷键（`Ctrl+J`，macOS 为 `⌘J`）属于应用，不属于 shell。`TerminalView` 把它从
  xterm 的按键处理中释放出来，使终端持有焦点时窗口监听器仍能看到它——否则 xterm
  会取消该事件并给 shell 发一个换行。唯一定义在 `features/terminal/shortcut.ts`。
* **输入法组合期间键盘归 IME。** 同样的自定义按键处理器对所有标记 `isComposing`
  的事件也退让，使组合中的按键到达输入法而不是终端。xterm 自带的组合启发式只懂
  Chromium 的"此键属于 IME"约定（`keyCode === 229`）；WebKit——桌面 webview 在
  Linux 上用的引擎——以真实 keyCode 报告被消费的按键（组合中的 Backspace 以
  `keyCode 0` 到达）。遇到这种键 xterm 会跑 `_finalizeComposition(false)`，把
  textarea 此刻持有的内容提交掉，于是*打字输入拼音、按 Backspace 改一个字母再提交，
  中文文本被送进 shell 两次*——命令行留下一份删除都清不掉的残留。策略与捕获的
  WebKit 序列在 `features/terminal/keyPolicy.ts`；IME 预编辑框在
  `styles/globals.css` 中用终端自己的调色板绘制（xterm 自带黑底白字的暗色主题
  默认值，而本应用只做亮色）。两个值得知道的实测细节：组合期间按带修饰键的键
  （如 `Ctrl+J`）会让 IME **取消**待定预编辑，且浏览器把该键报为
  `key: "Unidentified"`——所以面板快捷键在组合中无法被识别，需要再按一次（两种
  情况下都不会有东西漏进 shell）；候选词用空格或数字键选择/确认，而合成的
  `Enter` 会让本 IME 提交原始拼音（`nihao`）——已验证这是输入法自己的选择，因为
  一个普通 `<input>` 的行为完全相同。

## 工作目录

由服务端从会话解析；客户端从不发送路径。

1. 与该线程严格关联的工作区（`workspace_id`，因此聊天临时工作区与真实工作区永不
   混淆）；
2. 用户家目录——但**仅当**什么都没配置时，或用户显式确认该回退之后。

已配置但缺失/不是目录的路径以 `CWD_INVALID` 响亮失败。这很重要，因为
`portable-pty` 对坏 cwd 会静默替换成 `$HOME`（在先前的 spike 中已验证）；后端在
spawn 之前先行校验。

## 进程拆除

带作业控制的 shell 会把后台作业放进**各自**的进程组，所以单靠
`killpg(shell)` 会留下 `sleep 300 &` 继续跑。因此拆除按 *session* 遍历：向进程组
发 SIGTERM、有界等待、SIGKILL，再扫一遍 session 成员（Linux：`/proc`；其他 unix：
`ps -o sess`）。Windows 用 `taskkill /T /F`。这是回归测试而非理论——
`terminal::pty::tests::teardown_reaches_background_jobs`。

## 平台

| 平台 | 生成/IO | 拆除 | GUI 端到端 |
| --- | --- | --- | --- |
| Linux | 已验证 | 已验证（session 扫描） | 本环境**未运行** |
| macOS | 未运行 | 未运行（`ps -o sess` 路径） | 未运行 |
| Windows | 未运行 | 未运行（`taskkill /T`） | 未运行 |

上表之外不再声称任何能力。Windows 上的 Job Object 与 macOS 上的
`proc_listchildpids` 会严格优于当前的尽力而为路径。

键盘在 Linux 上超出应用自身测试套件之外**确实**被覆盖：在系统的 WebKitGTK（与
此处 Tauri webview 相同的引擎）里驱动真实 `TerminalView`，对接真实 PTY 跑用户登录
shell，以 **ibus-libpinyin** 作为输入法，经 `WebKitWebDriver`——IME 预编辑、提交、
组合中删除与面板快捷键都以真实事件观察。上面的组合中 Backspace 行为就是这样发现并
修复的。macOS 与 Windows 输入法仍**未运行**；策略只依赖 `KeyboardEvent.isComposing`
——所有引擎都会设置它。

## 手工验证（5 分钟，GUI）

1. `npm run tauri:dev`（或跑打包构建），打开一个会话。
2. 按 **Ctrl+J**（macOS 为 ⌘J）或点头部的终端按钮。
3. 敲 `pwd` ——必须打印该会话的工作区目录。
4. `sleep 300 &` 然后关掉 tab；`pgrep -f "sleep 300"` 必须为空。
5. 终端自身持有焦点时按 **Ctrl+J**（⌘J）：面板必须收起（不得给 shell 发换行），
   光标必须回到输入框。再按一次，或点面板头部的 ✕：屏幕与回滚仍在，shell 是同一
   个进程，你做的 `cd` 仍然有效。
6. 重载 webview（⌘R / Ctrl+R）：面板恢复同一屏幕，shell 继续跑（不会 spawn 新的
   shell）。
7. 在 shell 里 `exit`：tab 显示退出码并提供重启。
8. 用输入法打中文，用 Backspace 改一个字母，然后提交：字符必须只出现**一次**，
   删除后必须留下干净的一行（无残留副本、无多余退格）。预编辑必须用终端自己的
   颜色渲染，而不是黑框。

## 与 opencode 的差异

架构相同；以下是有意为之的偏差，免得未来读者把它们当事故：

| 领域 | opencode | 这里 | 原因 |
| --- | --- | --- | --- |
| 渲染器 | `ghostty-web`（libghostty WASM） | `@xterm/xterm` | 本仓库的依赖政策不让 WASM blob 进桌面包；xterm 无运行时依赖，且已是本应用面向的契约。终端组件除 import 外与渲染器无关。 |
| 游标单位 | UTF-16 字符串长度 | 字节 | PTY 产出字节；按解码字符计数在任何非 ASCII 输出上都会漂移。 |
| 作用域键 | 目录 | `threadId` → 工作区 | future-os 没有 worktree 概念，会话才拥有工作区；删除会话必须关闭它的 shell。 |
| 挂接到已死会话 | 拒绝 | 重放最终屏幕 + 退出码 | tab 能显示发生了什么，而不是一个空洞的错误。 |
| 拆除 | `killpg` | 会话级扫描 | 已验证：作业控制的后台作业在各自的进程组里。 |

## 已知限制

* UI 里没有拖拽排序、重命名、shell 选择器（服务端已暴露 `GET /terminal/shells`）。
* 还没有终端内搜索、链接处理、粘贴确认。
* 远程（手机遥控）与移动端界面按设计看不到终端。
* 面板的打开/高度偏好与 tab 视图状态在 `localStorage` 里；清掉存储丢的是*视图*，
  永远不会丢 shell。
