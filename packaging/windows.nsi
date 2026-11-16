Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!ifndef DUOCLE_EXE
!define DUOCLE_EXE "..\target\x86_64-pc-windows-msvc\release\duocle.exe"
!endif
!ifndef OUTPUT_EXE
!define OUTPUT_EXE "..\dist\Duocle-windows-x64-setup.exe"
!endif

!define WEBVIEW2_CLIENT "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

Name "Duocle"
OutFile "${OUTPUT_EXE}"
InstallDir "$LOCALAPPDATA\Programs\Duocle"
InstallDirRegKey HKCU "Software\Duocle" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define MUI_ICON "..\assets\duocle.ico"
!define MUI_UNICON "..\assets\duocle.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\Duocle.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "Duocle requires a 64-bit version of Windows."
    Abort
  ${EndIf}
  ReadRegStr $0 HKCU "${WEBVIEW2_CLIENT}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKLM "${WEBVIEW2_CLIENT}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "Duocle needs the Microsoft Edge WebView2 Evergreen Runtime. Select OK to open Microsoft's download page, then rerun this installer." IDCANCEL webview_missing
    ExecShell "open" "https://developer.microsoft.com/microsoft-edge/webview2/#download-section"
    webview_missing:
    Abort
  ${EndIf}
FunctionEnd

Section "Duocle"
  SetShellVarContext current
  IfFileExists "$INSTDIR\Duocle.exe" 0 +2
  ExecWait '"$INSTDIR\Duocle.exe" quit'
  Sleep 250
  SetOutPath "$INSTDIR"
  File /oname=Duocle.exe "${DUOCLE_EXE}"
!ifdef WEBVIEW2_LOADER
  File /oname=WebView2Loader.dll "${WEBVIEW2_LOADER}"
!endif
  SetOutPath "$INSTDIR\Licenses"
  File "..\ui\fonts\LICENSE-Material-Symbols.txt"
  File "..\ui\fonts\OFL-Roboto.txt"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Duocle.lnk" "$INSTDIR\Duocle.exe"
  WriteRegStr HKCU "Software\Duocle" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Classes\duocle" "" "URL:Duocle"
  WriteRegStr HKCU "Software\Classes\duocle" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\duocle\shell\open\command" "" '"$INSTDIR\Duocle.exe" "%1"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "DisplayName" "Duocle"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "DisplayVersion" "0.1.0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "DisplayIcon" "$INSTDIR\Duocle.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\Duocle.exe" 0 +2
  ExecWait '"$INSTDIR\Duocle.exe" quit'
  Sleep 250
  Delete "$SMPROGRAMS\Duocle.lnk"
  Delete "$INSTDIR\Duocle.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\Licenses\LICENSE-Material-Symbols.txt"
  Delete "$INSTDIR\Licenses\OFL-Roboto.txt"
  RMDir "$INSTDIR\Licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Duocle"
  DeleteRegKey HKCU "Software\Classes\duocle"
  DeleteRegKey HKCU "Software\Duocle"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle"
SectionEnd
