@echo off
REM Detached driver: complete the policy.rs sample (all 24 mutants) on the
REM hash-attested revision 9E746D4F...
REM
REM Witness set: `--lib policy:: bridge::` (244 tests) -- the two modules that can
REM observe PolicyEngine (policy::tests directly; bridge::tests through
REM Bridge::handle -> check_dm/check_group). Provider tests also reach policy
REM through the bridge, so any mutant this narrower set reports MISSED is
REM re-checked individually against the FULL lib suite before being believed.
REM
REM Why a filter at all: `--skip=<flaky test>` cannot be used -- cargo-mutants
REM inserts --cargo-test-arg values BEFORE cargo's `--`, and cargo rejects
REM `--skip` there (verified: "error: unexpected argument '--skip' found", run
REM 18:51). Filtering by module is the mechanism that is actually available, and
REM it removes the known-flaky provider/transport tests by construction.
REM
REM NO CARGO_TARGET_DIR -- one scratch target per job is what fixes the LNK1104 /
REM cross-job-stale-binary defect of the first round.
cd /d "%~dp0..\..\.."
REM cargo-mutants copies the workspace (~1 GB) into %TEMP%; point TEMP/TMP at a
REM drive with room for it if your system drive is small (the host these runs
REM came from had a full C: drive -- see section 7.1 of the mutation report).
set "CARGO_BUILD_JOBS=3"
set "RUST_TEST_THREADS=4"
echo STARTED %DATE% %TIME% > "%~dp0out-policy-final-run.log"
REM `--cargo-test-arg=--` puts a literal `--` into cargo's argv, so the two filters
REM reach the test binary as libtest filters (cargo itself accepts only ONE
REM TESTNAME positional: "unexpected argument 'bridge::' found", run 18:56).
cargo mutants -p future-channel --file channels/src/policy.rs --gitignore true --timeout 300 -j 2 -o "%~dp0out-policy-final" --cargo-test-arg --lib --cargo-test-arg=-- --cargo-test-arg policy:: --cargo-test-arg bridge:: >> "%~dp0out-policy-final-run.log" 2>&1
echo EXITCODE=%ERRORLEVEL% >> "%~dp0out-policy-final-run.log"
echo FINISHED %DATE% %TIME% >> "%~dp0out-policy-final-run.log"
