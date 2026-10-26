Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!ifndef VERSION
!define VERSION "0.10.3"
!endif
!ifndef STAGING
!define STAGING "..\dist\qing-zip-${VERSION}-portable"
!endif
Name "qing-zip"
OutFile "..\dist\qing-zip-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\qing-zip"
InstallDirRegKey HKCU "Software\qing-zip\Installer" "InstallDir"
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
 ${IfNot} ${RunningX64}
  MessageBox MB_ICONSTOP "qing-zip 需要 64 位 Windows。"
  Abort
 ${EndIf}
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
 CreateDirectory "$SMPROGRAMS\qing-zip"
 CreateShortcut "$SMPROGRAMS\qing-zip\qing-zip.lnk" "$INSTDIR\qing-zip.exe"
 CreateShortcut "$SMPROGRAMS\qing-zip\卸载.lnk" "$INSTDIR\Uninstall.exe"
 WriteRegStr HKCU "Software\qing-zip\Installer" "InstallDir" "$INSTDIR"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "DisplayName" "qing-zip"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "DisplayVersion" "${VERSION}"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "DisplayIcon" "$INSTDIR\qing-zip.exe,0"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "InstallLocation" "$INSTDIR"
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
 WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "URLInfoAbout" "https://github.com/qing-yi-5427/qing-zip"
 WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "NoModify" 1
 WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip" "NoRepair" 1
SectionEnd
Section /o "桌面快捷方式" Desktop
 CreateShortcut "$DESKTOP\qing-zip.lnk" "$INSTDIR\qing-zip.exe"
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
 Delete "$SMPROGRAMS\qing-zip\qing-zip.lnk"
 Delete "$SMPROGRAMS\qing-zip\卸载.lnk"
 RMDir "$SMPROGRAMS\qing-zip"
 Delete "$DESKTOP\qing-zip.lnk"
 DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip"
 DeleteRegKey HKCU "Software\qing-zip\Installer"
SectionEnd
