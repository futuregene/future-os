# FUTURE.md — Workspace Memory

## Worktree 支持设计方案（2026-07 定稿方向）

**完整设计文档已落盘：[WORKTREE_DESIGN.md](WORKTREE_DESIGN.md)**，以下内容以其为准。

用户正在为 future-os 设计 git worktree 支持，已确认的关键决策：

- **不加 agent tools**：保持现有 4 个工具（shell/read/write/edit）不变
- **实现通路**：agent 侧新增两个 gRPC host 命令 `enter_worktree` / `exit_worktree`，复用现有 `set_cwd` 命令的会话切换通路（commands.rs L576）；TUI 加一个 `/worktree` slash 命令，GUI 用按钮，channel 用 slash 解析
- **存放位置**：`<git-root>/.future/worktrees/<slug>`，分支名 `worktree-<slug>`；必须解析 canonical git root（commondir）防嵌套
- **git 污染处理**：enter 时自动往 `.git/info/exclude` 追加 `/.future/worktrees/`（用户已确认）
- **参考实现**：claude-code-leak（~/claude-code-leak/src/utils/worktree.ts）的 git CLI 封装路线；不学 grok-build（~/grok-build/crates/codegen/xai-fast-worktree）的 CoW/快照引擎
- **prompt/sandbox 联动已查清**：所有 cwd 依赖项每次 prompt/run 从 self.cwd 重解析（session_prompt.rs:19/116/278），无缓存、无需失效逻辑；mid-session enter 时 skills/AGENTS.md/FUTURE.md 锚定 original_cwd，仅 env working_directory 指向 worktree；enter/exit 需 is_streaming 互斥
