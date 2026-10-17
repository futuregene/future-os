#!/usr/bin/env bash
# Control for the partial-measurement guard: the patterns that decide PARTIAL must fire on a failing
# run's log and NOT on a green one. A guard that cannot fire protects nothing (the campaign's standing
# lesson), and a guard that fires on green output would mark every track partial.
set -uo pipefail
T=$(mktemp -d)
green="$T/green.log"; red="$T/red.log"; aborted="$T/aborted.log"

cat > "$green" <<'EOF'
   Compiling future-agent v0.0.0
    Finished test profile in 12.3s
     Running unittests src/lib.rs
running 2458 tests
test result: ok. 2458 passed; 0 failed; 0 ignored; 0 measured; finished in 41.2s
EOF

cat > "$red" <<'EOF'
running 2458 tests
test models::tests::registry_injects_future_models_from_disk_cache ... FAILED
test result: FAILED. 2457 passed; 1 failed; 0 ignored; finished in 40.9s
error: test failed, to rerun pass `-p future-agent --lib`
EOF

cat > "$aborted" <<'EOF'
running 1159 tests
thread 'main' panicked at library/std/src/io/stdio.rs:1123:9:
failed printing to stdout: Broken pipe (os error 32)
error: test failed
EOF

PAT='test result: FAILED|error: test failed|panicked at'
for f in green red aborted; do
  if grep -qE "$PAT" "$T/$f.log"; then verdict="PARTIAL (guard fired)"; else verdict="quotable"; fi
  printf "  %-9s -> %s\n" "$f" "$verdict"
done
rm -rf "$T"
echo
echo "expected: green=quotable, red=PARTIAL, aborted=PARTIAL"
