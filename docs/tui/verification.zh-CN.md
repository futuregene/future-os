# 工作台 TUI —— 验收记录

验了什么、截图上能看到什么、还有哪些是错的。§2 的每一行都标了图号；
`docs/tui/assets/` 里的每一帧都来自**真实**运行的 `future-tui`（对着隔离 agent），
不是 demo，也不是 mock。

- **构建**：`dev-huichen` 的 `ba916d63e`，
  `FUTURE_VERSION=0.0.2-9f0a8b76c+local cargo build -q -p future-tui`。
- **被测代码**：`tui/src/workbench` —— 15 个模块、9,076 行、119 个 `#[test]`
  （含选择器浮层与六套调色板）。
- **会话**：`20261021-005514-e1ee4ba485bc44c5b2d2ea602cd5cd82`
  （`全屏多会话视图设计方案`，2,360 条 entries，cwd `/Users/geilige/future-os`）。
- **实例**：`FUTURE_HOME=/tmp/dvhome` —— `settings.json`、`models.json`、`skills/`、
  一份 `tasks.db` 备份，加上那一个会话的行；**没有凭据**。Agent：
  `future agent --grpc-addr 127.0.0.1:5511`（版本串与客户端一致，客户端会强制校验）。
- **终端**：每次截图按目标尺寸开 pty（120×38 / 90×24 / 60×20），`TERM=xterm-256color`。
- **配方**：`tui/scripts/shot.sh`（见 §7），它是
  `scripts/screenshots/capture.py tui` 的一层包装。每张 PNG 都是 TUI 写进 pty 的字节流，
  由 `scripts/screenshots/tui_terminal.py` 按终端单元模型重放。

## 1. 图号索引

| 图 | 文件 | 尺寸 | 来源 | 看什么 |
|---|---|---|---|---|
| 图 1 | `workbench-launchpad.png` | 120×38 | 真机 | 启动台：鲸鱼 + 方块字标 + 主张句、带框输入框、参数 chip、最近会话、空的待办 |
| 图 2 | `workbench-120x38.png` | 120×38 | 真机 | 完整帧：页眉、真实 transcript、带待办面板的侧栏、activity、状态栏、提示行 |
| 图 3 | `workbench-panel-todo.png` | 120×38 | 真机 | 待办获得焦点（页签条里有插入符），真实目标待办带 `✔/○/✖` 与优先级 |
| 图 4 | `workbench-panel-usage.png` | 120×38 | 真机 | 用量：`上下文 128.9k/200.0k 64.4%`、进度条、`成本 ≈¥18.84`、`缓存命中 50%` |
| 图 5 | `workbench-panel-skills.png` | 120×38 | 真机 | 技能：22 个技能及描述（来自两个技能根） |
| 图 6 | `workbench-chat-scrolled.png` | 120×38 | 真机 | 滚轮上滚后的聊天：`↑ 更早` 徽标，键盘回到聊天（无插入符） |
| 图 7 | `workbench-help.png` | 120×38 | 真机 | `?` 帮助浮层（键位表 + 命令表） |
| 图 8 | `workbench-palette.png` | 120×38 | 真机 | `/` 命令面板，每条带描述 |
| 图 9 | `workbench-palette-filter.png` | 120×38 | 真机 | 面板被过滤到 `theme` |
| 图 10 | `workbench-picker-theme.png` | 120×38 | `capture.py tui` | `/theme` 选择器：六套调色板、当前值上的 `✓`、预览页脚 |
| 图 11 | `workbench-picker-model.png` | 120×38 | `capture.py tui` | `/model` 选择器：强度滑杆、provider 分组、每行 `ctx · thinking` |
| 图 12 | `workbench-picker-sessions.png` | 120×38 | `capture.py tui` | `/sessions` 选择器：搜索行、带 cwd 与时长的会话行 |
| 图 13 | `workbench-light-120x38.png` | 120×38 | 真机 | 选择器应用 `light` 之后的同一会话 |
| 图 14 | `workbench-90x24.png` | 90×24 | 真机 | 90 列仍是两栏；页眉退回文字字标；六条提示全在 |
| 图 15 | `workbench-60x20.png` | 60×20 | 真机 | `< 88`：没有侧栏也没有空轨；`< 24`：单行页眉、带框两行输入框、三条提示、无 activity 行 |
| 图 16 | `workbench-panel-full-60x20.png` | 60×20 | 真机 | 窄终端里 `ctrl+b`：面板全屏 |
| 图 17 | `workbench-drag-inverted.png` | 120×38 | 真机 | **缺陷**：把分隔条往右拖之后，侧栏停在 42 列 |
| 图 18 | `workbench-dead-usage.png` | 120×38 | 真机 | **缺陷**：面板里的 `/usage` 回 `Unknown command /usage` |
| 图 19 | `workbench-dead-skills.png` | 120×38 | 真机 | **缺陷**：面板里的 `/skills` 回 `Unknown command /skills` |

图 10–12 是仓库自己的确定性抓帧
（`python3 scripts/screenshots/capture.py tui`，跑 `future tui --offline` + 脚本化
agent）。这里没有重拍它们：选择器浮层整张盖住帧，真机会话在它背后不增加信息，而那个
工具两次运行字节相同，是更强的物证。

## 2. 逐条对照

### 2.1 布局

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| L1 | 120×38 每个分区都在：页眉（4 行鲸鱼 + 细线）、带侧栏的主体、带框输入、activity、状态、提示 | 图 2 | 1–4 行页眉、5 细线、6 空行、7 页签条、8–31 对话、32–34 带框输入（`╭─ 输入 …╮`、`│❯ …`、`╰───╯`）、35 空行、36 activity `· idle … 缓存 50%`、37 状态、38 提示 —— 正好 38 行 | 通过 |
| L2 | 聊天不撞侧栏；分隔条只占一列；聊天左右内边距各 2 格 | 图 2 | 聊天文本占 4–82 列，分隔条在第 84 列，侧栏占 85–120 列（36 列宽） | 通过 |
| L3 | 90 列仍两栏，页眉退回文字字标 | 图 14 | 第 1 行页眉为 `FUTURE OS · 全屏多会话视图设计方案`，分隔条在第 54 列（侧栏 36） | 通过 |
| L4 | 60×20 降级：无侧栏、无空轨、单行页眉、带框两行输入、三条完整提示、无 activity 行 | 图 15 | 页眉 `FUTURE OS · … ● future-tui`；无分隔条；`╭─ 输入…╮` + `│❯ …│`（只有两侧）；状态行在；提示 `ctrl+b 侧栏 · / 命令 · ? 帮助` | 通过 |
| L5 | 模型名可以裁，数字不行 | 图 15 | `future/deepse… ██████░░░░ ctx 128.9k/200.0k 64.4% · ≈¥18.84` —— 进度条与所有数字完整 | 通过（有意，见 §5.4） |
| L6 | 放不下侧栏的终端也能到每个面板 | 图 16 | 60×20 下 `ctrl+b` 把待办画成带框的全屏面板，底部 `↓ 2 more` | 通过 |
| L7 | 已知尺寸下首帧等于稳态帧 | 图 2 | 这个实例：启动后 3 秒、6 秒、9 秒各抓一次，细线 5 / 空行 6 / 页签 7 一致。**同一构建的另一个实例首帧曾高一行**，下一次重绘自愈 —— 见 §6 | 通过（带保留） |

### 2.2 鼠标

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| M1 | 点侧栏页签切面板 | 图 4 | `click.py dv5 7 用量` → `clicked '用量' at col=107 row=7`；面板标题变成 `用量`，页签条出现插入符 | 通过 |
| M2 | 点行切会话 / 应用选择器行 | 图 5 | 点 `技能`（col=115 row=7）切了面板；点 `/theme` 选择器里的 `Light` 行（col 27 row 13）会应用并关闭浮层 | 通过 |
| M3 | 滚轮每次滚三行，到顶请求更早的历史 | 图 6 | 在 (20,20) 连发三次上滚：正文从 `栏高与标签中线…` 变成 `Chrome.test.ts +2…`，并出现 `↑ 更早 · page up` | 通过 |
| M4 | 侧栏上的滚轮滚面板，不滚聊天 | 图 3 | 侧栏获得焦点时同样的三次上滚让待办列表移动，聊天没动 | 通过 |
| M5 | 拖分隔条改宽度 30..48，且边跟着指针 | 图 17 | 在第 84 列按下、拖到第 90 列、松手：分隔条**跑到第 78 列**，侧栏变成 **42** —— 边朝背离指针的方向走 | **不通过**（P0-1） |
| M6 | 只有工作台运行时终端才握着鼠标上报 | 图 2 | `tmux display -p '#{mouse_any_flag}'` 运行中为 `1`，`ctrl+q` 退出后为 `0` | 通过 |
| M7 | 点聊天区把键盘从侧栏拿回来 | 图 6 | 点击后页签条的插入符消失，随后的滚轮滚的是聊天 | 通过 |
| M8 | 悬停 | — | `MOUSE_ON` 是 `?1000h?1002h?1006h`；`MouseKind::Move` 解析后被丢弃（`mod.rs:584`） | 不适用 —— 还没有悬停（P1-1） |

### 2.3 侧栏

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| S1 | 五个页签，活动面板有名字，≥88 列时常驻 | 图 2、图 14 | `待办 任务 会话 用量 技能`；分隔条在 84（120 宽）与 54（90 宽） | 通过 |
| S2 | 待办读真实目标，标出状态与优先级 | 图 3 | `待办  4 待办 / 17 合计`；行如 `✔ P0 T2 文档：docs/tui/dsh-tu…`、`✖ P0 T4 文档与截图验收…`；超宽的行用 `…` 裁文本（从不裁数字） | 通过 |
| S3 | 用量只给完整值，数字绝不裁 | 图 4 | `上下文 128.9k/200.0k 64.4%`、`成本 ≈¥18.84`、`缓存命中 50%`、`输出 —` | 通过 |
| S4 | 技能列出本机技能与描述 | 图 5 | `技能  22 个技能`、`/future-slides  Create and revise presentations …` | 通过 |
| S5 | 面板可滚，并说明还剩多少 | 图 16 | 侧栏底部 `↓ 2 more` | 通过 |
| S6 | 聚焦与未聚焦的侧栏可分辨 | 图 2 与图 3 | 插入符 `›` 只在侧栏拥有键盘时出现在页签条前（`tab` 与点击聊天都会让它移动） | 通过 |

### 2.4 状态栏与 activity

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| A1 | 左侧模型、强度、权限 | 图 2 | `future/deepseek-flash · medium · all` | 通过 |
| A2 | 上下文进度条、已用/总量、百分比，右对齐 | 图 2 | `████████░░░░ ctx 128.9k/200.0k 64.4%` | 通过 |
| A3 | 会话成本 | 图 2 | `· ≈¥18.84` | 通过 |
| A4 | 缓存命中率 | 图 2 | activity 行右端 `缓存 50%` | 通过 |
| A5 | 空闲时静止（不闪） | 图 2 | `· idle`；间隔 1.5 秒连抓三次，没有任何转轮帧 | 通过 |
| A6 | 页眉里的 git 分支 | 图 2 | 页眉第 2 行只有 `/Users/geilige/future-os`；`SessionMeta::branch` 被硬编码为空 | **不通过**（P0-4） |
| A7 | 页眉里的会话标题 | 图 2 | 第 1 行 `· 全屏多会话视图设计方案` | 通过 |
| A8 | 右角按整段退化 | 图 15 与图 2 | 120 列 `● future-tui v0.0.2-9f0a8b76c+local` → 60 列 `● future-tui`；从不出现半个汉字 | 通过 |

### 2.5 命令面：面板、选择器、帮助

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| C1 | `/` 打开可过滤、带描述的面板 | 图 8 | `/new 新建会话`、`/sessions 浏览并切换会话` …… 共 12 条 | 通过 |
| C2 | 输入即过滤 | 图 9 | 输入 `theme` 后只剩 `/theme` 且被选中 | 通过 |
| C3 | 三条命令打开对应选择器 | 图 10、图 11、图 12 | `/theme` → 六套调色板带 `✓`；`/model` → provider 分组加强度滑杆；`/sessions` → 可搜索的会话行；三条都能从面板打开（`run.rs:249`），手打命令也能 | 通过 |
| C4 | 主题选择器可预览、可取消 | 图 13 | 高亮移到 `Light` 时整帧重绘（`48;5;235` → `48;5;252`）；`esc` 恢复 `48;5;235`；`enter` 持久化 | 通过 |
| C5 | 面板里每一条都能解析 | 图 18、图 19 | `/usage` → `Unknown command /usage`，`/skills` → `Unknown command /skills`；`/workspace`、`/export`、`/transcript` 同样（`live.rs:694`） | **不通过**（P0-3） |
| C6 | `?` 打开键位表，`esc` 只关一层 | 图 7 | 两段：键位（含两行鼠标）与带描述的命令表；`esc` 回到聊天，草稿未被改动 | 通过 |

### 2.6 主题

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| T1 | 六套 256 色调色板，id 与内联目录共用 | 图 10 | `Dark`、`Light`、`One Dark`、`One Light`、`High Contrast`、`Dracula`；当前值带 `✓`；`themeId` 能解析 `dark`/`light`（旧的 `future-dark`/`future-light` 写法仍然映射） | 通过 |
| T2 | 切换重写整帧，不是局部 | 图 2、图 13 | 同一帧里所有被绘制的背景都属于同一套调色板；`48;5;235`（深色面板）→ `48;5;252`（浅色面板），浅色正文 `38;5;236` | 通过 |
| T3 | 选择能跨重启保留 | 图 13 | 只有 `enter` 或点行才持久化，写进 `<home>/tui/settings.json` 的 `themeId`，`theme::resolve` 读回 | 通过（代码 + 真机预览/取消） |

### 2.7 降级

| ID | 要求 | 图 | 证据 | 判定 |
|---|---|---|---|---|
| D1 | 宽度：≥88 保留侧栏，<88 去掉且不留空轨 | 图 2、图 14、图 15 | 120 与 90 下侧栏 36 列；60 下聊天一直画到第 60 列 | 通过 |
| D2 | 宽度：侧栏放不下时面板仍然可达 | 图 16 | `ctrl+b` 全屏面板 | 通过 |
| D3 | 高度：≥24 保留 activity 与六条提示；<24 隐藏 activity、留三条 | 图 14、图 15 | 90×24 有 `· idle … 缓存 50%` 和全部六条提示；60×20 没有 activity 行，只有三条提示 | 通过 |
| D4 | 高度：<24 时输入框仍有边框 | 图 15 | 顶边 + `│❯ … │` 两侧，没有底边 | 通过 |
| D5 | 提示整条丢，绝不切半个词 | 图 15 | `ctrl+b 侧栏 · / 命令 · ? 帮助` —— `Esc 回到聊天`、`tab 焦点`、`ctrl+n 新会话` 整条去掉 | 通过 |
| D6 | 中文永远不挪列 | 图 2、图 14、图 15 | 分隔条在任何尺寸下都只占一列；每行中文右缘对齐 | 通过 |

## 3. 帧不变量

- 每一行都绝对定位（`CSI row;1H`），帧从不让光标前进
  （`tui/src/workbench/run.rs:368`），所以比终端高的帧只会被裁掉而不会顶屏 ——
  顶屏会让每一行落在 SGR 报告命名坐标的上面一格，而 `M1` 点的就是「点中与否」。
- 合成出的帧恰好 `h` 行、每行恰好 `w` 个显示列（`render` 里
  `debug_assert_eq!(rows.len(), h)`）；§2.1/L1 在 120×38 上逐行清点过分区。
- demo 的 `--self-test` 在 120×38 与 90×24 上通过（`self-test ok`），这是 CI 里的
  几何与命中区闸门。

## 4. 机器检查

| 检查 | 命令 | 结果 |
|---|---|---|
| 构建 | `FUTURE_VERSION=0.0.2-9f0a8b76c+local cargo build -p future-tui` | 通过 |
| 格式 | `cargo fmt -p future-tui --check` | 干净 |
| 工作台测试 | `cargo test -p future-tui workbench` | **119 passed**，0 failed |
| crate 测试 | `cargo test -p future-tui --lib` | **1,013 passed**，0 failed |
| 几何/命中自检 | `cargo run -q -p future-tui --example workbench_demo -- --self-test` | `self-test ok` |
| clippy | `cargo clippy -p future-tui --all-targets -- -D warnings` | **在 `future-agent` 里失败**，不是 `future-tui`：`agent/src/session/compaction_ops.rs:79` 在本机工具链上触发 `clippy::nonminimal_bool`。`git diff origin/main -- agent/` 为空，属于既有且无关的失败 |
| 可复现性 | 同一会话上跑两次 `tui/scripts/shot.sh usage 120x38` | 两张 PNG 字节相同，且与图 4 相同 |

## 5. 与 dsh-TUI 有意的不同

这些是选择，不是差距。「修」它们之前得先决定。

1. **没有第二个渲染器，也没有布局开关。** `future tui` 就是工作台；`--layout`、
   `--legacy`、`--classic` 不存在，帮助文本一旦写出这些名字测试就失败
   （`help.rs:96`）。dsh 保留内联模式，FutureOS 选择删除，而不是让两个渲染器共享一条
   输出流。
2. **没有子代理、shell 作业、插件面板。** agent 没有子代理，线上也没有作业列表。
   「任务」面板展示的是本机的**定时任务**，是另一回事，并且标注清楚。
3. **浮层整张盖住帧，而不是压暗。** `paint_overlay` 用它覆盖到的行铺面板底色
   （图 7、图 8、图 10–图 12）：浮层下面看不到对话。dsh 压暗背景、保留其下可读文本。
   我们的做法更便宜、在 60 列下也更不含糊，但丢掉了「你还在聊天里」的线索。
4. **模型名可以裁，数字绝不行。** 我们先布局进度条，再用 `…` 裁左侧标签（图 15）。
   60 列下读起来是 `future/deepse…`。
5. **界面只有中文。** dsh 可实时切语言；工作台只有中文。这是产品立场还是差距仍未定
   （parity 文档 §10.3）。
6. **审批用 `y`/`n`，没有卡片控件。** 挂起的审批接管这两个键，activity 行写出工具名
   （`live.rs:586`）；dsh 画一张带按钮的审批卡。
7. **没有鼠标拖选复制。** 工作台整场握着 SGR 上报，选文本要按终端的修饰键；dsh 实现了
   按面板选择与语义复制。
8. **三个动词共用一个选择器，不是三个屏幕。** 模型、主题、会话共用一张浮层与一套键位；
   dsh 是 `/model` 一页、会话管理器另一套。界面更少、要记的更少，而且两边都是点行即应用。

## 6. 验收时发现的缺陷

| 缺陷 | 证据 | 归到 |
|---|---|---|
| 分隔条拖拽方向反了 | 图 17：第 84 列按下 → 拖到第 90 列 → 分隔条停在第 78 列，侧栏 42（两个方向各测一次） | [workbench-redesign.zh-CN.md](workbench-redesign.zh-CN.md) 的 P0-1 |
| 五条面板命令回 `Unknown command` | 图 18（`/usage`）、图 19（`/skills`）；`/workspace`、`/export`、`/transcript` 同样缺处理函数 | P0-3 |
| git 分支从未到达页眉 | 图 2：`SessionMeta::branch` 是 `String::new()`（`live.rs:553`） | P0-4 |
| 首帧可能按过期高度合成 | 同一构建的一个 120×38 实例首帧高一行（细线 5、页签 6、无空行），下一次重绘自愈；另一个实例 3/6/9 秒都对 | P0-2 |
| 删内联 UI 时丢掉的屏幕 | `/sandbox`、`/worktree`、`/keymap`、`/copy`、外部编辑器 —— 工作台没有落点 | P1-3 |
| 没有悬停 | `MOUSE_ON` 缺 `?1003h`；`Move` 被丢弃 | P1-1 |
| 合成器没有图片处理 | 没有 `is_image_line` 等价物、没有 Kitty/iTerm2 行保留；未真机测（会话里没有图片） | P1-11 |
| 用 `ctrl+b` 隐藏的侧栏在终端变宽后不会自己回来 | 已观察；`ctrl+b` 可恢复（隐藏 → 显示+聚焦）。**按设计**：可见性是读者的选择 | 记录，不另立 |

## 7. 如何复现本记录

```bash
# 1. 隔离的数据根，不含凭据
export FUTURE_HOME=/tmp/dvhome
mkdir -p "$FUTURE_HOME/agent" "$FUTURE_HOME/tasks"
cp ~/.future/agent/{settings.json,models.json} "$FUTURE_HOME/agent/"
cp -R ~/.future/agent/skills "$FUTURE_HOME/agent/skills"
sqlite3 ~/.future/tasks/tasks.db ".backup $FUTURE_HOME/tasks/tasks.db"

# 2. 一个真实会话，逐行复制（sessions / entries / message_blocks / runs）
#    见 parity 文档 §1 的 ATTACH + INSERT 脚本。任何已结束的会话都行。

# 3. 同版本的 agent，开新端口（客户端会拒绝版本不一致）
FUTURE_VERSION=0.0.2-9f0a8b76c+local cargo build -q -p future-tui -p future-cli
FUTURE_HOME=$FUTURE_HOME ./target/debug/future agent --grpc-addr 127.0.0.1:5511 &

# 4. 出图
export FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5511
export SESSION_ID=20261021-005514-e1ee4ba485bc44c5b2d2ea602cd5cd82
tui/scripts/shot.sh chat 120x38
tui/scripts/shot.sh usage 120x38
tui/scripts/shot.sh chat 60x20

# 5. 选择器那一张：确定性、不需要 agent（→ .screenshots/tui.png）
python3 scripts/screenshots/capture.py tui --scenario palette
```

让截图可信的几条规矩：

- 一定用**新端口**和**私有 `FUTURE_HOME`**，绝不碰正在跑的 `future tui` 用的 agent；
- 会留下**粘住提示**的场景（`/usage`、`/skills`）放到最后，或者单独开一个实例；
- **pty 本身就是那个尺寸**（`shot.sh` 在应用开始读尺寸之前就开好了），所以命令里的
  `120x38` 就是画面尺寸——没有 resize 这一步；
- 每次截图都是**独立进程**，所以上面那条“粘住提示”说的是阅读顺序，而不是清理；
- 手打命令需要草稿非空（先打个空格），因为空草稿上的 `/` 会打开命令面板 ——
  这也是读者实际走的那条路。
