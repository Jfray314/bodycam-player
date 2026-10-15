#!/bin/bash
# tablet PLAY > HOST SERVERS flow: instance A clicks HOST, instance B clicks JOIN (dev only)
cd "$(dirname "$0")/.."
mkdir -p shots
G=C:/code/bodycam-player/target/debug/game-net.exe
export BODYCAM_ASSETS=C:/code/bodycam-player/assets-local BODYCAM_MAP=test RUST_LOG=info
NAV="Tab@1:0.1,Space@4:0.1,E@5.5:0.1,E@6.5:0.1"
BODYCAM_CONFIG=shots/cfg-h.json BODYCAM_AUTO=$NAV BODYCAM_CLICKS="9:${HX:-240},${HY:-350}" BODYCAM_SCREENSHOT=shots/uih.png BODYCAM_SCREENSHOT_AT=28 timeout 240 $G > shots/host.log 2>&1 &
sleep 8
BODYCAM_CONFIG=shots/cfg-c.json BODYCAM_AUTO=$NAV BODYCAM_CLICKS="9:${JX:-1000},${JY:-350}" BODYCAM_SCREENSHOT=shots/uic.png BODYCAM_SCREENSHOT_AT=14 timeout 240 $G > shots/client.log 2>&1 &
wait
