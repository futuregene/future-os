@echo off
REM Detached driver for the queue.rs mutation sample (attempt 3).
REM Launched via WMI Win32_Process.Create so the harness's per-turn process-tree
REM teardown cannot kill it (attempts 1 and 2 died that way / on disk-full).
REM NOTE: no CARGO_TARGET_DIR is set -- cargo-mutants uses one scratch copy per job.
cd /d "%~dp0..\..\.."
REM cargo-mutants copies the workspace (~1 GB) into %TEMP%; point TEMP/TMP at a
REM drive with room for it if your system drive is small (the host these runs
REM came from had a full C: drive -- see section 7.1 of the mutation report).
set "CARGO_BUILD_JOBS=3"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > "%~dp0out-queue-run.log"
cargo mutants -p future-channel --file channels/src/bridge/queue.rs --gitignore true --timeout 300 -j 2 -o "%~dp0out-queue" --cargo-test-arg --lib --cargo-test-arg bridge:: >> "%~dp0out-queue-run.log" 2>&1
echo EXITCODE=%ERRORLEVEL% >> "%~dp0out-queue-run.log"
echo FINISHED %DATE% %TIME% >> "%~dp0out-queue-run.log"
