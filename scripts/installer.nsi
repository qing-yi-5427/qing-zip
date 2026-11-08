Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!ifndef VERSION
!define VERSION "0.11.4"
!endif
!ifndef STAGING
!define STAGING "..\dist\qing-zip-${VERSION}-portable"
!endif
!ifdef ARM64_PAYLOAD
!define PRODUCT_KEY "qing-zip-arm64"
!define PRODUCT_LABEL "qing-zip (ARM64)"
!else
!define PRODUCT_KEY "qing-zip"
!define PRODUCT_LABEL "qing-zip"
!endif
Name "${PRODUCT_LABEL}"
!ifndef OUTPUT
!define OUTPUT "..\dist\qing-zip-${VERSION}-setup.exe"
!endif
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT_KEY}"
InstallDirRegKey HKCU "Software\${PRODUCT_KEY}\Installer" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
Icon "..\obz-gui\icons\icon.ico"
UninstallIcon "..\obz-gui\icons\icon.ico"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "qing-zip"
VIAddVersionKey "FileDescription" "qing-zip 安装程序"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "qing-zip contributors"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\qing-zip.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
Function .onInit
!ifdef ARM64_PAYLOAD
 ${IfNot} ${IsNativeARM64}
  SetErrorLevel 1633
  IfSilent +2
  MessageBox MB_ICONSTOP "This build requires Windows on ARM64."
  Abort
 ${EndIf}
!else
 ${IfNot} ${RunningX64}
  MessageBox MB_ICONSTOP "qing-zip 需要 64 位 Windows。"
  Abort
 ${EndIf}
!endif
 SetShellVarContext current
 SetRegView 64
FunctionEnd
Function un.onInit
 SetShellVarContext current
 SetRegView 64
FunctionEnd
!macro InstallShortcut SLOT TARGET
 InitPluginsDir
 CreateShortCut "$PLUGINSDIR\${SLOT}.lnk" "${TARGET}"
 ExecWait '$\"$INSTDIR\qing-zip.exe$\" --install-owned-shortcut ${SLOT} $\"$PLUGINSDIR\${SLOT}.lnk$\"' $0
 ${If} ${Errors}
 ${OrIf} $0 != 0
  DetailPrint "无法创建快捷方式：${SLOT}"
 ${EndIf}
!macroend
Section "qing-zip（必需）" Main
 SectionIn RO
 SetOutPath "$INSTDIR"
 File /r "${STAGING}\*"
 WriteUninstaller "$INSTDIR\Uninstall.exe"
 CreateDirectory "$SMPROGRAMS\${PRODUCT_KEY}"
 !insertmacro InstallShortcut start "$INSTDIR\qing-zip.exe"
 !insertmacro InstallShortcut uninstall "$INSTDIR\Uninstall.exe"
 WriteRegStr HKCU "Software\${PRODUCT_KEY}\Installer" "InstallDir" "$INSTDIR"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "DisplayName" "${PRODUCT_LABEL}"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "DisplayVersion" "${VERSION}"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "DisplayIcon" "$INSTDIR\qing-zip.exe,0"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "InstallLocation" "$INSTDIR"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "URLInfoAbout" "https://github.com/qing-yi-5427/qing-zip"
 WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "NoModify" 1
 WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "NoRepair" 1
SectionEnd
Section /o "桌面快捷方式" Desktop
 !insertmacro InstallShortcut desktop "$INSTDIR\qing-zip.exe"
SectionEnd
Section /o "资源管理器右键入口" Shell
 nsExec::ExecToLog '$\"$INSTDIR\obz.exe$\" shell-enable $\"$INSTDIR\qing-zip.exe$\"'
 Pop $0
 ${If} $0 != 0
  MessageBox MB_ICONEXCLAMATION "右键入口未能注册，可稍后在程序设置中重试。"
 ${EndIf}
SectionEnd
Section "Uninstall"
 ; The desktop binary derives its own path and atomically removes matching candidates.
 ReadRegStr $0 HKCU "Software\OpenArchive\Associations" "ManagedBy"
 ReadRegStr $1 HKCU "Software\Classes\OpenArchive.Archive\shell\open\command" ""
 ${If} $0 == "OpenArchive.Associations.v1"
 ${AndIf} $1 == '$\"$INSTDIR\qing-zip.exe$\" $\"%1$\"'
  ClearErrors
  ExecWait '$\"$INSTDIR\qing-zip.exe$\" --unregister-owned-associations' $0
  ${If} ${Errors}
  ${OrIf} $0 != 0
   SetErrorLevel 1
   IfSilent +2
   MessageBox MB_ICONSTOP "无法清理本安装的文件关联。请关闭相关程序，必要时重新安装后再卸载。"
   Abort
  ${EndIf}
 ${EndIf}
 ReadRegStr $0 HKCU "Software\Classes\*\shell\qing-zip\command" ""
 ${If} $0 == '$\"$INSTDIR\qing-zip.exe$\" $\"%1$\"'
  DeleteRegKey HKCU "Software\Classes\*\shell\qing-zip"
 ${EndIf}
 ReadRegStr $0 HKCU "Software\Classes\Directory\shell\qing-zip-compress\command" ""
 ${If} $0 == '$\"$INSTDIR\qing-zip.exe$\" --compress-folder $\"%1$\"'
  DeleteRegKey HKCU "Software\Classes\Directory\shell\qing-zip-compress"
 ${EndIf}
 IfFileExists "$INSTDIR\qing-zip.exe" 0 shortcuts_done
 ClearErrors
 ExecWait '$\"$INSTDIR\qing-zip.exe$\" --uninstall-owned-shortcuts' $0
 ${If} ${Errors}
 ${OrIf} $0 != 0
  SetErrorLevel 1
  IfSilent +2
  MessageBox MB_ICONSTOP "无法核对快捷方式。请关闭占用它的程序后重试。"
  Abort
 ${EndIf}
 shortcuts_done:
 ; Delete only shipped files. Archives or other files the user placed here survive.
 !include "uninstall-files.nsh"
 Delete "$INSTDIR\Uninstall.exe"
 RMDir "$INSTDIR"
 RMDir "$SMPROGRAMS\${PRODUCT_KEY}"
 ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}" "InstallLocation"
 ${If} $0 == "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}"
 ${EndIf}
 ReadRegStr $0 HKCU "Software\${PRODUCT_KEY}\Installer" "InstallDir"
 ${If} $0 == "$INSTDIR"
  DeleteRegKey HKCU "Software\${PRODUCT_KEY}\Installer"
 ${EndIf}
SectionEnd
