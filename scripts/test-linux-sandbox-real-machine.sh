#!/usr/bin/env bash
# Run the FutureOS Linux Bubblewrap acceptance checks and package the evidence.
# Usage: ./scripts/test-linux-sandbox-real-machine.sh [--full] [--output DIR]
set -u
set -o pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
RUN_FULL=0
OUTPUT_DIR=""

usage() {
  cat <<'EOF'
Usage: test-linux-sandbox-real-machine.sh [--full] [--output DIR]

  --full        Also run the complete Rust workspace test suite.
  --output DIR  Write evidence under DIR instead of the repository root.
  -h, --help    Show this help.
EOF
}

while (($#)); do
  case "$1" in
    --full)
      RUN_FULL=1
      shift
      ;;
    --output)
      if (($# < 2)); then
        echo "error: --output requires a directory" >&2
        exit 2
      fi
      OUTPUT_DIR="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cd "$REPO_ROOT"

HOST_LABEL="$(hostname 2>/dev/null || echo unknown-host)"
ARCH="$(uname -m 2>/dev/null || echo unknown-arch)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="$REPO_ROOT/target/linux-sandbox-evidence-${HOST_LABEL}-${ARCH}-${STAMP}"
fi
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
RESULTS_FILE="$OUTPUT_DIR/results.tsv"
: > "$RESULTS_FILE"
OVERALL=0

record() {
  local name="$1" status="$2" detail="$3"
  printf '%s\t%s\t%s\n' "$name" "$status" "$detail" >> "$RESULTS_FILE"
  printf '[%s] %s — %s\n' "$status" "$name" "$detail"
  if [[ "$status" == "FAIL" ]]; then
    OVERALL=1
  fi
}

run_step() {
  local name="$1" log="$2"
  shift 2
  echo
  echo "===== $name ====="
  set +e
  "$@" 2>&1 | tee "$OUTPUT_DIR/$log"
  local rc=${PIPESTATUS[0]}
  set -e
  if ((rc == 0)); then
    record "$name" PASS "exit 0; log=$log"
  else
    record "$name" FAIL "exit $rc; log=$log"
  fi
  return 0
}

# Keep collecting evidence after an individual failure.
set -e

{
  echo "timestamp_utc=$STAMP"
  echo "host=$HOST_LABEL"
  echo "repository=$REPO_ROOT"
  echo "commit=$(git rev-parse HEAD 2>/dev/null || echo unavailable)"
  echo "branch=$(git branch --show-current 2>/dev/null || echo unavailable)"
  echo "architecture=$ARCH"
  echo "kernel=$(uname -a 2>/dev/null || true)"
  echo
  echo "--- /etc/os-release ---"
  cat /etc/os-release 2>/dev/null || true
  echo
  echo "--- tool versions ---"
  command -v git || true
  git --version 2>/dev/null || true
  command -v cargo || true
  cargo --version 2>/dev/null || true
  rustc --version 2>/dev/null || true
  command -v bwrap || true
  bwrap --version 2>/dev/null || true
  dpkg-query -W -f='${Package} ${Version} ${Architecture}\n' bubblewrap 2>/dev/null || true
  rpm -q bubblewrap 2>/dev/null || true
} 2>&1 | tee "$OUTPUT_DIR/environment.log"

for required in git cargo rustc bwrap; do
  if command -v "$required" >/dev/null 2>&1; then
    record "prerequisite:$required" PASS "$(command -v "$required")"
  else
    record "prerequisite:$required" FAIL "command not found"
  fi
done

run_step "build future-agent" build.log cargo build -p future-agent

if [[ -x target/debug/future-agent ]]; then
  run_step "sandbox probe" probe.log target/debug/future-agent --probe-sandbox
  if grep -Eq '"available"[[:space:]]*:[[:space:]]*true' "$OUTPUT_DIR/probe.log" \
    && grep -Eq '"backend"[[:space:]]*:[[:space:]]*"linux_bubblewrap"' "$OUTPUT_DIR/probe.log"; then
    record "probe contract" PASS "available=true; backend=linux_bubblewrap"
  else
    record "probe contract" FAIL "probe did not report an available Linux Bubblewrap backend"
  fi
else
  record "sandbox probe" FAIL "target/debug/future-agent was not built"
  record "probe contract" FAIL "probe was not run"
fi

run_step "Linux sandbox unit tests" unit-tests.log \
  cargo test -p future-agent sandbox::linux -- --test-threads=1

run_step "Linux Bubblewrap real smoke" smoke.log \
  cargo test -p future-agent --test linux_sandbox_smoke -- \
    --ignored --test-threads=1 --nocapture

if grep -q "skipping Linux sandbox smoke" "$OUTPUT_DIR/smoke.log"; then
  record "smoke executed (not skipped)" FAIL "skip marker found in smoke.log"
elif grep -Eq "test result: ok\. 7 passed; 0 failed" "$OUTPUT_DIR/smoke.log"; then
  record "smoke executed (not skipped)" PASS "7 real smoke tests passed"
else
  record "smoke executed (not skipped)" FAIL "expected 7-pass result was not found"
fi

run_step "Rust formatting" fmt.log cargo fmt --all --check
run_step "future-agent clippy" clippy.log \
  cargo clippy -p future-agent --all-targets -- -D warnings

if ((RUN_FULL)); then
  run_step "complete Rust workspace tests" workspace-tests.log \
    cargo test --workspace -- --test-threads=1
else
  record "complete Rust workspace tests" "NOT RUN" "use --full to enable"
fi

COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unavailable)"
BRANCH="$(git branch --show-current 2>/dev/null || echo unavailable)"
{
  echo "# FutureOS Linux Sandbox Real-Machine Result"
  echo
  echo "- Timestamp (UTC): $STAMP"
  echo "- Host: $HOST_LABEL"
  echo "- Architecture: $ARCH"
  echo "- Branch: $BRANCH"
  echo "- Commit: $COMMIT"
  if ((OVERALL == 0)); then
    echo "- Overall: PASS"
  else
    echo "- Overall: FAIL"
  fi
  echo
  echo "| Check | Status | Detail |"
  echo "|---|---|---|"
  while IFS=$'\t' read -r name status detail; do
    printf '| %s | %s | %s |\n' "$name" "$status" "$detail"
  done < "$RESULTS_FILE"
  echo
  echo "Attach this summary and the adjacent logs when reporting the result."
} > "$OUTPUT_DIR/SUMMARY.md"

ARCHIVE="${OUTPUT_DIR}.tar.gz"
tar -czf "$ARCHIVE" -C "$(dirname "$OUTPUT_DIR")" "$(basename "$OUTPUT_DIR")"

echo
echo "===== RESULT ====="
if ((OVERALL == 0)); then
  echo "PASS: all required local real-machine checks passed."
else
  echo "FAIL: one or more required checks failed; inspect SUMMARY.md and logs."
fi
echo "Summary: $OUTPUT_DIR/SUMMARY.md"
echo "Upload this archive: $ARCHIVE"

exit "$OVERALL"
