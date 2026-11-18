#!/bin/sh
# usage: run.sh <outDir> <cmd> [args]   (dev helper; BODYCAM_GAME / BODYCAM_EXTRACTED override the defaults)
export PATH=$HOME/.dotnet-sdk:$PATH
R=$(cd "$(dirname "$0")/../.." && pwd)
D=$R/tools/ue-export/bin/Release/net10.0
O=$1; shift
cd "$D" && dotnet ue-export.dll "${BODYCAM_GAME:-C:/Program Files (x86)/Steam/steamapps/common/Bodycam}/Bodycam/Content/Paks" "${BODYCAM_EXTRACTED:-$R/extracted}/Bodycam.usmap" "$O" "$@" 2>&1 | grep -v "^\[Warning\]"
