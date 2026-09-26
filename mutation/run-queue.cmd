@echo off
REM Detached driver for the queue.rs mutation sample (attempt 3).
REM Launched via WMI Win32_Process.Create so the harness's per-turn process-tree
REM teardown cannot kill it (attempts 1 and 2 died that way / on disk-full).
REM NOTE: no CARGO_TARGET_DIR is set -- cargo-mutants uses one scratch copy per job.
cd /d D:\future-os\.worktrees\cov100
set "TEMP=D:\cov100-mut-tmp"
set "TMP=D:\cov100-mut-tmp"
set "CARGO_BUILD_JOBS=3"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > mutation\out-queue-run.log
D:\.cargo\bin\cargo.exe mutants -p future-channel --file channels/src/bridge/queue.rs --gitignore true --timeout 300 -j 2 -o mutation\out-queue --cargo-test-arg --lib --cargo-test-arg bridge:: >> mutation\out-queue-run.log 2>&1
echo EXITCODE=%ERRORLEVEL% >> mutation\out-queue-run.log
echo FINISHED %DATE% %TIME% >> mutation\out-queue-run.log
