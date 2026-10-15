#!/bin/bash
# install-future-goal.sh — install the future-goal CLI + skill locally.
#
#   CLI   -> ~/.local/bin/future-goal
#   skill -> ~/.future/agent/skills/future-goal/SKILL.md
#
# Usage: bash scripts/release/install-future-goal.sh [--release]
set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
SKILL_DIR="${HOME}/.future/agent/skills/future-goal"

MODE=debug
if [ "$1" = "--release" ]; then MODE=release; fi

echo "== building future-goal ($MODE) =="
cargo build -p future-goal --manifest-path "$REPO_ROOT/Cargo.toml" $([ "$MODE" = release ] && echo --release)

echo "== installing CLI =="
mkdir -p "$BIN_DIR"
cp "$REPO_ROOT/target/$MODE/future-goal" "$BIN_DIR/future-goal"
chmod +x "$BIN_DIR/future-goal"

echo "== linking skill (symlink to skills submodule SKILL.md, single source of truth) =="
mkdir -p "$SKILL_DIR"
ln -sf "$REPO_ROOT/skills/builtin/future-goal/SKILL.md" "$SKILL_DIR/SKILL.md"

echo "== verifying =="
"$BIN_DIR/future-goal" status
echo
echo "installed:"
echo "  CLI   -> $BIN_DIR/future-goal (add to PATH if needed: export PATH=\"$BIN_DIR:\$PATH\")"
echo "  skill -> $SKILL_DIR/SKILL.md"
echo
echo "next: tell the agent in conversation, or use the future-goal CLI directly."
