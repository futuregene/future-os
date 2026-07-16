@echo off
setlocal EnableExtensions EnableDelayedExpansion

rem FutureOS local GUI test (Windows). Mirrors scripts/start-gui-test.sh:
rem builds + starts future-agent, then runs the Tauri GUI in dev mode against it,
rem and stops the agent it started when the GUI exits.
rem
rem Knobs (set before running, e.g. `set REUSE_AGENT=1`):
rem   FUTURE_AGENT_GRPC_ADDR  default 127.0.0.1:50051
rem   GUI_DEV_PORT            default 5173
rem   REUSE_AGENT             1 = reuse an agent already listening on the port
rem   BUILD_AGENT             1 = cargo build the agent first (default 1)
rem   BUILD_CLI               1 = build the future CLI and put it on the agent's
rem                               PATH so skills that call `future` work (default 1)
rem   CLEAN_STALE_APP_TASKS   1 = cancel stale GUI runs/approvals first (default 1)
rem   RUN_CHECKS             1 = run lint/stylelint/test/build + cargo check first
rem   DRY_RUN                1 = print config and exit without starting anything

echo FutureOS local GUI test

set "SCRIPT_DIR=%~dp0"
pushd "%SCRIPT_DIR%.." || (echo Cannot resolve repo root & exit /b 1)
set "ROOT_DIR=%CD%"
popd
set "GUI_DIR=%ROOT_DIR%\gui"
set "AGENT_DIR=%ROOT_DIR%\agent"
set "CLI_DIR=%ROOT_DIR%\cli"
set "LOG_DIR=%ROOT_DIR%\.logs"

if not defined FUTURE_AGENT_GRPC_ADDR set "FUTURE_AGENT_GRPC_ADDR=127.0.0.1:50051"
set "AGENT_ADDR=%FUTURE_AGENT_GRPC_ADDR%"
for /f "tokens=1,2 delims=:" %%a in ("%AGENT_ADDR%") do (
  set "AGENT_HOST=%%a"
  set "AGENT_PORT=%%b"
)
if not defined GUI_DEV_PORT set "GUI_DEV_PORT=5173"
if not defined REUSE_AGENT set "REUSE_AGENT=0"
if not defined BUILD_AGENT set "BUILD_AGENT=1"
if not defined BUILD_CLI set "BUILD_CLI=1"
if not defined CLEAN_STALE_APP_TASKS set "CLEAN_STALE_APP_TASKS=1"
if not defined RUN_CHECKS set "RUN_CHECKS=0"
if not defined DRY_RUN set "DRY_RUN=0"

set "AGENT_LOG=%LOG_DIR%\future-agent-test.log"
set "AGENT_ERR=%LOG_DIR%\future-agent-test.err.log"
set "AGENT_PID_FILE=%LOG_DIR%\future-agent-test.pid"
set "AGENT_BIN=%AGENT_DIR%\target\debug\future-agent.exe"
set "AGENT_PID="

if not exist "%LOG_DIR%" mkdir "%LOG_DIR%"

echo Workspace: %ROOT_DIR%
echo Agent gRPC: %AGENT_ADDR%
echo GUI dev port: %GUI_DEV_PORT%

if "%DRY_RUN%"=="1" (
  echo DRY_RUN=1; startup checks only, not cleaning tasks or starting processes.
  exit /b 0
)

if "%CLEAN_STALE_APP_TASKS%"=="1" call :cancel_stale_app_tasks

if "%RUN_CHECKS%"=="1" (
  echo Running GUI checks...
  pushd "%GUI_DIR%" || exit /b 1
  call npm run lint || (popd & exit /b 1)
  call npm run stylelint || (popd & exit /b 1)
  call npm test || (popd & exit /b 1)
  call npm run build || (popd & exit /b 1)
  popd
  pushd "%GUI_DIR%\src-tauri" || exit /b 1
  cargo check || (popd & exit /b 1)
  popd
)

if not exist "%GUI_DIR%\node_modules" (
  echo Installing GUI dependencies...
  pushd "%GUI_DIR%" || exit /b 1
  call npm ci || (popd & exit /b 1)
  popd
)

if "%BUILD_AGENT%"=="1" (
  echo Building future-agent...
  pushd "%AGENT_DIR%" || exit /b 1
  cargo build || (popd & echo Failed to build future-agent & exit /b 1)
  popd
)

if "%BUILD_CLI%"=="1" call :build_cli
rem Put the built CLI on the agent's PATH so skills that shell out to `future`
rem resolve it. The agent (started below) inherits this process's environment.
if exist "%CLI_DIR%\dist\future.exe" set "PATH=%CLI_DIR%\dist;%PATH%"

call :port_in_use
set "PORT_BUSY=%ERRORLEVEL%"

if "%REUSE_AGENT%"=="1" if "%PORT_BUSY%"=="0" (
  echo Using existing future-agent at %AGENT_ADDR%
  goto :start_gui
)

if "%PORT_BUSY%"=="0" (
  echo Port %AGENT_PORT% is already in use.
  call :stop_pid_file_process
  call :port_in_use
  set "PORT_BUSY=%ERRORLEVEL%"
  if "!PORT_BUSY!"=="0" (
    echo Stop the old process, or run with REUSE_AGENT=1 to reuse it.
    exit /b 1
  )
)

if not exist "%AGENT_BIN%" (
  echo Agent binary not found at %AGENT_BIN%.
  echo Build it first ^(BUILD_AGENT defaults to 1^).
  exit /b 1
)

echo Starting future-agent...
rem Launch the agent binary directly via PowerShell so we capture its own PID
rem (not a wrapper's) and redirect stdout/stderr to log files reliably.
rem PowerShell writes the PID to the pid file; we read it back. Do NOT capture
rem the PID through `for /f` here: Start-Process redirection makes the spawned
rem agent inherit the for-pipe handle, so `for /f` blocks until the agent exits.
del /q "%AGENT_PID_FILE%" >nul 2>&1
powershell -NoProfile -ExecutionPolicy Bypass -Command "$p = Start-Process -FilePath $env:AGENT_BIN -ArgumentList @('--grpc-addr', $env:AGENT_ADDR) -WorkingDirectory $env:AGENT_DIR -RedirectStandardOutput $env:AGENT_LOG -RedirectStandardError $env:AGENT_ERR -WindowStyle Hidden -PassThru; [System.IO.File]::WriteAllText($env:AGENT_PID_FILE, [string]$p.Id)"

set "AGENT_PID="
if exist "%AGENT_PID_FILE%" set /p AGENT_PID=<"%AGENT_PID_FILE%"

if not defined AGENT_PID (
  echo Failed to start future-agent.
  echo Agent log: %AGENT_LOG%
  echo Agent err: %AGENT_ERR%
  exit /b 1
)

call :wait_for_agent || (call :cleanup & exit /b 1)
echo future-agent started pid=%AGENT_PID%
echo Agent log: %AGENT_LOG%

:start_gui
call :ensure_sidecars
echo Starting GUI...
echo Press Ctrl-C to stop the GUI. At the "Terminate batch job (Y/N)?" prompt choose N
echo so this script can stop the agent it started; choosing Y leaves the agent running
echo (the next run reclaims it via the pid file).
set "FUTURE_AGENT_GRPC_ADDR=%AGENT_ADDR%"
pushd "%GUI_DIR%" || (call :cleanup & exit /b 1)
if "%GUI_DEV_PORT%"=="5173" (
  call npm run tauri:dev
) else (
  set "TAURI_DEV_CONFIG_FILE=%TEMP%\futureos-tauri-dev-%RANDOM%.json"
  > "!TAURI_DEV_CONFIG_FILE!" (
    echo {
    echo   "build": {
    echo     "devUrl": "http://127.0.0.1:%GUI_DEV_PORT%",
    echo     "beforeDevCommand": "npm run dev -- --port %GUI_DEV_PORT%"
    echo   }
    echo }
  )
  call npm run tauri:dev -- --config "!TAURI_DEV_CONFIG_FILE!"
  del /q "!TAURI_DEV_CONFIG_FILE!" >nul 2>&1
)
set "GUI_EXIT=%ERRORLEVEL%"
popd

call :cleanup
exit /b %GUI_EXIT%

rem ---------------------------------------------------------------------------
:build_cli
rem Build the future CLI to a standalone dist\future.exe (matching make build-cli)
rem so the agent can shell out to it. Non-fatal: a failure only means skills that
rem call `future` won't work; the GUI test still runs.
where bun >nul 2>&1 || (
  echo bun not found; skipping future CLI build. Skills that call future will not work.
  exit /b 0
)
echo Building future CLI...
pushd "%CLI_DIR%" || exit /b 0
if not exist node_modules (
  call npm ci || (echo npm ci failed; skipping future CLI build. & popd & exit /b 0)
)
call npm run build || (echo CLI tsc build failed; skipping. & popd & exit /b 0)
call bun build --compile dist\index.js --outfile dist\future.exe || (echo bun compile failed; skipping. & popd & exit /b 0)
popd
exit /b 0

rem ---------------------------------------------------------------------------
:ensure_sidecars
rem Tauri validates bundle.externalBin sidecars (future-agent, future) at COMPILE
rem time — even for `tauri dev`. This script runs the agent standalone and the GUI
rem connects to it, so the sidecars are never launched here; they only need to
rem exist. Create empty placeholders (with the Windows .exe suffix) for any that
rem are missing (CI and the packaging scripts stage the real binaries). Mirrors
rem the sidecar block in start-gui-test.sh.
set "TRIPLE="
for /f "tokens=2" %%h in ('rustc -Vv ^| findstr /b /c:"host: "') do set "TRIPLE=%%h"
if not defined TRIPLE (
  echo Could not determine host triple from rustc; skipping sidecar placeholders.
  exit /b 0
)
set "BIN_DIR=%GUI_DIR%\src-tauri\binaries"
if not exist "%BIN_DIR%" mkdir "%BIN_DIR%"
for %%n in (future-agent future) do (
  if not exist "%BIN_DIR%\%%n-%TRIPLE%.exe" (
    echo Creating sidecar placeholder %%n-%TRIPLE%.exe
    type nul > "%BIN_DIR%\%%n-%TRIPLE%.exe"
  )
)
exit /b 0

:port_in_use
rem Returns 0 (errorlevel) if AGENT_PORT is LISTENING, 1 otherwise.
netstat -ano -p tcp | findstr /r /c:":%AGENT_PORT% .*LISTENING" >nul 2>&1
exit /b %ERRORLEVEL%

:wait_for_agent
set /a _attempts=0
:wait_loop
call :port_in_use && exit /b 0
set /a _attempts+=1
if %_attempts% GEQ 60 (
  echo future-agent did not become ready at %AGENT_ADDR%
  echo Agent log: %AGENT_LOG%
  echo Agent err: %AGENT_ERR%
  exit /b 1
)
rem `ping -n 2` waits ~1s and, unlike `timeout`, works when stdin is redirected
rem (CI / piped invocation), where `timeout` aborts with "Input redirection is
rem not supported".
ping -n 2 127.0.0.1 >nul
goto :wait_loop

:cleanup
if defined AGENT_PID (
  echo Stopping future-agent pid=%AGENT_PID%
  powershell -NoProfile -Command "Stop-Process -Id %AGENT_PID% -Force -ErrorAction SilentlyContinue" >nul 2>&1
  set "AGENT_PID="
)
if exist "%AGENT_PID_FILE%" del /q "%AGENT_PID_FILE%" >nul 2>&1
exit /b 0

:stop_pid_file_process
if not exist "%AGENT_PID_FILE%" exit /b 0
set /p OLD_AGENT_PID=<"%AGENT_PID_FILE%"
if not defined OLD_AGENT_PID (
  del /q "%AGENT_PID_FILE%" >nul 2>&1
  exit /b 0
)
for /f "usebackq delims=" %%s in (`powershell -NoProfile -Command "$id = 0; if (-not [int]::TryParse($env:OLD_AGENT_PID, [ref]$id)) { 'invalid'; exit }; $p = Get-Process -Id $id -ErrorAction SilentlyContinue; if ($p -and $p.ProcessName -eq 'future-agent') { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue; 'stopped' } elseif ($p) { 'ignored' } else { 'missing' }"`) do set "OLD_AGENT_STATUS=%%s"
if "%OLD_AGENT_STATUS%"=="stopped" echo Stopping previous future-agent pid=%OLD_AGENT_PID%
if "%OLD_AGENT_STATUS%"=="ignored" echo Ignoring stale future-agent pid file; pid=%OLD_AGENT_PID% is not future-agent.
del /q "%AGENT_PID_FILE%" >nul 2>&1
set "OLD_AGENT_PID="
set "OLD_AGENT_STATUS="
exit /b 0

:cancel_stale_app_tasks
set "APP_DB=%USERPROFILE%\.future\app\app.db"
if not exist "%APP_DB%" exit /b 0
where sqlite3 >nul 2>&1 || (echo sqlite3 not found; skipping stale app task cleanup. & exit /b 0)
echo Cancelling stale GUI runs and approvals in %APP_DB%
set "STALE_SQL=%TEMP%\futureos-stale-tasks-%RANDOM%.sql"
> "%STALE_SQL%" (
  echo UPDATE approval_requests
  echo SET status = 'cancelled',
  echo     decision_note = 'Cancelled by start-gui-test.bat before a fresh GUI test run.',
  echo     decided_at = CAST^(strftime^('%%s','now'^) AS INTEGER^) * 1000,
  echo     updated_at = CAST^(strftime^('%%s','now'^) AS INTEGER^) * 1000
  echo WHERE status = 'pending';
  echo UPDATE runs
  echo SET status = 'cancelled',
  echo     error_message = 'Cancelled by start-gui-test.bat before a fresh GUI test run.',
  echo     ended_at = COALESCE^(ended_at, CAST^(strftime^('%%s','now'^) AS INTEGER^) * 1000^),
  echo     updated_at = CAST^(strftime^('%%s','now'^) AS INTEGER^) * 1000
  echo WHERE status IN ^('queued', 'running', 'waiting_approval'^);
)
sqlite3 "%APP_DB%" < "%STALE_SQL%" || echo Skipping stale app task cleanup because the database is busy or not initialized.
del /q "%STALE_SQL%" >nul 2>&1
exit /b 0
