@echo off
REM Detached driver: FIRST mutation sample of orchestration/loop/src/compat.rs (72
REM mutants) -- the loop crate's waivers are the largest block of this goal and
REM mutation sampling is the only instrument that can ask whether they hide a
REM real test gap. Also proves cargo-mutants works on future-loop at all.
REM
REM WITNESS SET: `--lib` PLUS `--test compat_projection_contract`.
REM Deviation from the steering's bare `-- --lib`, and why: 18 of the 72 mutants
REM are in the markdown projection (url_encode / rfc3339 / future_loop_status /
REM future_loop_task_class / render_active_state / todo_line). Nothing in the lib
REM test target calls those functions -- their whole test surface is the 4 tests
REM of tests/compat_projection_contract.rs (rfc3339_matches_future_loop_shape,
REM todo_anchors_match_future_loop_format, enum_values_match_loopx,
REM file_layout_is_project_local). A `--lib`-only witness set would therefore
REM report them MISSED for a reason that is an artifact of the witness set, and a
REM witness-set gap is exactly the class of unusable verdict this re-run exists to
REM eliminate. `--lib` is kept as well (the 14 lock/pid/home unit tests in
REM compat.rs::tests only exist there).
REM
REM -j 3 (not the steering's -j 2) because the box was otherwise idle (verified:
REM no other cargo/rustc/cargo-llvm-cov alive) and -j 2 would push the total wall
REM clock past the 75-minute cap; CARGO_BUILD_JOBS is lowered to 2 so three jobs
REM never exceed the 6 concurrent rustc that already ran safely here.
REM
REM NO CARGO_TARGET_DIR (one scratch target per job). NEVER --in-place.
cd /d D:\future-os\.worktrees\cov100
set "TEMP=D:\cov100-mut-tmp"
set "TMP=D:\cov100-mut-tmp"
set "CARGO_BUILD_JOBS=2"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > mutation\out-compat-run.log
D:\.cargo\bin\cargo.exe mutants -p future-loop --file orchestration/loop/src/compat.rs --gitignore true --timeout 300 -j 3 -o mutation\out-compat --json --cargo-test-arg --lib --cargo-test-arg --test --cargo-test-arg compat_projection_contract >> mutation\out-compat-run.log 2>&1
echo EXITCODE=%ERRORLEVEL% >> mutation\out-compat-run.log
echo FINISHED %DATE% %TIME% >> mutation\out-compat-run.log
