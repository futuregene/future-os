; Preflight BOTH executables before Tauri copies anything. Its built-in
; CheckIfAppIsRunning covers only the desktop, not the persistent agent.
; Never silently kill tasks or allow Ignore to produce a mixed-version install.
!include x64.nsh
!define FUTUREOS_PREFLIGHT_SCRIPT "${__FILEDIR__}\installer-preflight.ps1"
; Tauri includes hooks BEFORE MUI_LANGUAGE defines LANG_*; use Windows LCIDs.

LangString FutureOSInstallRunning 1033 "FutureOS is still running in:$\r$\n$INSTDIR$\r$\n$\r$\nThe background service (future.exe) can keep running after the window closes. Setup must close this installation's desktop and background service before continuing.$\r$\n$\r$\nSave your work and wait for tasks to finish. Closing the service will interrupt its tasks and sessions.$\r$\n$\r$\nYes: close the old programs and continue.$\r$\nNo: check again after you close them yourself.$\r$\nCancel: stop setup without replacing files."
LangString FutureOSInstallRunning 2052 "以下目录中的 FutureOS 仍在运行：$\r$\n$INSTDIR$\r$\n$\r$\n关闭窗口后，后台服务（future.exe）可能仍在运行。安装前需要关闭此目录中的桌面程序和后台服务。$\r$\n$\r$\n请先保存工作，等待任务完成。关闭后台服务会中断它正在执行的任务和会话。$\r$\n$\r$\n是：关闭旧程序并继续安装。$\r$\n否：我自行关闭后，再检查一次。$\r$\n取消：停止安装，不替换程序文件。"
LangString FutureOSInstallLocked 1033 "FutureOS files are still in use:$\r$\n$INSTDIR$\r$\n$\r$\nExit FutureOS from the system tray and close its terminals. If necessary, open Task Manager > Details and end future.exe / futureos.exe (legacy: future-desktop.exe) belonging to this directory (this interrupts their tasks). Then click Retry.$\r$\n$\r$\nIf the problem persists, cancel setup, restart Windows, and run setup before opening FutureOS. Cancel stops setup without replacing files."
LangString FutureOSInstallLocked 2052 "以下目录中的程序文件仍被占用：$\r$\n$INSTDIR$\r$\n$\r$\n请退出托盘中的 FutureOS 并关闭相关终端。必要时，在任务管理器 → 详细信息中结束属于此目录的 future.exe / futureos.exe（旧版为 future-desktop.exe）（会中断其任务），然后点击“重试”。$\r$\n$\r$\n如果仍然失败，请取消安装，重启 Windows 后不要打开 FutureOS，直接运行安装包。取消不会替换程序文件。"
LangString FutureOSInstallNotWritable 1033 "Setup could not verify write access to:$\r$\n$INSTDIR$\r$\n$\r$\nNo program files have been replaced. Check that the folder and its files are writable, that enough disk space is available, and that security software is not blocking setup or Windows PowerShell.$\r$\n$\r$\nClick Retry after fixing the problem, or Cancel and choose a folder under your own user account (the default location is recommended). See setup details for diagnostics."
LangString FutureOSInstallNotWritable 2052 "安装程序无法确认可以写入以下目录：$\r$\n$INSTDIR$\r$\n$\r$\n尚未替换程序文件。请检查目录和文件是否允许写入、磁盘空间是否充足，以及安全软件是否拦截了安装程序或 Windows PowerShell。$\r$\n$\r$\n处理后点击“重试”；或取消安装，重新选择当前用户有写入权限的目录（推荐默认位置）。具体原因可查看安装详情。"

; Generate installer and uninstaller functions: upgrades may uninstall first.
!macro FutureOSPreflightFunctions PREFIX
Function ${PREFIX}FutureOSRunPreflight
  ; $R0 = Check or Close, $R1 = exit code. Preserve Tauri's other registers.
  Push $0
  Push $1
  StrCpy $0 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  ${If} ${RunningX64}
    ; NSIS is 32-bit; native PowerShell can inspect 64-bit process paths.
    StrCpy $0 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  ${EndIf}
  nsExec::ExecToStack /TIMEOUT=60000 '"$0" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\futureos-preflight.ps1" -InstallDir "$INSTDIR" -Mode $R0'
  Pop $R1
  Pop $1
  StrCmp $1 "" +2
  DetailPrint "$1"
  Pop $1
  Pop $0
FunctionEnd

Function ${PREFIX}FutureOSEnsureWritable
  Push $R0
  Push $R1
  InitPluginsDir
  File /oname=$PLUGINSDIR\futureos-preflight.ps1 "${FUTUREOS_PREFLIGHT_SCRIPT}"
futureos_preflight_check:
  StrCpy $R0 Check
  Call ${PREFIX}FutureOSRunPreflight
  StrCmp $R1 0 futureos_preflight_done
  StrCmp $R1 32 futureos_preflight_running futureos_preflight_failed

futureos_preflight_running:
  DetailPrint "$(FutureOSInstallLocked)"
  IfSilent futureos_preflight_abort_busy
  StrCmp $PassiveMode 1 futureos_preflight_abort_busy
  MessageBox MB_YESNOCANCEL|MB_ICONEXCLAMATION|MB_DEFBUTTON3 "$(FutureOSInstallRunning)" IDYES futureos_preflight_close IDNO futureos_preflight_check
  Goto futureos_preflight_abort_busy

futureos_preflight_close:
  StrCpy $R0 Close
  Call ${PREFIX}FutureOSRunPreflight
  StrCmp $R1 0 futureos_preflight_done
  StrCmp $R1 32 0 futureos_preflight_failed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSInstallLocked)" IDRETRY futureos_preflight_check
futureos_preflight_abort_busy:
  SetErrorLevel 32
  Quit

futureos_preflight_failed:
  DetailPrint "Preflight exit code: $R1"
  DetailPrint "$(FutureOSInstallNotWritable)"
  IfSilent futureos_preflight_abort_access
  StrCmp $PassiveMode 1 futureos_preflight_abort_access
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSInstallNotWritable)" IDRETRY futureos_preflight_check
futureos_preflight_abort_access:
  SetErrorLevel 5
  Quit

futureos_preflight_done:
  Pop $R1
  Pop $R0
FunctionEnd
!macroend
!insertmacro FutureOSPreflightFunctions ""
!insertmacro FutureOSPreflightFunctions "un."

!macro NSIS_HOOK_PREINSTALL
  Call FutureOSEnsureWritable
!macroend

; Sandbox cleanup remains unelevated and only revokes FutureOS-owned ACEs.
; An active sandbox Job still aborts uninstall rather than losing its cleanup exe.

LangString FutureOSSandboxCleanupFailed 1033 "FutureOS could not remove its write-protection permissions. Close FutureOS and any running tasks, then retry. Cancel keeps the app installed so you can try again later."
LangString FutureOSSandboxCleanupFailed 2052 "FutureOS 无法清理写保护权限。请关闭 FutureOS 和正在运行的任务后重试。取消会保留应用，您可以稍后再次卸载。"

!macro NSIS_HOOK_PREUNINSTALL
  Call un.FutureOSEnsureWritable
  IfFileExists "$INSTDIR\future.exe" 0 futureos_sandbox_cleanup_done

futureos_sandbox_cleanup_retry:
  StrCpy $0 -1
  ClearErrors
  ExecWait '"$INSTDIR\future.exe" agent --reset-windows-sandbox' $0
  IfErrors futureos_sandbox_cleanup_failed
  IntCmp $0 0 futureos_sandbox_cleanup_done futureos_sandbox_cleanup_failed futureos_sandbox_cleanup_failed

futureos_sandbox_cleanup_failed:
  DetailPrint "$(FutureOSSandboxCleanupFailed)"
  IfSilent futureos_sandbox_cleanup_abort
  StrCmp $PassiveMode 1 futureos_sandbox_cleanup_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSSandboxCleanupFailed)" IDRETRY futureos_sandbox_cleanup_retry

futureos_sandbox_cleanup_abort:
  SetErrorLevel 1
  Quit

futureos_sandbox_cleanup_done:
!macroend
