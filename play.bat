@echo off
rem Double-click to pick a map. Close the game window, then pick again to switch.
cd /d "%~dp0"
:menu
echo.
echo  1  Real lobby / range map
echo  2  Test grounds (F1 lobby, F3 range, F4 course, F5 CQB, F7 bots hostile, F10 respawn bots)
echo  3  Test grounds, start in the range
echo  4  Small flat test map
echo  Q  Quit
choice /c 1234Q /n /m "Pick: "
set BODYCAM_MAP=
set BODYCAM_START=
if errorlevel 5 exit /b
if errorlevel 4 set BODYCAM_MAP=test& goto run
if errorlevel 3 set BODYCAM_MAP=grid& set BODYCAM_START=range& goto run
if errorlevel 2 set BODYCAM_MAP=grid& goto run
:run
cargo run -q -p game
goto menu
