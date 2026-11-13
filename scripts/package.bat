@echo off
rem Build the Windows setup, the portable zip and SHA256SUMS.txt into
rem artifacts\installer. Builds only: nothing is installed or started.
rem
rem   scripts\package.bat                       uses the local NSIS 3.13
rem   scripts\package.bat -MakeNsis C:\nsis\makensis.exe
setlocal
cd /d "%~dp0.."
call "%~dp0_setup.bat" || exit /b 1
pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0package.ps1" %*
if errorlevel 1 (
    echo Packaging failed. See the messages above.
    pause
    exit /b 1
)
echo Packages are in artifacts\installer.
exit /b 0
