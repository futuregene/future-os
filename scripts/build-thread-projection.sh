#!/usr/bin/env bash
# Build the shared @future-os/thread-projection package when its compiled dist/
# is missing or older than the TypeScript sources. desktop/ and mobile/ both
# consume it through a `file:` dependency, so it must be up to date before either
# app builds, typechecks or starts. Every build-*/start-* script calls this so no
# one has to remember to rebuild it by hand.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TP="$ROOT/thread-projection"
# thread-projection is a root-workspace member, so npm hoists its node_modules
# (and the install stamp) to the repo root — not next to the package.
STAMP="$ROOT/node_modules/.package-lock.json"

if [[ ! -f "$STAMP" ]] || [[ "$TP/package.json" -nt "$STAMP" ]]; then
  echo "  npm install thread-projection/"
  (cd "$TP" && npm install)
fi

if [[ ! -f "$TP/dist/index.js" ]] || \
   find "$TP/src" -newer "$TP/dist/index.js" -print -quit | grep -q .; then
  echo "  build thread-projection/"
  (cd "$TP" && npm run build)
fi
