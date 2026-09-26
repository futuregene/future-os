@echo off
REM Detached driver: RE-RUN the complete policy.rs sample on the now-stable suite.
REM Why: the first complete run (out-policy-final) ran while channels/src/bridge
REM carried a shared literal port collision (127.0.0.1:18787) that made the lib
REM suite fail 17/24 times; w-flake removed it. A verdict that rests on an
REM unrelated failing test is worthless, so the whole file is re-measured here.
REM
REM Witness set: the FULL lib target (`--lib`), i.e. no module filter -- the
REM flake is fixed and the steering explicitly asks for the unfiltered suite.
REM
REM NO CARGO_TARGET_DIR (one scratch target per job). NEVER --in-place.
cd /d D:\future-os\.worktrees\cov100
set "TEMP=D:\cov100-mut-tmp"
set "TMP=D:\cov100-mut-tmp"
set "CARGO_BUILD_JOBS=3"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > mutation\out-policy-stable-run.log
D:\.cargo\bin\cargo.exe mutants -p future-channel --file channels/src/policy.rs --gitignore true --timeout 300 -j 2 -o mutation\out-policy-stable --json --cargo-test-arg --lib >> mutation\out-policy-stable-run.log 2>&1
echo EXITCODE=%ERRORLEVEL% >> mutation\out-policy-stable-run.log
echo FINISHED %DATE% %TIME% >> mutation\out-policy-stable-run.log
