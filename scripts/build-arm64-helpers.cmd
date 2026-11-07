@echo off
setlocal
for /f "usebackq tokens=*" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath`) do set "QING_VS=%%i"
if not defined QING_VS exit /b 2
call "%QING_VS%\Common7\Tools\VsDevCmd.bat" -arch=arm64 -host_arch=arm64
if errorlevel 1 exit /b %errorlevel%
python "%~dp0arm64_package_runtime.py" "%~1" "%~2"
exit /b %errorlevel%
