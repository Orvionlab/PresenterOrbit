; Presenter Orbit installer. Compiled by scripts/package-gpui.mts with these defines:
;   VERSION (semver text), VERSIONNUM (a.b.c.d), ARCH (x64|arm64), OUTFILE, ICON, LICENSEFILE,
;   FILES_INSTALL and FILES_UNINSTALL (generated .nsh lists of the staged app files).
;
; It replaces the Tauri NSIS installer in place: same product name, publisher, registry keys, install directory,
; per-machine scope, shortcuts, and the /P /UPDATE /R command line of the Tauri updater.
Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include LogicLib.nsh

!define MANUFACTURER "Orvion Lab"
!define PRODUCTNAME "Presenter Orbit"
!define MAINBINARY "Presenter Orbit.exe"
!define DEFAULTDIR "Orvionlab\PresenterOrbit"
!define BUNDLEID "com.presenterorbit.desktop"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUPRODUCTKEY "Software\${MANUFACTURER}\${PRODUCTNAME}"
!define WEBVIEW2GUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

Name "${PRODUCTNAME}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${DEFAULTDIR}"
RequestExecutionLevel admin
BrandingText "${MANUFACTURER}"

VIProductVersion "${VERSIONNUM}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 ${MANUFACTURER}"
VIAddVersionKey "CompanyName" "${MANUFACTURER}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

Var PassiveMode
Var UpdateMode
Var NoShortcutMode

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_LICENSE "${LICENSEFILE}"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_NOAUTOCLOSE
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateDesktopShortcut
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunApplication
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

!define MUI_PAGE_CUSTOMFUNCTION_PRE un.SkipIfPassive
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Dutch"
!insertmacro MUI_RESERVEFILE_LANGDLL

LangString createDesktop ${LANG_ENGLISH} "Create a desktop shortcut"
LangString createDesktop ${LANG_DUTCH} "Een snelkoppeling op het bureaublad maken"
LangString appRunning ${LANG_ENGLISH} "Presenter Orbit is running. Click OK to close it and continue."
LangString appRunning ${LANG_DUTCH} "Presenter Orbit is geopend. Klik op OK om het te sluiten en door te gaan."
LangString deleteData ${LANG_ENGLISH} "Also delete the application settings and data of the current user? Workspaces outside the application data folder are kept."
LangString deleteData ${LANG_DUTCH} "Ook de instellingen en gegevens van de applicatie van de huidige gebruiker verwijderen? Werkruimtes buiten de gegevensmap blijven behouden."
LangString webview2Failed ${LANG_ENGLISH} "The Microsoft Edge WebView2 runtime could not be installed. Web backgrounds need it; the rest of the application works without."
LangString webview2Failed ${LANG_DUTCH} "De Microsoft Edge WebView2-runtime kon niet worden geinstalleerd. Webachtergronden hebben die nodig; de rest van de applicatie werkt zonder."

Function .onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/NS" $NoShortcutMode
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  Call SetContext
  ; An existing installation keeps its folder, whichever version installed it.
  ReadRegStr $0 SHCTX "${MANUPRODUCTKEY}" ""
  ${If} $0 != ""
    StrCpy $INSTDIR $0
  ${EndIf}
  ${If} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    !insertmacro MUI_LANGDLL_DISPLAY
  ${EndIf}
FunctionEnd

Function SetContext
  SetShellVarContext all
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
FunctionEnd

Function SkipIfPassive
  ${If} $PassiveMode = 1
    Abort
  ${EndIf}
FunctionEnd

Function un.SkipIfPassive
  ${If} $PassiveMode = 1
    Abort
  ${EndIf}
FunctionEnd

; Closes a running instance. The output process runs from the same executable, so it is checked by name.
Function CloseRunningApplication
  StrCpy $1 0
  check:
    nsExec::Exec 'cmd /c tasklist /FI "IMAGENAME eq ${MAINBINARY}" /NH | find /I "${MAINBINARY}"'
    Pop $0
    ${If} $0 <> 0
      Return
    ${EndIf}
    ${If} $1 = 0
    ${AndIf} $PassiveMode <> 1
    ${AndIfNot} ${Silent}
      MessageBox MB_OKCANCEL|MB_ICONEXCLAMATION "$(appRunning)" IDOK +2
      Abort
    ${EndIf}
    ; An updating application is already closing itself: give it a few seconds before forcing it.
    ${If} $1 >= 20
      nsExec::Exec 'taskkill /F /T /IM "${MAINBINARY}"'
      Pop $0
      Sleep 500
      Return
    ${EndIf}
    IntOp $1 $1 + 1
    Sleep 500
    Goto check
FunctionEnd

Section "WebView2"
  ${If} $UpdateMode <> 1
    ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2GUID}" "pv"
    ${If} $4 == ""
      ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2GUID}" "pv"
    ${EndIf}
    ${If} $4 == ""
      Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
      DetailPrint "Downloading the Microsoft Edge WebView2 runtime"
      NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\MicrosoftEdgeWebview2Setup.exe"
      Pop $0
      ${If} $0 == "success"
        ExecWait '"$TEMP\MicrosoftEdgeWebview2Setup.exe" /silent /install' $1
        ${If} $1 <> 0
          DetailPrint "$(webview2Failed)"
        ${EndIf}
      ${Else}
        DetailPrint "$(webview2Failed)"
      ${EndIf}
    ${EndIf}
  ${EndIf}
SectionEnd

Section "Install"
  Call CloseRunningApplication
  SetOutPath $INSTDIR
  !include "${FILES_INSTALL}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR
  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARY}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${MAINBINARY}$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" $0

  Call CreateStartMenuShortcut
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Call CreateDesktopShortcut
  ${EndIf}
  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

; An update keeps the shortcuts the user has (they point at the same executable); a new install creates them.
Function CreateStartMenuShortcut
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    Return
  ${EndIf}
  CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARY}"
FunctionEnd

Function CreateDesktopShortcut
  ${If} $UpdateMode = 1
  ${OrIf} $NoShortcutMode = 1
    Return
  ${EndIf}
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARY}"
FunctionEnd

; The installer is elevated; Explorer starts the application with the user's own rights.
Function RunApplication
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\${MAINBINARY}"'
FunctionEnd

Function .onInstSuccess
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      Call RunApplication
    ${EndIf}
  ${EndIf}
FunctionEnd

Function un.onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  SetShellVarContext all
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
  !insertmacro MUI_UNGETLANGUAGE
FunctionEnd

Section "Uninstall"
  Call un.CloseRunningApplication
  !include "${FILES_UNINSTALL}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Delete "$DESKTOP\${PRODUCTNAME}.lnk"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
  ${EndIf}
  DeleteRegKey SHCTX "${UNINSTKEY}"
  ${If} $UpdateMode <> 1
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "Software\${MANUFACTURER}"
    DeleteRegKey HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "Software\${MANUFACTURER}"
    ; The data folder holds the settings; it goes only when the user says so in an interactive uninstall.
    ${If} $PassiveMode <> 1
    ${AndIfNot} ${Silent}
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(deleteData)" IDNO +4
      SetShellVarContext current
      RMDir /r "$APPDATA\${BUNDLEID}"
      RMDir /r "$LOCALAPPDATA\${BUNDLEID}"
    ${EndIf}
  ${EndIf}
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function un.CloseRunningApplication
  nsExec::Exec 'cmd /c tasklist /FI "IMAGENAME eq ${MAINBINARY}" /NH | find /I "${MAINBINARY}"'
  Pop $0
  ${If} $0 = 0
    nsExec::Exec 'taskkill /F /T /IM "${MAINBINARY}"'
    Pop $0
    Sleep 500
  ${EndIf}
FunctionEnd
