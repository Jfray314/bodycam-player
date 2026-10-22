@echo off
rem Double-click to play: opens straight into the lobby. Switch maps from the tablet (PLAY).
cd /d "%~dp0"
set BODYCAM_MAP=
set BODYCAM_START=
set BODYCAM_MODE=
echo Checking for code changes (only rebuilds if something changed)...
cargo build --release -q -p game
if errorlevel 1 (
    echo Build failed, see above.
    pause
    exit /b 1
)
echo Starting the game...
target\release\game.exe
if errorlevel 1 pause
