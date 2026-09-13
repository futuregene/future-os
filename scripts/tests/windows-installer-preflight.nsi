; Isolated integration harness: no registry, shortcuts, real app or user data.
Unicode true
!include MUI2.nsh
!include FileFunc.nsh
Var PassiveMode
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
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  IfErrors +2
  StrCpy $PassiveMode 1
  ${GetOptions} $CMDLINE "/ZH" $0
  IfErrors +2
  StrCpy $LANGUAGE ${LANG_SIMPCHINESE}
FunctionEnd
Section
  !insertmacro NSIS_HOOK_PREINSTALL
  SetOutPath $INSTDIR
  ; A real File instruction exercises replacement, not just a marker assertion.
  File /oname=future.exe "..\install.ps1"
  FileOpen $0 "$INSTDIR\installed.marker" w
  FileWrite $0 "installed"
  FileClose $0
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd
Function un.onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  IfErrors +2
  StrCpy $PassiveMode 1
FunctionEnd
Section Uninstall
  !insertmacro NSIS_HOOK_PREUNINSTALL
  FileOpen $0 "$INSTDIR\uninstalled.marker" w
  FileWrite $0 "uninstalled"
  FileClose $0
SectionEnd
