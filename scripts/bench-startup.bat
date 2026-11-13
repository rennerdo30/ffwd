@echo off
rem Measure startup: launch ffwd several times in a hidden, cloaked window
rem (never on screen), wait until history, details and diff are shown, and
rem print one JSON line per run. Release build: the real speed. Timings are
rem taken inside ffwd from its own process start, so cargo adds nothing.
rem
rem   scripts\bench-startup.bat                  this repository, 5 runs
rem   scripts\bench-startup.bat E:\some\repo 10  another repository, 10 runs
setlocal EnableDelayedExpansion
cd /d "%~dp0.."
call "%~dp0_setup.bat" || exit /b 1

set REPO="%CD%"
if not "%~1"=="" set REPO="%~1"
set "RUNS=5"
if not "%~2"=="" set "RUNS=%~2"

echo Building ffwd --release...
cargo build --release -j %JOBS% --quiet -p ffwd || (echo Build failed. & pause & exit /b 1)
if not exist artifacts mkdir artifacts
echo Repository: %REPO%   Runs: %RUNS%
for /l %%i in (1,1,%RUNS%) do (
    rem ffwd exits on its own after the smoke, or after 30 s at the latest.
    cargo run --release -j %JOBS% --quiet -p ffwd -- %REPO% --smoke > "%CD%\artifacts\bench.out" 2> "%CD%\artifacts\bench.err"
    type "%CD%\artifacts\bench.out"
)
echo first_frame_ms = window painted, first_rows_ms = first commits parsed, log_ms = whole history.
exit /b 0
