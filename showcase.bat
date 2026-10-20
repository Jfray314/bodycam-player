@echo off
rem Showcase for recording (release build, fullscreen-ish window; record it with OBS / Win+Alt+R).
rem   showcase.bat         hands-off demo on the test grid's shooting range: rifle (hip, ADS, burst, reload, inspect),
rem                        flashlight + laser, pistol, grenade throw + explosion, knife, kamikaze drone flight, hit + death
rem                        (ragdoll, death cut). Grenade slot forced to the frag (BODYCAM_EXPLOSIVE).
rem   showcase.bat play    same range, you drive (F3 range, F4 course, F5 CQB, F7 bots hostile, F10 respawn bots)
rem   showcase.bat map     the real lobby map, you drive
rem Timeline seconds count from "viewmodel ready"; tweak SHOW below to re-cut it.
setlocal
cd /d "%~dp0"
set SHOW=LookL@3:1.2,W@4:2,LookR@4.4:1.2,RMB@7:4,LMB@7.8,LMB@8.3,LMB@8.8,LMB@9.4:1.2,R@12,J@16,C@20,Z@21,C@23,Z@23.2,2@25,RMB@27:2.6,LMB@27.7,LMB@28.3,LMB@28.9,R@30,3@33,LookL@34.2:0.7,RMB@35.5:2.6,LMB@37.3,LookR@41.5:0.7,4@44,LMB@46,LMB@47.2,5@50,LMB@52,W@55:2.5,LookR@57.5:0.6,F@61,1@64,Hit@67,Kill@69

if /i "%1"=="map" (
  set BODYCAM_MAP=
) else (
  set BODYCAM_MAP=grid
  set BODYCAM_START=range
)
if "%1"=="" (
  set BODYCAM_AUTO=%SHOW%
  set "BODYCAM_EXPLOSIVE=Grenade G67"
)
cargo run --release -p game
