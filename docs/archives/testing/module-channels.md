# Archived testing records — `docs/testing/module-channels.md`

Moved out of `docs/testing/module-channels.md` on 2026-09-27 during the
historical-pollution cleanup (task T4a, goal `goal_39567c2a6b22`,
worktree `docs-testing`). The text below is verbatim, with its original
dates; the live record keeps a one-line pointer where each block was
removed.

---

```powershell
# the older two-step recipe (private target dir), kept for reference
$env:CARGO_TARGET_DIR='target/cov-chan2'
cargo llvm-cov -p future-channel --no-report --no-fail-fast -- --test-threads=2  # 1451 lib tests, 6 binary tests
cargo llvm-cov report --json --output-path coverage/channels-report.json
python .future/cov100/verify.py crate future-channels 99.99 coverage/channels-report.json
python .future/cov100/verify.py uncovered future-channels coverage/channels-report.json
python .future/cov100/verify.py windows-red-baseline future-channel
```
