@echo off
REM Detached driver: survivor re-check for the compat.rs sample.
REM
REM The primary sample (out-compat/, this directory) used `--lib --test
REM compat_projection_contract` as its witness set. Its 23 survivors include the
REM markdown-projection mutants (url_encode, home_dir, write_run, todo_line,
REM render_active_state), whose witnesses live in the crate's INTEGRATION test
REM targets -- so a survivor there may be a witness-set artifact rather than a
REM test gap. This run re-tests exactly those survivors with the integration
REM targets that reference compat rendering; the union is the DECIDED verdict
REM and every conversion is listed in summary.json's uncaught[].
REM
REM NOTE: this is a survivor re-check, NOT a second score for the file. The file's
REM primary measurement is out-compat/ (all 72 mutants, one witness set).
REM
REM NO CARGO_TARGET_DIR (one scratch target per job). NEVER --in-place.
cd /d "%~dp0..\..\.."
REM cargo-mutants copies the workspace (~1 GB) into %TEMP%; point TEMP/TMP at a
REM drive with room for it if your system drive is small (the host these runs
REM came from had a full C: drive -- see section 7.1 of the mutation report).
set "CARGO_BUILD_JOBS=2"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > "%~dp0out-compat-recheck-run.log"
cargo mutants -p future-loop --file orchestration/loop/src/compat.rs --gitignore true --timeout 300 -j 3 -o "%~dp0out-compat-recheck" --json -F "compat\.rs:(152:5|172:5|172:70|184:5|231:27|24:5|278:23|314:5|318:41|330:5|331:11|335:52|360:61|370:5|378:5|492:28|535:32|537:24|560:23|591:17|593:60)" --cargo-test-arg --lib --cargo-test-arg --test --cargo-test-arg compat_projection_contract --cargo-test-arg --test --cargo-test-arg misc_drive --cargo-test-arg --test --cargo-test-arg bughunt_regressions --cargo-test-arg --test --cargo-test-arg backfill_drive --cargo-test-arg --test --cargo-test-arg monitor_poll_contract --cargo-test-arg --test --cargo-test-arg run_loop_notes_drive --cargo-test-arg --test --cargo-test-arg console_drive_b >> "%~dp0out-compat-recheck-run.log" 2>&1
echo EXITCODE=%ERRORLEVEL% >> "%~dp0out-compat-recheck-run.log"
echo FINISHED %DATE% %TIME% >> "%~dp0out-compat-recheck-run.log"
