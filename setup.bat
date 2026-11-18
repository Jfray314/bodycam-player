@echo off
rem One-command setup: builds extracted/ and assets-local/ from your own Bodycam install. Usage: setup.bat [--game "<install dir>"] [--only step,...] [--force]
cd /d "%~dp0"
python tools\setup.py %*
if errorlevel 1 pause
