; Dioxus 0.7.9 renders this template; build-release.ps1 supplies the payload definitions.
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!include "$%MIKO_INSTALLER_DEFINES%"

Unicode true
Name "Miko-Kagura"
OutFile "{{output_path}}"
InstallDir "$LOCALAPPDATA\Programs\Miko-Kagura"
InstallDirRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\com.kushro.miko-kagura" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${WINDOWS_VERSION}"
VIAddVersionKey "ProductName" "Miko-Kagura"
VIAddVersionKey "FileDescription" "Miko-Kagura Setup"
VIAddVersionKey "FileVersion" "{{version}}"
VIAddVersionKey "ProductVersion" "{{version}}"
VIAddVersionKey "LegalCopyright" "Copyright (c) 2026 Octavio"

!define REGKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\com.kushro.miko-kagura"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_LICENSE "{{license}}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Spanish"
LangString CloseApp ${LANG_ENGLISH} "Close Miko-Kagura before continuing."
LangString CloseApp ${LANG_SPANISH} "Cierra Miko-Kagura antes de continuar."
LangString NeedWebview ${LANG_ENGLISH} "WebView2 could not be installed. Check your connection and try again."
LangString NeedWebview ${LANG_SPANISH} "No se pudo instalar WebView2. Revisa tu red y vuelve a intentarlo."

Function .onInit
    SetShellVarContext current
    ${IfNot} ${RunningX64}
        Abort "This package requires Windows x64."
    ${EndIf}
    SetRegView 64
    !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Function un.onInit
    SetShellVarContext current
    SetRegView 64
FunctionEnd

; Ownership manifests contain only generated relative paths: F|file or D|directory.
; No recursive directory deletion: user-added files and external engines survive.
!macro OwnershipFunctions PREFIX
Function ${PREFIX}RemoveOwned
    Exch $9
    Push $0
    Push $1
    Push $2
    ClearErrors
    FileOpen $0 "$INSTDIR\$9" r
    ${IfNot} ${Errors}
        ${Do}
            ClearErrors
            FileRead $0 $1
            ${If} ${Errors}
                ${ExitDo}
            ${EndIf}
            StrCpy $1 $1 -2
            StrCpy $2 $1 2
            StrCpy $1 $1 "" 2
            ${If} $2 == "F|"
                Delete "$INSTDIR\$1"
            ${ElseIf} $2 == "D|"
                RMDir "$INSTDIR\$1"
            ${EndIf}
        ${Loop}
        FileClose $0
        Delete "$INSTDIR\$9"
    ${EndIf}
    Pop $2
    Pop $1
    Pop $0
    Pop $9
FunctionEnd

Function ${PREFIX}CheckAppClosed
    IfFileExists "$INSTDIR\miko-kagura.exe" 0 done
    ClearErrors
    FileOpen $0 "$INSTDIR\miko-kagura.exe" a
    ${If} ${Errors}
        MessageBox MB_OK|MB_ICONEXCLAMATION "$(CloseApp)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
    FileClose $0
    done:
FunctionEnd
!macroend
!insertmacro OwnershipFunctions ""
!insertmacro OwnershipFunctions "un."

Section "Install"
    Call CheckAppClosed
    ; Only bootstrap WebView2 when it is absent. The Evergreen client key is 32-bit.
    SetRegView 32
    ReadRegStr $1 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
    ${If} $1 == ""
    ${OrIf} $1 == "0.0.0.0"
        ReadRegStr $1 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
    ${EndIf}
    ${If} $1 == ""
    ${OrIf} $1 == "0.0.0.0"
        {{webview_install_code}}
        ${If} $0 != 0
        ${AndIf} $0 != 3010
            MessageBox MB_OK|MB_ICONSTOP "$(NeedWebview)" /SD IDOK
            SetErrorLevel 1
            Abort
        ${EndIf}
    ${EndIf}
    SetRegView 64
    Push ".miko-core-files.txt"
    Call RemoveOwned
    !ifdef FULL_EDITION
        Push ".miko-engines-files.txt"
        Call RemoveOwned
    !endif
    SetOutPath "$INSTDIR"
    File /r "${PAYLOAD}\*"
    File /oname=.miko-core-files.txt "${CORE_FILES}"
    !ifdef FULL_EDITION
        File /oname=.miko-engines-files.txt "${ENGINE_FILES}"
    !endif
    WriteUninstaller "$INSTDIR\uninstall.exe"
    CreateDirectory "$SMPROGRAMS\Miko-Kagura"
    CreateShortcut "$SMPROGRAMS\Miko-Kagura\Miko-Kagura.lnk" "$INSTDIR\miko-kagura.exe"
    CreateShortcut "$DESKTOP\Miko-Kagura.lnk" "$INSTDIR\miko-kagura.exe"
    WriteRegStr HKCU "${REGKEY}" "DisplayName" "Miko-Kagura"
    WriteRegStr HKCU "${REGKEY}" "DisplayVersion" "{{version}}"
    WriteRegStr HKCU "${REGKEY}" "Publisher" "Octavio"
    WriteRegStr HKCU "${REGKEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${REGKEY}" "DisplayIcon" "$INSTDIR\miko-kagura.exe"
    WriteRegStr HKCU "${REGKEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
    WriteRegStr HKCU "${REGKEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
    WriteRegDWORD HKCU "${REGKEY}" "NoModify" 1
    WriteRegDWORD HKCU "${REGKEY}" "NoRepair" 1
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    WriteRegDWORD HKCU "${REGKEY}" "EstimatedSize" $0
SectionEnd

Section "Uninstall"
    Call un.CheckAppClosed
    Push ".miko-core-files.txt"
    Call un.RemoveOwned
    Push ".miko-engines-files.txt"
    Call un.RemoveOwned
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"
    Delete "$SMPROGRAMS\Miko-Kagura\Miko-Kagura.lnk"
    RMDir "$SMPROGRAMS\Miko-Kagura"
    Delete "$DESKTOP\Miko-Kagura.lnk"
    DeleteRegKey HKCU "${REGKEY}"
SectionEnd
