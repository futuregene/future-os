#!/usr/bin/env bash
#
# publish-wiki.sh — sync docs/wiki/ into the GitHub wiki repository.
#
# The GitHub wiki is a separate git repo (<repo>.wiki.git). This script clones
# it (or initializes it from scratch if it has never existed), replaces its
# Markdown pages with the contents of docs/wiki/, and pushes.
#
# Usage:
#   scripts/publish-wiki.sh
#
# Environment overrides:
#   WIKI_REMOTE        wiki git remote (default: SSH to futuregene/future-os.wiki)
#   WIKI_SRC           source dir of .md pages (default: <repo>/docs/wiki)
#   WIKI_BRANCH        wiki branch (default: master — GitHub wikis use master)
#   GIT_AUTHOR_NAME    commit author name  (default: wiki-sync)
#   GIT_AUTHOR_EMAIL   commit author email (default: wiki-sync@users.noreply.github.com)
#
# Prerequisite: the repository's "Wikis" feature must be ENABLED
# (Settings → Features → Wikis). That toggle requires repo admin and cannot be
# set from this script. Once it's on, this script creates and fills the wiki
# automatically — no need to hand-create a first page in the web UI.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WIKI_SRC="${WIKI_SRC:-$REPO_ROOT/docs/wiki}"
WIKI_REMOTE="${WIKI_REMOTE:-git@github.com:futuregene/future-os.wiki.git}"
WIKI_BRANCH="${WIKI_BRANCH:-master}"

if [ ! -d "$WIKI_SRC" ] || ! ls "$WIKI_SRC"/*.md >/dev/null 2>&1; then
  echo "error: no .md files found in $WIKI_SRC" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
WIKI_DIR="$TMP/wiki"

if git clone --depth 1 "$WIKI_REMOTE" "$WIKI_DIR" 2>"$TMP/clone.err"; then
  echo "==> Cloned existing wiki."
  cd "$WIKI_DIR"
  git checkout -B "$WIKI_BRANCH" >/dev/null 2>&1 || true
  # Replace the existing top-level pages (keep .git).
  find . -maxdepth 1 -type f -name '*.md' -delete
else
  echo "==> Wiki not initialized yet — creating it from scratch."
  mkdir -p "$WIKI_DIR"
  cd "$WIKI_DIR"
  git init -q
  git remote add origin "$WIKI_REMOTE"
  git checkout -q -B "$WIKI_BRANCH"
fi

cp "$WIKI_SRC"/*.md ./
git add -A

if git diff --cached --quiet; then
  echo "==> No changes — wiki already up to date."
  exit 0
fi

SRC_SHA="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
git \
  -c user.name="${GIT_AUTHOR_NAME:-wiki-sync}" \
  -c user.email="${GIT_AUTHOR_EMAIL:-wiki-sync@users.noreply.github.com}" \
  commit -q -m "docs(wiki): sync from docs/wiki @ ${SRC_SHA}"

echo "==> Pushing to $WIKI_BRANCH"
if ! git push origin "HEAD:$WIKI_BRANCH" 2>"$TMP/push.err"; then
  cat "$TMP/push.err" >&2
  echo >&2
  echo "error: push to the wiki failed." >&2
  echo "       Most likely the repository's Wikis feature is disabled." >&2
  echo "       A repo admin must enable it once: Settings → Features → Wikis." >&2
  echo "       After that, re-run this script (or the Publish Wiki workflow)." >&2
  exit 1
fi

echo "==> Wiki updated."
