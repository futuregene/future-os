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
if not defined CLEAN_STALE_APP_TASKS set "CLEAN_STALE_APP_TASKS=1"
if not defined RUN_CHECKS set "RUN_CHECKS=0"
if not defined DRY_RUN set "DRY_RUN=0"

set "AGENT_LOG=%LOG_DIR%\future-agent-test.log"
set "AGENT_ERR=%LOG_DIR%\future-agent-test.err.log"
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

call :port_in_use
set "PORT_BUSY=%ERRORLEVEL%"

if "%REUSE_AGENT%"=="1" if "%PORT_BUSY%"=="0" (
  echo Using existing future-agent at %AGENT_ADDR%
  goto :start_gui
)

if "%PORT_BUSY%"=="0" (
  echo Port %AGENT_PORT% is already in use.
  echo Stop the old process, or run with REUSE_AGENT=1 to reuse it.
  exit /b 1
)

if not exist "%AGENT_BIN%" (
  echo Agent binary not found at %AGENT_BIN%.
  echo Build it first ^(BUILD_AGENT defaults to 1^).
  exit /b 1
)

echo Starting future-agent...
rem Launch the agent binary directly via PowerShell so we capture its own PID
rem (not a wrapper's) and redirect stdout/stderr to log files reliably.
for /f "usebackq delims=" %%p in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "(Start-Process -FilePath '%AGENT_BIN%' -WorkingDirectory '%AGENT_DIR%' -RedirectStandardOutput '%AGENT_LOG%' -RedirectStandardError '%AGENT_ERR%' -WindowStyle Hidden -PassThru).Id"`) do set "AGENT_PID=%%p"

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
echo Starting GUI...
echo Press Ctrl-C here to stop the GUI; the agent this script started is stopped afterward.
set "FUTURE_AGENT_GRPC_ADDR=%AGENT_ADDR%"
pushd "%GUI_DIR%" || (call :cleanup & exit /b 1)
call npm run tauri:dev
set "GUI_EXIT=%ERRORLEVEL%"
popd

call :cleanup
exit /b %GUI_EXIT%

rem ---------------------------------------------------------------------------
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
timeout /t 1 /nobreak >nul
goto :wait_loop

:cleanup
if defined AGENT_PID (
  echo Stopping future-agent pid=%AGENT_PID%
  powershell -NoProfile -Command "Stop-Process -Id %AGENT_PID% -Force -ErrorAction SilentlyContinue" >nul 2>&1
  set "AGENT_PID="
)
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
  echo UPDATE tool_calls
  echo SET status = 'failed',
  echo     ended_at = COALESCE^(ended_at, CAST^(strftime^('%%s','now'^) AS INTEGER^) * 1000^)
  echo WHERE status = 'running';
)
sqlite3 "%APP_DB%" < "%STALE_SQL%" || echo Skipping stale app task cleanup because the database is busy or not initialized.
del /q "%STALE_SQL%" >nul 2>&1
exit /b 0
