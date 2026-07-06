# FUTURE.md 工作区记忆系统 — 实现计划

> 状态：**决策已锁定，Phase 1 已实现**（见 §8）。本文件放在 `gui/` 下，但**核心实现在 agent（Rust）**——prompt 组装与文件工具都在 `agent/`；GUI 只做可选的「查看/编辑记忆」展示层。
>
> **已拍板的决策**：①**分层**——FUTURE.md 纯做 workspace 记忆，与项目说明（CLAUDE.md/AGENTS.md/GEMINI.md）各成一节、互不遮蔽；②**有界主动 + 告知**——显式要求时记，学到耐久高价值事实也主动记（白名单约束），每次写完在回复里吭一声；③**Phase 1**——提示词驱动现有 write/edit，不做专用工具；④**保留** AGENTS.md/GEMINI.md 回退；⑤本期 **GUI 不展示**记忆。

## 1. 目标与范围

| 项 | 决定 |
|---|---|
| 记忆层级 | **只做 workspace 级**，不做全局记忆（`~/.future/...` 全局记忆不在范围内） |
| 存储位置 | 会话 cwd 下的 **`FUTURE.md`** |
| 读取 | **优先 `FUTURE.md`**，没有再回退 `CLAUDE.md`（→ 兼容保留 `AGENTS.md` / `GEMINI.md`） |
| 写入 | **只写 `FUTURE.md`**，绝不写 `CLAUDE.md` / `AGENTS.md` / `GEMINI.md` |

---

## 2. 调研：其他工具怎么做（读/写时机 + 提示词）

### 2.1 Claude Code

- **记忆文件分层**：托管策略 → 用户级 `~/.claude/CLAUDE.md` → 项目级 `./CLAUDE.md`（或 `.claude/CLAUDE.md`）→ 本地 `CLAUDE.local.md`（gitignore）→ 子目录 `CLAUDE.md`（**懒加载**，读到该目录文件时才载入）。
- **读取时机**：会话启动时从 cwd **向上走到根**逐层载入并**拼接**（root 在前）；子目录 CLAUDE.md 懒加载；支持 `@path` 导入（递归，**最多 4 跳**）；`.claude/rules/*.md` 可按 `paths:` 条件加载。
- **投喂方式**：CLAUDE.md 内容作为 **system prompt 之后的一条 user message** 注入，定位为「上下文，非强制配置」。`/compact` 后项目根 CLAUDE.md 会**从磁盘重新注入**。
- **写入时机**：
  - **CLAUDE.md 非自动**——仅靠 `#` 快捷追加、`/memory`（编辑器打开）、`/init`（脚手架生成）。Claude **不会主动**改 CLAUDE.md。
  - 另有独立的 **Auto Memory**（v2.1.59+ 默认开）：Claude **自主**把学习到的东西写到 `~/.claude/projects/<project>/memory/MEMORY.md`（启动只载入前 200 行 / 25KB，话题文件按需读）。
- **最佳实践**：单文件 **< 200 行**；具体可验证（「用 2 空格缩进」而非「格式化好」）；用 markdown 分节；矛盾的规则会被随机取舍，需定期清理。

### 2.2 OpenCode（`~/workspace/opencode`）

- **指令文件**：`AGENTS.md`（主）、`CLAUDE.md`（除非 `OPENCODE_DISABLE_CLAUDE_CODE`）、`CONTEXT.md`（废弃）、全局 `<config>/AGENTS.md` + `~/.claude/CLAUDE.md`、以及 `config.instructions[]`（路径/glob/URL）。
- **读取时机**：**每个 assistant 回合都重新读**（`instruction.system()`，`packages/opencode/src/session/prompt.ts:1309`）；从 cwd **向上走到 worktree 根**，**第一个命中的文件名胜出且停止**（不堆叠各级祖先）。另有**动态注入**：Read 工具读某文件时，会向上找该文件附近的 AGENTS.md/CLAUDE.md，包进 `<system-reminder>`（`tool/read.ts:300,355`）。
- **投喂格式**：每个文件前缀 `Instructions from: <abs path>\n`（`session/instruction.ts:166`）。
- **写入时机**：**无任何自动写回**。只有 `/init` 生成 `AGENTS.md`，或**模型自己**调通用 Write/Edit（被提示词怂恿）。提示词示例：
  - `default.txt:75`：「…proactively suggest writing it to AGENTS.md so that you will know to run it next time.」
  - `kimi.txt`：「If you modified any files/…/configurations mentioned in `AGENTS.md`, you MUST update the corresponding `AGENTS.md` files…」
  - `beast.txt`（gpt 系）有「# Memory」段，指向 `.github/instructions/memory.instruction.md`，让模型自管。

### 2.3 一句话对比

| 维度 | Claude Code | OpenCode | 我们要做的 |
|---|---|---|---|
| 读哪些 | CLAUDE.md 多层 + 全局 | AGENTS.md/CLAUDE.md + 全局 | **仅 cwd 的 FUTURE.md→CLAUDE.md** |
| 读时机 | 会话启动 + 子树懒加载 | **每回合重读** + Read 动态注入 | **每回合重读（已是现状）**，仅 cwd |
| 写机制 | CLAUDE.md 手动；Auto Memory 自主写 MEMORY.md | 无自动；模型用 Write/Edit | **模型驱动写 FUTURE.md**（见 §4） |
| 提示词 | 「context 非强制」 | 「proactively suggest writing to AGENTS.md」 | 见 §6 草拟 |

---

## 3. future-os 现状（实现基线）

| 作用 | 位置 | 说明 |
|---|---|---|
| 读项目上下文 | `agent/src/rpc/session_prompt.rs:28-38` | 在 **cwd** 查 `["CLAUDE.md","AGENTS.md","GEMINI.md"]`，**首个命中即止**，**不向上走父目录** |
| 拼进 prompt | `agent/src/prompt/mod.rs:31-35` | 包成 `# Project Context\n\nProject-specific instructions and guidelines:\n\n{content}` |
| prompt 构建时机 | `session_prompt.rs:40-52` | **每次 `prompt()`（每回合）重建** system prompt → 文件改动下一回合即生效 |
| prompt 守则注入点 | `session_prompt.rs:46-48` | `PromptOptions.prompt_guidelines: Vec<String>` |
| 会话 cwd | `agent/src/rpc/session.rs:24`（来自 `RpcCommand.cwd`，`protocol.rs:56`） | FUTURE.md 即在此目录 |
| 写文件工具 | `agent/src/tools/mod.rs`（`write` / `edit`） | 受 workspace 边界约束，可写 cwd 下文件；**目前无任何记忆写回** |
| 技能注入（参照） | `prompt/mod.rs:38-48` + `skills/mod.rs` | 额外上下文源如何接进 prompt 的范例 |

**结论**：读时机（每回合重读、cwd 范围）已经天然契合需求；要做的是 ①读取链加 FUTURE.md ②写入机制 ③提示词 ④（可选）GUI 展示。

---

## 4. 设计决策（含推荐）

### D1. 读取：回退 vs 分层（**需拍板**）
- **方案 A（字面回退，需求#4 原意）**：读取链 `["FUTURE.md","CLAUDE.md","AGENTS.md","GEMINI.md"]`，首个命中即止。改动最小。
  - ⚠️ **遮蔽问题**：一旦 FUTURE.md 存在，会**完全遮蔽** CLAUDE.md。若某仓库既有团队的 `CLAUDE.md`（项目说明）又有 agent 写的 `FUTURE.md`（记忆），项目说明会丢。
- **方案 B（分层，推荐）**：把「项目说明」与「工作区记忆」当作两层各自注入：
  - 项目说明：仍按 `["CLAUDE.md","AGENTS.md","GEMINI.md"]` 首个命中（`# Project Context`）。
  - 工作区记忆：**额外**读 `FUTURE.md`，单独成节（`# Workspace Memory`）。
  - 写入仍只进 FUTURE.md。两者不互相遮蔽，语义清晰（对标 Claude 的 CLAUDE.md vs Auto Memory 分离）。

> **推荐 B**。但若你坚持「FUTURE.md 就是唯一记忆/指令文件、读取单一来源」，则用 A，并接受遮蔽（可加缓解：首次写 FUTURE.md 时若不存在而 CLAUDE.md 存在，则把 CLAUDE.md 内容先迁入 FUTURE.md）。下文 §5 按 **A 为默认基线**给出最小改动，B 作为变体标注。

### D2. 读取时机与范围
- 保持**每回合重读**（现状），FUTURE.md 编辑下一回合即生效。
- **仅 cwd**，不向上走父目录、不做子树懒加载（符合需求#1「只做 workspace 级」）。

### D3. 写入机制（分阶段）
- **Phase 1（提示词驱动，最小可用）**：用现有 `write`/`edit` 工具，由提示词约束「记忆只写 FUTURE.md」。零新增工具，快速上线。
- **Phase 2（专用 `memory` 工具，推荐补强）**：新增受限工具，避免通用 write 误覆盖整文件：
  - 接口示意：`memory(action: "append"|"replace_section"|"remove", section?, content)`。
  - 行为：固定写 `{cwd}/FUTURE.md`，不存在则创建；按 markdown 小节增量更新；带大小护栏（如 > 200 行/超阈值时提示精简）。
  - 等价于 Claude Auto Memory，但目标文件锁定为 FUTURE.md、范围锁定为 cwd。

### D4. 写入触发策略
- **显式**：用户说「记住…」「以后用 pnpm」→ 写入。
- **主动（有界）**：发生**耐久型学习**时主动记一笔——重复出现的纠正、发现的 build/test/run 命令、明确的用户偏好、项目约定。
  - 护栏：保持**轻量、近静默**（像 Claude Auto Memory），或在 GUI 给一个「已更新记忆」提示让用户可回看/撤销（见 D5）。避免把临时性、一次性的东西写进去。

### D5. GUI 展示（Phase 3，可选）
- 「记忆」面板：查看/编辑当前 workspace 的 FUTURE.md（对标 `/memory`）。
- agent 写入 FUTURE.md 后发事件（仿现有 `review-updated` 事件模式），GUI 显示「记忆已更新」并可一键打开 diff/撤销。
- 设置项：workspace 自动记忆开关（对标 `autoMemoryEnabled`）。

---

## 5. 实施步骤（按阶段，含确切改动点）

### Phase 1 — 读取 + 提示词（最小闭环）

1. **读取链加 FUTURE.md** — `agent/src/rpc/session_prompt.rs:30`
   ```rust
   // 方案 A（默认）：
   for fname in &["FUTURE.md", "CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
   ```
   方案 B：保留原链读「项目说明」，另起一段单独读 `FUTURE.md` 作「工作区记忆」，分别塞进两个 `PromptOptions` 字段（需在 `prompt/mod.rs` 加一个 `memory_content` 段，见 §5 变体）。

2. **写入守则** — `session_prompt.rs:46-48` 的 `prompt_guidelines` 追加一条（文案见 §6）。

3. **（方案 B 变体）prompt 分节** — `agent/src/prompt/mod.rs`：新增 `# Workspace Memory` 段（在 `# Project Context` 之后），仅当 `memory_content` 非空时注入（文案见 §6）。

4. **验证**：`cd agent && cargo build && cargo clippy && cargo test`；手测：cwd 放 FUTURE.md → 下一回合模型能引用；让模型「记住 X」→ 检查只改 FUTURE.md。

### Phase 2 — 专用 memory 工具（推荐）
- 在 `agent/src/tools/mod.rs` 仿 `write`/`edit` 增加 `memory` 工具（schema + handler + 注册进 `coding_tools()`）；固定目标 `{cwd}/FUTURE.md`、走现有 workspace 边界校验；增量小节更新 + 大小护栏。
- 提示词从「用 write/edit 写 FUTURE.md」改为「用 memory 工具」。

### Phase 3 — GUI 展示（可选）
- agent 写 FUTURE.md 后通过事件总线通知；GUI 加「记忆」查看/编辑面板与「已更新」提示；设置加自动记忆开关。

---

## 6. 提示词草拟（verbatim 草稿，待定稿）

**(a) 写入守则**（加入 `prompt_guidelines`）：
```
你为当前工作目录维护一个工作区记忆文件 FUTURE.md。当用户要求“记住”某事，
或你学到关于本工作区的耐久事实（偏好、约定、build/test/run 命令、反复出现的纠正）时，
用 write/edit 工具把它记录到 FUTURE.md。条目要简洁、按 markdown 标题归类；
更新或删除过时条目而非重复堆叠。记忆只能写入 FUTURE.md，
绝不写入 CLAUDE.md、AGENTS.md 或 GEMINI.md。
```

**(b) 工作区记忆注入框架**（方案 B，`prompt/mod.rs` 新增段，FUTURE.md 命中时）：
```
# Workspace Memory

以下是本工作区的持久记忆（FUTURE.md）——你此前保存或用户提供的权威笔记：
偏好、约定、build/test/run 命令，以及值得跨会话记住的事实。

{memory_content}
```

**(c) 项目说明框架**（保持现状，CLAUDE.md/AGENTS.md/GEMINI.md 命中时）：
```
# Project Context

Project-specific instructions and guidelines:

{agent_content}
```

> 方案 A 下没有 (b)，FUTURE.md 命中时直接走 (c) 的「# Project Context」框架即可（或把标题改为中性的 `# Workspace Memory & Project Context`）。

---

## 7. 决策结论（已拍板）

1. **D1 读取语义** → **方案 B（分层）**。FUTURE.md 与项目说明各成一节，互不遮蔽。
2. **写入触发** → **有界主动 + 告知**。显式 + 耐久高价值事实主动记（白名单），写完吭一声。
3. **写机制** → **Phase 1**（现有 write/edit + 提示词约束）。
4. **回退** → **保留** AGENTS.md / GEMINI.md（项目说明链 `CLAUDE.md → AGENTS.md → GEMINI.md` 不变）。
5. **GUI 面板** → 本期**不做**（留待 Phase 3）。

## 8. 落地状态（Phase 1 已实现）

- ✅ `agent/src/prompt/mod.rs`：`PromptOptions` 加 `memory_content` 字段；新增 `# Workspace Memory` 段（在 `# Project Context` 之后，仅非空时注入）；加 2 个单测验证「分层、互不遮蔽 / 空时不注入」。
- ✅ `agent/src/rpc/session_prompt.rs`：每回合从 `{cwd}/FUTURE.md` 读记忆传入 `memory_content`；`prompt_guidelines` 加「有界主动 + 告知 + 只写 FUTURE.md」守则。
- ✅ 验证：`cargo build` / `cargo clippy`（我的文件零警告）/ `cargo test prompt::`（3 passed）。
- 项目说明读取链（`session_prompt.rs:30`）按决策#4 **保持不变**，FUTURE.md **不**混入该链。
- 未做（按决策）：专用 `memory` 工具（Phase 2）、GUI 记忆面板（Phase 3）。

---

## 附：关键源码索引
- 读项目上下文：`agent/src/rpc/session_prompt.rs:28-38`
- prompt 组装/分节：`agent/src/prompt/mod.rs:15-66`（项目说明段 `:31-35`）
- 每回合重建 prompt：`agent/src/rpc/session_prompt.rs:40-52`
- 守则注入：`agent/src/rpc/session_prompt.rs:46-48`
- 会话 cwd：`agent/src/rpc/session.rs:24`、`agent/src/rpc/protocol.rs:56`、`agent/src/rpc/commands.rs:91-155`
- 写文件工具：`agent/src/tools/mod.rs`（`write` / `edit`）
- 技能注入参照：`agent/src/prompt/mod.rs:38-48`、`agent/src/skills/mod.rs:23-48`
- OpenCode 参照：`~/workspace/opencode/packages/opencode/src/session/instruction.ts`、`.../session/prompt.ts:1309`、`.../session/prompt/{default,kimi,beast}.txt`、`.../command/template/initialize.txt`
