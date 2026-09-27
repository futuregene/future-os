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
cd /d "%~dp0..\..\.."
REM cargo-mutants copies the workspace (~1 GB) into %TEMP%; point TEMP/TMP at a
REM drive with room for it if your system drive is small (the host these runs
REM came from had a full C: drive -- see section 7.1 of the mutation report).
set "CARGO_BUILD_JOBS=3"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > "%~dp0out-policy-stable-run.log"
cargo mutants -p future-channel --file channels/src/policy.rs --gitignore true --timeout 300 -j 2 -o "%~dp0out-policy-stable" --json --cargo-test-arg --lib >> "%~dp0out-policy-stable-run.log" 2>&1
echo EXITCODE=%ERRORLEVEL% >> "%~dp0out-policy-stable-run.log"
echo FINISHED %DATE% %TIME% >> "%~dp0out-policy-stable-run.log"
