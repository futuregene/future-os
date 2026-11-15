#!/usr/bin/env bash
# Reclaim the GitHub Actions cache quota by deleting only per-ref npm caches.
#
# Why: actions/setup-node with `cache: npm` keys the tarball by ref, so every PR
# writes its own ~318 MB (Linux) + ~305 MB (macOS) copy that no other PR can
# restore. Measured 2026-11-15: 27 entries / 10.19 GB against a 10 GB quota, of
# which 21 entries / 6.40 GB were these per-ref node caches, all pointing at
# refs/pull/*/merge for PRs that have already merged, plus one on dev-huichen.
# The 6 v0-rust-* entries are the only ones any lane can restore, and they were
# being evicted to make room for the npm copies.
#
# This deletes nothing else: entries are filtered by key prefix `node-cache`.
# Rust caches, and any cache on refs/heads/main, are left alone.
#
# Deleting a cache needs only `repo` scope (no `workflow` scope), so this works
# on a machine whose PAT cannot push workflow edits.
#
# Usage:
#   scripts/ci/prune-node-caches.sh          # list what would be deleted
#   scripts/ci/prune-node-caches.sh --apply  # delete them
set -uo pipefail

APPLY=0
[[ "${1:-}" == "--apply" ]] && APPLY=1

command -v gh >/dev/null || { echo "gh is required" >&2; exit 1; }

gh api 'repos/:owner/:repo/actions/caches?per_page=100' --paginate \
  --jq '.actions_caches[] | "\(.id)|\(.size_in_bytes)|\(.ref)|\(.key)"' |
while IFS='|' read -r id bytes ref key; do
  [[ "$key" == node-cache* ]] || continue
  mb=$((bytes / 1048576))
  if ((APPLY)); then
    if gh api --method DELETE "repos/:owner/:repo/actions/caches/$id" >/dev/null 2>&1; then
      printf 'deleted  %4dMB  %-24s %s\n' "$mb" "$ref" "$key"
    else
      printf 'FAILED   %4dMB  %-24s %s\n' "$mb" "$ref" "$key" >&2
    fi
  else
    printf 'would delete  %4dMB  %-24s %s\n' "$mb" "$ref" "$key"
  fi
done

if ((APPLY)); then
  echo
  gh api 'repos/:owner/:repo/actions/caches?per_page=100' --paginate \
    --jq '[.actions_caches[] | .size_in_bytes] | add / 1073741824' |
    awk '{printf "total now: %.2f GB (quota 10 GB)\n", $1}'
fi
