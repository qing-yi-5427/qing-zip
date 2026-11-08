Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!ifndef VERSION
!define VERSION "0.11.5"
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
Section "qing-zip（必需）" Main
 SectionIn RO
 SetOutPath "$INSTDIR"
 File /r "${STAGING}\*"
 WriteUninstaller "$INSTDIR\Uninstall.exe"
 CreateDirectory "$SMPROGRAMS\${PRODUCT_KEY}"
 CreateShortcut "$SMPROGRAMS\${PRODUCT_KEY}\qing-zip.lnk" "$INSTDIR\qing-zip.exe"
 CreateShortcut "$SMPROGRAMS\${PRODUCT_KEY}\卸载.lnk" "$INSTDIR\Uninstall.exe"
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
 CreateShortcut "$DESKTOP\${PRODUCT_KEY}.lnk" "$INSTDIR\qing-zip.exe"
SectionEnd
Section /o "资源管理器右键入口" Shell
 nsExec::ExecToLog '$\"$INSTDIR\obz.exe$\" shell-enable $\"$INSTDIR\qing-zip.exe$\"'
 Pop $0
 ${If} $0 != 0
  MessageBox MB_ICONEXCLAMATION "右键入口未能注册，可稍后在程序设置中重试。"
 ${EndIf}
SectionEnd
Section "Uninstall"
 ReadRegStr $0 HKCU "Software\Classes\*\shell\qing-zip\command" ""
 ${If} $0 == '$\"$INSTDIR\qing-zip.exe$\" $\"%1$\"'
  DeleteRegKey HKCU "Software\Classes\*\shell\qing-zip"
 ${EndIf}
 ReadRegStr $0 HKCU "Software\Classes\Directory\shell\qing-zip-compress\command" ""
 ${If} $0 == '$\"$INSTDIR\qing-zip.exe$\" --compress-folder $\"%1$\"'
  DeleteRegKey HKCU "Software\Classes\Directory\shell\qing-zip-compress"
 ${EndIf}
 ; Delete only shipped files. Archives or other files the user placed here survive.
 !include "uninstall-files.nsh"
 Delete "$INSTDIR\Uninstall.exe"
 RMDir "$INSTDIR"
 Delete "$SMPROGRAMS\${PRODUCT_KEY}\qing-zip.lnk"
 Delete "$SMPROGRAMS\${PRODUCT_KEY}\卸载.lnk"
 RMDir "$SMPROGRAMS\${PRODUCT_KEY}"
 Delete "$DESKTOP\${PRODUCT_KEY}.lnk"
 DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_KEY}"
 DeleteRegKey HKCU "Software\${PRODUCT_KEY}\Installer"
SectionEnd
