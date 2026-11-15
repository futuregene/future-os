#!/usr/bin/env bash
# Reclaim the disk the incremental cache grows into.
#
# Why: `target/debug/incremental` is pure accelerator state — cargo rebuilds it
# on demand and no artefact inside it is needed to link a binary. On this
# workspace it is also the single biggest directory cargo creates: measured
# 2026-11-15, `target` held 148 GB with 67 GB of it under `debug/incremental`
# (and 66 GB under `debug/deps`, which IS needed), on a volume that was down to
# 54 GB free. A nearly-full volume also slows builds and, per
# .future/memory/coverage-llvm-cov.md, makes the coverage lane refuse to run.
#
# Deleting it costs one non-incremental rebuild per crate touched next, which
# measured 5.8 s for `cargo check -p future-config` on an otherwise warm tree.
#
# It refuses to run while a cargo/rustc process is alive: removing the directory
# out from under a running build corrupts that build's state.
#
# Usage:
#   scripts/dev/prune-target-incremental.sh            # report only
#   scripts/dev/prune-target-incremental.sh --apply    # delete
#   scripts/dev/prune-target-incremental.sh --apply --target-dir /path/to/target
set -uo pipefail

APPLY=0
TARGET=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --apply) APPLY=1; shift ;;
    --target-dir) TARGET="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$TARGET" ]]; then
  ROOT=$(git rev-parse --show-toplevel) || exit 1
  TARGET="$ROOT/target"
fi

DIR="$TARGET/debug/incremental"
[[ -d "$DIR" ]] || { echo "no incremental cache at $DIR"; exit 0; }

if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
  echo "refusing to touch $DIR: a cargo/rustc process is running" >&2
  pgrep -lx cargo rustc >&2
  exit 1
fi

SIZE=$(du -sh "$DIR" | cut -f1)
if ((APPLY)); then
  rm -rf "$DIR"
  echo "removed $SIZE from $DIR"
  du -sh "$TARGET" 2>/dev/null | awk '{print "target now: "$1}'
else
  echo "would remove $SIZE from $DIR"
  echo "run with --apply to delete it (cargo rebuilds it on demand)"
fi
