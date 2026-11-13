@echo off
rem Build and start ffwd from this checkout, to check the current state.
rem
rem   scripts\run.bat                  reopen the tabs of the last session (or this repository)
rem   scripts\run.bat E:\some\repo     open another repository
rem   scripts\run.bat --release        optimized build (slower to build, the real speed)
rem   scripts\run.bat --pull           update to the latest main first (fast-forward only)
rem   scripts\run.bat --trace          print startup and present timings when ffwd closes
rem
rem Other options (for example --help) are passed to ffwd.
setlocal EnableDelayedExpansion
cd /d "%~dp0.."
call "%~dp0_setup.bat" || exit /b 1

set "PULL="
set "RELEASE="
set "REPO="
set "FFWD_ARGS="

:parse
if "%~1"=="" goto parsed
if /i "%~1"=="--pull" (set "PULL=1" & shift & goto parse)
if /i "%~1"=="--release" (set "RELEASE=--release" & shift & goto parse)
if /i "%~1"=="--trace" (set "FFWD_TRACE=1" & shift & goto parse)
set "ARG=%~1"
if "!ARG:~0,1!"=="-" (
    set "FFWD_ARGS=!FFWD_ARGS! %1"
) else (
    set REPO="%~1"
)
shift
goto parse
:parsed

if defined PULL (
    echo Updating to the latest main...
    git pull --ff-only origin main || (echo Pull failed. Fix the checkout first. & exit /b 1)
)
rem First run ever (no saved tabs): open this checkout.
if not defined REPO if not exist "%APPDATA%\ffwd\state.toml" set REPO="%CD%"
if defined FFWD_RUN_DRY_RUN (echo Repo: !REPO! & echo Options: !FFWD_ARGS! & echo Profile: %RELEASE% & exit /b 0)

echo Building ffwd %RELEASE%...
cargo build %RELEASE% -j %JOBS% --quiet -p ffwd || (echo Build failed. & pause & exit /b 1)

cargo run %RELEASE% -j %JOBS% --quiet -p ffwd -- !REPO! !FFWD_ARGS!
if errorlevel 1 (
    echo ffwd exited with an error. See the messages above.
    pause
    exit /b 1
)
exit /b 0
