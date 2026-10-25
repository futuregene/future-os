# 主机模式使用指南

[English](desktop-host.md)

主机模式让 FutureOS Desktop 在终端中提供手机远程入口，不打开窗口或 WebView。
适合通过 SSH 使用服务器，也适合不需要桌面 UI 的本机运行。它复用 Desktop 的会话、
工作区、审批和文件处理，不是另一套 Remote 服务或 SDK。

**默认前台运行、持续占用终端；按 Ctrl+C 关闭远程入口并退出。**
不会自动转入后台，也不会安装系统服务。

统一命令随 CLI 发布；正式打包、签名与同候选验收进度见 [Headless 拆分开发计划](../architecture/headless-package-development.zh-CN.md)。

## 1. 启动前准备

- 安装统一 `future` CLI，CLI-only 压缩包也包含完整主机后端。
  使用 `future host`，无需单独安装 Desktop 或服务器程序。
- 默认端点和数据目录没有外部服务占用时，完整 Agent 在当前进程内运行。
  维护直接调用当前 CLI；外部 Agent 仍需通过兼容性和 readiness 检查才会连接。
- 手机安装匹配生产/测试环境的 FutureOS App。编译渠道与手机环境必须一致。
- 主机能够访问 Future OS 平台和配置的 NATS 中继；手机也需要联网。
- 使用普通系统用户运行。平台登录、主机配对和工具执行均属于该运行用户；不要为方便而使用 root。
- 同一数据目录下只能运行一个 Desktop 后端：启动主机模式前，先退出使用该目录的 GUI。

平台登录需要有效的 Future OS 账号凭证，但**不需要在服务器上打开浏览器**，启动时也
**不需要先登录**：未登录的主机会照常提供全部本地能力（Agent、会话、任务、自己的模型 provider），
只是远程桌面保持关闭。登录是一条独立命令——`future auth login`，它会打印授权链接和用户码；
运行中的主机会直接接管这个登录并开始服务，不需要重启。配设备是又一步独立操作——
`future remote devices add` 在运行它的终端里自己打印邀请。

## 2. 前台启动

`.deb` 安装后可以直接运行 `future host`；portable 或源码构建请进入该可执行程序所在目录。

Linux / macOS：

```bash
./future host
```

Windows PowerShell：

```powershell
.\future.exe host
```

不需要也不接受 `--headless` 参数。`futureos` 只启动图形 Desktop，原来的 `--headless` 选项已移除。
旧命令如 `futureos --headless --no-qr` 应改成 `future host`。
两个入口都不再接受配对相关参数：设备统一用 `future remote devices` 管理（见 3.1），
由它在运行中的 Desktop 里完成配对。
两个程序共享后端、数据和手机协议，不是另一套实现。

无需也没有 `--pair --qr` 这类参数：入口只服务已经配对到这台机器的设备，配一台新的是一步
独立命令（`future remote devices add`，见 3.1）。配对不会结束命令或返回 shell 提示符，
进程继续运行并处理手机请求。

如果远程桌面开关是关闭的（在窗口的 Remote 页面关掉，或用 `future remote disable`），
本进程启动后处于空转而不是服务：它仍作为这台机器的控制端点留在前台，
`future remote enable` 可以直接叫它开始服务，不需要重启（见 3.2）。

### 第一步：平台登录授权

主机入口自己不跑登录流程。未登录的服务器照样启动并提示当前是本地模式，
只有远程桌面保持关闭。需要远程时，在同一台机器的第二个终端登录：

```bash
future auth login
```

1. 用手机系统相机或浏览器扫描二维码，或在自己电脑/手机的浏览器中打开授权网址。
2. 登录 Future OS，按页面提示授权当前服务器；若页面要求用户码，输入终端显示的码。
3. 凭证经由运行中的 Agent 落盘，主机在一秒内接管它：打印 `Platform account signed in`、
   刷新模型目录，并开始服务已经配对到这台机器的设备——不需要重启。

这一步授权的是“服务器访问你的平台账号”，不是手机对服务器的远程控制。
不需要服务器浏览器、SSH 图形转发或入站浏览器回调端口。

运行中**换号登录或退出登录**会关闭远程访问并提示重启：旧账号的远程入口不能拖到它自己的
凭证过期才关。无论哪种情况，本地能力都不受影响。

已存的凭证在启动时仍会向平台核实（与从前一致）：有效则直接服务；平台拒绍（401/403）会打印
`Platform authorization is no longer valid. Please sign in again.`，然后按未登录状态继续本地服务，
等你用 `future auth login` 换一枚新凭证；网络不通仍按原来的方式报错退出，不会把“验不了”当成“已退出”。

### 第二步：手机 App 配对

主机入口自己**不会**打印配对码。配对由操作者显式发起：在同一台机器的另一个终端运行

```bash
future remote devices add
```

它会请运行中的主机签发一张邀请，并在**发起请求的那个终端**里打印出来：二维码、
完整的 `futureos://remote/pair?...` 链接，以及剩余有效期。

邀请由平台签发，所以这是唯一一个名字看着很“本地”、却需要账号的命令：未登录时 `add` 会拒绝
并提示先执行 `future auth login`。此时主机本身没有坏——会话、任务、自定义 provider 照常可用；
登录被接管后 `add` 就能成功。

- 用 **FutureOS App 内的添加设备/扫码入口**扫描这个二维码。
- 不方便扫码时，在 App 的“粘贴配对码”入口粘贴完整链接。

这里的“配对码”是完整邀请链接，不是几位数字的短码。邀请五分钟内有效且只能使用一次。
不要分享邀请、公开截图或将其保存到共享日志中。配对期间主机一直在运行，
手机领取邀请后它立刻开始服务这台新设备——不需要重启，也不需要另外再启动什么。

### 已经登录或配对

有效登录和配对会复用，对应步骤自动跳过。已配对的手机直接连接，不需要每次扫码。
“入口就绪/已有配对”不等于手机当前在线；手机需要打开 App 并连接该主机。
普通断网不会主动替换有效配对，网络恢复由现有 Remote 连接逻辑处理。

### 服务器自行维护的工作

主机模式提供与 GUI 相同的后端，所以手机上看到的是同一批会话、设置、技能、模型和任务。
有两步原本由 GUI 界面驱动的工作，改由这个入口自己完成 —— 服务器没有窗口来渲染它们：

- **模型目录刷新。** 启动时拉取一次 Future 模型目录，之后每 24 小时刷新，与 GUI 的定时任务
  使用同一套调度策略。刷新由 Agent 自己广播变更，因此已连接的手机通过 GUI 端同样的失效
  通知流即可看到新模型。
- **内置技能。** 首次登录成功后运行一次 bundled `future init`（之后每次启动也会运行，它是
  幂等的），安装平台内置技能，与 GUI 登录后引导流程里的那一步完全一致。它的输出带
  `[skills]` 前缀打印；失败只会报告，不会中断服务器。

GUI 的另外两个定时任务在服务器端没有对应物：应用更新检查依赖桌面端更新插件，余额刷新
只喂给手机不读取的窗口事件。

## 3. 启动选项

| 选项 | 行为 |
|---|---|
| `--relay URL` | 新邀请指向自建中继，而不是 Future 平台（仅本次启动有效） |
| `--relay-ca` | 该中继使用私有 CA 时，指向其 PEM 文件 |
| `--help` | 显示帮助，不启动 Desktop 或 Agent |

例如：

```bash
./future host --relay https://relay.example.com
./future host --help
```

`future remote devices add` 会把邀请以二维码加完整链接的形式打印。终端过窄时自动回退为
只打印链接并说明原因，因此不需要专门的开关；本入口自己也不再接受任何配对参数，见下文。

### 3.1 在终端管理已配对设备

一台机器可以同时配多台设备（手机、另一台电脑），但其中只有一台**在控**（in control），
只有它被允许指挥这台机器；其余设备保持配对、处于待命（standby）。设备统一用 `future` CLI 管理：

```bash
future remote devices                     # 列出已配对设备、谁在控、由平台还是中继签发
future remote devices add                 # 配一台新设备：打印它的邀请
future remote devices remove <id|序号>     # 只解绑这一台，其余保持配对
future remote devices remove --all        # 解绑全部设备
future remote devices use <id|序号>        # 指定哪一台在控
```

- 设备 id 即配对 id；`pair_` 前缀与大小写可省略，给出唯一前缀也够用，序号就是列表里打印的行号。
- **登录才让这些配对真正可用。** 没有 Future OS 账号、也没走自建中继时，`future remote devices`
  与 `use` 仍会成功——它们读写的只是本地配对簿——但两者都会明确说明：在 `future auth login`
  之前，任何设备都连不上。`add` 与 `enable` 则直接拒绝并给出这条命令，因为邀请和凭据都由平台签发。
  通过自建中继（`--relay`）服务的机器完全不需要账号。
- `remove` 先在签发方撤销（Future 平台，或删掉自建中继上的房间），再删除本地槽位。
  Desktop 正在运行时，**移除交给它执行**——只有它停掉那座桥，设备才会立刻断开；
  没有 Desktop（或对方版本不认识这个请求）时才由这里改配对簿，设备会一直服务到下一次
  凭据刷新为止（那是几十秒到十几分钟的窗口，不是永久）。撤销请求发不出去时会排队到
  下一个运行的 Desktop 重试，本地解绑照常生效。
- `use` 写入的就是窗口里"让它控制"写入的同一个指针；Desktop 正在运行时，它还会**请 Desktop 立刻交权**：
  控制权在那边是内存中的决定（设备主动接管也会改它），因此一次 takeover 才让选择当场生效，
  而不是等下次启动。Desktop 会把新控制器宣告为 `active`、把原控制器降为 `standby`，并把同一个指针写回。
  本机没有该设备桥时无法接手机器（没有通道给它发命令），此时选择仍会记录，并在 Desktop 下次开始服务时生效；
  桥在、但 App 已关闭的设备也会接过机器，命令会说明在它重连之前没有谁在控。
- `list` 读、`use` 写 `~/.future/remote_pairings.json`（旧版单配对文件按单槽位读取），
  不需要 Desktop 在运行，自建中继也不需要账号。
- 列表在原有的 `CONTROL` 列旁边加一列 `STATE`，标题同时给出总数与每种状态各自的条数
  （如 `2 entries in the pairing book: 1 paired / 1 unclaimed, expired`），
  不能让不能服务的行被归并进一个暗示它能服务的数字里。这些状态**只是标注**：
  不删任何一行，`control` 的含义不变（仍是存储里的指针）。状态为：`paired`、
  `unclaimed`（还没有手机扫过这一张邀请）、`unclaimed, expired`（从未被认领、已过期的邀请：
  既扫不了也连不上，`remove` 之前它背后不会有任何事发生；而 `remove` 会真的把它从平台上删掉——
  邀请那一行不管有没有人扫都占着平台给每台桌面的配额，所以丢弃它是一次真删除，而不是记账）、
  以及 `legacy`（v2 握手之前的旧凭据，
  在本版本连不上）。配对里那个 `expiresAt` 是**邀请**的期限，不是配对的期限：认领过的设备过了
  那个时刻仍照常工作，因为只有认领路径会检查它——所以一条 `paired` 行的期限可以在过去。
- `add`、`remove` 以及 `use` 的"立刻生效"那一半是例外，因为真正在服务一台设备的是运行中的进程：
  邀请只有在为该配对服务的桥已连接时才可用，而移除只有在把桥停掉时才立刻生效。三者都把请求交给
  运行中的 Desktop（窗口或主机进程）并把它自己的答复带回来。`add` 在发起请求的终端里打印二维码与
  链接。已有设备保持配对，这是"在旁边再加一台"；换手机则是 `remove --all` 之后 `add`，
  `add` 会先把移除遗留的旧桥停掉再签发新邀请。

"当前连上了没有""设备自称什么名字"只在运行的 Desktop 进程里，不在配对文件中；
配对簿为空时，机器不会自己产生邀请，直到有人运行 `add`——这也是 `remove --all` 之后的恢复方式。

### 3.2 远程桌面开关

机器只有在远程桌面开关打开时才服务已配对设备。这是**一个**偏好值，窗口的 Remote 页面、
`future host` 和 CLI 共用它，同时决定"现在是否服务"和"下次启动是否恢复服务"：

```bash
future remote status               # 现在开着吗？存下来的开关是什么？
future remote enable               # 现在打开，并且以后保持打开
future remote disable              # 现在关闭，并且以后保持关闭
```

**先登录，再谈开关。** 不带 `--relay` 时，远程相关的每一步都跑在你的 Future OS 账号上：
邀请、凭据、中继连接都由平台签发。所以 `future remote enable` 在动开关之前先查登录，
没有登录就拒绝并让你执行 `future auth login`——开关开着但没登录的机器谁也服务不了。
`future host` 本身**不需要**账号：它可以未登录启动、提供全部本地能力，并在
`future auth login` 存入账号的那一刻自行打开远程桌面。
`future remote disable` 不需要账号："停止服务"这件事不该被"本来就没法服务"挡住。
带 `--relay` 时根本没有账号，上述都不适用。

- `status` 把两半分开展示：运行态（`phase`、哪些设备连着、谁在控）和存下来的
  `autoConnectRemote`——因为"偏好是开"和"进程在服务"本来就是两件事。`--json` 打印一份
  文档，脚本判据看 `state`（`off`、`unknown`、`on`、`connecting`、`serving`、
  `degraded`、`failed`、`revoked`），详见 `future remote status --help`。
- Desktop 正在运行时，`enable`/`disable` **立刻生效**：请求交给该进程，由它启动或优雅
  停掉桥，不需要重启；关掉时还会告诉已连接的设备"这台机器下线了"。
- 没有 Desktop 运行时，没有桥可动，只把开关存下来给下次启动用。`disable` 退出码 0
  （"关"本来就是这次要的结果），`enable` 退出码 1 并明说此刻还没有任何东西在服务。
- 开关关闭时，主机进程启动后会（或继续）**空转等待**而不是退出：它就是这台机器的控制端点，
  所以 `future remote enable` 能把它直接叫回来，不用重启。开关是**开**时的停止仍按故障处理。
  「开着但没有设备」本身不算故障——你自己 `remove` 掉最后一台设备后，入口继续运行，等着被
  要求配新的——但平台**吊销**了设备就算：不可用的凭据会被丢弃，已经没有东西可服务，入口会
  以 `CredentialRevoked` 为理由停下，而不是空等一个没人要的邀请。
- 关闭开关不会解绑任何设备；解绑用 `future remote devices remove`。

### 3.3 让本机去驱动另一台桌面（客户端角色）

上面几节管的是**配对到本机**的设备。它的镜像方向是「本机连过去」的桌面：用一次邀请配对，
之后由 Desktop 维持连接。

```bash
future remote peers                     # 本机连着的每一台远程桌面
future remote peers add <邀请链接>       # 配一台：粘贴对方显示的链接
future remote peers connect <id|序号>    # 现在就连接
future remote peers disconnect <id|序号> # 断开连接，配对保留
future remote peers takeover <id|序号>   # 从正驱动它的设备手里接管回来
future remote peers rename <id|序号> <名字>  # 在本机起个名字（--clear 还原成机器 id）
future remote peers remove <id|序号>     # 解绑：不再连接，并吊销
```

- `add` 在**本机**执行，而不是交给 Desktop：配对是「向平台领取一次」加「与对方握手」，
  本机不需要任何服务在跑，所以 Desktop 还没打开过也能先配对。把对方「连接另一台电脑」
  显示的邀请粘进来即可，它只能使用一次、且有有效期。配对写进
  `~/.future/remote_peers.json`，命令随后连一次，因此配不上会**现在**报错，而不是等到下次启动。
- `list` 读那个文件；有 Desktop 在跑时，再读它持有的连接。没有 Desktop 时只报已配对
  的机器和「没有连接状态」——因为连接只存在于那个进程里，磁盘上没有。
- `connect` 与 `disconnect` 交给运行中的 Desktop 执行：连接本身就是它的，用户主动断开
  的标记也在它的内存里，只有问它才不会让自动重连把用户的决定撤销。`takeover` 同样交给它：
  控制权在同一时间只属于一台设备，被别的设备驱动时本机仍连着、但仍会被退成待命，命令全部
  以 `superseded` 拒绝——`list` 状态列写「已被接管」（知道名字时写「已被某某接管」），
  `takeover` 发出的就是手机上同一条 `takeover` 命令，让对方退为待命；控制权没有移动时
  回答也是成功，由 `--json` 的 `tookOver: false` 说明。`remove` 能问就问，
  问不到就在本机解绑，并从终端里吊销。
- 已配对的桌面会在 Desktop 启动时被连上。`future host` 只服务设备、不驱动别的桌面，
  所以客户端角色需要窗口版。

## 4. Ctrl+C、SSH 断开与再次启动

Ctrl+C 在配对、连接启动和正常运行阶段都可用。退出会先停止新的远程操作、
取消连接恢复工作，再有界清理资源。

- **内置 Agent**：取消所有客户端的活动和排队任务，包括 CLI/TUI 对话，然后停止服务。
- **原本独立运行的 Agent**：不终止它。主机模式 Desktop 的远程入口仍会关闭。
- **已保存的登录与完成的配对**：保留；下次 `future host` 启动时可复用。退出不等于解绑。

Unix 也处理 SIGTERM 和 SIGHUP。SSH 断开后不承诺继续运行；若确实需要跨 SSH 会话保活，
可以由用户显式选择 tmux 或系统服务托管，但这不是主机模式的默认行为。

## 5. 无图形依赖的服务器构建

统一 CLI 直接调用无 GUI 的 `future-host` 库，不加载 Tauri、GTK、WebKit，
无需 X11/Wayland 会话。官方 Linux CLI 产物使用静态 musl；本地 host 工具链构建不一定静态。

从仓库根目录构建和安装，无需 npm 或 Desktop：

```bash
make build-cli
make install-cli
make run-host ARGS=--help
# 兼容别名：make build-desktop-headless / install-desktop-headless
```

也可以显式构建：

```bash
cargo build --release -p future-cli --bin future
./target/release/future host
```

默认产物为 `target/release/future`（Windows 为 `future.exe`），不再需要独立
`futureos-headless`。原 Future home、登录和配对记录保持原位。
通用要求见[构建与安装](build-and-install.zh-CN.md)。

`--release` 是优化等级，不决定生产/测试渠道；主机与手机仍须匹配环境，不应修改邀请绕过校验。

进程夹具执行实际 CLI：

```bash
cargo test -p future-cli --test headless_cli --test remote_start_refusal
```

原生 Linux 上启动 Docker 后，可验证静态发行产物在无网络、无 GUI 的 Rocky Linux 8
容器中的启动和首次非交互拒绝行为：

```bash
bash scripts/ci/check-headless-linux.sh /path/to/future
```

检查会拒绝动态构建。发布 CI 对 CLI-only、deb 和 portable 中的实际程序执行此检查。
Agent 归属、日志和恢复见[Desktop Agent 生命周期](desktop-agent.zh-CN.md)。

## 6. 数据、权限与排障

主机模式沿用运行用户的现有数据目录，包括 `~/.future/agent/auth.json`、
`~/.future/remote_pairings.json` 和 `~/.future/app/`。本机已经登录，不代表 SSH 服务器已登录；
更换系统用户或 HOME 也不会自动继承原凭证。后台托管时应使用与首次交互设置相同的用户和目录。

启动时会打印日志文件路径，退出报错时再次给出该路径。Desktop 及其内置 Agent 写入
`<根目录>/app/logs/desktop.log`（见 [Desktop Agent 生命周期与恢复](desktop-agent.zh-CN.md)），
Agent 会按阶段记录启动过程（模型目录、数据库、历史会话导入、端点绑定），
因此冷启动或网络存储上启动较慢时，可以与卡死区分开。

| 现象 | 处理方式 |
|---|---|
| 缺少 `libwebkit2gtk-4.1.so.0` | 当前运行的是 GUI 二进制；请构建并运行 `future host` |
| 提示 `--headless` 已移除或未知参数 | 使用不带该参数的 `future host`；`--re-pair`/`--no-qr` 已移除，改用 `future remote devices add` 与 `remove --all`；`--relay`/`--relay-ca` 两个入口都支持 |
| Agent 不可用、不兼容或实例锁占用 | 根据错误确认 owner、版本、用户和数据根，升级或正常退出外部服务；不删除锁或绕过互斥。显式端点与 external 模式只连接、不接管 |
| 内置技能没有装上 | 查看 stderr 上的 `[skills]` 输出：维护使用当前程序执行 `future init`，需要能访问技能目录。手机仍可自行安装技能 |
| 未登录时远程桌面不工作 | 在另一个终端执行 `future auth login`；运行中的主机会直接接管，不需要重启。本地能力（会话、任务、自定义 provider）不受影响 |
| 平台授权拒绝或过期 | 用 `future auth login` 重新登录；运行中的主机会打印 `Platform account signed in` 并恢复服务。不要把登录码当作 App 配对码 |
| 配对邀请过期 | 用 `future remote devices add` 重新申请；已完成配对无需因此重配 |
| `future remote devices add` 超时 | 它问的是运行中的 Desktop，而对方只能用自己的构建回答。若提示"没有任何进程领取请求"，说明那个进程早于"设备请求"这个功能——用当前版本（`futureos` 或 `future host`）重启后再试 |
| 已有 Desktop 占用目录 | 退出使用该数据目录的 GUI 或另一主机进程；不要删除锁文件来绕过互斥 |
| 进程直接消失，或节点管理员反馈被清理 | 主机的 OOM killer 或登录节点按用户配额会直接发 SIGKILL，进程自身无法报告。先看 `<根目录>/app/logs/desktop.log` 最后记录到哪个启动阶段，再查主机侧记录（`dmesg -T`、`journalctl -k`，以及节点自己的配额/作业日志）。若触发配额的是历史会话导入，可用 `FUTURE_HOME` 把数据根目录移出共享存储 |
| 手机无法领取邀请 | 检查手机与主机的生产/测试环境是否匹配，确认邀请未过期且未被使用 |
| 平台账号退出、切换或续期授权被拒 | 退出或切换会关闭远程访问并要求重启（避免旧账号入口滞留）；本地服务继续。恢复正确登录后重新启动，或在未登录状态下等新登录被自动接管 |

### 从 tmux 或 SSH 登录里启动没有窗口

`futureos` 只启动图形窗口，因此需要一个能登记窗口的会话。tmux 服务器——以及它创建的每个窗格——
都保留自己启动时所在的 launchd 会话；由 SSH 登录起来的服务器，其窗格拿到的是 SSH 会话，而不是桌面会话。

在 macOS 26.6 上以 SSH 起的服务器实测：窗口确实被创建，尺寸和位置都对，`kCGWindowIsOnscreen` 也是 1，
但进程从不出现在 `lsappinfo` 里——没有 Dock 图标、没有 ⌘-Tab 条目——也没有任何手段能把它抬到屏幕最前面，
看上去就是"没有窗口"。从桌面终端起的 tmux 服务器没有这个问题：同一个二进制在那里正常登记窗口，
也能被抬起。注意 `launchctl managername` 在两种情况下都打印 `Background`，
`CGSessionCopyCurrentDictionary()` 在两种情况下也都不为 nil，所以这两条判据区分不出它们。

出路，从省事到彻底：

1. 让 tmux 单独用一个服务器，从桌面终端启动：`tmux -L gui`，之后用 `tmux -L gui attach` 进入。
   它的窗格能正常登记窗口，原来的服务器继续处理纯命令行工作。
2. 交给桌面启动：`open -a FutureOS`。LaunchServices 会在桌面会话里启动它；bundle 过期时先
   `make package-desktop` 重新打包。`open` 不传递环境变量，`FUTURE_HOME` 不会带过去——
   需要隔离数据根时用第 1 条。
3. 从桌面终端重启共享服务器：先 `tmux kill-server`，再在那里重新启动。
   这会丢掉旧服务器上的所有会话。

还有一种"静默返回"值得认得：已经有实例在运行时，再从任何 shell 执行 `futureos`，
它只是请运行中的那个实例到前台，约一秒后以 0 退出，且不打印任何东西。

手机操作的工具在主机上执行，沿用原有审批与文件访问规则。主机模式不会自动授予额外权限，
也不等于额外的沙箱隔离。传输与中继信任边界见[手机远程](../wiki/zh/Remote.md)，
权限说明见[审批与沙箱](../wiki/zh/Sandbox.md)。
