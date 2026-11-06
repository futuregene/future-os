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

## What a baseline is allowed to hide

- `threshold` comes from the case's own jitter (>= 5 repeats). Ten cases measured
  byte-identical frames (spread 0.0000), so 0.01 is a floor with margin.
- A case whose own runs disagree by more than 5% is refused rather than given a
  loose threshold.
- `--update-baseline` requires `--reason`: an implicit update silences every
  future regression.
