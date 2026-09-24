; Preflight every current/legacy executable before Tauri copies anything. Its
; built-in CheckIfAppIsRunning covers only the current desktop, not the bundled
; CLI/Agent or names left by older releases. Never let a locked old sidecar
; survive beside a newly copied desktop and create a mixed-version install.
!include x64.nsh
Var FutureOSRestorePending
!define FUTUREOS_PREFLIGHT_SCRIPT "${__FILEDIR__}\installer-preflight.ps1"
; Tauri includes hooks BEFORE MUI_LANGUAGE defines LANG_*; use Windows LCIDs.

LangString FutureOSInstallRunning 1033 "FutureOS is still running in:$\r$\n$INSTDIR$\r$\n$\r$\nThe background service (future.exe, or legacy future-agent.exe) can keep running after the window closes. Setup must close this installation's desktop and any verified Agent using the same FutureOS data before continuing.$\r$\n$\r$\nSave your work and wait for tasks to finish. Closing an Agent, including one from another installation directory, will interrupt its tasks and sessions.$\r$\n$\r$\nYes: close the old programs and continue.$\r$\nNo: check again after you close them yourself.$\r$\nCancel: stop setup without replacing files."
LangString FutureOSInstallRunning 2052 "以下目录中的 FutureOS 仍在运行：$\r$\n$INSTDIR$\r$\n$\r$\n关闭窗口后，后台服务（future.exe，旧版为 future-agent.exe）可能仍在运行。安装前需要关闭此目录中的桌面程序，以及使用同一 FutureOS 数据目录且身份已核实的 Agent。$\r$\n$\r$\n请先保存工作，等待任务完成。关闭 Agent（包括其他安装目录中的 Agent）会中断它正在执行的任务和会话。$\r$\n$\r$\n是：关闭旧程序并继续安装。$\r$\n否：我自行关闭后，再检查一次。$\r$\n取消：停止安装，不替换程序文件。"
LangString FutureOSInstallLocked 1033 "FutureOS files are still in use:$\r$\n$INSTDIR$\r$\n$\r$\nExit FutureOS from the system tray and close its terminals. If necessary, open Task Manager > Details and end future.exe / futureos.exe (legacy: future-agent.exe / future-desktop.exe) belonging to this directory (this interrupts their tasks). Then click Retry.$\r$\n$\r$\nIf the problem persists, cancel setup, restart Windows, and run setup before opening FutureOS. Cancel stops setup without replacing files."
LangString FutureOSInstallLocked 2052 "以下目录中的程序文件仍被占用：$\r$\n$INSTDIR$\r$\n$\r$\n请退出托盘中的 FutureOS 并关闭相关终端。必要时，在任务管理器 → 详细信息中结束属于此目录的 future.exe / futureos.exe（旧版为 future-agent.exe / future-desktop.exe）（会中断其任务），然后点击“重试”。$\r$\n$\r$\n如果仍然失败，请取消安装，重启 Windows 后不要打开 FutureOS，直接运行安装包。取消不会替换程序文件。"
LangString FutureOSInstallNotWritable 1033 "Setup could not verify write access to:$\r$\n$INSTDIR$\r$\n$\r$\nNo program files have been replaced. Check that the folder and its files are writable, that enough disk space is available, and that security software is not blocking setup or Windows PowerShell.$\r$\n$\r$\nClick Retry after fixing the problem, or Cancel and choose a folder under your own user account (the default location is recommended). See setup details for diagnostics."
LangString FutureOSInstallNotWritable 2052 "安装程序无法确认可以写入以下目录：$\r$\n$INSTDIR$\r$\n$\r$\n尚未替换程序文件。请检查目录和文件是否允许写入、磁盘空间是否充足，以及安全软件是否拦截了安装程序或 Windows PowerShell。$\r$\n$\r$\n处理后点击“重试”；或取消安装，重新选择当前用户有写入权限的目录（推荐默认位置）。具体原因可查看安装详情。"

!macro FutureOSBackupExecutable NAME
  IfFileExists "$INSTDIR\${NAME}" 0 +4
  ClearErrors
  CopyFiles /SILENT "$INSTDIR\${NAME}" "$PLUGINSDIR\${NAME}.futureos-backup"
  IfErrors futureos_preinstall_backup_failed
!macroend

!macro FutureOSRestoreExecutable NAME
  Delete "$INSTDIR\${NAME}"
  IfFileExists "$PLUGINSDIR\${NAME}.futureos-backup" 0 +3
  CopyFiles /SILENT "$PLUGINSDIR\${NAME}.futureos-backup" "$INSTDIR\${NAME}"
  DetailPrint "Restored ${NAME} after the lifecycle operation failed."
!macroend

!macro FutureOSDeleteExecutableBackup NAME
  Delete "$PLUGINSDIR\${NAME}.futureos-backup"
!macroend

!macro FutureOSUninstallBackupExecutable NAME
  IfFileExists "$INSTDIR\${NAME}" 0 +4
  ClearErrors
  CopyFiles /SILENT "$INSTDIR\${NAME}" "$PLUGINSDIR\${NAME}.futureos-backup"
  IfErrors futureos_uninstall_backup_failed
!macroend

; Generate installer and uninstaller functions: upgrades may uninstall first.
; The RunPreflight helper is shared, while EnsureWritable/CloseForUpdate are
; installer-only. Uninstall fails closed when executable removal is unsafe.
!macro FutureOSRunPreflightFunction PREFIX
Function ${PREFIX}FutureOSRunPreflight
  ; $R0 = helper mode, $R1 = exit code. Preserve Tauri's other registers.
  Push $0
  Push $1
  Push $R2
  System::Call 'kernel32::GetCurrentProcessId() i .r2'
  StrCpy $0 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  ${If} ${RunningX64}
    ; NSIS is 32-bit; native PowerShell can inspect 64-bit process paths.
    StrCpy $0 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  ${EndIf}
  nsExec::ExecToStack /TIMEOUT=60000 '"$0" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\futureos-preflight.ps1" -InstallDir "$INSTDIR" -Mode $R0 -LeaseDir "$PLUGINSDIR" -InstallerPid $R2'
  Pop $R1
  Pop $1
  StrCmp $1 "" +2
  DetailPrint "$1"
  Pop $R2
  Pop $1
  Pop $0
FunctionEnd
!macroend

!macro FutureOSInstallPreflightFunctions PREFIX
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

Function ${PREFIX}FutureOSCloseForUpdate
  ; /UPDATE is launched by the in-app updater after the user accepts the
  ; upgrade. Older desktops did not stop their Agent before starting NSIS, so
  ; the new installer must be able to repair those already-mixed installs.
  Push $R0
  Push $R1
  InitPluginsDir
  File /oname=$PLUGINSDIR\futureos-preflight.ps1 "${FUTUREOS_PREFLIGHT_SCRIPT}"
  StrCpy $R0 Close
  Call ${PREFIX}FutureOSRunPreflight
  StrCmp $R1 0 futureos_update_preflight_done
  DetailPrint "Automatic update preflight exit code: $R1"
  DetailPrint "$(FutureOSInstallNotWritable)"
  StrCmp $R1 32 0 futureos_update_preflight_access
  SetErrorLevel 32
  Quit
futureos_update_preflight_access:
  SetErrorLevel 5
  Quit
futureos_update_preflight_done:
  Pop $R1
  Pop $R0
FunctionEnd
!macroend
!insertmacro FutureOSRunPreflightFunction ""
!insertmacro FutureOSRunPreflightFunction "un."
!insertmacro FutureOSInstallPreflightFunctions ""

!macro NSIS_HOOK_PREINSTALL
  ; Explicit updater mode owns the current installation and closes only
  ; exact-path FutureOS processes. This bootstraps recovery for old desktop
  ; versions which launched NSIS while leaving their old Agent alive. Generic
  ; silent/passive installs remain fail-closed and never kill tasks implicitly.
  StrCmp $UpdateMode 1 futureos_preinstall_update futureos_preinstall_manual
futureos_preinstall_update:
  Call FutureOSCloseForUpdate
  Goto futureos_preinstall_checked
futureos_preinstall_manual:
  Call FutureOSEnsureWritable
futureos_preinstall_checked:
  ; A short preflight check leaves a race before extraction. A helper keeps
  ; the same Agent byte-range lock until installation and verification finish.
  StrCpy $R0 AcquireLease
  Call FutureOSRunPreflight
  StrCmp $R1 0 futureos_preinstall_lease_acquired
  DetailPrint "Could not reserve the FutureOS Agent lock (exit $R1)."
  SetErrorLevel $R1
  Abort
futureos_preinstall_lease_acquired:
  ; Preserve every executable that this installer replaces or retires. NSIS
  ; writes files in place; without these copies, a disk/AV/extraction failure
  ; after the first write can leave neither a complete old nor new version.
  InitPluginsDir
  StrCpy $FutureOSRestorePending 0
  !insertmacro FutureOSBackupExecutable "futureos.exe"
  !insertmacro FutureOSBackupExecutable "future.exe"
  !insertmacro FutureOSBackupExecutable "future-desktop.exe"
  !insertmacro FutureOSBackupExecutable "future-agent.exe"
  StrCpy $FutureOSRestorePending 1

  ; Make sidecar replacement unconditional, including same-version repairs.
  ClearErrors
  Delete "$INSTDIR\future.exe"
  Delete "$INSTDIR\future-agent.exe"
  IfErrors 0 futureos_preinstall_sidecars_removed
futureos_preinstall_backup_failed:
  DetailPrint "$(FutureOSInstallNotWritable)"
  Call FutureOSRestoreAfterFailure
  SetErrorLevel 5
  Abort
futureos_preinstall_sidecars_removed:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Run bounded --help/--version probes for both new executables. These return
  ; before GUI/Agent initialization but still detect missing DLLs, corrupt files
  ; and wrong architectures. Failure invokes .onInstFailed below.
  Push $R0
  Push $R1
  StrCpy $R0 VerifyInstall
  Call FutureOSRunPreflight
  StrCmp $R1 0 futureos_postinstall_valid
  DetailPrint "FutureOS installation verification failed (exit $R1)."
  Pop $R1
  Pop $R0
  Call FutureOSRestoreAfterFailure
  SetErrorLevel 5
  Abort
futureos_postinstall_valid:
  StrCpy $R0 ReleaseLease
  Call FutureOSRunPreflight
  StrCmp $R1 0 futureos_postinstall_lease_released
  DetailPrint "Could not release the FutureOS Agent lock (exit $R1)."
  SetErrorLevel 5
  Abort
futureos_postinstall_lease_released:
  Pop $R1
  Pop $R0
  !insertmacro FutureOSDeleteExecutableBackup "futureos.exe"
  !insertmacro FutureOSDeleteExecutableBackup "future.exe"
  !insertmacro FutureOSDeleteExecutableBackup "future-desktop.exe"
  !insertmacro FutureOSDeleteExecutableBackup "future-agent.exe"
  StrCpy $FutureOSRestorePending 0
!macroend

Function FutureOSRestoreAfterFailure
  StrCmp $FutureOSRestorePending 1 0 futureos_restore_done
  !insertmacro FutureOSRestoreExecutable "futureos.exe"
  !insertmacro FutureOSRestoreExecutable "future.exe"
  !insertmacro FutureOSRestoreExecutable "future-desktop.exe"
  !insertmacro FutureOSRestoreExecutable "future-agent.exe"
  StrCpy $FutureOSRestorePending 0
futureos_restore_done:
FunctionEnd

; NSIS extraction failures reach this callback while $PLUGINSDIR still exists.
; Our own fail-closed paths call the same restoration function immediately.
Function .onInstFailed
  Call FutureOSRestoreAfterFailure
  StrCpy $R0 ReleaseLease
  Call FutureOSRunPreflight
FunctionEnd

; Sandbox cleanup remains unelevated and only revokes FutureOS-owned ACEs.
; Cleanup is best-effort during uninstall: a missing/old/broken CLI must never
; make FutureOS impossible to remove.

LangString FutureOSSandboxCleanupFailed 1033 "Warning: FutureOS could not remove all of its Windows sandbox permissions. Uninstall will continue. Cleanup metadata is retained so a later FutureOS install or repair can retry."
LangString FutureOSSandboxCleanupFailed 2052 "警告：FutureOS 无法清理全部 Windows 沙箱权限。卸载将继续；清理元数据会保留，以便以后安装或修复 FutureOS 时重试。"

!macro NSIS_HOOK_PREUNINSTALL
  ; Starting uninstall is authority to stop this installation. Close exact-path
  ; processes, then require write access so removal never reports success while
  ; executable files remain installed.
  InitPluginsDir
  File /oname=$PLUGINSDIR\futureos-preflight.ps1 "${FUTUREOS_PREFLIGHT_SCRIPT}"
futureos_uninstall_retry:
  StrCpy $R0 CloseFiles
  Call un.FutureOSRunPreflight
  StrCmp $R1 0 futureos_uninstall_cleanup
  DetailPrint "FutureOS could not close or unlock every installed executable (exit $R1)."
  IfSilent futureos_uninstall_abort
  StrCmp $PassiveMode 1 futureos_uninstall_abort
  StrCmp $R1 32 0 futureos_uninstall_access
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSInstallLocked)" IDRETRY futureos_uninstall_retry
  Goto futureos_uninstall_abort
futureos_uninstall_access:
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSInstallNotWritable)" IDRETRY futureos_uninstall_retry
futureos_uninstall_abort:
  SetErrorLevel $R1
  Quit

futureos_uninstall_cleanup:
  IfFileExists "$INSTDIR\future.exe" 0 futureos_sandbox_cleanup_done

  ; A broken mixed-version CLI can also hang rather than returning an error.
  ; The PowerShell helper gives the maintenance child a hard total deadline
  ; (rather than nsExec's output-inactivity timeout) and terminates it on expiry.
  StrCpy $R0 ResetSandbox
  Call un.FutureOSRunPreflight
  StrCmp $R1 0 futureos_sandbox_cleanup_done futureos_sandbox_cleanup_failed

futureos_sandbox_cleanup_failed:
  DetailPrint "$(FutureOSSandboxCleanupFailed)"
  ; Deliberately continue. A mixed install may contain a pre-sandbox future.exe
  ; that exits 2 for the unknown maintenance flag. Capability metadata lives
  ; outside $INSTDIR and remains available for a later repair attempt.

futureos_sandbox_cleanup_done:
  ; Delete and verify every current/retired executable before Tauri removes the
  ; uninstall registry entry. /REBOOTOK is not a valid current-user guarantee:
  ; Windows only permits scheduling those reboot deletions to administrators.
  ; Keep temporary copies so a lock race between preflight and deletion cannot
  ; turn a reported failure into a partially removed installation.
  !insertmacro FutureOSUninstallBackupExecutable "futureos.exe"
  !insertmacro FutureOSUninstallBackupExecutable "future.exe"
  !insertmacro FutureOSUninstallBackupExecutable "future-agent.exe"
  !insertmacro FutureOSUninstallBackupExecutable "future-desktop.exe"
  ClearErrors
  Delete "$INSTDIR\futureos.exe"
  Delete "$INSTDIR\future.exe"
  Delete "$INSTDIR\future-agent.exe"
  Delete "$INSTDIR\future-desktop.exe"
  IfErrors futureos_uninstall_restore
  !insertmacro FutureOSDeleteExecutableBackup "futureos.exe"
  !insertmacro FutureOSDeleteExecutableBackup "future.exe"
  !insertmacro FutureOSDeleteExecutableBackup "future-agent.exe"
  !insertmacro FutureOSDeleteExecutableBackup "future-desktop.exe"
  Goto futureos_uninstall_executables_removed
futureos_uninstall_restore:
  !insertmacro FutureOSRestoreExecutable "futureos.exe"
  !insertmacro FutureOSRestoreExecutable "future.exe"
  !insertmacro FutureOSRestoreExecutable "future-agent.exe"
  !insertmacro FutureOSRestoreExecutable "future-desktop.exe"
  DetailPrint "$(FutureOSInstallLocked)"
  IfSilent futureos_uninstall_delete_abort
  StrCmp $PassiveMode 1 futureos_uninstall_delete_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSInstallLocked)" IDRETRY futureos_uninstall_retry
futureos_uninstall_delete_abort:
  SetErrorLevel 32
  Quit
futureos_uninstall_backup_failed:
  DetailPrint "$(FutureOSInstallNotWritable)"
  SetErrorLevel 5
  Quit
futureos_uninstall_executables_removed:
!macroend
