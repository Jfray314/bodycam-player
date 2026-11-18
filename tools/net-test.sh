#!/bin/bash
# local two-instance net test (dev only)
cd "$(dirname "$0")/.."
mkdir -p shots
G=${G:-$PWD/target/debug/game-net.exe}
export BODYCAM_ASSETS=${BODYCAM_ASSETS:-$PWD/assets-local} BODYCAM_MAP=test RUST_LOG=info
BODYCAM_CONFIG=shots/cfg-h.json BODYCAM_HOST=7777 BODYCAM_POS=0:0:${HYAW:-180} BODYCAM_SCREENSHOT=shots/host.png BODYCAM_SCREENSHOT_AT=${HAT:-12} timeout 240 $G > shots/host.log 2>&1 &
sleep 8
BODYCAM_CONFIG=shots/cfg-c.json BODYCAM_JOIN=127.0.0.1:7777 BODYCAM_POS=0:${CZ:--12}:180 BODYCAM_AUTO="${AUTO:-W@2:2,LMB@5:1.2}" BODYCAM_SCREENSHOT=shots/client.png BODYCAM_SCREENSHOT_AT=${CAT:-9} timeout 240 $G > shots/client.log 2>&1 &
wait
