; Isolated integration harness: no registry, shortcuts, real app or user data.
Unicode true
!include MUI2.nsh
!include FileFunc.nsh
Var PassiveMode
Var UpdateMode
Var FailAfterCopy
Var FailHealth
!define MAINBINARYNAME "futureos"
!include "..\..\desktop\src-tauri\windows\installer-hooks.nsh"
Name "FutureOS preflight regression"
OutFile "${TEST_OUTFILE}"
RequestExecutionLevel user
ShowInstDetails show
AutoCloseWindow true
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"
Function .onInit
  StrCpy $LANGUAGE ${LANG_ENGLISH}
  StrCpy $FailAfterCopy 0
  StrCpy $FailHealth 0
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  IfErrors +2
  StrCpy $PassiveMode 1
  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  IfErrors +2
  StrCpy $UpdateMode 1
  ${GetOptions} $CMDLINE "/ZH" $0
  IfErrors +2
  StrCpy $LANGUAGE ${LANG_SIMPCHINESE}
  ${GetOptions} $CMDLINE "/FAILAFTERCOPY" $0
  IfErrors +2
  StrCpy $FailAfterCopy 1
  ${GetOptions} $CMDLINE "/FAILHEALTH" $0
  IfErrors +2
  StrCpy $FailHealth 1
FunctionEnd
Section
  !insertmacro NSIS_HOOK_PREINSTALL
  SetOutPath $INSTDIR
  ; A real File instruction exercises replacement, not just a marker assertion.
  File /oname=future.exe "${TEST_FIXTURE}"
  File /oname=futureos.exe "${TEST_FIXTURE}"
  StrCmp $FailAfterCopy 1 0 +3
  SetErrorLevel 5
  Abort
  StrCmp $FailHealth 1 0 +4
  FileOpen $0 "$INSTDIR\futureos.exe" w
  FileWrite $0 "not an executable"
  FileClose $0
  !insertmacro NSIS_HOOK_POSTINSTALL
  FileOpen $0 "$INSTDIR\installed.marker" w
  FileWrite $0 "installed"
  FileClose $0
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd
Function un.onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  IfErrors +2
  StrCpy $PassiveMode 1
  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  IfErrors +2
  StrCpy $UpdateMode 1
FunctionEnd
Section Uninstall
  !insertmacro NSIS_HOOK_PREUNINSTALL
  FileOpen $0 "$INSTDIR\uninstalled.marker" w
  FileWrite $0 "uninstalled"
  FileClose $0
  ; Mirror Tauri's production template: passive/update uninstallers close the
  ; progress page automatically after the section finishes.
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd
