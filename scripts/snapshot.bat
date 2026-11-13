@echo off
rem Render the real ffwd UI offscreen to a PNG and open it. No window opens.
rem
rem   scripts\snapshot.bat                         this repository, artifacts\snapshot.png
rem   scripts\snapshot.bat E:\some\repo            another repository
rem   scripts\snapshot.bat E:\some\repo --select 5 --focus diff
rem   scripts\snapshot.bat E:\some\repo --palette main --size 1920x1080 --scale 1.5
rem
rem Options after the repository are passed to ffwd --snapshot.
setlocal EnableDelayedExpansion
cd /d "%~dp0.."
call "%~dp0_setup.bat" || exit /b 1

set REPO="%CD%"
set "ARG=%~1"
if not "%~1"=="" if not "!ARG:~0,1!"=="-" (
    set REPO="%~1"
    shift
)
set "REST="
:rest
if "%~1"=="" goto go
set "REST=!REST! %1"
shift
goto rest
:go

if not exist artifacts mkdir artifacts
set "OUT=%CD%\artifacts\snapshot.png"
echo Building ffwd --release...
cargo build --release -j %JOBS% --quiet -p ffwd || (echo Build failed. & pause & exit /b 1)
rem cargo waits for ffwd, so the PNG is complete when this line returns.
cargo run --release -j %JOBS% --quiet -p ffwd -- !REPO! --snapshot "%OUT%" !REST! 2> "%CD%\artifacts\snapshot.log"
set "CODE=%ERRORLEVEL%"
type "%CD%\artifacts\snapshot.log"
if not "%CODE%"=="0" (
    echo Snapshot failed.
    pause
    exit /b 1
)
echo Saved %OUT%
rem FFWD_NO_OPEN=1 skips opening the image (for agent runs).
if not defined FFWD_NO_OPEN start "" "%OUT%"
exit /b 0
