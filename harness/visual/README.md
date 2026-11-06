# Visual regression baselines

Each directory holds a case's recording as downscaled grayscale frames plus a
`meta.json` with the case's threshold and the record of who updated it.

## ⚠️ These baselines are dirty-env captures

dirty-env 采集：这批基线是用未隔离 FUTURE_HOME 的 cli.mjs 录的，那次运行会落到真后端、往真实会话库写空壳。像素本身可用作本轮对照，但不能当长期/CI 基准——必须在隔离 FUTURE_HOME 下重建一次。

Every `meta.json` here carries `"dirtyEnv": true` for that reason. To rebuild one
under the isolated home the harness now uses:

```bash
node harness/scripts/visual-regression.mjs --case <id> --update-baseline \
     --reason "rebuild under isolated FUTURE_HOME"
```

A baseline is an anchor. An anchor dropped in a dirty environment passes its bias
on to every comparison that follows, so rebuilding is a precondition for using
any of this as a CI reference — not a cleanup chore.

## ⚠️ They were also recorded inside a mutation window

录制时间落在变异窗口里：这批帧采集于 2026-11-06，起点是变异提交 `3f878d573`
（14:58 记录，把折叠态的浮动按钮从 `right-4` 挪到 `right-3`，即 4px），采集会话
记下的时间是变异之后约 25 分钟；收录这批帧的提交 `d184f764d` 在 16:24。所以凡是
拍到会话面板的帧，都可能带着那 4px 位移。

The window is what matters, not the exact minute: the mutation lived from 14:58
until `693f4d5d9` reverted it, and these frames were recorded inside that span.
`page-map.json` did not flag it. Its `prefixes` do hold two entries inside
`desktop/src/components/layout/` — `ActivityRail` and `Workspace` — but prefixes
are matched with `f.startsWith(prefix)`, and neither `ContextPanel.tsx` there nor
`ThreadHeader.tsx` under `desktop/src/features/agent/` starts with any of them,
so no case was declared affected and no baseline was rejected. That is a limit of
judging "which frames does this change reach" from a prefix list, not evidence
that the frames are clean.

Treat every frame that shows the conversation panel as suspect: rebuild before
using this set as a reference, and do not measure a delta against these pixels to
decide whether the 4px shift is still there.

### Identity when a mutation has to be committed

A mutation only needs a commit when the revert itself is the evidence you are keeping.
When you do commit one, use an author that cannot be mistaken for a person:

```bash
git -c user.name=harness -c user.email=devnull@localhost commit -m "mutation: <what>"
# or: git commit --author="harness <devnull@localhost>"
```

A placeholder such as `x <x@y>` is worse than useless: `x@y` is registered to a real
GitHub account, so the commit is attributed to a stranger, and that attribution rides
along into any PR that squashes it (PR #1565 carries two such commits). Better still,
do not commit at all: edit the file, run the case, `git checkout -- <path>`.

## What a baseline is allowed to hide

- `threshold` comes from the case's own jitter (>= 5 repeats). Ten cases measured
  byte-identical frames (spread 0.0000), so 0.01 is a floor with margin.
- A case whose own runs disagree by more than 5% is refused rather than given a
  loose threshold.
- `--update-baseline` requires `--reason`: an implicit update silences every
  future regression.
