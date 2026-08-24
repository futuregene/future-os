; FutureOS Windows sandbox uninstall cleanup.
;
; This runs before NSIS removes the bundled `future.exe` sidecar. The cleanup
; command is intentionally unelevated and only revokes FutureOS-owned
; capability ACEs recorded in the current user's state file. If a sandbox Job
; is still active, the command fails rather than terminating it; uninstall then
; stops so the cleanup binary remains available for a safe retry.

LangString FutureOSSandboxCleanupFailed ${LANG_ENGLISH} "FutureOS could not remove its write-protection permissions. Close FutureOS and any running tasks, then retry. Cancel keeps the app installed so you can try again later."
LangString FutureOSSandboxCleanupFailed ${LANG_SIMPCHINESE} "FutureOS 无法清理写保护权限。请关闭 FutureOS 和正在运行的任务后重试。取消会保留应用，您可以稍后再次卸载。"

!macro NSIS_HOOK_PREUNINSTALL
  IfFileExists "$INSTDIR\future.exe" 0 futureos_sandbox_cleanup_done

futureos_sandbox_cleanup_retry:
  ClearErrors
  ExecWait '"$INSTDIR\future.exe" agent --reset-windows-sandbox' $0
  IntCmp $0 0 futureos_sandbox_cleanup_done futureos_sandbox_cleanup_failed futureos_sandbox_cleanup_failed

futureos_sandbox_cleanup_failed:
  IfSilent futureos_sandbox_cleanup_abort
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(FutureOSSandboxCleanupFailed)" IDRETRY futureos_sandbox_cleanup_retry

futureos_sandbox_cleanup_abort:
  SetErrorLevel 1
  Abort

futureos_sandbox_cleanup_done:
!macroend
