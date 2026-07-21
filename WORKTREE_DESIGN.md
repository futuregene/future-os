# Worktree 支持设计文档

> 状态：设计定稿（2026-07-21）
> 范围：agent（Rust 后端）、TUI、GUI、channel bridge、CLI
> 前置阅读：FUTURE.md「Worktree 支持设计方案」

## 1. 背景与目标

git worktree 允许同一仓库同时拥有多个工作目录，各自检出不同分支。对 agent 产品的价值：会话可以在隔离的工作区里做实验性修改，主 checkout 保持干净；多个会话可以并行工作互不干扰。

**目标**：为 FutureOS 增加 worktree 支持，满足——

1. 用户可在会话启动时或会话中途进入/退出 worktree
2. 不增加 agent tools（保持 shell/read/write/edit 四工具不变）
3. 四个前端（TUI、GUI、channel、CLI）共享一套实现
4. 生命周期安全：删除前必须证明无未提交改动、无未推送提交

**非目标**：超大仓库的创建性能优化（CoW 引擎、快照池）；会话 fork 时的物理工作区派生。

## 2. 参考实现调研结论

调研了两个项目（详见本文件 §9），核心结论：

| | claude-code-leak（Claude Code） | grok-build（xai-fast-worktree） |
|---|---|---|
| 定位 | 会话的临时隔离工作区 | 超大仓库的高性能 worktree 基础设施 |
| 手段 | 封装 `git worktree` CLI + 会话状态管理 | 并行 CoW 复制、btrfs/overlay 快照、SQLite 池化 |
| 存放 | `<repo>/.claude/worktrees/<slug>` | `~/.grok/worktrees/<repo-slug>/`（集中式 + DB 跟踪） |
| 会话关系 | 会话单槽持有 worktree 状态，可跨会话 resume | worktree 是资源，DB 按 session_id 归属 |

**采用 Claude Code 路线**：FutureOS 的沙箱按 workspace 边界划定、无中央 daemon/DB，与 Claude Code 同构。grok 的集中式依赖 SQLite 元数据库和快照池基础设施，过重。

## 3. 核心决策

1. **不加 agent tools**。worktree 的进入/退出是宿主（host）侧能力，与 `set_model`、`fork` 同级，通过 gRPC 命令暴露；模型通过系统提示词 env section + 会话内注入消息感知环境变化
2. **复用 `set_cwd` 通路**。现有 `set_cwd` gRPC 命令（`agent/src/rpc/commands.rs`）已实现"切换会话 cwd + 持久化到 session JSONL"，worktree 命令在其前后包上 git 操作和级联刷新
3. **存放在仓库内**：`<git-root>/.future/worktrees/<slug>`，分支名 `worktree-<slug>`。仓库内 = 沙箱边界内，为未来的 skill 模式（模型用 shell 直接操作）留路
4. **会话 : worktree = 1 : 0..1**（任一时刻单槽）；worktree 可被多个会话先后 resume（1 : N 时间维度）

## 4. 详细设计

### 4.1 目录与命名规范

```
<git-root>/.future/worktrees/<slug>     # worktree 目录
分支名：worktree-<slug>
```

**slug 规则**（抄 Claude Code 的 `validateWorktreeSlug`）：

- 每段（`/` 分隔）仅含 `[a-zA-Z0-9._-]`，总长 ≤ 64
- 禁止 `.` / `..` 段（防路径穿越）
- **保留字**：`list`、`exit`、`keep`、`remove` 禁止作为 slug（与 `/worktree` 子命令撞车）
- 嵌套 slug `user/feature` 拍平为 `user+feature` 用于目录名和分支名（避免 git ref D/F 冲突与目录嵌套删除）
- 未提供名字时自动生成：`{形容词}-{名词}-{4位随机}`

**canonical git root**：创建前必须通过 `git rev-parse --git-common-dir` 解析主仓库 root，保证从 worktree 内发起的创建也落在主仓库的 `.future/worktrees/` 下，杜绝 `<worktree>/.future/worktrees/` 嵌套（父 worktree 被 remove 会连带删除子 worktree 中未提交的工作）。

### 4.2 git 污染处理

`.future/worktrees/` 会以 untracked 目录出现在 `git status` 中，且 `rg` 等工具会搜到 worktree 内的重复内容。处理：

- `enter_worktree` 时自动向 `<git-root>/.git/info/exclude` 追加一行 `/.future/worktrees/`（幂等，已存在则跳过）
- 用 `info/exclude` 而非 `.gitignore`：仓库本地、不提交，不触碰用户的跟踪文件；`rg` 等尊重 gitignore 的工具自动跳过
- Claude Code 未做此处理（依赖用户自行 gitignore `.claude/`），这是我们相对它的改进点

### 4.3 Agent 侧：新增模块与 gRPC 命令

**新模块** `agent/src/worktree/mod.rs`：

```
validate_slug(slug)                          # 白名单校验
canonical_git_root(cwd)                      # commondir 解析
create_or_resume(root, slug, opts)           # git worktree add（含 resume 快速路径）
enter(state, name?, path?, migrate?)         # 进入（见 4.5 状态机）
exit(state, action, discard_changes?)        # 退出（fail-closed 检查）
adopt(state, path)                           # 接管外部创建的 worktree
migrate_dirty_state(root, slug)              # stash 迁移（见 4.6）
ensure_info_exclude(root)                    # §4.2
cleanup_stale(root, cutoff)                  # 过期清扫（见 4.8）
```

git 调用全部通过子进程执行 `git` CLI，统一加固（抄 grok 的 `GIT_AUTH_SUPPRESSION_ENVS` + Claude 的 `GIT_NO_PROMPT_ENV`）：

- env：`GIT_TERMINAL_PROMPT=0`、`GIT_ASKPASS=`、`GIT_LFS_SKIP_SMUDGE=1`（LFS 仓库创建时不拉对象）、`GIT_SSH_COMMAND="ssh -o BatchMode=yes"`
- 每个命令加 `--no-optional-locks`（与并发 git 进程共存）；stdin 关闭；禁 pager
- 从 `.git/` 读出的 ref/SHA 必须过白名单校验再流入命令参数（`isSafeRefName` 等价物：`[a-zA-Z0-9/._+@-]`、禁 `..`、禁前导 `-`；SHA 仅接受 40/64 hex）——HEAD 是纯文本可被篡改

**proto 新增命令**（`proto/future.proto`，`rpc/commands.rs` dispatch）：

```protobuf
// RpcCommand 新增字段复用现有 message/mode 或新增字段
enter_worktree {
  string name = 1;              // 可选；缺省自动生成
  string path = 2;              // 可选；adopt 外部 worktree
  string migrate = 3;           // "yes" | "no" | "ask"（默认 ask→返回交互选项）
}
// 响应
{ worktree_path, worktree_branch, original_cwd, resumed: bool, adopted: bool,
  migrated_files: number, warnings: string[] }

exit_worktree {
  string action = 1;            // "keep" | "remove"
  bool discard_changes = 2;     // remove 且有改动时必须为 true
}
// 响应
{ action, original_cwd, worktree_path, worktree_branch,
  discarded_files: number, discarded_commits: number }

get_worktree_state {}
// 响应
{ active: bool, worktree_path, worktree_branch, original_cwd,
  occupied_by_other_session: bool }

list_worktrees {}
// 响应：git worktree list --porcelain 合并会话状态
{ worktrees: [{
    path, branch, head: string,          // 短 SHA
    managed: bool,                       // 是否位于 .future/worktrees/ 下
    active_in_this_session: bool,
    occupied_by_other_session: bool,
    dirty: bool, changed_files: number,  // diff-index 快速判定 + 计数
}] }
```

### 4.4 会话状态与持久化

`ServerSession` 增加字段：

```rust
worktree: Option<WorktreeState>

struct WorktreeState {
    original_cwd: String,
    worktree_path: String,
    worktree_branch: Option<String>,
    head_commit_at_enter: String,   // 进入时的 HEAD，用于退出时检测新提交
    adopted: bool,                  // 外部 worktree：退出时只 keep 不 remove
}
```

**持久化**：session JSONL 新增 entry 类型 `worktree_session`（content 为 WorktreeState 序列化），enter 时写入、exit 时写入 tombstone。会话文件**始终留在原 cwd 的 sessions 目录**（`sessions/<encoded-original-cwd>/`），不随 worktree 迁移——worktree 被删后会话仍可 resume（回到 original_cwd）。resume 会话时回放该 entry 恢复状态。

**身份锚定双重语义**（抄 Claude Code）：

- **会话中 enter**：cwd 切换，但 sessions 目录、skills、项目级配置锚定原 cwd 不变
- **启动时进入**（CLI `--worktree` / TUI 启动 flag）：worktree 即会话的项目，会话文件天然存在 worktree 的 encoded-cwd 目录下（现有机制零改动）

### 4.5 进入/退出状态机

单活跃 worktree 状态机：

```
[主仓库] --enter--> [worktree 活跃] --exit keep--> [主仓库]（worktree 保留）
                                   --exit remove--> [主仓库]（worktree+分支删除，需 fail-closed 检查）
[worktree 活跃] --enter--> 拒绝（报错提示先 exit）
```

**enter 流程**：

1. 已在 worktree 中 → 报错 `Already in a worktree. Run exit first.`
2. 解析 canonical git root；非 git repo → 报错（见 §4.7）
3. 若带 `path` 参数 → adopt 流程：`git worktree list --porcelain` 验证属于本 repo → 直接进 §4.5.1 的切换步骤，`adopted=true`
4. slug 校验；目标路径 `.future/worktrees/<slug>` 已存在且 HEAD 可读 → **resume 快速路径**（跳过 fetch 和创建）。HEAD 检测用免子进程读取：直接读 `<worktree>/.git` 指针文件 → gitdir → HEAD → loose/packed-refs 解析（**不向上 walk**——目录不存在时向上 walk 会误报父仓库 HEAD）；resume 成功时 `utimes` bump 目录 mtime，防过期清扫误伤（见 §4.8）
5. 新建：解析 base（`origin/<default>` 本地存在则跳过 `git fetch`，否则 fetch 失败回退 `HEAD`；默认分支解析顺序：`refs/remotes/origin/HEAD` symref → `main` → `master`）→ `git worktree add -B worktree-<slug> <path> <base>`（`-B` 自动重置孤儿分支，省一次 `branch -D`）
6. **半成品防护**：`worktree add` 之后的任何失败（含后续步骤出错）必须先 `git worktree remove --force <path>` 拆除再报错——否则"已注册但空"的半成品会被 resume 快速路径误认为正常（Claude 的 sparse-failure tearDown 同款；Rust 侧用 RAII guard 实现，抄 grok 的 `PartialWorktreeGuard`）
7. 占用检测：扫描活跃会话列表，若另一会话的 worktree_path 相同 → 响应带 warning（不阻断）
8. 脏状态处理（见 §4.6）
9. `ensure_info_exclude`
10. **子目录偏移**：若会话 cwd 是 repo 的子目录（如 `/repo/crates/foo`），set_cwd 目标应为 `<worktree>/crates/foo`（该子目录存在时），而非 worktree 根——抄 grok 的 `sourceGitRoot` 偏移机制；original_cwd 仍记录完整原 cwd，退出时对称还原
11. 切换：写 `worktree_session` entry → 走 `set_cwd` 通路 → 级联刷新（§4.5.1）→ 注入会话内通知消息

**exit 流程**：

1. 无活跃 worktree → 报错
2. `action=keep` → set_cwd 回 original_cwd → 清状态（写 tombstone）→ 结束
3. `action=remove` 且 `adopted=true` → 报错提示外部 worktree 不可删除
4. `action=remove` → **fail-closed 变更检测**：
   - tracked 快速检查：`git -C <worktree> diff-index --quiet HEAD --`（比 `git status` 便宜，错误时 over-report 视为有改动）；有 tracked 改动或需精确清单时再 `status --porcelain`
   - `git -C <worktree> rev-list --count <head_commit_at_enter>..HEAD` > 0 → 有新提交
   - 任一 git 命令失败或无法判定 → **视为有改动**（fail-closed，绝不当 0/0）
   - 有改动且 `discard_changes != true` → 拒绝，返回改动清单（changed_files、commits、文件列表前 20 条）+ 可自愈提示（"确认后以 discard_changes=true 重试，或 action=keep 保留"）
5. set_cwd 回 original_cwd → `git worktree remove --force <path>`（cwd 必须在主 repo，不能在被删的 worktree 里）→ 等待 100ms 让 git 释放锁 → `git branch -D worktree-<slug>` → 清状态
6. 盘上删除成功后才清会话状态（失败保留状态，可重试）

#### 4.5.1 切换时的级联行为

**架构事实（已核实）**：本仓库所有 cwd 依赖项都在每次 prompt/run 时从 `self.cwd` 重新解析，无缓存——system prompt 每次 `prompt()` 重建（session_prompt.rs:19），工具 scope 与 `ResolvedSandbox`（含 `${WS}/.future/approval_rule.json`）每次 run 启动时 resolve（session_prompt.rs:116/278）。因此 `set_cwd` 后下一轮自动生效，**无需缓存失效逻辑**（Claude Code 才需要 `clearSystemPromptSections()`，它 memoize 了 sections）。

切换时真正要做的：

1. `sess.set_cwd(new)` + session_info cwd 持久化（复用现有 set_cwd 代码）
2. 写入/清除 `sess.worktree` 状态（见 §4.4）——`build_system_prompt` 据此做锚定判断（见 §4.5.2）
3. **运行中互斥**：`is_streaming` 时拒绝 enter/exit（scope/sandbox 在 run 启动时捕获，中途切换会让一个 run 横跨两个 workspace）——复用 prompt 的同款守卫
4. 注入会话内通知（assistant 可见）：`[host] Switched to worktree .future/worktrees/feat-x (branch worktree-feat-x). Original project directory: /repo`
5. approval rules 随 run 自动从新 workspace 解析（worktree 内无 `.future/approval_rule.json` 时自然回落全局规则）；session 内存规则（`add_session_rule`）跨切换保留——session 作用域，可接受
6. 代价提示：cwd 变化使 provider 侧 prompt cache 失效一轮，之后恢复（sections 确定性排序不受影响）

#### 4.5.2 system prompt 锚定规则

`build_system_prompt` 按 `sess.worktree` 状态选择锚点：

| prompt 成分 | 无 worktree / 启动时进入 | mid-session enter 后 |
|---|---|---|
| Environment 的 working_directory | cwd | **worktree 路径**（文件操作上下文） |
| Skills 发现（项目级 `.future/agent/skills`） | cwd | **original_cwd**（`.future/` 通常不入 git，worktree 内没有，不锚定会凭空消失） |
| AGENTS.md/CLAUDE.md | cwd | **original_cwd**（与 skills/memory 保持一致） |
| FUTURE.md 读写 | cwd | **original_cwd**（避免读到 worktree 内过时的已提交版本；防止 memory 写入 stranded 在 worktree 里） |

worktree 活跃时 Environment section 追加一行：

```
You are in a git worktree (branch worktree-<slug>). Original project directory: <original_cwd>.
Skills, project settings, and memory remain anchored to the original project.
```

启动时进入的会话（CLI `--worktree`）不设置 `sess.worktree`——worktree 即项目，全部锚定其自身。

### 4.6 忘记创建 worktree 的补救：脏状态迁移

场景：用户本会话已在主 checkout 产生未提交改动，现在想进 worktree。`enter_worktree` 的 `migrate` 参数：

- 进入前检测主 checkout：先用 `git diff-index --quiet HEAD --`（tracked-only 快速判定）+ `git ls-files --others --exclude-standard`（untracked）；需要展示清单时再跑完整 `status --porcelain`（若自行解析输出，用 `--porcelain=v2 -z`——v1 对重命名和含空格/引号文件名的处理有坑）
- `migrate=yes`：stash 迁移（git 原生机制，跨 worktree 共享，最安全）：
  ```bash
  git stash push -u -m "future-worktree-migration"
  git worktree add -B worktree-<slug> <path> <base>
  git -C <path> stash pop        # 脏状态出现在 worktree，主 checkout 恢复干净
  ```
  `stash pop` 冲突时保留 stash 并在响应中 warning，让用户手工处理（stash 仍在，不丢数据）
- `migrate=no`：worktree 从干净 base 创建，改动留在主 checkout
- 缺省：响应中返回改动摘要 + 两个选项，由前端弹确认

**预防**（后续迭代）：项目级 `.future/agent/settings.json` 加 `worktree.autoEnter: true`，该项目新会话启动时自动进入随机命名 worktree。

### 4.7 非 git 仓库

`git rev-parse` 向上找不到 repo：`enter_worktree` 返回明确错误：

```
Cannot enter worktree: <cwd> is not inside a git repository. Worktree isolation requires git.
```

不做静默目录复制降级（没有分支/合并能力，隔离语义不成立）。

**扩展口**（不实现，留设计）：settings.json 支持 `worktreeCreate`/`worktreeRemove` hook 命令，把隔离委托给用户脚本（可支持 jj、sapling 等 VCS）——Claude Code 的 WorktreeCreate/WorktreeRemove hooks 同款。

### 4.8 过期清扫

- 触发：agent 启动时（节流：每 24h 最多一次）
- 范围：主仓库 `.future/worktrees/` 下，目录名匹配临时模式（`agent-*`、`auto-*` 等工具生成的固定前缀），mtime 超过 30 天
- **绝不动用户命名的 worktree**
- 删除前 fail-closed 双检查：`git status --porcelain -uno` 干净（`-uno` 跳过 untracked 扫描，大仓库快 5-10×——30 天前的残留里 untracked 只是构建产物）且 `rev-list HEAD --not --remotes` 为空（命令失败 = 跳过）
- 当前任一会话活跃的 worktree 跳过；resume 路径会 bump mtime（§4.5 第 4 步），活跃使用的不会误判为过期
- 结束后 `git worktree prune`

## 5. 前端接线

### 5.1 TUI

slash 命令表（`tui/src/app.ts`）加一项，子命令式设计：

```
{ value: "/worktree", label: "/worktree", description: "manage git worktrees" }
```

| 输入 | 行为 |
|---|---|
| `/worktree list` | `list_worktrees` → 渲染列表（名称、分支、短 HEAD、active/occupied/dirty/external 标记） |
| `/worktree`（无参） | 报错并显示用法（不允许隐式进入，避免随机名制造垃圾 worktree） |
| `/worktree <name>` | `enter_worktree {name}`——已存在走 resume，不存在先创建；名为保留字时报错 |
| `/worktree exit` | 弹选择器：keep / remove / cancel |
| `/worktree exit keep` | `exit_worktree {action: keep}` |
| `/worktree exit remove` | `exit_worktree {action: remove}`；有改动被拒时展示清单并二次确认 → 重发 `discard_changes: true` |

- Footer 状态栏：worktree 活跃时显示 `⎇ worktree-feat-x`

### 5.1.1 Channel（飞书/钉钉）的 exit 差异

无弹窗能力：`/worktree exit` 无参默认 **keep**，回复中提示"如需删除请发送 `/worktree exit remove`"；其余子命令与 TUI 一致。

### 5.2 GUI

- thread 视图头部显示 worktree badge（分支名）
- 「进入 worktree」按钮 → dialog 输入名字；「退出」按钮 → keep/remove 选择弹窗（复用 approval 弹窗模式）
- 有改动拒绝时展示改动清单 + 「Discard and remove」确认

### 5.3 Channel（飞书/钉钉）

`bridge.rs` slash 解析加 `/worktree` 分支，映射同一对 gRPC 命令；卡片回复显示 worktree 路径与分支。

### 5.4 CLI

`future run` 加 `--worktree [name]`：启动前先创建/resume worktree，再以 worktree 路径作为 `--cwd` 启动会话（方案 1 通路，零 agent 改动即可先行落地）。

## 6. 边界情况清单

| 场景 | 行为 |
|---|---|
| 会话已在 worktree 中再次 enter | 拒绝，提示先 exit |
| 从 worktree 内发起创建 | canonical git root 解析，落在主仓库 |
| 两会话进入同一 worktree | 允许 + warning（git 层不冲突，编辑层有风险） |
| adopt 外部 worktree | 验证属本 repo；exit 时禁止 remove |
| worktree 目录被手工删除 | resume 检测 HEAD 不可读 → 走新建；exit 时 remove 失败仅 warning |
| 残留孤儿分支 | 创建用 `-B` 重置，无需预删 |
| 主 checkout 有未提交改动 | migrate 三选一（§4.6） |
| stash pop 冲突 | 保留 stash + warning，不丢数据 |
| exit remove 时 git 命令失败 | fail-closed 视为有改动，需 discard_changes |
| 会话 resume 时 worktree 已被删 | 状态恢复失败 → 回落 original_cwd，清状态 |
| 凭证提示挂住 | GIT_TERMINAL_PROMPT=0 + 关闭 stdin |

## 7. 分阶段实施

**Phase 1（核心）**：`agent/src/worktree/mod.rs` + proto 四个命令（`enter_worktree`/`exit_worktree`/`get_worktree_state`/`list_worktrees`）+ 级联刷新 + session JSONL entry + TUI `/worktree` 子命令。CLI `--worktree` 可独立先行。

**Phase 2（前端补全）**：GUI badge/按钮/弹窗；channel slash；TUI Footer 状态。

**Phase 3（增强，按需）**：
- `worktree` skill（模型经 shell 自治操作，worktree 在沙箱边界内天然可行）
- `worktree.autoEnter` 项目级设置
- settings.json `worktree.symlinkDirectories`（node_modules 符号链接防磁盘膨胀）、`worktree.sparsePaths`
- 过期清扫、WorktreeCreate/Remove hooks
- subagent 隔离 worktree（依赖 subagent 能力落地）

## 8. 验收标准

1. TUI 中 `/worktree feat` 后，agent 所有文件操作落在 worktree，Footer 显示分支
2. 会话中途退出（keep）→ resume 会话 → 状态正确恢复，可再次 enter resume 同一 worktree
3. worktree 内提交 3 个 commit 后 `/worktree exit remove` → 拒绝并列出 commit；`discard_changes` 后 → worktree 与分支均删除
4. 主 checkout 有 5 个未提交文件时 migrate 进入 → worktree 内看到这 5 个文件改动，主 checkout `git status` 干净
5. `git status` 在主仓库不显示 `.future/worktrees/`（info/exclude 生效）
6. 非 git 目录 enter → 明确报错
7. 飞书群 `/worktree test` 全流程可用

## 9. 调研附录：两个参考实现

### 9.1 claude-code-leak（Claude Code）

- 工具：`EnterWorktreeTool` / `ExitWorktreeTool`；核心 `src/utils/worktree.ts`
- 存放 `<repo>/.claude/worktrees/<slug>`，分支 `worktree-<slug>`；嵌套 slug 拍平 `+`
- 创建：slug 校验 → resume 快速路径（读 `.git` 指针文件取 HEAD）→ 跳过已有 fetch → `git worktree add -B` → 后处理（复制 settings.local.json、core.hooksPath 指主仓库、settings 配置的目录符号链接、`.worktreeinclude` 复制的 gitignored 文件）
- 退出：keep/remove，fail-closed 变更检测，remove 后删分支
- 会话关系：模块级单槽 `currentWorktreeSession`；状态以 entry 存于会话 transcript，resume 回放恢复；`--worktree` 启动与会话中进入是两种身份语义（projectRoot 锚定不同）
- 子 agent：`createAgentWorktree` 不动会话状态，临时 slug 固定模式，30 天 fail-closed 清扫
- 扩展：WorktreeCreate/Remove hooks 支持非 git VCS；tmux 快进路径
- 未处理：`.claude/worktrees/` 的 git status 污染（依赖用户 gitignore）

### 9.2 grok-build（xai-fast-worktree）

- 独立 Rust crate（约 1.1 万行），目标是大仓库创建速度
- Linked 模式：`git worktree add --no-checkout` + 并行 CoW 复制（rapidhash 分片 + crossbeam，macOS 上限 8 线程防 FD 耗尽）+ 从源 index 重建 index，绕开 git 单线程 checkout
- Linux 快速路径：overlayfs-on-FUSE → btrfs 快照 → 文件复制兜底；沙箱内经 BtrfsDelegate 委托特权操作
- Standalone 模式：连 `.git/` 一起 CoW 的独立仓库，可 rename 提升；写 `.git/grok-worktree-source` 回链标记
- 集中存放 `~/.grok/worktrees/<repo-slug>/`（repo-slug = 路径末两组件）+ SQLite worktrees.db（session_id 归属、WorktreeKind: Session/Ab/Pool/Fork/Manual/Subagent）
- WorktreeSync：池化 worktree 认领时增量同步（reset --hard / clean / 脏文件复制 / staged 重放）
- Fork：`CreateWorktreeFromWorktree` 连脏状态派生新 worktree + 新会话（会话 fork 的物理实现）

## 10. 实现细节清单（从参考实现深挖）

按模块组织，标注来源（C = claude-code-leak，G = grok-build）与实施阶段。已并入 §4 正文的不再重复。

### 10.1 git 命令层（P1）

| 细节 | 来源 | 说明 |
|---|---|---|
| LFS/SSH 加固 env | G | `GIT_LFS_SKIP_SMUDGE=1`、`GIT_SSH_COMMAND="ssh -o BatchMode=yes"`——Claude 只有前两件，LFS 仓库差距明显 |
| `--no-optional-locks` | G | 每个 git 命令都加，与编辑器/其他 agent 的并发 git 共存 |
| ref/SHA 白名单校验 | C | `isSafeRefName`：`[a-zA-Z0-9/._+@-]`、禁 `..`/前导 `-`/空段；`isValidGitSha`：40/64 hex。`.git/` 是纯文本可被篡改，读出物流入命令参数前必检 |
| 免子进程读取 | C | `resolveGitDir`（`.git` 指针文件）、`getCommonDir`（`commondir` 文件）、`readGitHead`、`resolveRef`（loose→packed-refs）、`readWorktreeHeadSha`（不向上 walk）。Rust 侧实现为小工具函数，resume/计数等热路径省 15ms/次 |
| 默认分支解析 | C | `refs/remotes/origin/HEAD` symref → `main` → `master` → 回退 `main` |
| worktree 计数 | C | `readdir(<commonDir>/worktrees) + 1`（主 worktree 不在列） |
| 浅克隆检测 | C | `<commonDir>/shallow` 存在即浅克隆（影响 fetch 策略） |
| ENOSPC 友好报错 | G | 沿 error chain 找 "No space left on device"/`StorageFull`，统一改写为 "not enough free disk space" 上下文 |
| `diff-index --quiet` 快速脏检 | G | tracked-only，错误时 over-report（fail-safe）；需要清单才跑 status |

### 10.2 生命周期与安全（P1）

| 细节 | 来源 | 说明 |
|---|---|---|
| 检查/执行分离 | C | 安全门在 validate 阶段，执行阶段重计数仅用于展示；可变状态执行时防御性重查（validate→call race） |
| 可自愈错误消息 | C | 拒绝时列出改动 + 明确指示下一步（"确认后以 discard_changes=true 重试，或 keep 保留"），前端/模型都能直接引导用户 |
| no-op 显式声明 | C | 无活跃 worktree 时 exit 返回 "No-op: ... No filesystem changes were made." |
| 身份判定避开陷阱 | C | 判断"是否启动时进入"用持久化标志位，不用 cwd（shell 会改）也不用 join 路径（未 realpath） |
| exit 作用域声明 | C | 只作用于本会话创建的 worktree；手工创建/前会话创建的一律不碰（adopt 的只 keep） |
| 盘上删除成功才清状态 | G | 失败保留登记可重试，避免"盘上还在、状态没了"的孤儿 |
| 删除时 cwd 在主 repo | C+G | 不能在待删 worktree 内执行 remove；remove 后 sleep 100ms 再 `branch -D`（等锁释放） |

### 10.3 UX（P2）

| 细节 | 来源 | 说明 |
|---|---|---|
| 成功渲染 | C | `Switched to worktree on branch **<branch>**` + dimmed path，两行 |
| 改动分类计数 | G | modified/untracked/deleted 分开计数，拒绝消息更清晰 |
| 子目录偏移 | G | 见 §4.5 第 10 步；会话 cwd 是 repo 子目录时落到 worktree 内相同相对位置 |
| 显式输入优先于缓存配置 | C | 教训（issue #27044）：stale 的 feature flag 缓存曾静默吞掉用户显式 `--worktree`。若做 `worktree.autoEnter` 类配置，显式参数永远优先 |

### 10.4 后续增强备用（P3）

| 细节 | 来源 | 说明 |
|---|---|---|
| 后台复制 ignored 文件 | G | 前景可用后再复制 node_modules 等，并行度 2 留核给前台；worktree 删除时 CancellationToken 取消（token 同时驱动 async 与 sync 检查点） |
| `.worktreeinclude` 复制 | C | `git ls-files --others --ignored --exclude-standard --directory` 折叠目录（500k→数百条目），仅对精确前缀/锚定 glob 命中的目录二次展开 |
| staged 状态重放 | G | 不整体复制 index（会毁 stat cache）；`git add`/`git rm --cached` 逐条重放 |
| in-progress 认领锁 | G | contains+insert 同一把锁内完成；prepare 只读不置标记（置了清不掉会卡死重试） |
| WorktreeCreate hook 契约 | C | hook 收 slug 参数，返回 JSON `hookSpecificOutput.worktreePath`；与 git 路径互斥（hook 优先） |
| `-B` 重置孤儿分支 | C | 省一次 `branch -D` 子进程 |
| 空 index 防护 | G | 0 字节 index 会让解析器（gix）panic，解析前查文件长度 |
