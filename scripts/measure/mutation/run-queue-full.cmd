@echo off
REM Detached driver: COMPLETE re-run of all 37 queue.rs mutants on the current
REM revision FE40F333... The previous "32 caught" was stitched together from a
REM 23-catch full run plus a 10-mutant targeted re-run of only the survivors --
REM not one measurement on one revision. Adding tests can in principle flip a
REM previously-caught mutant, so all 37 are re-run here.
REM
REM Witness set: the FULL lib target (`--lib`), no module filter (steering step 1).
REM
REM NO CARGO_TARGET_DIR (one scratch target per job). NEVER --in-place.
cd /d "%~dp0..\..\.."
REM cargo-mutants copies the workspace (~1 GB) into %TEMP%; point TEMP/TMP at a
REM drive with room for it if your system drive is small (the host these runs
REM came from had a full C: drive -- see section 7.1 of the mutation report).
set "CARGO_BUILD_JOBS=3"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > "%~dp0out-queue-full-run.log"
cargo mutants -p future-channel --file channels/src/bridge/queue.rs --gitignore true --timeout 300 -j 2 -o "%~dp0out-queue-full" --json --cargo-test-arg --lib >> "%~dp0out-queue-full-run.log" 2>&1
echo EXITCODE=%ERRORLEVEL% >> "%~dp0out-queue-full-run.log"
echo FINISHED %DATE% %TIME% >> "%~dp0out-queue-full-run.log"
