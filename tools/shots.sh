#!/usr/bin/env bash
# Scenario screenshots on the test map, one launch per scenario -> shots/<scenario>_N.png
# usage: tools/shots.sh [scenario...]   (default: all)   env: GAME=path/to/game.exe, BODYCAM_ASSETS=...
# Times are sim seconds after the viewmodel is ready (see REWRITE.md "Fast iteration").
cd "$(dirname "$0")/.." || exit 1
GAME=${GAME:-target/debug/game.exe}
export BODYCAM_MAP=test WGPU_BACKEND=${WGPU_BACKEND:-vulkan} # vulkan skips ~10 s of adapter probing here
declare -A S=(
  [idle]="|0.5,1.5"
  [walk]="W@0:3|1,1.3,1.6,2.5"
  [sprint]="W@0:3,Shift@0:3|1,1.3,1.6,2.5"
  [crouch]="Ctrl@0:3|1,2"
  [aim]="RMB@0:3|1,2"
  [hip_burst]="LMB@0.3:0.5|0.4,0.55,0.7,0.9,1.4"
  [ads_burst]="RMB@0:3,LMB@1:0.5|0.9,1.1,1.25,1.4,2"
  [lean]="Q@0:3|0.5,1.5,2.5"
  [lean_r]="E@0:3|0.5,1.5,2.5"
  [reload]="R@0.3|0.6,1.2,2,3,4"
  [swap]="2@0.3|0.5,1,1.5,2.5"
  [swap_back]="2@0.3,1@3.5|3.7,4.2,5"
)
names=("$@"); [ ${#names[@]} -eq 0 ] && names=(idle walk sprint crouch aim hip_burst ads_burst lean lean_r reload swap swap_back)
for n in "${names[@]}"; do
  spec=${S[$n]:?unknown scenario $n}
  rm -f shots/${n}_*.png
  echo "== $n"
  BODYCAM_AUTO="${spec%%|*}" BODYCAM_SHOTS="${spec#*|}" BODYCAM_SCREENSHOT="shots/$n" "$GAME" 2>&1 | grep -i "ready at\|panic\|error"
done
ls shots
