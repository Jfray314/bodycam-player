#!/bin/sh
# usage: run.sh <outDir> <cmd> [args]   (dev helper)
export PATH=$HOME/.dotnet-sdk:$PATH
D=$(dirname "$0")/bin/Release/net10.0
O=$1; shift
cd "$D" && dotnet ue-export.dll "F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks" F:/code/bodycam-player/extracted/Bodycam.usmap "$O" "$@" 2>&1 | grep -v "^\[Warning\]"
