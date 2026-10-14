@echo off
rem Default embedded hosting; external debugging connects to an existing Agent.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-desktop-windows.ps1" %*
exit /b %ERRORLEVEL%
