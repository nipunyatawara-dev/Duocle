Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "nsDialogs.nsh"
!include "x64.nsh"

!ifndef DUOCLE_EXE
!define DUOCLE_EXE "..\target\x86_64-pc-windows-msvc\release\duocle.exe"
!endif
!ifndef OUTPUT_EXE
!define OUTPUT_EXE "..\dist\Duocle-windows-x64-setup.exe"
!endif

!define WEBVIEW2_CLIENT "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

Name "Duocle"
Caption "Duocle 1.0 (3) Setup"
VIProductVersion "1.0.0.3"
VIAddVersionKey /LANG=1033 "ProductName" "Duocle"
VIAddVersionKey /LANG=1033 "ProductVersion" "1.0 (3)"
VIAddVersionKey /LANG=1033 "FileDescription" "Duocle installer"
VIAddVersionKey /LANG=1033 "FileVersion" "1.0.0.3"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright (c) 2026 Nipun Yatawara"
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
Page custom FinishPage FinishPageLeave
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Var HadInstall
Var DesktopShortcut
Var StartMenuShortcut
Var OpenDuocle

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

Function FinishPage
  !insertmacro MUI_HEADER_TEXT "Duocle is ready" "Choose your shortcuts."
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}

  ${NSD_CreateCheckbox} 12u 18u 90% 12u "Add a Desktop shortcut"
  Pop $DesktopShortcut
  ${If} ${FileExists} "$DESKTOP\Duocle.lnk"
    ${NSD_Check} $DesktopShortcut
  ${EndIf}

  ${NSD_CreateCheckbox} 12u 39u 90% 12u "Add a Start menu shortcut"
  Pop $StartMenuShortcut
  ${If} $HadInstall == 0
  ${OrIf} ${FileExists} "$SMPROGRAMS\Duocle.lnk"
    ${NSD_Check} $StartMenuShortcut
  ${EndIf}

  ${NSD_CreateCheckbox} 12u 60u 90% 12u "Open Duocle"
  Pop $OpenDuocle
  ${NSD_Check} $OpenDuocle

  GetDlgItem $0 $HWNDPARENT 1
  ${NSD_SetText} $0 "Finish"
  GetDlgItem $0 $HWNDPARENT 2
  EnableWindow $0 0
  nsDialogs::Show
FunctionEnd

Function FinishPageLeave
  ${NSD_GetState} $DesktopShortcut $0
  ${If} $0 == ${BST_CHECKED}
    CreateShortcut "$DESKTOP\Duocle.lnk" "$INSTDIR\Duocle.exe"
  ${Else}
    Delete "$DESKTOP\Duocle.lnk"
  ${EndIf}

  ${NSD_GetState} $StartMenuShortcut $0
  ${If} $0 == ${BST_CHECKED}
    CreateShortcut "$SMPROGRAMS\Duocle.lnk" "$INSTDIR\Duocle.exe"
  ${Else}
    Delete "$SMPROGRAMS\Duocle.lnk"
  ${EndIf}

  ${NSD_GetState} $OpenDuocle $0
  ${If} $0 == ${BST_CHECKED}
    Exec '"$INSTDIR\Duocle.exe"'
  ${EndIf}
FunctionEnd

Section "Duocle"
  SetShellVarContext current
  StrCpy $HadInstall 0
  ${If} ${FileExists} "$INSTDIR\Duocle.exe"
    StrCpy $HadInstall 1
    ExecWait '"$INSTDIR\Duocle.exe" quit'
    Sleep 250
  ${EndIf}
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
  ${If} ${Silent}
  ${AndIf} $HadInstall == 0
    CreateShortcut "$SMPROGRAMS\Duocle.lnk" "$INSTDIR\Duocle.exe"
  ${EndIf}
  WriteRegStr HKCU "Software\Duocle" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Classes\duocle" "" "URL:Duocle"
  WriteRegStr HKCU "Software\Classes\duocle" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\duocle\shell\open\command" "" '"$INSTDIR\Duocle.exe" "%1"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "DisplayName" "Duocle"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Duocle" "DisplayVersion" "1.0 (3)"
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
  Delete "$DESKTOP\Duocle.lnk"
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
