# 计划：可选开启的「智能技能推荐」（跨桌面 / TUI / 移动）

> **历史文档 —— 计划已实现，保留作为当时设计决策的记录。** 实际交付与本文的差异：
> 交付落在 `feat/skill-reco-ui` 一个分支上（不是本文建议的 5 个 PR），推荐放 agent 单 RPC（方案 A），
> 触发判定在客户端（桌面 / TUI / 移动各自实现，日预算也各端自存），开关**默认开启**（PRD 后来定的，
> 本文写的是"默认关"），阈值改为 **≥30 字节**（10 个中文字），超时 **3 s**（原为 1.5 s，低于
> Jev 实测 p95 ≈1.4 s，尾部答案会被丢弃且与"无匹配"不可区分；等待期间三端都会锁住输入框并在发送键上转圈），
> Jev 已进 Future
> provider（因此"临时 key 怎么配"这一问作废，见下 §需要你拍板的 4 件事）。**判据以代码和 PRD 为准**，
> 不要按本文的旧数值实现。评测数据仍有效：[evaluation.md](evaluation.md)。
> 产品侧的规则以 `skill-recommend-prd-v1.9` 为准（该文件不在仓库内）。

## 你这条需求里两个会被否决的点（先回应）

1. **"只推荐未安装技能 ⇒ 已安装的不用放进 Jev 输入"** —— **对，而且比"对"更强**。
   `state` 一次都别放已安装技能。因为已安装的技能本来就立即可用，推荐它们只会浪费、还可能让用户
   "重复安装"。所以 Jev 的选项表 = **目录中未安装技能的子集**。这正是 §5 的输入构造。
2. **这个 demo 的两段式（stage 1 + stage 2）不是这次要抄的形态**。
   你这条需求明确要**单次、≤1 秒、≤1 个推荐**，所以它对应的是 **demo 的 stage 1 单 Choice 版**
   （一次调用 + none 门控，0.82 s / 8,765 tok），不是带复核的两段式。成本/延迟就按这个算。

## 目标形态（按你的描述整理，确认这 6 条是你要的）

| # | 规则 | 说明 |
|---|------|------|
| 1 | 管理页面新增开关「智能技能推荐」，**默认关** | 三端各自的设置页 |
| 2 | 只在**新会话、第 1 条消息、且输入长度 ≥ 阈值**时触发 | 阈值建议沿用 `MIN_QUERY_CHARS=6`，可调 |
| 3 | 触发时**拦截提交**，调 Jev，超时 **1 s** | 超时/无推荐/出错 → 直接放行提交 |
| 4 | 命中 → **暂不提交**，在输入区展示 1 张推荐卡片 | 卡片含技能名/描述/版本 + 「安装并使用」「忽略并发送」 |
| 5 | 用户点「安装并使用」→ 安装该技能 + **把它的斜杠命令追加到输入尾部** → 再发送 | 复用现有 `install_skill` |
| 6 | 用户点「忽略并发送」→ 不装，原样发送 | |
| 7 | **只要用户在输入里已经选了 ≥1 个技能，就不触发** | 检测"已带斜杠命令/已 @技能" |

## 架构关键事实（已核对，决定放哪一层）

- **技能目录与安装**：桌面/移动用平台目录 `list_available_skills` / `install_skill` / `uninstall_skill`
  （`desktop/src/integrations/skills/skillsClient.ts`，Tauri command → 平台 `GET`）。TUI 走 agent 的
  `refresh_skills` / `get_commands`。安装落到 `~/.future/agent/skills/`（`agent/src/skills/mod.rs`）。
- **Jev 调用放哪**：推荐需要"未安装目录 + Jev key + 一次 HTTP"。**放 agent（Rust）一个 RPC command
  最省三端重复**（见下"推荐放哪"），但它会引入一个新的 agent 依赖 + 一个临时 key 配置。
  **替代**：先放在**发起端各自调用**——桌面/移动经 Tauri 侧/远端，TUI 经 agent。**倾向 agent 单点**。
- **"是否新会话第 1 条"**：客户端自己最清楚（输入框空 + 会话无消息），**判定放发起端**，别下放给 agent。
- **超时 1 s**：发起端 `Promise.race` / `tokio::time::timeout`，超时就当"无推荐"放行，**绝不能让
  推荐把发送卡死**（这点和 demo 的"阈值/门控在我们手里"一致：失败=放行）。

## 推荐放哪（两个候选，**我倾向 A**，你定）

- **A. agent 新增 RPC command `suggest_skill`**（`packages/rpc/proto/future.proto` 加 typed payload +
  `agent/src/rpc/commands/*` 实现 + `bench/second-call.mjs` 那段逻辑翻成 Rust）。
  优点：三端共用一份逻辑与阈值；key 只配在 agent；以后 Jev 进 future provider 时**只改 agent 一处**。
  缺点：跨 crate（`packages/rpc` 是 wire 契约，动它要"测直接消费者"），工作量最大。
- **B. 先在发起端实现**（桌面/移动复用现有 TS skill client + 各自的 Jev fetch；TUI 用 agent）。
  优点：快、不动 wire 契约。缺点：阈值/逻辑散三份，Jev 迁 provider 时要改三处。

## 成本口径（你点名要进规划成本）

- **模型成本 = 每次触发一次 Jev 调用**（不是每会话/每用户一次）。按 demo 实测的单 Choice：
  **8,765 tok = ¥0.0027/次**（$0.042/Mtok，输出免费，USD→¥ 按 7.2）。
- 触发频率受 4 道闸限制：开关默认关 + 仅新会话首条 + 长度阈值 + 未预选技能。粗算：
  若某用户开了开关、每天新会话首条且长度达标 **N 次**，Jev 成本 ≈ **N × ¥0.0027/天**。
- **这只是模型成本**，不含平台目录/安装本身（沿用现有，无新增）；Jev 进 future provider 后，
  这项应改计到 future 的 token 口径。

## 拆解（建议 5 个 PR，各自独立可测）

1. **`feat/skill-reco-agent`**：agent `suggest_skill` RPC + Jev 客户端（含 1 s 超时、none 门控 0.15、
   未安装子集、dev key 环境变量）。**这是地基**，其余 PR 都依赖它。
2. **`feat/skill-reco-settings`**：三端设置页加开关（默认关）+ 持久化。
3. **`feat/skill-reco-desktop`**：新会话首条拦截 + 推荐卡片 + 安装 + 追加斜杠命令 + 超时放行。
4. **`feat/skill-reco-mobile`**：同上（React Native 交互）。
5. **`feat/skill-reco-tui`**：同上（终端交互，最简形态：命中就提示，安装后把命令拼到首条消息）。

> 每个 PR 都按仓库规矩：`origin/main` 同步 → 只测改动的 crate/模块 → 开 PR 即挂 auto-merge。

## 测试（你要求"充分"）

- **agent**：`suggest_skill` 单测（构造目录子集、门控阈值、超时=放行、key 缺失=放行/报错）。
- **桌面/移动/TUI**：组件/交互测试（拦截→卡片→安装→追加→发送；忽略→发送；超时→直接发送；
  已带技能→不触发；开关关→不触发）。桌面用现有 `*.test.tsx` + vitest，移动用 jest，TUI 用 Rust 测试。
- **复用 demo 的评测**当回归基准：换阈值或改逻辑后跑 `bench/predict-jev.mjs` 确认分数没掉。

## 需要你拍板的 4 件事

1. **推荐放 agent 单 RPC（A）还是先在发起端（B）？** 我倾向 A。
2. **Jev key 现在的临时配置方式**：是写进 `~/.future/agent/auth.json` 一类，还是先只读
   `FUTURE_SKILL_RECO_KEY` 环境变量？（demo 是后者；进 provider 前建议后者，免得动 auth 结构）
3. **"长度超过某值"的阈值**用 6（demo 的 `MIN_QUERY_CHARS`）还是更长（比如 20，更保守、更少打扰）？
4. **要不要先只做一个端**（比如桌面）打通端到端，再复制到移动/TUI？还是先 agent 地基、三端并行？
