#!/bin/bash
# install-future-loop.sh — install the future-loop CLI + skill locally.
#
#   CLI   -> ~/.local/bin/future-loop
#   skill -> ~/.future/agent/skills/future-loop/SKILL.md
#
# Usage: bash scripts/install-future-loop.sh [--release]
set -e
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
SKILL_DIR="${HOME}/.future/agent/skills/future-loop"

MODE=debug
if [ "$1" = "--release" ]; then MODE=release; fi

echo "== building future-loop ($MODE) =="
cargo build -p future-loop --manifest-path "$REPO_ROOT/Cargo.toml" $([ "$MODE" = release ] && echo --release)

echo "== installing CLI =="
mkdir -p "$BIN_DIR"
cp "$REPO_ROOT/target/$MODE/future-loop" "$BIN_DIR/future-loop"
chmod +x "$BIN_DIR/future-loop"

echo "== linking skill (symlink to skills submodule SKILL.md, single source of truth) =="
mkdir -p "$SKILL_DIR"
ln -sf "$REPO_ROOT/skills/builtin/future-loop/SKILL.md" "$SKILL_DIR/SKILL.md"

echo "== verifying =="
"$BIN_DIR/future-loop" status
echo
echo "installed:"
echo "  CLI   -> $BIN_DIR/future-loop (add to PATH if needed: export PATH=\"$BIN_DIR:\$PATH\")"
echo "  skill -> $SKILL_DIR/SKILL.md"
echo
echo "next: tell the agent in conversation, or use the future-loop CLI directly."
